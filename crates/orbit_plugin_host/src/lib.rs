//! Native editor plugins, **host side**: the editor ships without plugins and
//! the user installs them from the plugin manager. A plugin is a `cdylib`
//! built with `orbit_editor_api::export_plugin!` (see
//! `orbit_editor_api::native` for the contract and its rules).
//!
//! Before a library is loaded for real it goes through, in order:
//!
//! 1. [`binary`]: the file is a dynamic library for this OS and processor
//!    (headers only, nothing executed);
//! 2. the descriptor, read in a short-lived child process
//!    ([`verify::ProcessProber`], `orbit-editor --probe-plugin <file>`) so
//!    that a broken library cannot crash the editor: magic, layout version;
//! 3. [`verify::verify`]: build fingerprint equal to the editor's (rustc,
//!    egui, API sources and ABI, type layouts, target, profile), valid id,
//!    manifest id and editor range;
//! 4. at each start: the SHA-256 recorded at install (when size or date
//!    changed), the probe cached per editor build, the id not taken, and a
//!    marker that disables a plugin which took the editor down while loading.
//!
//! [`PluginManager`] keeps the plugins folder (install, uninstall, enable,
//! staged updates) and loads the enabled ones at start-up.

pub mod binary;
mod load;
mod manager;
pub mod manifest;
pub mod sha256;
pub mod store;
pub mod verify;

pub use load::{OpenLibrary, open, probe_main, read_descriptor};
pub use manager::{
    DATA_DIR, InstallError, Installed, InstalledPlugin, LoadedPlugin, PACKAGE_EXTENSION, PluginManager,
    StartupReport,
};
pub use manager::bundled::{BUNDLED_DIR, BundledReport, bundled_dir_for_exe};
pub use manifest::{MANIFEST_FILE, PackageManifest, RangeError};
pub use verify::{EditorFacts, InProcessProber, Problem, ProcessProber, Prober};
