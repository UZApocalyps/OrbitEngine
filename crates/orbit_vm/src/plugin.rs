//! Runtime plugin hook (ADR 0015 §4, M8-04).
//!
//! The VM does not know plugins: a host attaches a [`PluginHost`]
//! (`orbit_plugin_api::RuntimeRegistry` implements it) and the VM calls it
//! when it reaches a `NodeKind::Custom` node, and when the UI fires a
//! `UiAction::Custom` (through [`crate::Vm::run_ui_action`]).
//!
//! Same rules as scripts: a handler gets a **read-only view** of the
//! variables and the VM's seeded RNG, and returns **operations**
//! ([`VarOp`]) and events. The VM applies them after the handler returns,
//! all or nothing. Handlers are `&self` and `Send + Sync`: a plugin keeps no
//! hidden mutable game state, everything that must survive a save or a
//! rollback lives in story variables (declared in the project), and the RNG
//! draws are part of the saved VM state. Re-running a node from the same
//! state therefore gives the same result (rollback, seek, replays).

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

use orbit_core::{Condition, CustomData, EvalError, OpError, Value, VarOp};

use crate::state::Rng;

/// What a custom node asks the VM to do once it ran.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginOutcome {
    /// Go on with the next node in the same `step()`.
    Continue,
    /// Block: the host waits this many seconds, then calls `step()` again
    /// (`StepResult::WaitTime`). The node is done; the next one runs after.
    Wait(f32),
}

/// An event a handler sends to the host (sound, screen effect, analytics…),
/// delivered as `PresentationEvent::Custom { plugin_id, kind: name, data }`.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginEvent {
    pub name: String,
    pub data: CustomData,
}

/// The view a handler works with. Reads see the variables as they were
/// before the handler plus the handler's own pending writes, so a handler
/// can read back what it set.
pub struct PluginContext<'a> {
    vars: &'a BTreeMap<String, Value>,
    persistent: &'a BTreeMap<String, Value>,
    rng: Rng,
    pending: BTreeMap<String, Value>,
    ops: Vec<VarOp>,
    events: Vec<PluginEvent>,
    notifications: Vec<TextNotification>,
    warnings: Vec<String>,
    error: Option<String>,
}

/// A notification (toast) asked by a script or a plugin with a literal
/// text (`notify("…")`), delivered as `PresentationEvent::NotifyText` once
/// the handler succeeded.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextNotification {
    /// Text shown (markup allowed).
    pub text: String,
    /// A named notification style of the theme (`None`: `default`).
    #[serde(default)]
    pub style: Option<String>,
    /// Project path of an image shown left of the text.
    #[serde(default)]
    pub icon: Option<String>,
    /// Project path of a sound played with it.
    #[serde(default)]
    pub sound: Option<String>,
    /// Seconds on screen (else the style's / theme's duration).
    #[serde(default)]
    pub duration: Option<f32>,
}

impl<'a> PluginContext<'a> {
    /// A context over these stores (the VM builds one per handler call;
    /// tools such as the editor's script tester build their own).
    pub fn new(
        vars: &'a BTreeMap<String, Value>,
        persistent: &'a BTreeMap<String, Value>,
        rng: Rng,
    ) -> Self {
        Self {
            vars,
            persistent,
            rng,
            pending: BTreeMap::new(),
            ops: Vec::new(),
            events: Vec::new(),
            notifications: Vec::new(),
            warnings: Vec::new(),
            error: None,
        }
    }

    /// Current value of a variable of either scope (`None`: not declared).
    pub fn var(&self, name: &str) -> Option<Value> {
        self.pending
            .get(name)
            .or_else(|| self.vars.get(name))
            .or_else(|| self.persistent.get(name))
            .cloned()
    }

    /// Every variable of either scope with its current value (pending
    /// writes included), in name order. Used by the script host, whose
    /// engine cannot borrow the context.
    pub fn variables(&self) -> BTreeMap<String, Value> {
        let mut all = self.persistent.clone();
        all.extend(self.vars.iter().map(|(k, v)| (k.clone(), v.clone())));
        all.extend(self.pending.iter().map(|(k, v)| (k.clone(), v.clone())));
        all
    }

    /// A copy of the RNG as it is now. Drawing from the copy does not
    /// advance the context: a host that drew `n` numbers from it calls
    /// [`PluginContext::random_u64`] `n` times to commit them.
    pub fn rng(&self) -> Rng {
        self.rng
    }

    /// Evaluates a no-code condition against [`PluginContext::var`].
    pub fn eval(&self, cond: &Condition) -> Result<bool, EvalError> {
        cond.eval(&|n| self.var(n))
    }

    /// Queues a variable operation. It is checked now (unknown variable,
    /// type mismatch: `Err`, and the whole node is cancelled when it
    /// returns) and applied by the VM after the handler, with the others.
    pub fn apply(&mut self, op: VarOp) -> Result<(), OpError> {
        match op.apply(&|n| self.var(n)) {
            Ok(value) => {
                self.pending.insert(op.var.clone(), value);
                self.ops.push(op);
                Ok(())
            }
            Err(e) => {
                self.fail(format!("variable operation on `{}`: {e}", op.var));
                Err(e)
            }
        }
    }

    /// Operations queued so far, in order.
    pub fn queued(&self) -> &[VarOp] {
        &self.ops
    }

    /// Shorthand for `apply(VarOp::set(name, value))`.
    pub fn set(&mut self, name: &str, value: impl Into<Value>) -> Result<(), OpError> {
        self.apply(VarOp::set(name, value))
    }

    /// Next number of the VM's seeded RNG (saved with the game).
    pub fn random_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    /// Uniform integer in `lo..=hi` (bounds swapped if reversed), without
    /// modulo bias.
    pub fn random_range(&mut self, lo: i64, hi: i64) -> i64 {
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        let span = (hi as i128 - lo as i128 + 1) as u128;
        if span > u64::MAX as u128 {
            return self.random_u64() as i64;
        }
        let span = span as u64;
        let zone = u64::MAX - (u64::MAX % span);
        loop {
            let r = self.random_u64();
            if r < zone {
                return (lo as i128 + (r % span) as i128) as i64;
            }
        }
    }

    /// Uniform float in `[0, 1)`.
    pub fn random_f64(&mut self) -> f64 {
        (self.random_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Sends an event to the host.
    pub fn emit(&mut self, name: impl Into<String>, data: CustomData) {
        self.events.push(PluginEvent {
            name: name.into(),
            data,
        });
    }

    /// Shows a notification (toast) once the handler succeeded.
    pub fn notify(&mut self, notification: TextNotification) {
        self.notifications.push(notification);
    }

    /// Notifications queued so far, in order.
    pub fn notifications(&self) -> &[TextNotification] {
        &self.notifications
    }

    /// A non-fatal message (shown in the console / the editor's warnings).
    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    /// Cancels the node: nothing it queued is applied, the RNG is not
    /// advanced, the VM records `message` as a warning and moves on.
    pub fn fail(&mut self, message: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some(message.into());
        }
    }

    pub(crate) fn finish(self) -> PluginEffects {
        PluginEffects {
            rng: self.rng,
            values: self.pending,
            events: self.events,
            notifications: self.notifications,
            warnings: self.warnings,
            error: self.error,
        }
    }
}

pub(crate) struct PluginEffects {
    pub rng: Rng,
    /// Final value of every variable written, in name order.
    pub values: BTreeMap<String, Value>,
    pub events: Vec<PluginEvent>,
    pub notifications: Vec<TextNotification>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

/// Result of [`PluginHost::run_node`] / [`PluginHost::run_action`].
#[derive(Clone, Debug, PartialEq)]
pub enum Handled<T> {
    /// No installed plugin handles this node / action.
    Unhandled,
    Done(T),
}

/// What the VM calls for plugin-defined nodes and UI actions.
pub trait PluginHost: Debug + Send + Sync {
    /// Runs a custom node. `Unhandled` makes the VM skip it with a warning
    /// (the presentation event is still emitted).
    fn run_node(
        &self,
        plugin_id: &str,
        kind: &str,
        version: u32,
        data: &CustomData,
        ctx: &mut PluginContext<'_>,
    ) -> Handled<PluginOutcome>;

    /// Runs a `UiAction::Custom { name, args }`.
    fn run_action(&self, name: &str, args: &[String], ctx: &mut PluginContext<'_>) -> Handled<()>;

    /// Ids of the plugins this host provides.
    fn plugin_ids(&self) -> Vec<String>;
}

/// A host shared between the VMs of a game (new game, loads).
pub type SharedPluginHost = Arc<dyn PluginHost>;
