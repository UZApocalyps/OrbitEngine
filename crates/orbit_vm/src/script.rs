//! Script hook (ADR 0015 §1-§3, M8-01).
//!
//! The VM does not embed a script language: a host attaches a
//! [`ScriptHost`] (`orbit_script::ScriptEngine` implements it with Rhai) and
//! the VM calls it for `NodeKind::Script` nodes and `Expr::Rhai` conditions.
//! A game without scripts never needs one; a VM without a host skips script
//! nodes with a warning and reads Rhai conditions as `false`.
//!
//! The rules are the plugins' ones ([`crate::PluginContext`]): a script
//! reads variables, queues writes and draws from the VM's seeded RNG; the
//! VM applies everything after the script returns, all or nothing. A
//! failing script (syntax error, runtime error, sandbox limit) changes
//! nothing, not even the RNG. No script state survives a node: saves and
//! rollback only see variables and the RNG.

use std::fmt::Debug;
use std::sync::Arc;

use orbit_core::Value;

use crate::plugin::PluginContext;

/// Why a script or a Rhai condition failed. `line` / `column` are 1-based
/// positions in the source when the engine knows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptFailure {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl ScriptFailure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            line: None,
            column: None,
        }
    }
}

impl std::fmt::Display for ScriptFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(l), Some(c)) => write!(f, "{} (line {l}, column {c})", self.message),
            (Some(l), None) => write!(f, "{} (line {l})", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

/// What a script node produced, besides the operations queued in the
/// context.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptOutcome {
    /// Branch chosen by the script (`branch(…)` or a returned text), if any.
    pub branch: Option<String>,
    /// Messages of `log(…)`, in order.
    pub logs: Vec<String>,
}

/// What the VM calls for scripts.
pub trait ScriptHost: Debug + Send + Sync {
    /// Runs the source of a `Script` node. Variable writes and RNG draws go
    /// through `ctx` (applied by the VM after a success only).
    fn run(&self, source: &str, ctx: &mut PluginContext<'_>) -> Result<ScriptOutcome, ScriptFailure>;

    /// Evaluates a Rhai condition: reads only (no writes, no RNG).
    fn eval(
        &self,
        source: &str,
        vars: &dyn Fn(&str) -> Option<Value>,
    ) -> Result<Value, ScriptFailure>;
}

/// A host shared between the VMs of a game (new game, loads, editor).
pub type SharedScriptHost = Arc<dyn ScriptHost>;
