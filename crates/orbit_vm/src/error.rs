//! VM errors.

use orbit_core::{CharacterId, EvalError, NodeId, SceneId};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum VmError {
    #[error("the story has no node to start from")]
    EmptyStory,
    #[error("unknown node {0}")]
    UnknownNode(NodeId),
    #[error("unknown scene {0}")]
    UnknownScene(SceneId),
    #[error("node {node}: unknown character {character}")]
    UnknownCharacter {
        node: NodeId,
        character: CharacterId,
    },
    #[error("node {node}: character {character} has no expression `{expression}`")]
    UnknownExpression {
        node: NodeId,
        character: CharacterId,
        expression: String,
    },
    #[error("node {node}: unknown stage position")]
    UnknownPosition { node: NodeId },
    #[error("node {node}: {source}")]
    Eval {
        node: NodeId,
        #[source]
        source: EvalError,
    },
    #[error("node {0}: condition did not evaluate to a bool")]
    ConditionNotBool(NodeId),
    #[error("choice {0} has no visible option")]
    NoVisibleChoice(NodeId),
    #[error("call stack overflow (more than {0} nested calls)")]
    StackOverflow(usize),
    #[error("node {0}: Return without a matching Call")]
    ReturnWithoutCall(NodeId),
    #[error("more than {0} nodes executed without waiting: probable infinite loop")]
    StepLimit(usize),
    #[error("the VM is not waiting for a choice")]
    NotAwaitingChoice,
    #[error("invalid choice {index}: {available} option(s) available")]
    InvalidChoice { index: usize, available: usize },
}
