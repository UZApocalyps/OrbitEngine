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
pub mod particles;
pub mod scripting;
pub mod story;
pub mod validate;
pub mod value;
pub mod varops;

pub use custom::CustomData;
pub use particles::{
    BgLayer, Emitter, EmitterInstance, MAX_PARTICLES_PER_EMITTER, MAX_PARTICLES_TOTAL, Particle,
    ParticlePreset, hash01,
};
pub use effects::{
    AnimProp, AnimSpec, AnimTarget, CameraFx, Effect, EntityFx, StageFx, StoryTrack, shake_wave,
};
pub use orbit_anim::{Angle, Key, Loop, Rgba, Track, TrackData, Value as AnimValue, Vec2};
pub use orbit_anim::Loop as AnimLoop;
pub use orbit_anim::{Curve, Ease};
pub use defs::{
    AnimFrames, Animation, AssetEntry, AssetKind, AssetManifest, Character, Color, Expression,
    FREE_POSITION_RANGE, LoopMode, Motion, RiggedMotion, with_legacy_motion_layout,
    Position, PositionDef, SCALE_MAX, SCALE_MIN, StringTable, Transition, TransitionKind, VarDecl,
    VarScope, scale_is_valid,
};
pub use error::{AnimationIssue, CoreError, CurveOwner, ValidationError, ValidationWarning};
pub use expr::{
    BinOp, EvalError, Expr, Func, ParseError, RHAI_PREFIX, TypeError, UnOp, fold_list, fold_map,
};
pub use ids::{AssetId, CharacterId, Node3dId, NodeId, SceneId, StringId};
pub use story::{
    CURRENT_SCHEMA_VERSION, ChoiceOption, Node, NodeKind, Scene, ScreenParam, ScriptBranch, Story,
};
pub use validate::{MAX_BG_LAYERS, ValidationReport, check_asset_files, validate};
pub use value::{MAX_COLLECTION_LEN, MAX_COLLECTION_TEXT, ScalarType, Value, ValueType};
pub use varops::{
    Cmp, Condition, ITEM_INDEX_VAR, ITEM_KEY_VAR, ITEM_VAR, OpError, OpKind, Operand, PARAM_VAR_PREFIX,
    VarOp, coerce,
};
