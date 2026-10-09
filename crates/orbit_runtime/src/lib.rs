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

/// 3D renderer (ADR 0014), compiled only with the `3d` feature.
#[cfg(feature = "3d")]
pub use orbit_render3d as render3d;

pub mod achievements;
pub mod compose3d;
pub mod director;
pub mod format;
mod game;
pub mod gamepad_bindings;
mod hotspots;
pub mod input;
#[cfg(feature = "3d")]
pub mod library3d;
pub mod loading;
pub mod overlay;
pub mod preview;
pub mod rigs;
mod runtime;
mod scripts;
pub mod scene_view;
mod shell;
mod splash;
mod splash_draw;
pub mod stage;

pub use achievements::{AchievementEvent, AchievementHook, SharedAchievementHook, achievement_icon_paths};
pub use director::{AudioCommand, Director, Flow, Line, StageTransition};
pub use input::{Hotkey, InputAction, InputMapper};
pub use loading::{AssetStore, LoadedStory, parse_story, scene_asset_paths, successor_scenes};
pub use runtime::{
    ENGINE_PAK_PREFIX, EmbeddedConfig, LoadOutcome, Notice, PlayFrom, PlaySpeed, PlayStatus,
    PreviewSnapshot, Runtime, RuntimeConfig, RuntimeError, UiPreviewTarget, error_markup,
    escape_markup,
    parse_translation_index,
};
pub use orbit_vm::SharedPluginHost;
pub use shell::{AutosaveConfig, REPLAY_SCREEN, gallery_paths};
pub use stage::{
    CharacterPlacement, Stage3d, StageCharacter, StageEffect, StageState, character_placement,
};
