//! The virtual machine.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use orbit_core::{NodeId, NodeKind, Story, StringId, Transition, Value, VarScope};

use crate::error::VmError;
use crate::event::{ChoiceView, PresentationEvent, StepResult};
use crate::history::History;
use crate::state::{CharacterOnStage, Interaction, MusicState, Pending, Rng, Scene3dState, VmState};

/// Maximum number of nodes executed by one `step()` call without reaching a
/// blocking node. Guards against `Jump` loops with no `Say`/`Wait`.
pub const MAX_NODES_PER_STEP: usize = 10_000;
/// Maximum depth of nested `Call`s.
pub const MAX_CALL_DEPTH: usize = 256;

/// Screens the story can keep open at once (the oldest non-waiting one is
/// closed beyond that).
pub const MAX_OPEN_SCREENS: usize = 16;

/// Animations kept at once (finished ones stay until the scene changes).
pub const MAX_ACTIVE_ANIMS: usize = 256;
/// Seconds past the end `complete_anim` puts an animation, so that rounding
/// to `f32` can never leave it just short of its final state.
const COMPLETE_MARGIN: f64 = 1e-3;

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
    /// Runtime plugins (M8-04); not part of the state.
    plugins: Option<crate::plugin::SharedPluginHost>,
    /// Script engine (M8-01); not part of the state.
    scripts: Option<crate::script::SharedScriptHost>,
    /// Scene seen / replay tracking (M7-03, M7-04), see `crate::replay`;
    /// not part of the state.
    pub(crate) tracking: crate::replay::Tracking,
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
                active_anims: Vec::new(),
                scene_clock: 0.0,
                open_screens: Vec::new(),
            },
            read: BTreeSet::new(),
            events: Vec::new(),
            fault: None,
            persistent,
            persistent_names,
            warnings: Vec::new(),
            history: History::default(),
            plugins: None,
            scripts: None,
            tracking: Default::default(),
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
        if let Some(result) = self.pending_result() {
            return result;
        }
        self.history.ahead = true;
        match self.run() {
            Ok(result) => {
                if result.is_interaction() {
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
        if let Some(result) = self.pending_result() {
            return Some(result);
        }
        self.history.ahead = true;
        match self.run_until(Some(target)) {
            Ok(None) => None,
            Ok(Some(result)) => {
                if result.is_interaction() {
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

    /// Evaluates a condition. A Rhai condition (ADR 0015) goes to the
    /// script host; when it fails, or no host is attached, it reads as
    /// `false` with a warning (a hidden option, the `else` branch) rather
    /// than stopping the game.
    fn eval_bool(&mut self, node: NodeId, expr: &orbit_core::Expr) -> Result<bool, VmError> {
        if let orbit_core::Expr::Rhai(code) = expr {
            let Some(host) = self.scripts.clone() else {
                self.warnings.push(format!(
                    "node {node}: this player cannot run scripts, the Rhai condition reads as false"
                ));
                return Ok(false);
            };
            let result = host.eval(code, &|name| self.variable(name).cloned());
            return match result {
                Ok(Value::Bool(b)) => Ok(b),
                Ok(other) => {
                    self.warnings.push(format!(
                        "node {node}: the Rhai condition gave a {} instead of a bool, read as false",
                        other.value_type()
                    ));
                    Ok(false)
                }
                Err(e) => {
                    self.warnings
                        .push(format!("node {node}: Rhai condition failed, read as false: {e}"));
                    Ok(false)
                }
            };
        }
        match self.eval(node, expr)? {
            Value::Bool(b) => Ok(b),
            _ => Err(VmError::ConditionNotBool(node)),
        }
    }

    /// What `step()` returns again while a choice or a screen is pending.
    fn pending_result(&self) -> Option<StepResult> {
        match &self.state.pending {
            Pending::None => None,
            Pending::Choice { node, visible } => Some(self.choice_result(*node, visible)),
            Pending::Screen { node } => Some(
                match self.state.open_screens.iter().find(|s| s.node == *node && s.waiting) {
                    Some(s) => StepResult::Screen {
                        node: *node,
                        screen: s.screen.clone(),
                    },
                    None => StepResult::Error(VmError::UnknownNode(*node)),
                },
            ),
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
            if let Some(end) = self.enter_scene(si) {
                return Ok(Some(end));
            }
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
                    self.clear_anims();
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
                        // Data-dependent failures (division by zero, index
                        // out of bounds, missing key, collection too large)
                        // leave the variable unchanged (ADR 0016).
                        Err(VmError::Eval { source, .. }) if source.is_recoverable() => self
                            .warnings
                            .push(format!("node {pc}: {source}, `{var}` unchanged")),
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
                    self.record_image(*asset);
                    self.state.scene.bg_layers.clear();
                    // A 2D background replaces a 3D scene (ADR 0014).
                    self.state.scene.scene3d = None;
                    self.events.push(PresentationEvent::Background {
                        asset: *asset,
                        transition: transition.clone(),
                    });
                    if let Some(duration) = blocking_duration(transition) {
                        return Ok(Some(StepResult::WaitTransition { duration }));
                    }
                }
                NodeKind::BackgroundLayers { layers, transition } => {
                    self.state.pc = next;
                    if let Some(first) = layers.first() {
                        self.state.scene.background = Some(first.asset);
                        self.state.scene.bg_layers = layers.clone();
                        self.state.scene.scene3d = None;
                        self.events.push(PresentationEvent::BackgroundLayers {
                            layers: layers.clone(),
                            transition: transition.clone(),
                        });
                        if let Some(duration) = blocking_duration(transition) {
                            return Ok(Some(StepResult::WaitTransition { duration }));
                        }
                    }
                }
                NodeKind::Particles(emitter) => {
                    self.state.pc = next;
                    // Same name = same emitter: the new one replaces it.
                    let index = &self.index;
                    let story = &self.story;
                    let name = &emitter.name;
                    self.state.active_anims.retain(|a| {
                        index.get(&a.node).is_none_or(|&(s, n)| {
                            !matches!(&story.scenes[s].nodes[n].kind,
                                NodeKind::Particles(e) if e.name == *name)
                        })
                    });
                    self.start_anim(pc);
                }
                NodeKind::StopParticles { .. } => {
                    self.state.pc = next;
                    self.start_anim(pc);
                }
                NodeKind::ShowScene3d {
                    scene,
                    camera,
                    transition,
                } => {
                    self.state.pc = next;
                    self.state.scene.scene3d = Some(Scene3dState {
                        scene: *scene,
                        camera: camera.clone(),
                    });
                    self.events.push(PresentationEvent::Scene3d {
                        scene: *scene,
                        camera: camera.clone(),
                        transition: transition.clone(),
                    });
                    if let Some(duration) = blocking_duration(transition) {
                        return Ok(Some(StepResult::WaitTransition { duration }));
                    }
                }
                NodeKind::Camera3d { camera, transition } => {
                    self.state.pc = next;
                    match &mut self.state.scene.scene3d {
                        Some(shown) => {
                            shown.camera = Some(camera.clone());
                            let scene = shown.scene;
                            self.events.push(PresentationEvent::Scene3d {
                                scene,
                                camera: Some(camera.clone()),
                                transition: transition.clone(),
                            });
                            if let Some(duration) = blocking_duration(transition) {
                                return Ok(Some(StepResult::WaitTransition { duration }));
                            }
                        }
                        None => self.warnings.push(format!(
                            "node {pc}: camera `{camera}` but no 3D scene is shown, ignored"
                        )),
                    }
                }
                NodeKind::HideScene3d { transition } => {
                    self.state.pc = next;
                    // Hiding when nothing is shown is a no-op (like `Hide`).
                    if self.state.scene.scene3d.take().is_some() {
                        self.events.push(PresentationEvent::HideScene3d {
                            transition: transition.clone(),
                        });
                        if let Some(duration) = blocking_duration(transition) {
                            return Ok(Some(StepResult::WaitTransition { duration }));
                        }
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
                        transition: transition.clone(),
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
                    // Animations of a character that leaves must not follow
                    // it back on stage.
                    self.drop_anims_of(&story, *character);
                    let before = self.state.scene.characters.len();
                    self.state
                        .scene
                        .characters
                        .retain(|c| c.character != *character);
                    // Hiding a character that is not on stage is a no-op.
                    if self.state.scene.characters.len() != before {
                        self.events.push(PresentationEvent::Hide {
                            character: *character,
                            transition: transition.clone(),
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
                NodeKind::Effect(_) | NodeKind::Timeline { .. } => {
                    self.state.pc = next;
                    if let Some(spec) = node.kind.animation() {
                        self.start_anim(pc);
                        // A blocking animation waits its whole length; an
                        // endless one cannot block (validation refuses it).
                        if spec.wait
                            && let Some(duration) = spec.total_duration()
                            && duration > 0.0
                            && duration.is_finite()
                        {
                            return Ok(Some(StepResult::WaitAnimation {
                                node: pc,
                                duration,
                            }));
                        }
                    }
                }
                NodeKind::OpenScreen {
                    screen,
                    wait,
                    params,
                    ..
                } => {
                    self.state.pc = next;
                    let mut values = Vec::with_capacity(params.len());
                    for p in params {
                        let value = match &p.value {
                            orbit_core::Operand::Lit(v) => v.clone(),
                            orbit_core::Operand::Var(name) => self
                                .variable(name)
                                .cloned()
                                .ok_or_else(|| VmError::Eval {
                                    node: pc,
                                    source: orbit_core::EvalError::UnknownVariable(name.clone()),
                                })?,
                            // Refused by validation: there is no screen
                            // context in the story.
                            _ => {
                                self.warnings.push(format!(
                                    "node {pc}: parameter `{}` has no value outside a screen, ignored",
                                    p.name
                                ));
                                continue;
                            }
                        };
                        values.push((p.name.clone(), value));
                    }
                    // Opening a screen that is open replaces it.
                    self.state.open_screens.retain(|s| s.screen != *screen);
                    self.state.open_screens.push(crate::state::OpenScreenState {
                        screen: screen.clone(),
                        node: pc,
                        params: values,
                        waiting: *wait,
                    });
                    if self.state.open_screens.len() > MAX_OPEN_SCREENS
                        && let Some(oldest) =
                            self.state.open_screens.iter().position(|s| !s.waiting)
                    {
                        self.state.open_screens.remove(oldest);
                    }
                    if *wait {
                        self.state.pending = Pending::Screen { node: pc };
                        self.state.shown = Some(Interaction::Screen {
                            node: pc,
                            screen: screen.clone(),
                        });
                        return Ok(Some(StepResult::Screen {
                            node: pc,
                            screen: screen.clone(),
                        }));
                    }
                }
                NodeKind::CloseScreen { screen } => {
                    self.state.pc = next;
                    self.state.open_screens.retain(|s| s.screen != *screen);
                }
                NodeKind::Custom {
                    plugin_id,
                    kind,
                    version,
                    data,
                } => {
                    self.state.pc = next;
                    let outcome = self.run_plugin_node(plugin_id, kind, *version, data);
                    if let crate::plugin::PluginOutcome::Wait(secs) = outcome
                        && secs.is_finite()
                        && secs > 0.0
                    {
                        return Ok(Some(StepResult::WaitTime(secs)));
                    }
                }
                NodeKind::Script {
                    source,
                    branches,
                    on_error,
                } => {
                    self.state.pc = next;
                    match self.run_script_node(pc, source) {
                        Ok(Some(branch)) => {
                            match branches.iter().find(|b| b.name == branch) {
                                Some(b) => self.jump(b.target)?,
                                None => self.warnings.push(format!(
                                    "node {pc}: the script chose the unknown branch `{branch}`, going on"
                                )),
                            }
                        }
                        Ok(None) => {}
                        Err(()) => {
                            if let Some(target) = on_error {
                                self.jump(*target)?;
                            }
                        }
                    }
                }
            }
        }
        Err(VmError::StepLimit(MAX_NODES_PER_STEP))
    }

    // ------------------------------------------------------------ scripts

    /// Attaches the script engine (M8-01). Not part of the state: a host
    /// sets it again on every VM it creates (new game, load).
    pub fn set_script_host(&mut self, host: Option<crate::script::SharedScriptHost>) {
        self.scripts = host;
    }

    pub fn script_host(&self) -> Option<&crate::script::SharedScriptHost> {
        self.scripts.as_ref()
    }

    /// Runs a script node and commits its effects (all or nothing).
    /// `Ok(branch)` on success, `Err(())` when it failed (nothing applied,
    /// a warning recorded).
    fn run_script_node(&mut self, node: NodeId, source: &str) -> Result<Option<String>, ()> {
        use crate::plugin::PluginContext;
        let Some(host) = self.scripts.clone() else {
            self.warnings.push(format!(
                "node {node}: this player cannot run scripts, script skipped"
            ));
            return Err(());
        };
        let mut ctx = PluginContext::new(&self.state.vars, &self.persistent, self.state.rng);
        let result = host.run(source, &mut ctx);
        let effects = ctx.finish();
        match result {
            Ok(outcome) => {
                self.warnings.extend(
                    outcome
                        .logs
                        .into_iter()
                        .map(|m| format!("script {node}: {m}")),
                );
                if self.commit_plugin(&format!("script {node}"), effects, false) {
                    Ok(outcome.branch)
                } else {
                    Err(())
                }
            }
            Err(e) => {
                self.warnings
                    .push(format!("script {node} failed, nothing applied: {e}"));
                Err(())
            }
        }
    }

    // ------------------------------------------------------------ plugins

    /// Attaches the runtime plugins (M8-04). Not part of the state: a host
    /// sets it again on every VM it creates (new game, load).
    pub fn set_plugin_host(&mut self, host: Option<crate::plugin::SharedPluginHost>) {
        self.plugins = host;
    }

    pub fn plugin_host(&self) -> Option<&crate::plugin::SharedPluginHost> {
        self.plugins.as_ref()
    }

    /// Commits what a handler produced (all or nothing). Returns `false`
    /// when the handler failed and nothing was applied.
    fn commit_plugin(
        &mut self,
        plugin_id: &str,
        effects: crate::plugin::PluginEffects,
        patch_history: bool,
    ) -> bool {
        self.warnings.extend(
            effects
                .warnings
                .into_iter()
                .map(|w| format!("plugin `{plugin_id}`: {w}")),
        );
        if let Some(err) = effects.error {
            self.warnings
                .push(format!("plugin `{plugin_id}` cancelled: {err}"));
            return false;
        }
        self.state.rng = effects.rng;
        for (name, value) in effects.values {
            if self.variable(&name) == Some(&value) {
                continue;
            }
            if patch_history && !self.persistent_names.contains(&name) {
                self.history.patch_cursor_var(&name, &value);
            }
            self.store_var(&name, value);
        }
        for event in effects.events {
            self.events.push(PresentationEvent::Custom {
                plugin_id: plugin_id.to_owned(),
                kind: event.name,
                data: event.data,
            });
        }
        true
    }

    fn run_plugin_node(
        &mut self,
        plugin_id: &str,
        kind: &str,
        version: u32,
        data: &orbit_core::CustomData,
    ) -> crate::plugin::PluginOutcome {
        use crate::plugin::{Handled, PluginContext, PluginOutcome};
        // The generic event comes first: hosts that present plugin nodes
        // themselves (editor preview) keep working.
        self.events.push(PresentationEvent::Custom {
            plugin_id: plugin_id.to_owned(),
            kind: kind.to_owned(),
            data: data.clone(),
        });
        let Some(host) = self.plugins.clone() else {
            self.warnings.push(format!(
                "plugin `{plugin_id}` is not available: custom node `{kind}` skipped"
            ));
            return PluginOutcome::Continue;
        };
        let mut ctx = PluginContext::new(&self.state.vars, &self.persistent, self.state.rng);
        let handled = host.run_node(plugin_id, kind, version, data, &mut ctx);
        let effects = ctx.finish();
        match handled {
            Handled::Unhandled => {
                self.warnings.push(format!(
                    "plugin `{plugin_id}` is not available: custom node `{kind}` skipped"
                ));
                PluginOutcome::Continue
            }
            Handled::Done(outcome) => {
                if self.commit_plugin(plugin_id, effects, false) {
                    outcome
                } else {
                    PluginOutcome::Continue
                }
            }
        }
    }

    /// Runs a `UiAction::Custom { name, args }` through the plugins (UI-18).
    /// Its variable writes are applied like [`Vm::apply_var_op`] (kept by
    /// the rollback checkpoint on screen). Returns `false` (with a warning)
    /// when no plugin handles `name` or the handler failed.
    pub fn run_ui_action(&mut self, name: &str, args: &[String]) -> bool {
        use crate::plugin::{Handled, PluginContext};
        let Some(host) = self.plugins.clone() else {
            self.warnings
                .push(format!("custom UI action `{name}` has no handler"));
            return false;
        };
        let mut ctx = PluginContext::new(&self.state.vars, &self.persistent, self.state.rng);
        let handled = host.run_action(name, args, &mut ctx);
        let effects = ctx.finish();
        match handled {
            Handled::Unhandled => {
                self.warnings
                    .push(format!("custom UI action `{name}` has no handler"));
                false
            }
            Handled::Done(()) => {
                // Plugin ids are dotted names; the action name is
                // `<plugin id>.<action>`.
                let owner = name.rsplit_once('.').map_or(name, |(p, _)| p).to_owned();
                self.commit_plugin(&owner, effects, true)
            }
        }
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

    /// Finds a scene by id (canonical ULID text) or, failing that, by name
    /// (first match). `UiAction::SceneGoto` stores the id.
    pub fn find_scene(&self, key: &str) -> Option<orbit_core::SceneId> {
        let key = key.trim();
        if let Ok(id) = key.parse::<orbit_core::SceneId>()
            && self.story.scenes.iter().any(|s| s.id == id)
        {
            return Some(id);
        }
        self.story.scenes.iter().find(|s| s.name == key).map(|s| s.id)
    }

    /// Jumps to the start of `scene` from outside a node (a UI button, a
    /// host): the next `step()` runs the scene's first node.
    ///
    /// The call stack and anything pending (choice, pause, transition) are
    /// dropped and a previous error is cleared. Variables, the RNG and the
    /// declarative stage (background, sprites, music) are kept: the target
    /// scene sets up its own stage. For rollback the jump behaves like
    /// picking another choice: the checkpoint on screen stays the rollback
    /// point (one rollback step re-shows it, with the state it had before
    /// the jump) and the known future after it is cut. Saves need nothing
    /// special: the state after the jump is an ordinary `VmState`.
    ///
    /// An empty scene ends the story at the next `step()`. Unknown scene:
    /// `VmError::UnknownScene`, nothing changes.
    pub fn goto_scene(&mut self, scene: orbit_core::SceneId) -> Result<(), VmError> {
        let target = self
            .story
            .scenes
            .iter()
            .find(|s| s.id == scene)
            .ok_or(VmError::UnknownScene(scene))?;
        self.state.pc = target.nodes.first().map(|n| n.id);
        self.state.pending = Pending::None;
        // A screen the story was waiting for is left with the scene; other
        // screens (HUDs) stay open.
        self.state.open_screens.retain(|s| !s.waiting);
        self.state.call_stack.clear();
        self.clear_anims();
        self.fault = None;
        self.events.clear();
        self.history.on_goto();
        Ok(())
    }

    // --------------------------------------------------------------- screens

    /// Screens opened by the story (`OpenScreen`) and not closed yet, in
    /// opening order. The host shows them; they come back with rollback and
    /// loads.
    pub fn open_screens(&self) -> &[crate::state::OpenScreenState] {
        &self.state.open_screens
    }

    /// The player closed `screen`, a screen the story opened (UI-14). With a
    /// `result` (the value of `UiAction::CloseWith`), the `result` variable
    /// of its `OpenScreen` node receives it, keeping its declared type (a
    /// value of another type is ignored with a warning); `None` (Escape, a
    /// plain `Close`) leaves the variable unchanged.
    ///
    /// Returns `true` when the story was waiting for this screen: the next
    /// `step()` continues after the node. For rollback, closing a screen the
    /// story waits for is like answering a choice (one rollback step reopens
    /// it, the known future is cut); closing any other screen is recorded in
    /// the checkpoint on screen, like a variable changed from a screen.
    /// Unknown screen: `VmError::ScreenNotOpen`, nothing changes.
    pub fn close_screen(&mut self, screen: &str, result: Option<Value>) -> Result<bool, VmError> {
        let at = self
            .state
            .open_screens
            .iter()
            .position(|s| s.screen == screen)
            .ok_or_else(|| VmError::ScreenNotOpen(screen.to_owned()))?;
        let entry = self.state.open_screens.remove(at);
        let mut patched_var = None;
        if let Some(value) = result {
            let target = match self.node(entry.node).map(|n| &n.kind) {
                Ok(NodeKind::OpenScreen {
                    result: Some(var), ..
                }) => Some(var.clone()),
                _ => None,
            };
            match target {
                Some(var) => match self.variable(&var).map(Value::value_type) {
                    Some(ty) => match orbit_core::coerce(value, ty) {
                        Ok(v) => {
                            if !self.persistent_names.contains(&var) {
                                patched_var = Some((var.clone(), v.clone()));
                            }
                            self.store_var(&var, v);
                        }
                        Err(e) => self
                            .warnings
                            .push(format!("screen `{screen}`: result ignored, {e}")),
                    },
                    None => self
                        .warnings
                        .push(format!("screen `{screen}`: unknown result variable `{var}`")),
                },
                None => self.warnings.push(format!(
                    "screen `{screen}` closed with a value but its node has no result variable"
                )),
            }
        }
        let waited = entry.waiting && self.state.pending == Pending::Screen { node: entry.node };
        if waited {
            self.state.pending = Pending::None;
            self.history.on_screen_closed();
        } else {
            self.history.patch_cursor(|s| {
                s.open_screens.retain(|o| o.screen != screen);
                if let Some((name, value)) = &patched_var {
                    s.vars.insert(name.clone(), value.clone());
                }
            });
        }
        Ok(waited)
    }

    // ------------------------------------------------------------ animations

    /// Seconds of game time since the scene was entered.
    pub fn scene_clock(&self) -> f64 {
        self.state.scene_clock
    }

    /// Advances the scene clock by `dt` seconds (the host calls this each
    /// frame the game is not paused). Negative and non-finite values are
    /// ignored.
    pub fn advance_clock(&mut self, dt: f64) {
        if dt.is_finite() && dt > 0.0 {
            self.state.scene_clock += dt;
        }
    }

    /// Running and finished animations of the current scene, in start order.
    pub fn active_anims(&self) -> &[crate::state::ActiveAnim] {
        &self.state.active_anims
    }

    /// The animating nodes and their elapsed seconds, in start order. Nodes
    /// that vanished from the story are skipped.
    pub fn anim_items(&self) -> Vec<(&NodeKind, f32)> {
        let clock = self.state.scene_clock;
        self.state
            .active_anims
            .iter()
            .filter_map(|a| {
                let node = self.node(a.node).ok()?;
                node.kind
                    .is_timed()
                    .then(|| (&node.kind, (clock - a.started_at).max(0.0) as f32))
            })
            .collect()
    }

    /// Whether the scene has ambient motion: a running particle emitter or a
    /// timed animation that has not finished (endless ones never do). The
    /// picture then changes with time alone, even while a line waits for the
    /// player's click, so the host must keep drawing frames.
    pub fn has_running_anims(&self) -> bool {
        self.anim_items()
            .iter()
            .any(|(kind, elapsed)| kind.animation().is_some_and(|s| !s.is_finished(*elapsed)))
            || !self.stage_fx().emitters.is_empty()
    }

    /// What the running animations add on top of the staging right now.
    pub fn stage_fx(&self) -> orbit_core::StageFx {
        orbit_core::StageFx::compose(&self.story.curves, self.anim_items())
    }

    /// Whether the animation started by `node` has reached its end (`true`
    /// for a node that is not running).
    pub fn anim_finished(&self, node: NodeId) -> bool {
        let Some(a) = self.state.active_anims.iter().find(|a| a.node == node) else {
            return true;
        };
        let Ok(n) = self.node(node) else { return true };
        let elapsed = (self.state.scene_clock - a.started_at).max(0.0) as f32;
        n.kind.animation().is_none_or(|s| s.is_finished(elapsed))
    }

    /// Skip: puts the animation started by `node` at its final state. Endless
    /// animations are left alone (they are decoration).
    pub fn complete_anim(&mut self, node: NodeId) {
        let Ok(n) = self.node(node) else { return };
        let Some(total) = n.kind.animation().and_then(|s| s.total_duration()) else {
            return;
        };
        let latest = self.state.scene_clock - f64::from(total) - COMPLETE_MARGIN;
        if let Some(a) = self.state.active_anims.iter_mut().find(|a| a.node == node)
            && a.started_at > latest
        {
            a.started_at = latest;
        }
    }

    /// Editor scrubbing: puts the running animation of `node` `elapsed`
    /// seconds into its playback (the clock is not moved). `false` when that
    /// node has no animation running.
    pub fn set_anim_elapsed(&mut self, node: NodeId, elapsed: f32) -> bool {
        let clock = self.state.scene_clock;
        let elapsed = if elapsed.is_finite() { f64::from(elapsed.max(0.0)) } else { 0.0 };
        match self.state.active_anims.iter_mut().find(|a| a.node == node) {
            Some(a) => {
                a.started_at = clock - elapsed;
                true
            }
            None => false,
        }
    }

    /// Skip: every finite animation jumps to its final state.
    pub fn complete_anims(&mut self) {
        let nodes: Vec<NodeId> = self.state.active_anims.iter().map(|a| a.node).collect();
        for node in nodes {
            self.complete_anim(node);
        }
    }

    fn start_anim(&mut self, node: NodeId) {
        let anims = &mut self.state.active_anims;
        anims.retain(|a| a.node != node);
        anims.push(crate::state::ActiveAnim {
            node,
            started_at: self.state.scene_clock,
        });
        if anims.len() > MAX_ACTIVE_ANIMS {
            anims.remove(0);
        }
    }

    /// A scene change: animations stop and the clock restarts.
    fn clear_anims(&mut self) {
        self.state.active_anims.clear();
        self.state.scene_clock = 0.0;
    }

    fn drop_anims_of(&mut self, story: &Story, character: orbit_core::CharacterId) {
        let index = &self.index;
        self.state.active_anims.retain(|a| {
            index
                .get(&a.node)
                .map(|&(s, n)| &story.scenes[s].nodes[n].kind)
                .is_none_or(|k| !k.animates_character(character))
        });
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
        // A waiting screen must be an open screen of an `OpenScreen` node.
        if let Pending::Screen { node } = &state.pending
            && !state.open_screens.iter().any(|s| s.node == *node && s.waiting)
        {
            return Err(VmError::UnknownNode(*node));
        }
        for s in &state.open_screens {
            if !matches!(self.node(s.node).map(|n| &n.kind), Ok(NodeKind::OpenScreen { .. })) {
                return Err(VmError::UnknownNode(s.node));
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
        scene
            .bg_layers
            .retain(|l| story.assets.get(l.asset).is_some());
        if scene.background.is_none() {
            // The layers stand for the background: without it, none.
            scene.bg_layers.clear();
        }
        if scene
            .scene3d
            .as_ref()
            .is_some_and(|s| story.assets.get(s.scene).is_none())
        {
            scene.scene3d = None;
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
        // An animation whose node is gone (or no longer animates) is dropped
        // silently: the story was edited since the save.
        let index = &self.index;
        state.active_anims.retain(|a| {
            index
                .get(&a.node)
                .is_some_and(|&(s, n)| story.scenes[s].nodes[n].kind.is_timed())
        });
        if !state.scene_clock.is_finite() || state.scene_clock < 0.0 {
            state.scene_clock = 0.0;
        }
        state.active_anims.truncate(MAX_ACTIVE_ANIMS);
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
    transition.as_ref().map(|t| t.duration).filter(|d| *d > 0.0)
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
