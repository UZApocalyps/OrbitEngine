//! The SDK of native editor plugins: a plugin crate (`crate-type =
//! ["cdylib"]`) depends on this crate only, at the editor's exact version
//! (same git tag, same `Cargo.lock`), and builds with the editor's toolchain
//! and profile.
//!
//! * everything of `orbit_editor_api` (panels, actions, the editor context,
//!   tasks, texts...), its [`native`] module ([`native::NativePlugin`],
//!   [`native::PluginHost`]) and the [`export_plugin!`] macro;
//! * [`egui`]: the editor's own egui, the one to draw with;
//! * the project crates (`orbit_core`, `orbit_project`, `orbit_plugin_api`).
//!
//! Depending on this crate also pins the dependency resolution of the editor
//! (see its `Cargo.toml`): the plugin's egui is then the very same build as
//! the editor's, which the plugin fingerprint requires.
//!
//! See `examples/editor_plugins/hello` and the manual chapter "Native editor
//! plugins".

pub use orbit_core;
pub use orbit_editor_api::*;
pub use orbit_editor_api::{egui, export_plugin, native};
pub use orbit_plugin_api;
pub use orbit_project;
