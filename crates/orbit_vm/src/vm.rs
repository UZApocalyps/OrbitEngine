//! The virtual machine.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use orbit_core::{NodeId, NodeKind, Story, StringId, Transition, Value, VarScope};

use crate::error::VmError;
use crate::event::{ChoiceView, PresentationEvent, StepResult};
use crate::history::History;
use crate::state::{CharacterOnStage, Interaction, MusicState, Pending, Rng, VmState};

/// Maximum number of nodes executed by one `step()` call without reaching a
/// blocking node. Guards against `Jump` loops with no `Say`/`Wait`.
pub const MAX_NODES_PER_STEP: usize = 10_000;
/// Maximum depth of nested `Call`s.
pub const MAX_CALL_DEPTH: usize = 256;

const DEFAULT_SEED: u64 = 0x0B17_0B17_0B17_0B17;

/// A deterministic, non-blocking story interpreter.
///
/// * No I/O, no clock, no threads: time and input come from the host through
///   `step()` / `choose()`.
/// * The game state ([`VmState`]) only changes through this API.
/// * Non-blocking commands are emitted as [`PresentationEvent`]s *and* applied
///   to the declarative `SceneState` inside the state.
///
/// Host loop sketch:
///
/// ```ignore
/// loop {
///     match vm.step() {
///         StepResult::Say { string_id, .. } => { show(vm.text(string_id)); wait_for_click(); }
///         StepResult::Choice { options, .. } => { let i = ask(&options); vm.choose(i)?; }
///         StepResult::WaitTime(s) => wait(s),
///         StepResult::WaitTransition { .. } => wait_for_transition(),
///         StepResult::End => break,
///         StepResult::Error(e) => return Err(e),
///     }
///     for event in vm.take_events() { present(event); }
/// }
/// ```
///
/// `snapshot()` / `restore()` copy the raw state. Player-facing rollback and
/// the dialogue backlog are built in (see `rollback`, `roll_forward`,
/// `backlog`): a checkpoint is taken at every `Say` / `Choice`, and a
/// rollback returns a [`crate::Presentation`] the host rebuilds the screen
/// from, without re-running nodes.
///
/// Variables declared `VarScope::Persistent` live in a separate store
/// (`persistent_vars` / `set_persistent_vars`) that the host loads and saves
/// on its own: they are not part of [`VmState`] and are never rolled back.
pub struct Vm {
    pub(crate) story: Arc<Story>,
    /// node id -> (scene index, node index)
    pub(crate) index: HashMap<NodeId, (usize, usize)>,
    pub(crate) state: VmState,
    read: BTreeSet<NodeId>,
    pub(crate) events: Vec<PresentationEvent>,
    pub(crate) fault: Option<VmError>,
    persistent: BTreeMap<String, Value>,
    persistent_names: HashSet<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) history: History,
}

impl Vm {
    /// Starts executing `story` at node `entry` (usually a `Label`).
    pub fn new(story: impl Into<Arc<Story>>, entry: NodeId) -> Result<Self, VmError> {
        Self::with_seed(story, entry, DEFAULT_SEED)
    }

    /// Like [`Vm::new`] with an explicit RNG seed.
    pub fn with_seed(
        story: impl Into<Arc<Story>>,
        entry: NodeId,
        seed: u64,
    ) -> Result<Self, VmError> {
        let story = story.into();
        let index = build_index(&story);
        if !index.contains_key(&entry) {
            return Err(VmError::UnknownNode(entry));
        }
        let vars = story
            .variables
            .iter()
            .filter(|v| v.scope == VarScope::Game)
            .map(|v| (v.name.clone(), v.initial.clone()))
            .collect();
        let persistent: BTreeMap<String, Value> = story
            .persistent_variables()
            .map(|v| (v.name.clone(), v.initial.clone()))
            .collect();
        let persistent_names = persistent.keys().cloned().collect();
        Ok(Self {
            story,
            index,
            state: VmState {
                pc: Some(entry),
                pending: Pending::None,
                call_stack: Vec::new(),
                vars,
                scene: Default::default(),
                rng: Rng::new(seed),
                shown: None,
            },
            read: BTreeSet::new(),
            events: Vec::new(),
            fault: None,
            persistent,
            persistent_names,
            warnings: Vec::new(),
            history: History::default(),
        })
    }

    /// Starts at the first node of the story.
    pub fn from_start(story: impl Into<Arc<Story>>) -> Result<Self, VmError> {
        let story = story.into();
        let entry = story.entry().ok_or(VmError::EmptyStory)?;
        Self::new(story, entry)
    }

    // ------------------------------------------------------------ execution

    /// Runs until the next blocking point. Never blocks, never sleeps.
    pub fn step(&mut self) -> StepResult {
        if let Some(err) = &self.fault {
            return StepResult::Error(err.clone());
        }
        if let Pending::Choice { node, visible } = &self.state.pending {
            return self.choice_result(*node, visible);
        }
        self.history.ahead = true;
        match self.run() {
            Ok(result) => {
                if matches!(result, StepResult::Say { .. } | StepResult::Choice { .. }) {
                    self.history.record(&self.state);
                }
                result
            }
            Err(err) => {
                self.fault = Some(err.clone());
                StepResult::Error(err)
            }
        }
    }

    /// Like [`Vm::step`], but stops *before* executing `target`: returns
    /// `None` as soon as the next node to execute is `target` (the state is
    /// then exactly what a player would have right before that node runs).
    /// Any other result is the one `step()` would give. Used to reach an
    /// arbitrary node ("play from here"), see [`crate::seek`].
    pub fn step_to(&mut self, target: NodeId) -> Option<StepResult> {
        if let Some(err) = &self.fault {
            return Some(StepResult::Error(err.clone()));
        }
        if let Pending::Choice { node, visible } = &self.state.pending {
            return Some(self.choice_result(*node, visible));
        }
        self.history.ahead = true;
        match self.run_until(Some(target)) {
            Ok(None) => None,
            Ok(Some(result)) => {
                if matches!(result, StepResult::Say { .. } | StepResult::Choice { .. }) {
                    self.history.record(&self.state);
                }
                Some(result)
            }
            Err(err) => {
                self.fault = Some(err.clone());
                Some(StepResult::Error(err))
            }
        }
    }

    /// Answers the pending `Choice` with an index into the options returned
    /// by `step()`. Invalid calls leave the VM untouched.
    pub fn choose(&mut self, index: usize) -> Result<(), VmError> {
        let Pending::Choice { node, visible } = &self.state.pending else {
            return Err(VmError::NotAwaitingChoice);
        };
        let Some(&option) = visible.get(index) else {
            return Err(VmError::InvalidChoice {
                index,
                available: visible.len(),
            });
        };
        let node = *node;
        let NodeKind::Choice { options } = &self.node(node)?.kind else {
            return Err(VmError::UnknownNode(node));
        };
        let target = options[option].target;
        if !self.index.contains_key(&target) {
            return Err(VmError::UnknownNode(target));
        }
        self.history.on_choose(node, index);
        self.state.pending = Pending::None;
        self.state.pc = Some(target);
        Ok(())
    }

    fn node(&self, id: NodeId) -> Result<&orbit_core::Node, VmError> {
        let &(s, n) = self.index.get(&id).ok_or(VmError::UnknownNode(id))?;
        Ok(&self.story.scenes[s].nodes[n])
    }

    /// The node executed after `id` when it does not jump: the next node of
    /// its scene. A scene never falls into the next one: after its last node
    /// the story ends (`None`), unless a `JumpScene` leaves it.
    fn next_after(&self, id: NodeId) -> Option<NodeId> {
        let &(s, n) = self.index.get(&id)?;
        self.story.scenes[s].nodes.get(n + 1).map(|next| next.id)
    }

    fn eval(&self, node: NodeId, expr: &orbit_core::Expr) -> Result<Value, VmError> {
        expr.eval(&|name| self.variable(name).cloned())
            .map_err(|source| VmError::Eval { node, source })
    }

    fn eval_bool(&self, node: NodeId, expr: &orbit_core::Expr) -> Result<bool, VmError> {
        match self.eval(node, expr)? {
            Value::Bool(b) => Ok(b),
            _ => Err(VmError::ConditionNotBool(node)),
        }
    }

    fn choice_result(&self, node: NodeId, visible: &[usize]) -> StepResult {
        let Ok(orbit_core::Node {
            kind: NodeKind::Choice { options },
            ..
        }) = self.node(node)
        else {
            return StepResult::Error(VmError::UnknownNode(node));
        };
        StepResult::Choice {
            node,
            options: visible
                .iter()
                .map(|&i| ChoiceView {
                    string_id: options[i].text_id,
                })
                .collect(),
        }
    }

    fn jump(&mut self, target: NodeId) -> Result<(), VmError> {
        if !self.index.contains_key(&target) {
            return Err(VmError::UnknownNode(target));
        }
        self.state.pc = Some(target);
        Ok(())
    }

    fn run(&mut self) -> Result<StepResult, VmError> {
        Ok(self.run_until(None)?.unwrap_or(StepResult::End))
    }

    /// Executes nodes until a blocking point, or (`None`) until the next node
    /// to execute is `stop`.
    fn run_until(&mut self, stop: Option<NodeId>) -> Result<Option<StepResult>, VmError> {
        let story = Arc::clone(&self.story);
        for _ in 0..MAX_NODES_PER_STEP {
            let Some(pc) = self.state.pc else {
                return Ok(Some(StepResult::End));
            };
            if stop == Some(pc) {
                return Ok(None);
            }
            let &(si, ni) = self.index.get(&pc).ok_or(VmError::UnknownNode(pc))?;
            let node = &story.scenes[si].nodes[ni];
            let next = self.next_after(pc);

            match &node.kind {
                NodeKind::Label { .. } => self.state.pc = next,
                NodeKind::Say { speaker, text_id } => {
                    self.state.pc = next;
                    self.state.shown = Some(Interaction::Say {
                        node: pc,
                        speaker: *speaker,
                        string_id: *text_id,
                    });
                    let already_read = !self.read.insert(pc);
                    return Ok(Some(StepResult::Say {
                        node: pc,
                        speaker: *speaker,
                        string_id: *text_id,
                        already_read,
                    }));
                }
                NodeKind::Choice { options } => {
                    let mut visible = Vec::with_capacity(options.len());
                    for (i, option) in options.iter().enumerate() {
                        let shown = match &option.condition {
                            Some(cond) => self.eval_bool(pc, cond)?,
                            None => true,
                        };
                        if shown {
                            visible.push(i);
                        }
                    }
                    if visible.is_empty() {
                        return Err(VmError::NoVisibleChoice(pc));
                    }
                    let result = self.choice_result(pc, &visible);
                    if let StepResult::Choice { options, .. } = &result {
                        self.state.shown = Some(Interaction::Choice {
                            node: pc,
                            options: options.clone(),
                        });
                    }
                    self.state.pending = Pending::Choice { node: pc, visible };
                    return Ok(Some(result));
                }
                NodeKind::Jump { target } => self.jump(*target)?,
                NodeKind::JumpScene { scene, label } => {
                    if story.scene(*scene).is_none() {
                        return Err(VmError::UnknownScene(*scene));
                    }
                    match label {
                        Some(l) => self.jump(*l)?,
                        // An empty scene has nothing to run: the story ends.
                        None => self.state.pc = story.scene_entry(*scene, None),
                    }
                }
                NodeKind::SetVar { var, value } => {
                    match self.eval(pc, value) {
                        Ok(v) => {
                            // Keep the declared type (a float result stored
                            // in an int variable is rounded).
                            let v = match self.variable(var) {
                                Some(cur) => {
                                    orbit_core::coerce(v.clone(), cur.value_type()).unwrap_or(v)
                                }
                                None => v,
                            };
                            self.store_var(var, v);
                        }
                        Err(VmError::Eval {
                            source: orbit_core::EvalError::DivisionByZero,
                            ..
                        }) => self
                            .warnings
                            .push(format!("node {pc}: division by zero, `{var}` unchanged")),
                        Err(e) => return Err(e),
                    }
                    self.state.pc = next;
                }
                NodeKind::If {
                    condition,
                    then_target,
                    else_target,
                } => {
                    if self.eval_bool(pc, condition)? {
                        self.jump(*then_target)?;
                    } else if let Some(e) = else_target {
                        self.jump(*e)?;
                    } else {
                        self.state.pc = next;
                    }
                }
                NodeKind::Background { asset, transition } => {
                    self.state.pc = next;
                    self.state.scene.background = Some(*asset);
                    self.events.push(PresentationEvent::Background {
                        asset: *asset,
                        transition: *transition,
                    });
                    if let Some(duration) = blocking_duration(transition) {
                        return Ok(Some(StepResult::WaitTransition { duration }));
                    }
                }
                NodeKind::Show {
                    character,
                    expression,
                    position,
                    transition,
                    scale,
                    scale_y,
                    flip_x,
                    flip_y,
                } => {
                    let def = story
                        .character(*character)
                        .ok_or(VmError::UnknownCharacter {
                            node: pc,
                            character: *character,
                        })?;
                    let image = def
                        .expression(expression)
                        .ok_or_else(|| VmError::UnknownExpression {
                            node: pc,
                            character: *character,
                            expression: expression.clone(),
                        })?
                        .image;
                    let (x, y) = story
                        .resolve_position(position)
                        .ok_or(VmError::UnknownPosition { node: pc })?;
                    self.state.pc = next;
                    // A per-show override wins over the character's default;
                    // an absent vertical scale follows the horizontal one.
                    let scale_x = scale.unwrap_or(def.default_scale);
                    let scale = [scale_x, scale_y.unwrap_or(scale_x)];
                    let entry = CharacterOnStage {
                        character: *character,
                        expression: expression.clone(),
                        position: position.clone(),
                        scale,
                        flip_x: *flip_x,
                        flip_y: *flip_y,
                        source: Some(pc),
                    };
                    match self
                        .state
                        .scene
                        .characters
                        .iter_mut()
                        .find(|c| c.character == *character)
                    {
                        Some(existing) => *existing = entry,
                        None => self.state.scene.characters.push(entry),
                    }
                    self.events.push(PresentationEvent::Show {
                        character: *character,
                        expression: expression.clone(),
                        image,
                        position: position.clone(),
                        x,
                        y,
                        scale,
                        flip_x: *flip_x,
                        flip_y: *flip_y,
                        transition: *transition,
                    });
                    if let Some(duration) = blocking_duration(transition) {
                        return Ok(Some(StepResult::WaitTransition { duration }));
                    }
                }
                NodeKind::Hide {
                    character,
                    transition,
                } => {
                    self.state.pc = next;
                    let before = self.state.scene.characters.len();
                    self.state
                        .scene
                        .characters
                        .retain(|c| c.character != *character);
                    // Hiding a character that is not on stage is a no-op.
                    if self.state.scene.characters.len() != before {
                        self.events.push(PresentationEvent::Hide {
                            character: *character,
                            transition: *transition,
                        });
                        if let Some(duration) = blocking_duration(transition) {
                            return Ok(Some(StepResult::WaitTransition { duration }));
                        }
                    }
                }
                NodeKind::PlayMusic {
                    asset,
                    fade_in,
                    looping,
                } => {
                    self.state.pc = next;
                    self.state.scene.music = Some(MusicState {
                        asset: *asset,
                        looping: *looping,
                    });
                    self.events.push(PresentationEvent::PlayMusic {
                        asset: *asset,
                        fade_in: *fade_in,
                        looping: *looping,
                    });
                }
                NodeKind::StopMusic { fade_out } => {
                    self.state.pc = next;
                    self.state.scene.music = None;
                    self.events.push(PresentationEvent::StopMusic {
                        fade_out: *fade_out,
                    });
                }
                NodeKind::PlaySound { asset } => {
                    self.state.pc = next;
                    self.events
                        .push(PresentationEvent::PlaySound { asset: *asset });
                }
                NodeKind::Wait { seconds } => {
                    self.state.pc = next;
                    return Ok(Some(StepResult::WaitTime(*seconds)));
                }
                NodeKind::Call { label } => {
                    if self.state.call_stack.len() >= MAX_CALL_DEPTH {
                        return Err(VmError::StackOverflow(MAX_CALL_DEPTH));
                    }
                    self.jump(*label)?;
                    self.state.call_stack.push(next);
                }
                NodeKind::Return => {
                    let back = self
                        .state
                        .call_stack
                        .pop()
                        .ok_or(VmError::ReturnWithoutCall(pc))?;
                    self.state.pc = back;
                }
                NodeKind::End => self.state.pc = None,
                NodeKind::Custom {
                    plugin_id,
                    kind,
                    data,
                    ..
                } => {
                    self.state.pc = next;
                    self.events.push(PresentationEvent::Custom {
                        plugin_id: plugin_id.clone(),
                        kind: kind.clone(),
                        data: data.clone(),
                    });
                }
            }
        }
        Err(VmError::StepLimit(MAX_NODES_PER_STEP))
    }

    // -------------------------------------------------------- observation

    pub fn story(&self) -> &Arc<Story> {
        &self.story
    }

    /// Source-language text of a string id (`None` if the id is unknown).
    pub fn text(&self, id: StringId) -> Option<&str> {
        self.story.text(id)
    }

    /// Display name of a character in the source language.
    pub fn character_name(&self, id: orbit_core::CharacterId) -> Option<&str> {
        self.story.text(self.story.character(id)?.name_id)
    }

    /// Declarative scene state: background, characters on stage, music.
    pub fn scene(&self) -> &crate::state::SceneState {
        &self.state.scene
    }

    /// Value of a variable of either scope.
    pub fn variable(&self, name: &str) -> Option<&Value> {
        self.state
            .vars
            .get(name)
            .or_else(|| self.persistent.get(name))
    }

    fn store_var(&mut self, name: &str, value: Value) {
        if self.persistent_names.contains(name) {
            self.persistent.insert(name.to_owned(), value);
        } else {
            self.state.vars.insert(name.to_owned(), value);
        }
    }

    /// Applies a no-code [`VarOp`] from outside a node (a UI button, a
    /// host). The variable keeps its declared type; on error (unknown
    /// variable, type mismatch, division by zero) nothing changes and a
    /// warning is recorded (see [`Vm::take_warnings`]).
    ///
    /// Playthrough variables are part of the saved state and of the rollback
    /// history: the checkpoint of the screen the player is looking at is
    /// updated too, so rolling back to it keeps the change, and a known
    /// future that no longer follows from it is cut (like picking another
    /// choice). Persistent variables are not rolled back; the host saves
    /// them (`persistent_vars`).
    pub fn apply_var_op(&mut self, op: &orbit_core::VarOp) -> Result<(), orbit_core::OpError> {
        let result = op.apply(&|name| self.variable(name).cloned());
        let value = match result {
            Ok(v) => v,
            Err(e) => {
                self.warnings
                    .push(format!("variable operation on `{}` ignored: {e}", op.var));
                return Err(e);
            }
        };
        if self.variable(&op.var) == Some(&value) {
            return Ok(());
        }
        if !self.persistent_names.contains(&op.var) {
            self.history.patch_cursor_var(&op.var, &value);
        }
        self.store_var(&op.var, value);
        Ok(())
    }

    /// Evaluates a no-code condition against the current variables. An
    /// evaluation error (unknown variable, type mismatch) counts as `false`
    /// and records a warning.
    pub fn eval_condition(&mut self, cond: &orbit_core::Condition) -> bool {
        match cond.eval(&|name| self.variable(name).cloned()) {
            Ok(b) => b,
            Err(e) => {
                self.warnings.push(format!("condition ignored: {e}"));
                false
            }
        }
    }

    /// Non-fatal problems recorded since the last call (division by zero,
    /// rejected variable operations).
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    /// Playthrough (`Game`) variables only, as stored in the state.
    pub fn variables(&self) -> &BTreeMap<String, Value> {
        &self.state.vars
    }

    /// The persistent variables (`VarScope::Persistent`), including their
    /// initial values for those never assigned. The host saves this map.
    pub fn persistent_vars(&self) -> &BTreeMap<String, Value> {
        &self.persistent
    }

    /// Loads persistent variables from the host's store. Only declared
    /// persistent variables whose value has the declared type are taken;
    /// anything else (removed or retyped variable) keeps its initial value.
    pub fn set_persistent_vars(&mut self, values: &BTreeMap<String, Value>) {
        for decl in self.story.persistent_variables() {
            let value = values
                .get(&decl.name)
                .filter(|v| v.value_type() == decl.initial.value_type())
                .unwrap_or(&decl.initial);
            self.persistent.insert(decl.name.clone(), value.clone());
        }
    }

    /// Next node to execute (`None` once finished).
    pub fn position(&self) -> Option<NodeId> {
        self.state.pc
    }

    pub fn call_depth(&self) -> usize {
        self.state.call_stack.len()
    }

    /// `true` once the story has ended (`End` node or fell off the last node).
    pub fn is_finished(&self) -> bool {
        self.state.pc.is_none() && self.state.pending == Pending::None
    }

    /// Removes and returns the presentation events emitted since the last
    /// call, in execution order.
    pub fn take_events(&mut self) -> Vec<PresentationEvent> {
        std::mem::take(&mut self.events)
    }

    /// Draws a number from the seeded RNG stored in the state. Not used by
    /// the story language yet; this is the hook scripting (M8) will use.
    pub fn next_random(&mut self) -> u64 {
        self.state.rng.next_u64()
    }

    // ------------------------------------------------------ snapshot / read

    /// Copies the complete VM state.
    pub fn snapshot(&self) -> VmState {
        self.state.clone()
    }

    /// Checks that `state` only references nodes (and options) that exist in
    /// this story.
    pub fn validate_state(&self, state: &VmState) -> Result<(), VmError> {
        let known = |id: &NodeId| self.index.contains_key(id);
        let mut referenced: Vec<NodeId> = state.pc.into_iter().collect();
        referenced.extend(state.call_stack.iter().flatten().copied());
        if let Some(shown) = &state.shown {
            referenced.push(shown.node());
        }
        if let Pending::Choice { node, visible } = &state.pending {
            referenced.push(*node);
            let ok = matches!(
                self.node(*node).map(|n| &n.kind),
                Ok(NodeKind::Choice { options }) if visible.iter().all(|&i| i < options.len())
            );
            if !ok {
                return Err(VmError::UnknownNode(*node));
            }
        }
        if let Some(missing) = referenced.iter().find(|id| !known(id)) {
            return Err(VmError::UnknownNode(*missing));
        }
        Ok(())
    }

    /// Makes a state taken from a possibly older version of the story
    /// consistent with it: declared playthrough variables that are missing or
    /// of another type get their initial value, persistent ones are removed,
    /// and stage elements whose asset or character vanished are dropped.
    /// A state from the same story is left unchanged.
    pub(crate) fn reconcile(&self, state: &mut VmState) {
        for decl in &self.story.variables {
            match decl.scope {
                VarScope::Game => {
                    let ok = state
                        .vars
                        .get(&decl.name)
                        .is_some_and(|v| v.value_type() == decl.initial.value_type());
                    if !ok {
                        state.vars.insert(decl.name.clone(), decl.initial.clone());
                    }
                }
                VarScope::Persistent => {
                    state.vars.remove(&decl.name);
                }
            }
        }
        let story = &self.story;
        let scene = &mut state.scene;
        if scene
            .background
            .is_some_and(|a| story.assets.get(a).is_none())
        {
            scene.background = None;
        }
        if scene
            .music
            .as_ref()
            .is_some_and(|m| story.assets.get(m.asset).is_none())
        {
            scene.music = None;
        }
        scene.characters.retain(|c| {
            story
                .character(c.character)
                .is_some_and(|def| def.expression(&c.expression).is_some())
                && story.resolve_position(&c.position).is_some()
        });
    }

    /// Replaces the state with `state` (taken from a VM running the same
    /// story, or an older version of it: see `reconcile`). Fails, leaving the
    /// VM untouched, if it references nodes that do not exist in this story.
    /// Clears a previous fatal error, drops pending presentation events and
    /// **starts a fresh rollback history** (use `restore_with_history` to keep
    /// one).
    pub fn restore(&mut self, mut state: VmState) -> Result<(), VmError> {
        self.validate_state(&state)?;
        self.reconcile(&mut state);
        self.history.clear();
        if state.shown.is_some() {
            self.history.record(&state);
        }
        self.state = state;
        self.fault = None;
        self.events.clear();
        Ok(())
    }

    /// Lines already shown (persistent across rollback and, later, sessions).
    pub fn read_nodes(&self) -> &BTreeSet<NodeId> {
        &self.read
    }

    pub fn is_read(&self, node: NodeId) -> bool {
        self.read.contains(&node)
    }

    /// Replaces the set of read lines (e.g. loaded from persistent storage).
    pub fn set_read_nodes(&mut self, read: BTreeSet<NodeId>) {
        self.read = read;
    }
}

fn blocking_duration(transition: &Option<Transition>) -> Option<f32> {
    transition.map(|t| t.duration).filter(|d| *d > 0.0)
}

fn build_index(story: &Story) -> HashMap<NodeId, (usize, usize)> {
    let mut index = HashMap::new();
    for (s, scene) in story.scenes.iter().enumerate() {
        for (n, node) in scene.nodes.iter().enumerate() {
            index.entry(node.id).or_insert((s, n));
        }
    }
    index
}
