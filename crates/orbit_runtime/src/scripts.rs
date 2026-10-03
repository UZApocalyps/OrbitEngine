//! Loading the shared script modules of a game (ADR 0015, M8-01).
//!
//! A story that uses scripts imports modules by name (`import "shop"`):
//! the runtime fetches `scripts/<name>.rhai` through the story loader (the
//! pak of an exported game, the project folder otherwise), then the
//! modules those import, and so on. Only imported modules are fetched; a
//! missing one is logged and left out (the script importing it fails at
//! run time with a clear message, like any script error).

use std::collections::{BTreeMap, BTreeSet};

use orbit_assets::{AssetHandle, AssetLoader, AssetPoll};
use orbit_core::Story;
use orbit_core::scripting::{module_path, rhai_imports, script_sources};

/// Modules being fetched.
pub(crate) struct ScriptLoad {
    pending: Vec<(String, AssetHandle<Vec<u8>>)>,
    requested: BTreeSet<String>,
    pub(crate) modules: BTreeMap<String, String>,
}

impl ScriptLoad {
    /// Starts fetching the modules imported by the story's scripts.
    pub(crate) fn start(loader: &AssetLoader, story: &Story) -> Self {
        let mut load = Self {
            pending: Vec::new(),
            requested: BTreeSet::new(),
            modules: BTreeMap::new(),
        };
        let sources: Vec<String> = script_sources(story).into_iter().map(str::to_owned).collect();
        for source in &sources {
            load.request_imports(loader, source);
        }
        load
    }

    fn request_imports(&mut self, loader: &AssetLoader, source: &str) {
        for (name, ..) in rhai_imports(source) {
            if orbit_core::scripting::is_valid_module_name(&name) && self.requested.insert(name.clone()) {
                let handle = loader.load_bytes(&module_path(&name));
                self.pending.push((name, handle));
            }
        }
    }

    /// Polls the fetches; `true` once every module (and the modules they
    /// import) arrived or failed.
    pub(crate) fn poll(&mut self, loader: &AssetLoader) -> bool {
        let mut i = 0;
        while i < self.pending.len() {
            match self.pending[i].1.poll() {
                AssetPoll::Loading => i += 1,
                AssetPoll::Failed(err) => {
                    let (name, _) = self.pending.remove(i);
                    log::warn!("script module `{name}` cannot be loaded: {err}");
                }
                AssetPoll::Ready(bytes) => {
                    let (name, _) = self.pending.remove(i);
                    match String::from_utf8(bytes.to_vec()) {
                        Ok(source) => {
                            self.request_imports(loader, &source);
                            self.modules.insert(name, source);
                        }
                        Err(_) => log::warn!("script module `{name}` is not UTF-8 text"),
                    }
                }
            }
        }
        self.pending.is_empty()
    }
}

/// The script engine of a game with these modules, when this runtime can
/// run scripts.
#[cfg(feature = "script")]
pub(crate) fn engine(modules: BTreeMap<String, String>) -> Option<orbit_vm::SharedScriptHost> {
    Some(orbit_script::ScriptEngine::shared(modules))
}

#[cfg(not(feature = "script"))]
pub(crate) fn engine(_modules: BTreeMap<String, String>) -> Option<orbit_vm::SharedScriptHost> {
    None
}

/// The message of a game that needs scripts this player cannot run.
pub(crate) const NO_SCRIPTING: &str = "This game uses scripts (Rhai), but this player was built \
without scripting. Export the game again with this version of OrbitEngine.";
