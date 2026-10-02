//! User configuration: theme, dock layout, recent projects, plugin
//! preferences. One RON file in the per-user config folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use egui_dock::DockState;
use orbit_editor_api::{Lang, RecentProject, ThemeMode};
use serde::{Deserialize, Serialize};

use crate::storage::write_atomic;

/// Most recent projects remembered.
pub const MAX_RECENT: usize = 10;

const CONFIG_FILE: &str = "config.ron";

/// Per-user config folder: `%APPDATA%\OrbitEngine\Editor` on Windows,
/// `$XDG_CONFIG_HOME/orbit-engine/editor` (or `~/.config/...`) elsewhere.
pub fn default_config_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("APPDATA")
            .map(|p| PathBuf::from(p).join("OrbitEngine").join("Editor"));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("orbit-engine").join("editor"))
}

/// What is remembered between launches.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserConfig {
    pub theme: ThemeMode,
    /// Interface language chosen by the user; `None` follows the system
    /// language (French for `fr*`, English otherwise).
    pub language: Option<Lang>,
    /// Dock layout: the tabs are panel ids.
    pub dock: Option<DockState<String>>,
    pub recent: Vec<RecentProject>,
    pub plugin_prefs: BTreeMap<String, BTreeMap<String, String>>,
}

impl UserConfig {
    /// Puts a project first in the recent list (deduplicated, bounded).
    pub fn remember_project(&mut self, name: &str, path: &Path) {
        self.recent.retain(|r| r.path != path);
        self.recent.insert(
            0,
            RecentProject {
                name: name.to_owned(),
                path: path.to_owned(),
            },
        );
        self.recent.truncate(MAX_RECENT);
    }

    pub fn forget_project(&mut self, path: &Path) {
        self.recent.retain(|r| r.path != path);
    }
}

/// Reads and writes the config file. A store without a folder never touches
/// the disk (tests, `--no-config`).
#[derive(Clone, Debug)]
pub struct ConfigStore {
    dir: Option<PathBuf>,
}

impl ConfigStore {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The saved config; defaults when absent. A corrupt file is reported
    /// through `log` and ignored (the next save replaces it).
    pub fn load(&self) -> UserConfig {
        let Some(dir) = &self.dir else {
            return UserConfig::default();
        };
        let path = dir.join(CONFIG_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return UserConfig::default();
        };
        match ron::from_str(&text) {
            Ok(config) => config,
            Err(e) => {
                log::warn!("config {} ignored: {e}", path.display());
                UserConfig::default()
            }
        }
    }

    pub fn save(&self, config: &UserConfig) {
        let Some(dir) = &self.dir else {
            return;
        };
        let cfg = ron::ser::PrettyConfig::new().indentor("  ".to_owned());
        match ron::ser::to_string_pretty(config, cfg) {
            Ok(text) => {
                if let Err(e) = write_atomic(&dir.join(CONFIG_FILE), text.as_bytes()) {
                    log::warn!("cannot save the editor config: {e}");
                }
            }
            Err(e) => log::warn!("cannot serialize the editor config: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::testing::TempDir;

    #[test]
    fn config_round_trips_including_the_dock_layout() {
        let dir = TempDir::new("config");
        let store = ConfigStore::new(Some(dir.path().to_owned()));
        assert!(store.load().dock.is_none());

        let mut dock = DockState::new(vec!["a".to_owned(), "b".to_owned()]);
        dock.main_surface_mut().split_left(
            egui_dock::NodeIndex::root(),
            0.25,
            vec!["c".to_owned()],
        );
        let mut config = UserConfig {
            theme: ThemeMode::Light,
            language: Some(Lang::Fr),
            dock: Some(dock),
            ..UserConfig::default()
        };
        config.remember_project("Demo", Path::new("/p/demo"));
        config
            .plugin_prefs
            .entry("x.y".into())
            .or_default()
            .insert("k".into(), "v".into());
        store.save(&config);

        let back = store.load();
        assert_eq!(back.theme, ThemeMode::Light);
        assert_eq!(back.language, Some(Lang::Fr));
        assert_eq!(back.recent.len(), 1);
        assert_eq!(back.plugin_prefs["x.y"]["k"], "v");
        let tabs: Vec<String> = back
            .dock
            .unwrap()
            .iter_all_tabs()
            .map(|(_, t)| t.clone())
            .collect();
        assert_eq!(tabs.len(), 3);
        assert!(tabs.contains(&"c".to_owned()));
    }

    #[test]
    fn a_config_without_a_language_follows_the_system() {
        // Configs written before the language setting existed still load.
        let dir = TempDir::new("nolang");
        std::fs::write(dir.path().join(CONFIG_FILE), "(theme: Dark)").unwrap();
        let store = ConfigStore::new(Some(dir.path().to_owned()));
        assert_eq!(store.load().language, None);
    }

    #[test]
    fn corrupt_config_falls_back_to_defaults() {
        let dir = TempDir::new("corrupt");
        std::fs::write(dir.path().join(CONFIG_FILE), "((( not ron").unwrap();
        let store = ConfigStore::new(Some(dir.path().to_owned()));
        assert!(store.load().recent.is_empty());
    }

    #[test]
    fn store_without_folder_is_inert() {
        let store = ConfigStore::new(None);
        store.save(&UserConfig::default());
        assert!(store.load().dock.is_none());
    }

    #[test]
    fn recent_list_is_deduplicated_and_bounded() {
        let mut config = UserConfig::default();
        for i in 0..15 {
            config.remember_project("p", Path::new(&format!("/p/{i}")));
        }
        assert_eq!(config.recent.len(), MAX_RECENT);
        config.remember_project("p", Path::new("/p/10"));
        assert_eq!(config.recent[0].path, Path::new("/p/10"));
        assert_eq!(config.recent.len(), MAX_RECENT);
        config.forget_project(Path::new("/p/10"));
        assert_eq!(config.recent.len(), MAX_RECENT - 1);
    }
}
