//! Game plugins in the store (see `docs/plugin-store.md`, "Game plugins").
//!
//! A game plugin (runtime plugin) is a Rust crate linked statically into the
//! player at export (`orbit_export::plugins`). The store distributes it as
//! **sources**, not as a binary: a `.orbitsrc` package (a zip with the crate
//! files and an `orbit-game-plugin.json` manifest), listed in the optional
//! `game_plugins` array of a catalog, and installed **per project** in
//! `plugins/<dir>/`.
//!
//! * [`catalog`]: the `game_plugins` entries of `index.json` (ignored by
//!   editors that predate them), compatibility by plugin API version and
//!   engine version range, the signed package message, update detection;
//! * [`package`]: the package format, its safe reading (no path escaping the
//!   folder, no link, size limits, every file hashed in the manifest) and
//!   packing (`cargo xtask game-plugin pack`);
//! * [`download`]: a package downloaded, its size, SHA-256 and signature
//!   checked (same trust levels as editor plugins);
//! * [`project`]: install / update / uninstall in a project, with an install
//!   record per plugin folder; local changes are never overwritten silently.
//!
//! Nothing here compiles or runs the plugin: that happens at export, when
//! the exporter builds a player with the project's `plugins/` crates.

pub mod catalog;
pub mod download;
pub mod package;
pub mod project;

use crate::sources::{SourceEntry, Trust};

pub use catalog::{GameAvailability, GamePlugin, GameVersion, parse_game_plugins, select_game};
pub use package::{PACKAGE_EXTENSION, Package, PackageError, PackageManifest};
pub use project::{
    Change, Conflict, InstallError, InstallOptions, InstallOutcome, InstallRecord, InstalledGame,
    OnConflict,
};

/// Where a game plugin package comes from: what its signature is checked
/// against and what the install record keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageSource {
    pub id: String,
    pub name: String,
    pub url: String,
    pub trust: Trust,
    /// The key the package must be signed with (Official, Trusted).
    pub key: Option<[u8; 32]>,
    /// The official source: ids `orbit.*` are allowed.
    pub official: bool,
}

impl PackageSource {
    pub fn of(source: &SourceEntry) -> Self {
        Self {
            id: source.id.clone(),
            name: source.name.clone(),
            url: source.url.clone(),
            trust: source.trust(),
            key: source.key(),
            official: source.builtin,
        }
    }
}
