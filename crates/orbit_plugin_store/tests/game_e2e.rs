//! Game plugins end to end, without network: `examples/plugins/dice` packed
//! like `cargo xtask game-plugin pack`, signed with a key made here, listed
//! in a `file://` catalog that also has editor plugins, fetched and cached
//! like the store does, the version for this engine chosen, downloaded and
//! verified, installed in a temporary project with its engine dependencies
//! pointed at this source tree, then uninstalled. Tampering is refused.

use std::path::{Path, PathBuf};

use orbit_plugin_store::catalog::{Catalog, parse};
use orbit_plugin_store::download::DownloadError;
use orbit_plugin_store::fetch::{self, MAX_AGE};
use orbit_plugin_store::game::PackageSource;
use orbit_plugin_store::game::catalog::{
    self as game_catalog, GamePlugin, GameVersion, select_game, sign_package,
};
use orbit_plugin_store::game::download::{GameDownloadPlan, download};
use orbit_plugin_store::game::package::{PackOptions, pack};
use orbit_plugin_store::game::project::{self, Expected, InstallOptions, OnConflict};
use orbit_plugin_store::signing::{Ed25519, SignatureError, parse_public_key, public_key_hex};
use orbit_plugin_store::{Localized, SourceEntry, StorePaths, Trust};

fn engine_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = engine_root()
        .join("target/test-tmp")
        .join(format!("orbit_plugin_store_e2e_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn file_url(path: &Path) -> String {
    let s = path.display().to_string();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

#[test]
fn the_dice_example_goes_from_a_catalog_into_a_project() {
    let engine = env!("CARGO_PKG_VERSION");
    let api = orbit_plugin_api::API_VERSION;
    let work = scratch("dice");
    let catalog_dir = work.join("catalog");
    std::fs::create_dir_all(catalog_dir.join("packages")).unwrap();

    // Author side: pack and sign.
    let packed = pack(
        &engine_root().join("examples/plugins/dice"),
        &PackOptions {
            api_version: api,
            min_engine: engine.to_owned(),
            max_engine: None,
            id: None,
            name: Some("Dice".into()),
        },
    )
    .unwrap();
    let m = &packed.manifest;
    assert_eq!(
        (m.id.as_str(), m.crate_name.as_str(), m.dir.as_str()),
        ("orbit.dice", "orbit-dice", "orbit-dice")
    );
    assert_eq!(
        m.engine_crates,
        ["orbit_core", "orbit_plugin_api", "orbit_vm"]
    );
    assert!(m.files.contains_key("Cargo.toml") && m.files.contains_key("src/lib.rs"));
    let file_name = format!("orbit.dice-{}.orbitsrc", m.version);
    std::fs::write(catalog_dir.join("packages").join(&file_name), &packed.bytes).unwrap();
    let seed = [42u8; 32];
    let sha256 = orbit_sign::sha256::digest_hex(&packed.bytes);
    let version = GameVersion {
        version: m.version.clone(),
        released: Some("2026-11-12".into()),
        changelog: Localized::Text("First".into()),
        api_version: api,
        min_engine: m.min_engine.clone(),
        max_engine: None,
        url: format!("packages/{file_name}"),
        signature: Some(sign_package(&seed, &m.id, &m.version, api, &sha256)),
        sha256: sha256.clone(),
        size: packed.bytes.len() as u64,
    };
    let games = vec![GamePlugin {
        id: m.id.clone(),
        name: m.name.clone(),
        description: Localized::Text(m.description.clone()),
        author: String::new(),
        homepage: None,
        license: m.license.clone(),
        icon: None,
        tags: vec!["dice".into()],
        versions: vec![version],
    }];
    let text = game_catalog::catalog_json(&Catalog::new("Official (test)"), &games);
    std::fs::write(catalog_dir.join("index.json"), &text).unwrap();
    // The catalog stays a valid editor-plugin catalog.
    assert!(parse(&text, true).unwrap().warnings.is_empty());

    // Editor side: the source is fetched (file://) and cached like the
    // store does; `orbit.*` ids need the official source, whose key is
    // replaced by the test key for the signature check.
    let paths = StorePaths {
        root: work.join("store"),
    };
    let source = SourceEntry {
        id: "official".into(),
        name: "Official (test)".into(),
        url: file_url(&catalog_dir.join("index.json")),
        enabled: true,
        public_key: None,
        builtin: true,
    };
    fetch::fetch(&paths, &source, true, MAX_AGE).unwrap();
    let (cached, warnings) = game_catalog::cached(&paths, &source).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let chosen = select_game(&cached[0], engine, api)
        .version()
        .unwrap()
        .clone();
    let key = parse_public_key(&public_key_hex(&seed));
    let package_source = PackageSource {
        trust: Trust::Trusted,
        key,
        ..PackageSource::of(&source)
    };
    let plan = GameDownloadPlan {
        source: package_source.clone(),
        plugin_id: cached[0].id.clone(),
        version: chosen.clone(),
    };
    let verified = download(&paths, &plan, &Ed25519, &|_, _| true).unwrap();

    // Install in a project, engine dependencies pointed at this tree.
    let project = work.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let root = engine_root();
    let out = project::install(
        &project,
        &std::fs::read(&verified.file).unwrap(),
        &package_source,
        Some(&Expected {
            id: cached[0].id.clone(),
            version: chosen.version.clone(),
            sha256: chosen.sha256.clone(),
        }),
        &InstallOptions {
            engine_version: engine,
            api_version: api,
            engine_root: Some(&root),
            on_conflict: OnConflict::Refuse,
        },
    )
    .unwrap();
    assert_eq!(out.dir, project.join("plugins/orbit-dice"));
    assert_eq!(out.plugin_ids, ["orbit.dice"]);
    let cargo = std::fs::read_to_string(out.dir.join("Cargo.toml")).unwrap();
    assert!(!cargo.contains("../../../crates"), "{cargo}");
    assert!(!cargo.contains("[lints]"), "{cargo}");
    // Every engine path dependency exists.
    for line in cargo.lines().filter(|l| l.contains("path = \"")) {
        let path = line
            .split("path = \"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(Path::new(path).join("Cargo.toml").is_file(), "{path}");
    }
    assert_eq!(
        std::fs::read(out.dir.join("src/lib.rs")).unwrap(),
        std::fs::read(root.join("examples/plugins/dice/src/lib.rs")).unwrap()
    );
    let games = project::scan(&project);
    assert_eq!(games[0].plugin_ids, ["orbit.dice"]);
    assert_eq!(games[0].record.as_ref().unwrap().source.id, "official");

    // Tampering: another key, a changed package.
    let mut wrong_key = plan.clone();
    wrong_key.source.key = parse_public_key(&public_key_hex(&[1u8; 32]));
    assert_eq!(
        download(&paths, &wrong_key, &Ed25519, &|_, _| true).unwrap_err(),
        DownloadError::Signature(SignatureError::Invalid)
    );
    let mut bytes = packed.bytes.clone();
    let last = bytes.len() - 30;
    bytes[last] ^= 0xff;
    std::fs::write(catalog_dir.join("packages").join(&file_name), &bytes).unwrap();
    assert!(matches!(
        download(&paths, &plan, &Ed25519, &|_, _| true),
        Err(DownloadError::Checksum { .. })
    ));

    // Uninstall.
    let gone = project::uninstall(&project, "orbit.dice", OnConflict::Refuse).unwrap();
    assert_eq!(gone.plugin_ids, ["orbit.dice"]);
    assert!(project::scan(&project).is_empty());
}
