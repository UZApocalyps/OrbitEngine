//! The store end to end with a real signed package (no network: a
//! `file://` catalog): catalog written like `cargo xtask plugin-index`, the
//! build chosen by the editor's build id, downloaded, size / SHA-256 /
//! signature checked, then installed by the plugin host (probe in the real
//! editor binary). Tampering with the package or the catalog is refused.
//!
//! Needs the packages and the editor binary first (debug profile):
//!
//! ```text
//! cargo build -p orbit_editor --bin orbit-editor
//! cargo xtask plugins --debug --examples --only orbit_hello_plugin --sign <keyfile>
//! ORBIT_STORE_E2E_KEY=<public key hex> cargo test -p orbit_plugin_store --test e2e -- --ignored
//! ```

use std::path::{Path, PathBuf};

use orbit_plugin_host::{EditorFacts, PluginManager, ProcessProber};
use orbit_plugin_store::catalog::{
    Build, Catalog, CatalogPlugin, CatalogVersion, Localized, parse, select,
};
use orbit_plugin_store::download::{DownloadError, DownloadPlan, download};
use orbit_plugin_store::signing::{Ed25519, SignatureError};
use orbit_plugin_store::{SourceEntry, StorePaths, fingerprint_id, fingerprint_key_from_json};

fn target() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")
}

fn editor_exe() -> PathBuf {
    target()
        .join("debug")
        .join(format!("orbit-editor{}", std::env::consts::EXE_SUFFIX))
}

fn editor_facts() -> EditorFacts {
    let out = std::process::Command::new(editor_exe())
        .arg("--plugin-abi")
        .output()
        .expect("orbit-editor --plugin-abi");
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .find(|l| l.starts_with('{'))
        .unwrap()
        .to_owned();
    EditorFacts {
        fingerprint: serde_json::from_str(&line).unwrap(),
        version: orbit_editor_api::native::API_VERSION.into(),
    }
}

#[test]
#[ignore = "needs the debug editor and a signed hello package (see the module doc)"]
fn a_signed_package_installs_from_a_file_catalog() {
    let key = std::env::var("ORBIT_STORE_E2E_KEY").expect("ORBIT_STORE_E2E_KEY=<public key hex>");
    let packages = target().join("plugins");
    let descriptor_path = std::fs::read_dir(&packages)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.to_string_lossy().contains("hello")
                && p.to_string_lossy().ends_with(".orbitplugin.json")
        })
        .expect("a hello build descriptor in target/plugins");
    let d: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&descriptor_path).unwrap()).unwrap();

    let facts = editor_facts();
    let my_id = fingerprint_id(&facts.cache_key());
    let json = serde_json::to_value(&facts.fingerprint).unwrap();
    assert_eq!(
        fingerprint_id(&fingerprint_key_from_json(&json).unwrap()),
        my_id
    );
    assert_eq!(
        d["fingerprint"], my_id,
        "the package was built for this editor"
    );

    let dir = target().join("test-tmp").join("store_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("catalog")).unwrap();
    let file = d["file"].as_str().unwrap();
    std::fs::copy(packages.join(file), dir.join("catalog").join(file)).unwrap();
    let s = |k: &str| d[k].as_str().unwrap().to_owned();
    let build = Build {
        editor_version: s("editor_version"),
        fingerprint: s("fingerprint"),
        target: s("target"),
        profile: s("profile"),
        url: file.to_owned(),
        sha256: s("sha256"),
        size: d["size"].as_u64().unwrap(),
        signature: d["signature"].as_str().map(str::to_owned),
    };
    let mut catalog = Catalog::new("E2E");
    catalog.plugins.push(CatalogPlugin {
        id: s("id"),
        name: s("name"),
        description: Localized::Text(s("description")),
        author: s("author"),
        homepage: None,
        license: None,
        icon: None,
        tags: vec![],
        versions: vec![CatalogVersion {
            version: s("version"),
            released: None,
            changelog: Localized::default(),
            builds: vec![build],
        }],
    });
    let index = dir.join("catalog").join("index.json");
    std::fs::write(&index, catalog.to_json()).unwrap();
    let parsed = parse(&std::fs::read_to_string(&index).unwrap(), false).unwrap();
    assert!(parsed.warnings.is_empty());
    let plugin = &parsed.catalog.plugins[0];
    let (version, build) = select(plugin, &my_id)
        .build()
        .expect("a build for this editor");

    let source = SourceEntry {
        id: "e2e".into(),
        name: "E2E".into(),
        url: orbit_http::file_url(&index),
        enabled: true,
        public_key: Some(key),
        builtin: false,
    };
    let paths = StorePaths::for_config_dir(&dir.join("config"));
    let plan = DownloadPlan {
        source: source.clone(),
        plugin_id: plugin.id.clone(),
        version: version.version.clone(),
        build: build.clone(),
    };

    // Another key: refused before anything is downloaded.
    let mut wrong = plan.clone();
    wrong.source.public_key = Some("11".repeat(32));
    assert_eq!(
        download(&paths, &wrong, &Ed25519, &|_, _| true).unwrap_err(),
        DownloadError::Signature(SignatureError::Invalid)
    );
    // The catalog lies about the hash: the signature no longer matches.
    let mut lying = plan.clone();
    lying.build.sha256 = "00".repeat(32);
    assert_eq!(
        download(&paths, &lying, &Ed25519, &|_, _| true).unwrap_err(),
        DownloadError::Signature(SignatureError::Invalid)
    );

    let verified = download(&paths, &plan, &Ed25519, &|_, _| true).expect("verified download");
    let prober = ProcessProber {
        program: editor_exe(),
        args: vec!["--probe-plugin".into()],
        timeout: std::time::Duration::from_secs(30),
    };
    let mut manager = PluginManager::for_config_dir(&dir.join("config"), facts);
    let installed = manager
        .install(&verified.file, &prober, &|_| false, &|_| false)
        .expect("installed");
    assert_eq!(installed.id, plugin.id);
    assert_eq!(installed.version, version.version);

    // A package modified after signing: SHA-256 refused.
    let served = dir.join("catalog").join(file);
    let mut bytes = std::fs::read(&served).unwrap();
    let n = bytes.len();
    bytes[n / 2] ^= 0xff;
    std::fs::write(&served, bytes).unwrap();
    assert!(matches!(
        download(&paths, &plan, &Ed25519, &|_, _| true),
        Err(DownloadError::Checksum { .. })
    ));
}
