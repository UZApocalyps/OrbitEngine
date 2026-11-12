//! Plugins shipped with the editor: `.orbitplugin` packages in a
//! `bundled-plugins/` folder next to the executable (the installers and the
//! portable archive put them there). At start-up, after the staged changes
//! and before the loading, [`PluginManager::install_bundled`]:
//!
//! * installs a shipped package whose id was never offered before, through
//!   [`PluginManager::install`] (same checks as an install from a file:
//!   format, descriptor, fingerprint, manifest, SHA-256 recorded), enabled,
//!   so that it loads at that very start;
//! * never installs it again once offered (`PluginStore::bundled_offered`):
//!   uninstalling or disabling a shipped plugin is the user's choice;
//! * replaces an installed plugin by the shipped package only when that
//!   plugin itself came from a shipped package (`PluginRecord::bundled`) and
//!   the package has a strictly newer version, or the same version but the
//!   installed build no longer fits this editor (changed fingerprint or
//!   editor range: the shipped build is the compatible one);
//! * leaves a plugin installed by hand or from the store alone.
//!
//! Only the one folder next to the executable is read: it is as trusted as
//! the executable itself, and nothing is relaxed for it. A shipped package
//! that is invalid or incompatible is skipped with a log line.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::{Installed, PACKAGE_EXTENSION, PluginManager};
use crate::manifest::{MANIFEST_FILE, PackageManifest};
use crate::verify::{Problem, Prober, valid_id};

/// Folder of the shipped packages, next to the editor executable.
pub const BUNDLED_DIR: &str = "bundled-plugins";
/// Largest `manifest.ron` read from a shipped package.
const MAX_MANIFEST_BYTES: u64 = 1 << 20;

/// `<folder of exe>/bundled-plugins`.
pub fn bundled_dir_for_exe(exe: &Path) -> Option<PathBuf> {
    exe.parent().map(|dir| dir.join(BUNDLED_DIR))
}

/// What [`PluginManager::install_bundled`] did.
#[derive(Debug, Default)]
pub struct BundledReport {
    /// Installed for the first time (`updated == false`) or replaced by a
    /// newer shipped build (`updated == true`).
    pub installed: Vec<Installed>,
    /// Shipped packages left aside, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
}

/// `major.minor.patch...` as numbers (a `-pre` / `+build` suffix ignored).
fn version_parts(text: &str) -> Option<Vec<u64>> {
    let core = text.trim().split(['-', '+']).next()?;
    core.split('.').map(|p| p.parse().ok()).collect()
}

/// Compares two versions numerically; `None` when one is not a version.
fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let (mut a, mut b) = (version_parts(a)?, version_parts(b)?);
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    Some(a.cmp(&b))
}

/// The manifest of a package, read without extracting it (same place as
/// the installer looks: the root, or the only sub-folder).
fn peek_manifest(file: &Path) -> Result<PackageManifest, String> {
    let reader = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
    let mut candidates: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index(i) else { continue };
        let name = entry.name().replace('\\', "/");
        if name == MANIFEST_FILE {
            candidates.clear();
            candidates.push(name);
            break;
        }
        if name
            .strip_suffix(MANIFEST_FILE)
            .is_some_and(|dir| dir.ends_with('/') && dir.matches('/').count() == 1)
        {
            candidates.push(name);
        }
    }
    let [name] = candidates.as_slice() else {
        return Err(format!("no {MANIFEST_FILE}"));
    };
    let mut entry = archive.by_name(name).map_err(|e| e.to_string())?;
    let mut text = String::new();
    entry
        .by_ref()
        .take(MAX_MANIFEST_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    PackageManifest::parse(&text)
}

enum Action {
    /// Nothing to do.
    Leave,
    /// Remember the id as offered, install nothing.
    Mark,
    Install { update: bool, keep_disabled: bool },
}

impl PluginManager {
    /// Installs and updates the plugins shipped in `dir` (see the module
    /// documentation). A missing or empty `dir` does nothing. Meant for the
    /// start-up, after [`Self::prepare_startup`] and before
    /// [`Self::load_enabled`]: nothing is loaded yet, so installs and
    /// updates are applied at once and take effect in this very session.
    /// `reserved(id)`: the id belongs to a built-in tool.
    pub fn install_bundled(
        &mut self,
        dir: &Path,
        prober: &dyn Prober,
        reserved: &dyn Fn(&str) -> bool,
    ) -> BundledReport {
        let mut report = BundledReport::default();
        let Ok(entries) = std::fs::read_dir(dir) else { return report };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(PACKAGE_EXTENSION))
            })
            .collect();
        files.sort();
        let mut seen = BTreeSet::new();
        let skip = |report: &mut BundledReport, file: &Path, why: String| {
            log::warn!("bundled plugin {} skipped: {why}", file.display());
            report.skipped.push((file.to_path_buf(), why));
        };
        for file in files {
            let manifest = match peek_manifest(&file) {
                Ok(m) if valid_id(&m.id) => m,
                Ok(m) => {
                    skip(&mut report, &file, format!("invalid plugin id `{}`", m.id));
                    continue;
                }
                Err(e) => {
                    skip(&mut report, &file, format!("invalid package: {e}"));
                    continue;
                }
            };
            if !seen.insert(manifest.id.clone()) {
                skip(&mut report, &file, format!("another package already provides `{}`", manifest.id));
                continue;
            }
            match self.bundled_action(&manifest, prober) {
                Action::Leave => {}
                Action::Mark => self.mark_offered(&manifest.id),
                Action::Install { update, keep_disabled } => {
                    match self.install(&file, prober, reserved, &|_| false) {
                        Ok(done) => {
                            self.finish_bundled(&done.id, keep_disabled);
                            log::info!(
                                "bundled plugin {} {} {}",
                                done.id,
                                done.version,
                                if update { "updated" } else { "installed" }
                            );
                            report.installed.push(done);
                        }
                        Err(e) => skip(&mut report, &file, e.to_string()),
                    }
                }
            }
        }
        report
    }

    /// What to do with the shipped package `manifest`.
    fn bundled_action(&mut self, manifest: &PackageManifest, prober: &dyn Prober) -> Action {
        let id = manifest.id.as_str();
        let offered = self.store.bundled_offered.contains(id);
        let Some(installed) = self.installed().into_iter().find(|p| p.id() == id) else {
            // Not installed: new for the user, or removed by them.
            return if offered { Action::Leave } else { Action::Install { update: false, keep_disabled: false } };
        };
        if !offered && !installed.record.bundled {
            // Already there from before (by hand, store): not ours to update.
            return Action::Mark;
        }
        if !installed.record.bundled || installed.record.uninstall {
            return Action::Leave;
        }
        let order = compare_versions(&manifest.version, &installed.manifest.version);
        let newer = order == Some(Ordering::Greater);
        // The same version rebuilt for a new editor: the installed build
        // fails the fingerprint / range checks, the shipped one fits.
        let stale = order == Some(Ordering::Equal)
            && matches!(
                self.check(&installed, prober),
                Err(Problem::Incompatible(_) | Problem::EditorRange(_))
            );
        if newer || stale {
            // A plugin the user turned off stays off; one a crash turned
            // off (error recorded) gets its second chance with the new build.
            let keep_disabled = !installed.record.enabled && installed.record.error.is_none();
            Action::Install { update: true, keep_disabled }
        } else {
            Action::Leave
        }
    }

    fn mark_offered(&mut self, id: &str) {
        if self.store.bundled_offered.insert(id.to_owned()) {
            let _ = self.save();
        }
    }

    /// After a successful shipped install: origin and offered marks.
    fn finish_bundled(&mut self, id: &str, keep_disabled: bool) {
        let record = self.store.plugins.entry(id.to_owned()).or_default();
        record.bundled = true;
        if keep_disabled {
            record.enabled = false;
        }
        self.store.bundled_offered.insert(id.to_owned());
        let _ = self.save();
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use orbit_editor_api::native::DescriptorInfo;

    use super::*;
    use crate::binary::{Format, native_arch};
    use crate::store::{PluginRecord, PluginStore, STORE_FILE};
    use crate::verify::EditorFacts;

    fn temp(tag: &str) -> PathBuf {
        // Under the target folder, never the system temp folder.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bundled-tests").join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A header the platform's library check accepts, then the marker the
    /// fake prober reads: `ORBITFAKE <id> <version> <fingerprint>`.
    fn fake_library(id: &str, version: &str, fp: &str) -> Vec<u8> {
        let arm = native_arch() == "aarch64";
        let mut b = match Format::native() {
            Format::Pe => {
                let mut b = vec![0u8; 512];
                b[0..2].copy_from_slice(b"MZ");
                b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
                b[0x80..0x84].copy_from_slice(b"PE\0\0");
                let machine: u16 = if arm { 0xaa64 } else { 0x8664 };
                b[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
                b[0x80 + 22..0x80 + 24].copy_from_slice(&0x2022u16.to_le_bytes());
                b
            }
            Format::Elf => {
                let mut b = vec![0u8; 64];
                b[0..4].copy_from_slice(b"\x7fELF");
                b[4] = 2;
                b[5] = 1;
                b[16..18].copy_from_slice(&3u16.to_le_bytes());
                b[18..20].copy_from_slice(&(if arm { 183u16 } else { 62 }).to_le_bytes());
                b
            }
            _ => {
                let mut b = vec![0u8; 32];
                b[0..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
                let cpu: u32 = if arm { 0x0100_000c } else { 0x0100_0007 };
                b[4..8].copy_from_slice(&cpu.to_le_bytes());
                b[12..16].copy_from_slice(&6u32.to_le_bytes());
                b
            }
        };
        b.extend_from_slice(format!("\nORBITFAKE {id} {version} {fp}\n").as_bytes());
        b
    }

    struct FakeProber;

    impl Prober for FakeProber {
        fn probe(&self, library: &Path) -> Result<DescriptorInfo, Problem> {
            let bytes = std::fs::read(library).map_err(|e| Problem::ProbeFailed(e.to_string()))?;
            let text = String::from_utf8_lossy(&bytes);
            let line = text
                .lines()
                .find_map(|l| l.strip_prefix("ORBITFAKE "))
                .ok_or_else(|| Problem::BadDescriptor("no marker".into()))?;
            let parts: Vec<&str> = line.split(' ').collect();
            let mut fingerprint = orbit_editor_api::native::Fingerprint::current();
            fingerprint.layout_hash = parts[2].to_owned();
            Ok(DescriptorInfo {
                id: parts[0].to_owned(),
                name: parts[0].to_owned(),
                version: parts[1].to_owned(),
                description: String::new(),
                author: String::new(),
                fingerprint,
            })
        }
    }

    fn facts(fp: &str) -> EditorFacts {
        let mut f = EditorFacts::current();
        f.fingerprint.layout_hash = fp.to_owned();
        f
    }

    fn package(dir: &Path, id: &str, version: &str, fp: &str) -> PathBuf {
        let lib = format!("{id}.lib");
        let path = dir.join(format!("{id}-{version}-{fp}.orbitplugin"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.ron", options).unwrap();
        zip.write_all(format!("(id: \"{id}\", name: \"{id}\", version: \"{version}\", library: \"{lib}\")").as_bytes())
            .unwrap();
        zip.start_file(lib.as_str(), options).unwrap();
        zip.write_all(&fake_library(id, version, fp)).unwrap();
        zip.finish().unwrap();
        path
    }

    fn fresh(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
    }

    fn manager(root: &Path, fp: &str) -> PluginManager {
        PluginManager::new(root.join("plugins"), facts(fp))
    }

    fn run(m: &mut PluginManager, bundled: &Path) -> BundledReport {
        m.install_bundled(bundled, &FakeProber, &|id| id == "orbit.builtin")
    }

    fn version(m: &PluginManager, id: &str) -> String {
        m.installed().iter().find(|p| p.id() == id).unwrap().manifest.version.clone()
    }

    #[test]
    fn missing_or_empty_folder_does_nothing() {
        let root = temp("empty");
        let mut m = manager(&root, "a");
        let report = run(&mut m, &root.join("nope"));
        assert!(report.installed.is_empty() && report.skipped.is_empty());
        std::fs::create_dir_all(root.join("bundled")).unwrap();
        let report = run(&mut m, &root.join("bundled"));
        assert!(report.installed.is_empty() && report.skipped.is_empty());
        assert!(!root.join("plugins").exists());
    }

    #[test]
    fn first_install_is_enabled_marked_and_recorded() {
        let root = temp("first");
        let bundled = root.join("bundled");
        fresh(&bundled);
        package(&bundled, "acme.one", "1.0.0", "a");
        let mut m = manager(&root, "a");
        let report = run(&mut m, &bundled);
        assert_eq!(report.installed.len(), 1);
        assert!(!report.installed[0].updated && !report.installed[0].staged);
        let installed = m.installed();
        assert_eq!(installed.len(), 1);
        let r = &installed[0].record;
        assert!(r.enabled && r.bundled && !r.sha256.is_empty() && r.probe.is_some());
        assert!(m.store().bundled_offered.contains("acme.one"));
        // Written to plugins.ron, and a second run changes nothing.
        let reread = PluginStore::load(m.root());
        assert!(reread.bundled_offered.contains("acme.one") && reread.plugins["acme.one"].bundled);
        let again = run(&mut m, &bundled);
        assert!(again.installed.is_empty() && again.skipped.is_empty());
    }

    #[test]
    fn an_uninstalled_or_disabled_plugin_does_not_come_back() {
        let root = temp("removed");
        let bundled = root.join("bundled");
        fresh(&bundled);
        package(&bundled, "acme.one", "1.0.0", "a");
        package(&bundled, "acme.two", "1.0.0", "a");
        let mut m = manager(&root, "a");
        run(&mut m, &bundled);
        m.uninstall("acme.one", false).unwrap();
        m.set_enabled("acme.two", false).unwrap();
        // A fresh manager, as after a restart.
        let mut m = manager(&root, "a");
        let report = run(&mut m, &bundled);
        assert!(report.installed.is_empty(), "{report:?}");
        let ids: Vec<String> = m.installed().iter().map(|p| p.id().to_owned()).collect();
        assert_eq!(ids, ["acme.two"]);
        assert!(!m.store().plugins["acme.two"].enabled);
        // Uninstalled while loaded: removed at the next start, still not back.
        let mut m = manager(&root, "a");
        m.uninstall("acme.two", true).unwrap();
        let mut m = manager(&root, "a");
        m.prepare_startup();
        let report = run(&mut m, &bundled);
        assert!(report.installed.is_empty());
        assert!(m.installed().is_empty());
    }

    #[test]
    fn updates_only_bundled_origin_and_strictly_newer() {
        let root = temp("update");
        let bundled = root.join("bundled");
        let v1 = root.join("v1");
        fresh(&bundled);
        fresh(&v1);
        let mut m = manager(&root, "a");
        // `one` comes from a shipped package, `hand` was installed by hand.
        package(&v1, "acme.one", "1.0.0", "a");
        let hand = package(&v1, "acme.hand", "1.0.0", "a");
        let man = root.join("man");
        fresh(&man);
        let manual = package(&man, "acme.one", "3.0.0", "a");
        run(&mut m, &v1);
        m.uninstall("acme.hand", false).unwrap();
        m.install(&hand, &FakeProber, &|_| false, &|_| false).unwrap();
        assert!(!m.store().plugins["acme.hand"].bundled);

        // Shipped: same version for `one`, newer for `hand` (not shipped-origin).
        package(&bundled, "acme.one", "1.0.0", "a");
        package(&bundled, "acme.hand", "2.0.0", "a");
        let mut m = manager(&root, "a");
        let report = run(&mut m, &bundled);
        assert!(report.installed.is_empty(), "{report:?}");
        assert_eq!(version(&m, "acme.hand"), "1.0.0");
        assert_eq!(version(&m, "acme.one"), "1.0.0");

        // Newer for `one`: updated, and the user's "disabled" is kept.
        fresh(&bundled);
        package(&bundled, "acme.one", "1.10.0", "a");
        package(&bundled, "acme.hand", "2.0.0", "a");
        m.set_enabled("acme.one", false).unwrap();
        let report = run(&mut m, &bundled);
        assert_eq!(report.installed.len(), 1);
        assert!(report.installed[0].updated && report.installed[0].id == "acme.one");
        assert_eq!(version(&m, "acme.one"), "1.10.0");
        assert_eq!(version(&m, "acme.hand"), "1.0.0");
        let r = &m.store().plugins["acme.one"];
        assert!(!r.enabled && r.bundled);

        // Older shipped package: nothing.
        fresh(&bundled);
        package(&bundled, "acme.one", "1.9.0", "a");
        assert!(run(&mut m, &bundled).installed.is_empty());
        assert_eq!(version(&m, "acme.one"), "1.10.0");

        // A manual install over a shipped plugin makes it the user's.
        m.install(&manual, &FakeProber, &|_| false, &|_| false).unwrap();
        assert!(!m.store().plugins["acme.one"].bundled);
        fresh(&bundled);
        package(&bundled, "acme.one", "4.0.0", "a");
        assert!(run(&mut m, &bundled).installed.is_empty());
        assert_eq!(version(&m, "acme.one"), "3.0.0");
    }

    #[test]
    fn same_version_is_replaced_when_the_editor_build_changed() {
        let root = temp("rebuilt");
        let old = root.join("old");
        let new = root.join("new");
        fresh(&old);
        fresh(&new);
        package(&old, "acme.one", "1.0.0", "old");
        let mut m = manager(&root, "old");
        run(&mut m, &old);
        // The editor is updated: its fingerprint changes, the shipped build fits it.
        package(&new, "acme.one", "1.0.0", "new");
        let mut m = manager(&root, "new");
        let report = run(&mut m, &new);
        assert_eq!(report.installed.len(), 1, "{report:?}");
        assert!(report.installed[0].updated);
        let plugin = m.installed().remove(0);
        assert!(m.check(&plugin, &FakeProber).is_ok());
        // Compatible and unchanged: nothing more.
        assert!(run(&mut m, &new).installed.is_empty());
    }

    #[test]
    fn invalid_or_incompatible_packages_are_skipped() {
        let root = temp("invalid");
        let bundled = root.join("bundled");
        fresh(&bundled);
        std::fs::write(bundled.join("broken.orbitplugin"), b"not a zip").unwrap();
        std::fs::write(bundled.join("notes.txt"), b"ignored").unwrap();
        package(&bundled, "acme.stale", "1.0.0", "other-build");
        package(&bundled, "orbit.builtin", "1.0.0", "a");
        package(&bundled, "acme.good", "1.0.0", "a");
        let mut m = manager(&root, "a");
        let report = run(&mut m, &bundled);
        let ids: Vec<&str> = report.installed.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["acme.good"]);
        assert_eq!(report.skipped.len(), 3, "{report:?}");
        // The failed ones are not marked: they are retried with a fixed build.
        assert!(!m.store().bundled_offered.contains("acme.stale"));
        assert!(m.installed().iter().all(|p| p.id() == "acme.good"));
    }

    #[test]
    fn plugins_ron_compatibility_both_ways() {
        // A file written by an earlier editor (no new field) still loads.
        let root = temp("compat");
        std::fs::write(
            root.join(STORE_FILE),
            "(plugins: {\"acme.one\": (enabled: true, sha256: \"ab\", size: 3, modified: 4)})",
        )
        .unwrap();
        let store = PluginStore::load(&root);
        assert!(store.plugins["acme.one"].enabled && !store.plugins["acme.one"].bundled);
        assert!(store.bundled_offered.is_empty());

        // An earlier editor reading the new file: its structs (without the
        // fields) must still deserialize, unknown fields being ignored.
        #[derive(Debug, serde::Deserialize)]
        struct OldRecord {
            enabled: bool,
        }
        #[derive(Debug, serde::Deserialize)]
        struct OldStore {
            #[serde(default)]
            plugins: std::collections::BTreeMap<String, OldRecord>,
        }
        let mut new = PluginStore::default();
        new.bundled_offered.insert("acme.one".into());
        new.plugins
            .insert("acme.one".into(), PluginRecord { enabled: true, bundled: true, ..Default::default() });
        new.save(&root).unwrap();
        let text = std::fs::read_to_string(root.join(STORE_FILE)).unwrap();
        assert!(text.contains("bundled_offered") && text.contains("bundled: true"), "{text}");
        let old: OldStore = ron::from_str(&text).expect("an earlier editor reads the new file");
        assert!(old.plugins["acme.one"].enabled);
    }

    #[test]
    fn version_comparison() {
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Some(Ordering::Greater));
        assert_eq!(compare_versions("1.0", "1.0.0"), Some(Ordering::Equal));
        assert_eq!(compare_versions("0.4.1-dev", "0.4.0"), Some(Ordering::Greater));
        assert_eq!(compare_versions("soon", "1.0.0"), None);
    }
}
