//! The plugins folder (`<editor config>/plugins/`): one folder per installed
//! plugin (`<id>/` with `manifest.ron`, the library, `locales/`, `assets/`
//! and the plugin's own `data/`), the store (`plugins.ron`), and the
//! start-up loading of the enabled, verified plugins.
//!
//! Nothing is ever unloaded while the editor runs: enabling, disabling,
//! updating and uninstalling take effect at the next start (a library in
//! use cannot even be deleted on Windows; such changes are staged).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use orbit_editor_api::native::{DescriptorInfo, NativePlugin, PluginHost};
use orbit_editor_api::{EditorRegistry, Lang, PluginGuard, PluginManifest};

use crate::binary::{check_library_file, library_extension};
use crate::manifest::{MANIFEST_FILE, PackageManifest};
use crate::store::{PluginRecord, PluginStore, ProbeCache};
use crate::verify::{EditorFacts, Problem, Prober, valid_id, verify};

pub mod bundled;

/// Extension of a plugin package (a zip archive).
pub const PACKAGE_EXTENSION: &str = "orbitplugin";
/// Folder (in the plugins folder) of the plugins being installed.
const INCOMING: &str = ".incoming";
/// Folder of the updates waiting for the next start.
const STAGED: &str = ".staged";
/// Written while a plugin loads: found at the next start, it names the plugin
/// that took the editor down.
const LOADING_MARKER: &str = ".loading";
/// The plugin's own data folder, kept across updates.
pub const DATA_DIR: &str = "data";
/// Largest package accepted (uncompressed).
const MAX_PACKAGE_BYTES: u64 = 1 << 30;

/// A plugin folder as found on disk.
#[derive(Clone, Debug)]
pub struct InstalledPlugin {
    pub manifest: PackageManifest,
    pub dir: PathBuf,
    pub library: PathBuf,
    pub record: PluginRecord,
    /// What is wrong with the folder itself (no library...).
    pub problem: Option<Problem>,
}

impl InstalledPlugin {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }
}

/// Why an install did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Not a `.orbitplugin` nor a library of this platform.
    Unsupported(String),
    Io(String),
    /// The package is broken (no manifest, unsafe paths, too big...).
    Package(String),
    /// The library does not pass the checks.
    Rejected(Problem),
    /// The id belongs to a built-in tool.
    Reserved(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(e) => write!(f, "unsupported file `{e}`"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Package(e) => write!(f, "invalid package: {e}"),
            Self::Rejected(p) => write!(f, "{p}"),
            Self::Reserved(id) => write!(f, "the id `{id}` belongs to a built-in tool"),
        }
    }
}

impl From<std::io::Error> for InstallError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

/// A successful install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub version: String,
    /// It replaced an installed version.
    pub updated: bool,
    /// It waits in the staging folder (the old version is loaded).
    pub staged: bool,
}

/// A plugin loaded by [`PluginManager::load_enabled`].
pub struct LoadedPlugin {
    pub id: String,
    pub plugin: Box<dyn NativePlugin>,
    pub guard: PluginGuard,
}

/// What the start-up loading did.
#[derive(Default)]
pub struct StartupReport {
    pub loaded: Vec<LoadedPlugin>,
    /// Why each enabled plugin that is not running is not.
    pub problems: BTreeMap<String, Problem>,
}

fn mtime(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    Ok(crate::sha256::file_hex(path, &|| false)?.unwrap_or_default())
}

/// Moves the entries of `from` into `to`, replacing what `to` held except
/// its `data/` folder, then removes `from`.
fn replace_contents(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(to)?.flatten() {
        if entry.file_name() == DATA_DIR {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    for entry in std::fs::read_dir(from)?.flatten() {
        if entry.file_name() == DATA_DIR {
            continue;
        }
        std::fs::rename(entry.path(), to.join(entry.file_name()))?;
    }
    std::fs::remove_dir_all(from)
}

/// Extracts a package into `dest` (paths checked, size bounded).
fn extract_package(file: &Path, dest: &Path) -> Result<(), InstallError> {
    let reader = std::fs::File::open(file)?;
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| InstallError::Package(e.to_string()))?;
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| InstallError::Package(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(InstallError::Package(format!("unsafe path `{}`", entry.name())));
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        total += entry.size();
        if total > MAX_PACKAGE_BYTES {
            return Err(InstallError::Package("larger than 1 GiB".into()));
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.by_ref().take(MAX_PACKAGE_BYTES).read_to_end(&mut bytes)?;
        std::fs::write(&out, bytes)?;
    }
    Ok(())
}

/// The manifest folder of an extracted package: its root, or its only
/// sub-folder.
fn package_root(dir: &Path) -> Option<PathBuf> {
    if dir.join(MANIFEST_FILE).is_file() {
        return Some(dir.to_path_buf());
    }
    let subs: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    match subs.as_slice() {
        [one] if one.join(MANIFEST_FILE).is_file() => Some(one.clone()),
        _ => None,
    }
}

/// The installed plugins and their store.
pub struct PluginManager {
    root: PathBuf,
    store: PluginStore,
    editor: EditorFacts,
}

impl PluginManager {
    /// The plugins folder `root` (created when something is installed).
    pub fn new(root: PathBuf, editor: EditorFacts) -> Self {
        let store = PluginStore::load(&root);
        Self { root, store, editor }
    }

    /// `<config_dir>/plugins`.
    pub fn for_config_dir(config_dir: &Path, editor: EditorFacts) -> Self {
        Self::new(config_dir.join("plugins"), editor)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn store(&self) -> &PluginStore {
        &self.store
    }

    pub fn editor(&self) -> &EditorFacts {
        &self.editor
    }

    fn save(&self) -> std::io::Result<()> {
        self.store.save(&self.root)
    }

    /// The plugin folders, sorted by name (folders starting with `.` are
    /// the manager's own).
    pub fn installed(&self) -> Vec<InstalledPlugin> {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return Vec::new() };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let dir = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !dir.is_dir() || name.starts_with('.') {
                continue;
            }
            let (manifest, problem) = match std::fs::read_to_string(dir.join(MANIFEST_FILE))
                .map_err(|e| e.to_string())
                .and_then(|t| PackageManifest::parse(&t))
            {
                Ok(m) => (m, None),
                Err(e) => (
                    PackageManifest {
                        id: name.clone(),
                        name: name.clone(),
                        version: String::new(),
                        description: String::new(),
                        author: String::new(),
                        library: String::new(),
                        min_editor: None,
                        max_editor: None,
                        homepage: None,
                    },
                    Some(Problem::Missing(format!("{MANIFEST_FILE}: {e}"))),
                ),
            };
            let library = dir.join(&manifest.library);
            let problem = problem.or_else(|| {
                if manifest.id != name {
                    Some(Problem::IdMismatch { manifest: manifest.id.clone(), library: name.clone() })
                } else if !library.is_file() {
                    Some(Problem::Missing(manifest.library.clone()))
                } else {
                    None
                }
            });
            let record = self.store.plugins.get(&manifest.id).cloned().unwrap_or_default();
            out.push(InstalledPlugin { manifest, dir, library, record, problem });
        }
        out.sort_by_key(|p| p.manifest.name.to_lowercase());
        out
    }

    /// Enables or disables a plugin (next start). Enabling clears the error
    /// that disabled it.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> std::io::Result<()> {
        let record = self.store.plugins.entry(id.to_owned()).or_default();
        record.enabled = enabled;
        if enabled {
            record.error = None;
        }
        self.save()
    }

    /// Disables a plugin after a crash, keeping the reason.
    pub fn record_failure(&mut self, id: &str, message: &str) -> std::io::Result<()> {
        let record = self.store.plugins.entry(id.to_owned()).or_default();
        record.enabled = false;
        record.error = Some(message.to_owned());
        self.save()
    }

    /// Installs (or updates) a plugin from a `.orbitplugin` package or a bare
    /// library. The library is checked (format, descriptor in `prober`,
    /// fingerprint, manifest) before anything replaces an installed plugin.
    /// `reserved(id)`: the id belongs to a built-in tool. `loaded(id)`: that
    /// plugin runs now, so the new version is staged for the next start.
    pub fn install(
        &mut self,
        file: &Path,
        prober: &dyn Prober,
        reserved: &dyn Fn(&str) -> bool,
        loaded: &dyn Fn(&str) -> bool,
    ) -> Result<Installed, InstallError> {
        let ext = file
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let nanos = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let incoming = self.root.join(INCOMING).join(format!("{nanos}"));
        std::fs::create_dir_all(&incoming)?;
        let result = self.install_into(file, &ext, &incoming, prober, reserved, loaded);
        let _ = std::fs::remove_dir_all(&incoming);
        result
    }

    fn install_into(
        &mut self,
        file: &Path,
        ext: &str,
        incoming: &Path,
        prober: &dyn Prober,
        reserved: &dyn Fn(&str) -> bool,
        loaded: &dyn Fn(&str) -> bool,
    ) -> Result<Installed, InstallError> {
        let (package, manifest) = if ext == PACKAGE_EXTENSION || ext == "zip" {
            let unpacked = incoming.join("package");
            extract_package(file, &unpacked)?;
            let root = package_root(&unpacked)
                .ok_or_else(|| InstallError::Package(format!("no {MANIFEST_FILE}")))?;
            let text = std::fs::read_to_string(root.join(MANIFEST_FILE))?;
            let manifest = PackageManifest::parse(&text).map_err(InstallError::Package)?;
            (root, Some(manifest))
        } else if ext == library_extension() {
            let root = incoming.join("package");
            std::fs::create_dir_all(&root)?;
            let name = file
                .file_name()
                .ok_or_else(|| InstallError::Unsupported(file.display().to_string()))?;
            std::fs::copy(file, root.join(name))?;
            (root, None)
        } else {
            return Err(InstallError::Unsupported(file.display().to_string()));
        };

        let library_name = match &manifest {
            Some(m) => m.library.clone(),
            None => file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        let library = package.join(&library_name);
        if !library.is_file() {
            return Err(InstallError::Package(format!("library `{library_name}` missing")));
        }
        check_library_file(&library).map_err(|e| InstallError::Rejected(Problem::Binary(e)))?;
        let info = prober.probe(&library).map_err(InstallError::Rejected)?;
        let manifest = manifest.unwrap_or_else(|| PackageManifest {
            id: info.id.clone(),
            name: info.name.clone(),
            version: info.version.clone(),
            description: info.description.clone(),
            author: info.author.clone(),
            library: library_name.clone(),
            min_editor: None,
            max_editor: None,
            homepage: None,
        });
        verify(&info, Some(&manifest), &self.editor).map_err(InstallError::Rejected)?;
        if reserved(&info.id) {
            return Err(InstallError::Reserved(info.id.clone()));
        }
        std::fs::write(package.join(MANIFEST_FILE), manifest.to_ron())?;

        let meta = std::fs::metadata(&library)?;
        let sha = sha256_of(&library)?;
        let id = info.id.clone();
        let dest = self.root.join(&id);
        let updated = dest.exists();
        let staged = loaded(&id);
        if staged {
            let stage = self.root.join(STAGED).join(&id);
            if stage.exists() {
                std::fs::remove_dir_all(&stage)?;
            }
            std::fs::create_dir_all(&stage)?;
            replace_contents(&package, &stage)?;
        } else {
            replace_contents(&package, &dest)?;
        }
        // Size and time as they will be in the plugin folder (rename keeps them).
        let modified = std::fs::metadata(if staged {
            self.root.join(STAGED).join(&id).join(&library_name)
        } else {
            dest.join(&library_name)
        })
        .map_or(mtime(&meta), |m| mtime(&m));
        let record = self.store.plugins.entry(id.clone()).or_default();
        record.enabled = true;
        record.error = None;
        record.uninstall = false;
        record.staged = staged;
        // Any install makes the plugin the user's (bundled installs mark it after).
        record.bundled = false;
        record.sha256 = sha.clone();
        record.size = meta.len();
        record.modified = modified;
        record.probe = Some(ProbeCache {
            key: format!("{}|{sha}", self.editor.cache_key()),
            info: info.clone(),
        });
        self.save()?;
        Ok(Installed {
            id,
            name: manifest.name,
            version: manifest.version,
            updated,
            staged,
        })
    }

    /// Uninstalls a plugin: its folder (data included) is removed now, or at
    /// the next start when it is `loaded`.
    pub fn uninstall(&mut self, id: &str, loaded: bool) -> std::io::Result<()> {
        if !valid_id(id) {
            return Err(std::io::Error::other(format!("invalid id `{id}`")));
        }
        if loaded {
            let record = self.store.plugins.entry(id.to_owned()).or_default();
            record.uninstall = true;
            record.enabled = false;
        } else {
            let dir = self.root.join(id);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            let _ = std::fs::remove_dir_all(self.root.join(STAGED).join(id));
            self.store.plugins.remove(id);
        }
        self.save()
    }

    /// Start of the editor, before loading: finishes staged updates and
    /// uninstalls, and disables the plugin that crashed the last load.
    /// Returns that plugin's id, if any.
    pub fn prepare_startup(&mut self) -> Option<String> {
        let _ = std::fs::remove_dir_all(self.root.join(INCOMING));
        let mut changed = false;
        let ids: Vec<String> = self.store.plugins.keys().cloned().collect();
        for id in ids {
            let Some(record) = self.store.plugins.get(&id).cloned() else { continue };
            if record.uninstall {
                let _ = std::fs::remove_dir_all(self.root.join(&id));
                let _ = std::fs::remove_dir_all(self.root.join(STAGED).join(&id));
                self.store.plugins.remove(&id);
                changed = true;
            } else if record.staged {
                let stage = self.root.join(STAGED).join(&id);
                if stage.is_dir() && replace_contents(&stage, &self.root.join(&id)).is_ok() {
                    if let Some(r) = self.store.plugins.get_mut(&id) {
                        r.staged = false;
                    }
                    changed = true;
                }
            }
        }
        let crashed = std::fs::read_to_string(self.root.join(LOADING_MARKER))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        if let Some(id) = &crashed {
            let record = self.store.plugins.entry(id.clone()).or_default();
            record.enabled = false;
            record.error = Some(Problem::CrashedAtLoad.to_string());
            changed = true;
            let _ = std::fs::remove_file(self.root.join(LOADING_MARKER));
        }
        if changed {
            let _ = self.save();
        }
        crashed
    }

    /// Checksum, probe (cached per editor build and file) and checks of an
    /// installed plugin. Updates the store's cache.
    pub fn check(&mut self, plugin: &InstalledPlugin, prober: &dyn Prober) -> Result<DescriptorInfo, Problem> {
        if let Some(p) = &plugin.problem {
            return Err(p.clone());
        }
        let meta = std::fs::metadata(&plugin.library).map_err(|e| Problem::Missing(e.to_string()))?;
        let id = plugin.id().to_owned();
        let mut record = self.store.plugins.get(&id).cloned().unwrap_or_default();
        let same_file = record.size == meta.len() && record.modified == mtime(&meta) && !record.sha256.is_empty();
        let sha = if same_file {
            record.sha256.clone()
        } else {
            let sha = sha256_of(&plugin.library).map_err(|e| Problem::Missing(e.to_string()))?;
            if record.sha256.is_empty() {
                // Copied by hand into the plugins folder: recorded as is.
                record.sha256 = sha.clone();
            } else if record.sha256 != sha {
                return Err(Problem::Modified);
            }
            record.size = meta.len();
            record.modified = mtime(&meta);
            sha
        };
        let key = format!("{}|{sha}", self.editor.cache_key());
        let info = match &record.probe {
            Some(cache) if cache.key == key => cache.info.clone(),
            _ => {
                check_library_file(&plugin.library).map_err(Problem::Binary)?;
                let info = prober.probe(&plugin.library)?;
                record.probe = Some(ProbeCache { key, info: info.clone() });
                info
            }
        };
        let changed = self.store.plugins.get(&id) != Some(&record);
        self.store.plugins.insert(id, record);
        if changed {
            let _ = self.save();
        }
        verify(&info, Some(&plugin.manifest), &self.editor)?;
        Ok(info)
    }

    /// Loads every enabled plugin that passes [`Self::check`] into
    /// `registry` and runs its `on_load`. To be called once, at start-up,
    /// after the built-in tools are registered (their ids are taken).
    pub fn load_enabled(
        &mut self,
        prober: &dyn Prober,
        registry: &mut EditorRegistry,
        lang: Lang,
        editor_config_dir: Option<&Path>,
    ) -> StartupReport {
        let mut report = StartupReport::default();
        for plugin in self.installed() {
            let id = plugin.id().to_owned();
            if !plugin.record.enabled || plugin.record.uninstall {
                continue;
            }
            match self.load_one(&plugin, prober, registry, lang, editor_config_dir) {
                Ok(loaded) => {
                    if let Some(failure) = loaded.guard.failure() {
                        let problem = match failure.strip_prefix(INIT_PREFIX) {
                            Some(message) => Problem::InitFailed(message.to_owned()),
                            None => Problem::Panicked(failure),
                        };
                        report.problems.insert(id, problem);
                    }
                    report.loaded.push(loaded);
                }
                Err(problem) => {
                    log::warn!("plugin {id} not loaded: {problem}");
                    report.problems.insert(id, problem);
                }
            }
        }
        report
    }

    fn load_one(
        &mut self,
        plugin: &InstalledPlugin,
        prober: &dyn Prober,
        registry: &mut EditorRegistry,
        lang: Lang,
        editor_config_dir: Option<&Path>,
    ) -> Result<LoadedPlugin, Problem> {
        let id = plugin.id().to_owned();
        if registry.project.has_plugin(&id) {
            return Err(Problem::DuplicateId(id));
        }
        self.check(plugin, prober)?;
        let marker = self.root.join(LOADING_MARKER);
        let _ = std::fs::write(&marker, &id);
        let result = (|| {
            let open = crate::load::open(&plugin.library)?;
            verify(&open.info, Some(&plugin.manifest), &self.editor)?;
            let mut native = open.create()?;
            let manifest = PluginManifest::new(id.clone(), plugin.manifest.name.clone(), plugin.manifest.version.clone());
            registry.begin_plugin(&manifest).map_err(|e| match e {
                orbit_editor_api::PluginError::DuplicatePlugin(id) => Problem::DuplicateId(id),
                other => Problem::InvalidId(other.to_string()),
            })?;
            load_package_texts(&plugin.dir, registry);
            let data_dir = plugin.dir.join(DATA_DIR);
            let _ = std::fs::create_dir_all(&data_dir);
            let guard = PluginGuard::new(id.clone());
            let mut host = PluginHost::new(
                registry,
                plugin.dir.clone(),
                data_dir,
                editor_config_dir.map(Path::to_path_buf),
                lang,
                guard.clone(),
            );
            if let Err(message) = native.on_load(&mut host) {
                // Its elements stay registered but inert (failed guard).
                guard.fail(format!("{INIT_PREFIX}{message}"));
            }
            Ok(LoadedPlugin { id: id.clone(), plugin: native, guard })
        })();
        let _ = std::fs::remove_file(&marker);
        result
    }
}

/// Prefix of a guard failure that is an `on_load` error, not a panic.
const INIT_PREFIX: &str = "on_load: ";

/// The package's text tables, `locales/<lang>/*.ron`.
fn load_package_texts(dir: &Path, registry: &mut EditorRegistry) {
    for lang in Lang::ALL {
        let Ok(entries) = std::fs::read_dir(dir.join("locales").join(lang.code())) else { continue };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "ron"))
            .collect();
        files.sort();
        for file in files {
            if let Ok(text) = std::fs::read_to_string(&file) {
                registry.texts(lang, &text);
            }
        }
    }
}
