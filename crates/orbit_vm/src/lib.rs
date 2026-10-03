//! OrbitEngine story VM.
//!
//! A pure, non-blocking, deterministic interpreter for [`orbit_core::Story`].
//! No graphics, windowing, audio or filesystem dependency: it compiles to
//! `wasm32-unknown-unknown` and is tested without a GPU.
//!
//! The host drives it with [`Vm::step`] / [`Vm::choose`], applies the
//! [`PresentationEvent`]s it emits, and may save/restore the whole progress as
//! a serializable [`VmState`].

mod error;
mod event;
mod history;
mod plugin;
mod rollback;
mod seek;
mod state;
mod vm;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod anim_tests;

#[cfg(test)]
mod collection_tests;

pub use error::VmError;
pub use event::{ChoiceView, PresentationEvent, StepResult};
pub use history::{
    BacklogEntry, BacklogItem, Checkpoint, DEFAULT_BACKLOG_CAPACITY, DEFAULT_HISTORY_CAPACITY,
    Presentation, RollbackHistory,
};
pub use plugin::{
    Handled, PluginContext, PluginEvent, PluginHost, PluginOutcome, SharedPluginHost,
};
pub use rollback::scene_events;
pub use seek::{ChoiceStep, SeekLimits, SeekOutcome, SeekPath, seek};
pub use state::{
    ActiveAnim, CharacterOnStage, Interaction, MusicState, Pending, Rng, Scene3dState, SceneState,
    VmState,
};
pub use vm::{MAX_ACTIVE_ANIMS, MAX_CALL_DEPTH, MAX_NODES_PER_STEP, Vm};
