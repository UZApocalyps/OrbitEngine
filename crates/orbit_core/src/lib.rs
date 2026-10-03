//! OrbitEngine core: the canonical story IR.
//!
//! This crate has no graphics, windowing or filesystem dependency and compiles
//! to `wasm32-unknown-unknown`. The story model is serialized as RON, one file
//! per story, and every node carries a stable ULID.

pub mod anim;
pub mod custom;
pub mod defs;
pub mod effects;
pub mod error;
pub mod expr;
pub mod ids;
pub mod migrate;
pub mod story;
pub mod validate;
pub mod value;
pub mod varops;

pub use custom::CustomData;
pub use effects::{
    AnimProp, AnimSpec, AnimTarget, CameraFx, Effect, EntityFx, StageFx, StoryTrack, shake_wave,
};
pub use orbit_anim::{Angle, Key, Loop, Rgba, Track, TrackData, Value as AnimValue, Vec2};
pub use orbit_anim::Loop as AnimLoop;
pub use orbit_anim::{Curve, Ease};
pub use defs::{
    AnimFrames, Animation, AssetEntry, AssetKind, AssetManifest, Character, Color, Expression,
    FREE_POSITION_RANGE, LoopMode, Motion,
    Position, PositionDef, SCALE_MAX, SCALE_MIN, StringTable, Transition, TransitionKind, VarDecl,
    VarScope, scale_is_valid,
};
pub use error::{AnimationIssue, CoreError, CurveOwner, ValidationError, ValidationWarning};
pub use expr::{BinOp, EvalError, Expr, Func, ParseError, TypeError, UnOp, fold_list, fold_map};
pub use ids::{AssetId, CharacterId, Node3dId, NodeId, SceneId, StringId};
pub use story::{CURRENT_SCHEMA_VERSION, ChoiceOption, Node, NodeKind, Scene, ScreenParam, Story};
pub use validate::{ValidationReport, check_asset_files, validate};
pub use value::{MAX_COLLECTION_LEN, MAX_COLLECTION_TEXT, ScalarType, Value, ValueType};
pub use varops::{
    Cmp, Condition, ITEM_INDEX_VAR, ITEM_KEY_VAR, ITEM_VAR, OpError, OpKind, Operand, PARAM_VAR_PREFIX,
    VarOp, coerce,
};
