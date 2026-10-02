//! OrbitEngine core: the canonical story IR.
//!
//! This crate has no graphics, windowing or filesystem dependency and compiles
//! to `wasm32-unknown-unknown`. The story model is serialized as RON, one file
//! per story, and every node carries a stable ULID.

pub mod custom;
pub mod defs;
pub mod error;
pub mod expr;
pub mod ids;
pub mod migrate;
pub mod story;
pub mod validate;
pub mod value;

pub use custom::CustomData;
pub use defs::{
    AssetEntry, AssetKind, AssetManifest, Character, Color, Expression, Position, PositionDef,
    StringTable, Transition, TransitionKind, VarDecl, VarScope,
};
pub use error::{CoreError, ValidationError, ValidationWarning};
pub use expr::{BinOp, EvalError, Expr, ParseError, TypeError, UnOp};
pub use ids::{AssetId, CharacterId, NodeId, SceneId, StringId};
pub use story::{CURRENT_SCHEMA_VERSION, ChoiceOption, Node, NodeKind, Scene, Story};
pub use validate::{ValidationReport, check_asset_files, validate};
pub use value::{Value, ValueType};
