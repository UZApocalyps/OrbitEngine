//! OrbitEngine runtime: plays a story RON file.
//!
//! It glues the story VM ([`orbit_vm`]), the asset loader, the 2D renderer,
//! the text engine, the audio engine and the in-game UI together, independently
//! of any binary: [`Runtime`] implements the platform [`App`](orbit_platform::App)
//! trait and `orbit_player` only has to create it.
//!
//! Flow: load the story (async, also on the web), its translations and validate
//! it, preload the assets its manifest declares behind a progress bar, wait for
//! a "click to start" gesture (it unlocks audio, mandatory on the web), then run:
//! main menu, game, and the screens over them (pause, save, load, preferences,
//! history). A load or validation failure shows a readable error screen
//! instead of panicking.
//!
//! Layers, from the pure to the GPU-bound:
//! * [`stage`]: the stage as data and its layout math;
//! * [`director`]: VM + presentation state machine (lines, choices, pauses,
//!   transitions, audio requests, rollback), driven by an explicit `dt`;
//! * `game`: the director plus the dialogue widgets, skip and auto modes;
//! * [`input`]: raw input to logical actions (one central mapping);
//! * `shell`: the state machine above a playthrough: screens, saves and
//!   autosave, preferences, persistent data, language. GPU free, unit tested;
//! * [`Runtime`]: owns the renderer/text/audio, draws everything, takes the
//!   save thumbnails.
//!
//! Choices made here:
//! * a click or key press during a transition **completes it at once**
//!   (rather than being ignored);
//! * hot reload (native) restarts the story from the beginning;
//! * a save thumbnail is the *stage* (background and characters, no UI)
//!   rendered into a 320x180 target at the moment the save is requested, so
//!   neither a menu nor the textbox ever appears in it; it is read back
//!   asynchronously and the save is written when it arrives (or after a
//!   timeout, without thumbnail).

pub mod director;
pub mod format;
mod game;
pub mod input;
pub mod loading;
mod runtime;
mod shell;
pub mod stage;

pub use director::{AudioCommand, Director, Flow, Line, StageTransition};
pub use input::{Hotkey, InputAction, InputMapper};
pub use loading::{AssetStore, LoadedStory, parse_story};
pub use runtime::{
    Runtime, RuntimeConfig, RuntimeError, error_markup, escape_markup, parse_translation_index,
};
pub use shell::AutosaveConfig;
pub use stage::{StageCharacter, StageEffect, StageState};
