//! OrbitEngine runtime: plays a story RON file.
//!
//! It glues the story VM ([`orbit_vm`]), the asset loader, the 2D renderer,
//! the text engine, the audio engine and the in-game UI together, independently
//! of any binary: [`Runtime`] implements the platform [`App`](orbit_platform::App)
//! trait and `orbit_player` only has to create it.
//!
//! Flow: load the story (async, also on the web) and validate it, preload the
//! assets its manifest declares behind a progress bar, wait for a "click to
//! start" gesture (it unlocks audio, mandatory on the web), then play. A load
//! or validation failure shows a readable error screen instead of panicking.
//!
//! Layers, from the pure to the GPU-bound:
//! * [`stage`]: the stage as data and its layout math;
//! * [`director`]: VM + presentation state machine (lines, choices, pauses,
//!   transitions, audio requests), driven by an explicit `dt`;
//! * [`input`]: raw input to logical actions;
//! * [`Runtime`]: owns the renderer/text/audio, draws everything.
//!
//! Choices made here, documented as the spec asked:
//! * a click or key press during a transition **completes it at once**
//!   (rather than being ignored);
//! * hot reload (native) restarts the story from the beginning.

pub mod director;
mod game;
pub mod input;
pub mod loading;
mod runtime;
pub mod stage;

pub use director::{AudioCommand, Director, Flow, Line, StageTransition};
pub use input::{InputMapper, UiAction};
pub use loading::{AssetStore, LoadedStory, parse_story};
pub use runtime::{Runtime, RuntimeConfig, RuntimeError, error_markup, escape_markup};
pub use stage::{StageCharacter, StageEffect, StageState};
