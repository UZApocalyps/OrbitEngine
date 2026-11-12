//! Native editor plugins in the editor: loading the enabled ones at start-up
//! (`orbit_plugin_host`), watching them (a plugin that panics is disabled
//! with an error notice), unloading at exit, restarting the editor.
//!
//! The plugin manager panel (`builtin::plugins`) shares [`PluginsState`].

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use orbit_editor_api::{EditorRegistry, EditorState, Lang, Level};
use orbit_plugin_host::{EditorFacts, LoadedPlugin, PluginManager, Problem, ProcessProber, bundled_dir_for_exe};

/// What the editor knows about native plugins this session.
#[derive(Default)]
pub struct PluginsState {
    /// The user config folder (`None`: no plugins at all, e.g. tests).
    pub config_dir: Option<PathBuf>,
    /// Safe mode (`--no-plugins`): nothing loaded, the manager still works.
    pub safe_mode: bool,
    /// Plugins loaded this session (never unloaded before exit).
    pub loaded: Vec<LoadedPlugin>,
    /// Why enabled plugins are not running (start-up, crashes).
    pub problems: BTreeMap<String, Problem>,
    /// Plugin ids of the built-in tools (an installed plugin cannot take them).
    pub builtin_ids: BTreeSet<String>,
    /// Crashes already reported.
    reported: BTreeSet<String>,
    /// Something changed that only a restart applies.
    pub restart_needed: bool,
    /// The plugin that crashed the editor while loading last time.
    pub crashed_at_load: Option<String>,
}

pub type SharedPlugins = Rc<RefCell<PluginsState>>;

impl PluginsState {
    /// The plugins folder manager, when there is a config folder.
    pub fn manager(&self) -> Option<PluginManager> {
        self.config_dir
            .as_deref()
            .map(|dir| PluginManager::for_config_dir(dir, EditorFacts::current()))
    }

    pub fn is_loaded(&self, id: &str) -> bool {
        self.loaded.iter().any(|p| p.id == id)
    }

    /// The loaded plugin `id` and whether it failed (panicked) since.
    pub fn failure(&self, id: &str) -> Option<String> {
        self.loaded.iter().find(|p| p.id == id).and_then(|p| p.guard.failure())
    }
}

/// Loads the enabled, verified plugins into `registry` (start-up, after the
/// built-in tools). Each library is first inspected in a child process
/// (`orbit-editor --probe-plugin`), unless an earlier probe of the same file
/// by the same editor build is cached.
pub fn load_at_startup(state: &mut PluginsState, registry: &mut EditorRegistry, lang: Lang) {
    state.builtin_ids = registry
        .project
        .plugins()
        .iter()
        .map(|m| m.id.clone())
        .collect();
    let Some(mut manager) = state.manager() else { return };
    state.crashed_at_load = manager.prepare_startup();
    if state.safe_mode {
        log::info!("safe mode: native plugins are not loaded");
        return;
    }
    let Some(prober) = ProcessProber::current_exe() else {
        log::error!("cannot find the editor executable to inspect plugins");
        return;
    };
    install_bundled(state, &mut manager, &prober);
    if manager.installed().iter().all(|p| !p.record.enabled) {
        return;
    }
    let started = std::time::Instant::now();
    let report = manager.load_enabled(&prober, registry, lang, state.config_dir.as_deref());
    log::info!(
        target: "orbit_editor::perf",
        "native plugins: {} loaded, {} not loaded in {:.0} ms",
        report.loaded.len(),
        report.problems.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    for (id, problem) in &report.problems {
        log::warn!("plugin {id}: {problem}");
    }
    state.loaded = report.loaded;
    state.problems = report.problems;
    // Already reported as a load problem.
    for p in &state.loaded {
        if p.guard.is_failed() {
            state.reported.insert(p.id.clone());
        }
    }
}

/// Installs / updates the plugins shipped with the editor (`bundled-plugins/`
/// next to the executable, nowhere else), before the loading: they run from
/// this very start. Nothing to do (and no message) without that folder.
fn install_bundled(state: &PluginsState, manager: &mut PluginManager, prober: &ProcessProber) {
    let Some(dir) = std::env::current_exe().ok().and_then(|exe| bundled_dir_for_exe(&exe)) else { return };
    if !dir.is_dir() {
        return;
    }
    // Logged by the manager (one line per install, update or skipped package).
    manager.install_bundled(&dir, prober, &|id| state.builtin_ids.contains(id));
}

/// Each frame: a plugin that panicked since the last check is reported once
/// (toast, console) and disabled for the next starts. Cheap: one atomic load
/// per loaded plugin.
pub fn check_health(state: &mut PluginsState, editor: &mut EditorState) {
    let mut failed = Vec::new();
    for p in &state.loaded {
        if p.guard.is_failed() && !state.reported.contains(&p.id) {
            failed.push((p.id.clone(), p.guard.failure().unwrap_or_default()));
        }
    }
    if failed.is_empty() {
        return;
    }
    let mut manager = state.manager();
    for (id, message) in failed {
        state.reported.insert(id.clone());
        state.problems.insert(id.clone(), Problem::Panicked(message.clone()));
        if let Some(m) = manager.as_mut() {
            let _ = m.record_failure(&id, &message);
        }
        let text = editor.tf("plugins.crashed", &[("id", &id), ("error", &message)]);
        editor.push_toast(Level::Error, text);
    }
}

/// Calls `on_unload` of every loaded plugin (editor exit). The libraries
/// stay mapped until the process ends.
pub fn unload_all(state: &mut PluginsState) {
    for mut p in state.loaded.drain(..) {
        if !p.guard.is_failed() {
            p.plugin.on_unload();
        }
        // The plugin object's code lives in the library: never dropped.
        std::mem::forget(p.plugin);
    }
}

/// Starts a new editor process (same executable and config folder),
/// reopening `project`. The caller then quits.
pub fn spawn_restart(project: Option<&Path>, config_dir: Option<&Path>) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut command = std::process::Command::new(exe);
    if let Some(dir) = config_dir {
        command.arg("--config-dir").arg(dir);
    }
    if let Some(project) = project {
        command.arg(project);
    }
    command.spawn().map(|_| ())
}
