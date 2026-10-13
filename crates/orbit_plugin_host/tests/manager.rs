//! The plugins folder without real libraries: a file with a valid header for
//! this platform and a fake prober stand in for a plugin. Covers install
//! (library and package), checks, enable / disable persistence, staged
//! updates, uninstall, the crash marker and the checksum.

use std::io::Write;
use std::path::{Path, PathBuf};

use orbit_editor_api::native::{DescriptorInfo, Fingerprint};
use orbit_plugin_host::binary::{Format, library_extension, native_arch};
use orbit_plugin_host::store::PluginStore;
use orbit_plugin_host::{EditorFacts, InstallError, PluginManager, Problem, Prober, RangeError};

/// A fresh folder under the target directory (never the system temp).
fn temp(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugin-host").join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Bytes that pass `binary::check_header` here (only the header matters).
fn fake_library() -> Vec<u8> {
    match Format::native() {
        Format::Pe => {
            let mut b = vec![0u8; 1024];
            b[0..2].copy_from_slice(b"MZ");
            b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
            b[0x80..0x84].copy_from_slice(b"PE\0\0");
            let machine: u16 = if native_arch() == "aarch64" { 0xaa64 } else { 0x8664 };
            b[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
            b[0x80 + 22..0x80 + 24].copy_from_slice(&0x2022u16.to_le_bytes());
            b
        }
        Format::Elf => {
            let mut b = vec![0u8; 1024];
            b[0..4].copy_from_slice(b"\x7fELF");
            b[4] = 2;
            b[5] = 1;
            b[16..18].copy_from_slice(&3u16.to_le_bytes());
            let machine: u16 = if native_arch() == "aarch64" { 183 } else { 62 };
            b[18..20].copy_from_slice(&machine.to_le_bytes());
            b
        }
        _ => {
            let mut b = vec![0u8; 1024];
            b[0..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
            let cpu: u32 = if native_arch() == "aarch64" { 0x0100_000c } else { 0x0100_0007 };
            b[4..8].copy_from_slice(&cpu.to_le_bytes());
            b[12..16].copy_from_slice(&6u32.to_le_bytes());
            b
        }
    }
}

/// Answers for every library: the id is the file stem with `_` -> `.`.
struct FakeProber {
    fingerprint: Fingerprint,
}

impl FakeProber {
    fn matching() -> Self {
        Self { fingerprint: Fingerprint::current() }
    }
}

impl Prober for FakeProber {
    fn probe(&self, library: &Path) -> Result<DescriptorInfo, Problem> {
        let stem = library.file_stem().unwrap().to_string_lossy().replace('_', ".");
        Ok(DescriptorInfo {
            id: stem.clone(),
            name: format!("Plugin {stem}"),
            version: "1.2.3".into(),
            description: "A test plugin".into(),
            author: "Tests".into(),
            fingerprint: self.fingerprint.clone(),
        })
    }
}

fn library_file(dir: &Path, stem: &str) -> PathBuf {
    let path = dir.join(format!("{stem}.{}", library_extension()));
    std::fs::write(&path, fake_library()).unwrap();
    path
}

fn package(dir: &Path, stem: &str, manifest_extra: &str) -> PathBuf {
    let path = dir.join(format!("{stem}.orbitplugin"));
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    let lib = format!("{stem}.{}", library_extension());
    let id = stem.replace('_', ".");
    zip.start_file("manifest.ron", options).unwrap();
    write!(
        zip,
        "(id: \"{id}\", name: \"Packaged\", version: \"2.0.0\", author: \"Tests\", library: \"{lib}\"{manifest_extra})"
    )
    .unwrap();
    zip.start_file(lib.as_str(), options).unwrap();
    zip.write_all(&fake_library()).unwrap();
    zip.start_file("locales/en/x.ron", options).unwrap();
    zip.write_all(b"{ \"x.hello\": \"Hello\" }").unwrap();
    zip.finish().unwrap();
    path
}

fn none(_: &str) -> bool {
    false
}

#[test]
fn install_a_library_and_a_package_then_list_them() {
    let root = temp("install");
    let files = temp("install-files");
    let mut m = PluginManager::new(root.join("plugins"), EditorFacts::current());

    let done = m
        .install(&library_file(&files, "acme_one"), &FakeProber::matching(), &none, &none)
        .unwrap();
    assert_eq!((done.id.as_str(), done.updated, done.staged), ("acme.one", false, false));
    let done = m
        .install(&package(&files, "acme_two", ""), &FakeProber::matching(), &none, &none)
        .unwrap();
    assert_eq!(done.name, "Packaged");

    let list = m.installed();
    let ids: Vec<&str> = list.iter().map(|p| p.id()).collect();
    assert_eq!(ids, ["acme.two", "acme.one"], "sorted by name: Packaged, Plugin acme.one");
    for p in &list {
        assert!(p.problem.is_none(), "{:?}", p.problem);
        assert!(p.record.enabled, "installed plugins start enabled");
        assert_eq!(p.record.sha256.len(), 64);
        assert!(p.library.is_file());
    }
    // The package's other files came along; the bare library got a manifest.
    assert!(root.join("plugins/acme.two/locales/en/x.ron").is_file());
    let manifest = std::fs::read_to_string(root.join("plugins/acme.one/manifest.ron")).unwrap();
    assert!(manifest.contains("acme.one"), "{manifest}");
    // Nothing left in the staging folders.
    assert!(!root.join("plugins/.incoming").read_dir().is_ok_and(|mut d| d.next().is_some()));

    // The checks pass with the cached probe (no prober call needed).
    struct Never;
    impl Prober for Never {
        fn probe(&self, _: &Path) -> Result<DescriptorInfo, Problem> {
            panic!("the probe should be cached")
        }
    }
    for p in &list {
        m.check(p, &Never).unwrap();
    }
}

#[test]
fn refused_installs_leave_nothing_behind() {
    let root = temp("refused");
    let files = temp("refused-files");
    let mut m = PluginManager::new(root.join("plugins"), EditorFacts::current());

    let mut other = Fingerprint::current();
    other.rustc = "rustc 1.80.0 (old)".into();
    let err = m
        .install(&library_file(&files, "acme_old"), &FakeProber { fingerprint: other }, &none, &none)
        .unwrap_err();
    match err {
        InstallError::Rejected(Problem::Incompatible(m)) => assert_eq!(m[0].plugin, "rustc 1.80.0 (old)"),
        other => panic!("{other:?}"),
    }

    let err = m
        .install(&library_file(&files, "orbit_core"), &FakeProber::matching(), &|id| id == "orbit.core", &none)
        .unwrap_err();
    assert_eq!(err, InstallError::Reserved("orbit.core".into()));

    let err = m
        .install(&package(&files, "acme_future", ", min_editor: \"99.0.0\""), &FakeProber::matching(), &none, &none)
        .unwrap_err();
    assert!(matches!(err, InstallError::Rejected(Problem::EditorRange(RangeError::TooOld { .. }))), "{err:?}");

    let text = files.join("notes.txt");
    std::fs::write(&text, "hello").unwrap();
    assert!(matches!(m.install(&text, &FakeProber::matching(), &none, &none), Err(InstallError::Unsupported(_))));

    let fake = files.join(format!("acme_text.{}", library_extension()));
    std::fs::write(&fake, "not a library").unwrap();
    assert!(matches!(
        m.install(&fake, &FakeProber::matching(), &none, &none),
        Err(InstallError::Rejected(Problem::Binary(_)))
    ));

    // A package whose entries escape the folder.
    let evil = files.join("evil.orbitplugin");
    {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
        zip.start_file("../evil.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
    }
    assert!(matches!(m.install(&evil, &FakeProber::matching(), &none, &none), Err(InstallError::Package(_))));
    assert!(!root.join("evil.txt").exists());

    assert!(m.installed().is_empty());
}

#[test]
fn enabled_state_is_persisted() {
    let root = temp("enable");
    let files = temp("enable-files");
    let plugins = root.join("plugins");
    let mut m = PluginManager::new(plugins.clone(), EditorFacts::current());
    m.install(&library_file(&files, "acme_one"), &FakeProber::matching(), &none, &none)
        .unwrap();
    m.set_enabled("acme.one", false).unwrap();
    assert!(!PluginStore::load(&plugins).is_enabled("acme.one"));
    let m2 = PluginManager::new(plugins.clone(), EditorFacts::current());
    assert!(!m2.installed()[0].record.enabled);

    // A crash disables it with its reason; enabling clears the reason.
    let mut m = PluginManager::new(plugins.clone(), EditorFacts::current());
    m.set_enabled("acme.one", true).unwrap();
    m.record_failure("acme.one", "boom").unwrap();
    let r = &PluginStore::load(&plugins).plugins["acme.one"];
    assert!(!r.enabled);
    assert_eq!(r.error.as_deref(), Some("boom"));
    m.set_enabled("acme.one", true).unwrap();
    let r = &PluginStore::load(&plugins).plugins["acme.one"];
    assert!(r.enabled && r.error.is_none());

    // A broken store is an empty one (every plugin disabled).
    std::fs::write(plugins.join("plugins.ron"), "garbage(").unwrap();
    assert!(!PluginStore::load(&plugins).is_enabled("acme.one"));
}

#[test]
fn updates_and_uninstalls_of_a_loaded_plugin_wait_for_the_next_start() {
    let root = temp("staged");
    let files = temp("staged-files");
    let plugins = root.join("plugins");
    let mut m = PluginManager::new(plugins.clone(), EditorFacts::current());
    m.install(&library_file(&files, "acme_one"), &FakeProber::matching(), &none, &none)
        .unwrap();
    std::fs::create_dir_all(plugins.join("acme.one/data")).unwrap();
    std::fs::write(plugins.join("acme.one/data/settings.txt"), "keep me").unwrap();

    // Update while loaded: staged.
    let done = m
        .install(&package(&files, "acme_one", ""), &FakeProber::matching(), &none, &|id| id == "acme.one")
        .unwrap();
    assert!(done.staged && done.updated);
    assert_eq!(m.installed()[0].manifest.version, "1.2.3", "old version still in place");
    let mut m = PluginManager::new(plugins.clone(), EditorFacts::current());
    assert_eq!(m.prepare_startup(), None);
    let p = &m.installed()[0];
    assert_eq!(p.manifest.version, "2.0.0");
    assert_eq!(std::fs::read_to_string(plugins.join("acme.one/data/settings.txt")).unwrap(), "keep me");
    m.check(p, &FakeProber::matching()).unwrap();

    // Uninstall while loaded: pending, removed at the next start.
    m.uninstall("acme.one", true).unwrap();
    assert!(plugins.join("acme.one").exists());
    let mut m = PluginManager::new(plugins.clone(), EditorFacts::current());
    m.prepare_startup();
    assert!(!plugins.join("acme.one").exists());
    assert!(m.installed().is_empty());
    assert!(!PluginStore::load(&plugins).plugins.contains_key("acme.one"));

    // Not loaded: removed at once.
    m.install(&library_file(&files, "acme_two"), &FakeProber::matching(), &none, &none)
        .unwrap();
    m.uninstall("acme.two", false).unwrap();
    assert!(!plugins.join("acme.two").exists());
    assert!(m.uninstall("../escape", false).is_err());
}

#[test]
fn a_crash_while_loading_disables_the_plugin_at_the_next_start() {
    let root = temp("marker");
    let files = temp("marker-files");
    let plugins = root.join("plugins");
    let mut m = PluginManager::new(plugins.clone(), EditorFacts::current());
    m.install(&library_file(&files, "acme_one"), &FakeProber::matching(), &none, &none)
        .unwrap();
    // What a load that never came back leaves behind.
    std::fs::write(plugins.join(".loading"), "acme.one").unwrap();
    let mut m = PluginManager::new(plugins.clone(), EditorFacts::current());
    assert_eq!(m.prepare_startup().as_deref(), Some("acme.one"));
    let r = &PluginStore::load(&plugins).plugins["acme.one"];
    assert!(!r.enabled);
    assert!(r.error.is_some());
    assert!(!plugins.join(".loading").exists());
}

#[test]
fn a_library_changed_after_install_is_refused() {
    let root = temp("checksum");
    let files = temp("checksum-files");
    let plugins = root.join("plugins");
    let mut m = PluginManager::new(plugins.clone(), EditorFacts::current());
    m.install(&library_file(&files, "acme_one"), &FakeProber::matching(), &none, &none)
        .unwrap();
    let p = m.installed().remove(0);
    let mut bytes = fake_library();
    bytes.extend_from_slice(b"tampered");
    std::fs::write(&p.library, bytes).unwrap();
    assert_eq!(m.check(&p, &FakeProber::matching()), Err(Problem::Modified));

    // A new editor build probes again and reports the mismatch.
    let mut m = PluginManager::new(plugins.clone(), EditorFacts::current());
    std::fs::write(&p.library, fake_library()).unwrap();
    let p = m.installed().remove(0);
    let mut newer = EditorFacts::current();
    newer.fingerprint.api_version = "9.9.9".into();
    newer.version = "9.9.9".into();
    let mut m2 = PluginManager::new(plugins.clone(), newer);
    assert!(matches!(m2.check(&p, &FakeProber::matching()), Err(Problem::Incompatible(_))));
    m.check(&p, &FakeProber::matching()).unwrap();
}
