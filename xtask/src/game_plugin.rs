//! Game plugins (runtime plugin crates distributed as sources) in the
//! store, see `docs/plugin-store.md` "Game plugins":
//!
//! * `cargo xtask game-plugin pack <crate dir>`: a `.orbitsrc` package and
//!   its descriptor `<package>.json` (sha256, size, API version, engine
//!   range, signature with `--sign` / `--sign-env`);
//! * `cargo xtask game-plugin index`: merges descriptors into the
//!   `game_plugins` array of a catalog (`index.json`), keeping its editor
//!   plugins.

use std::path::PathBuf;

use orbit_plugin_store::catalog::{self, Catalog, Localized};
use orbit_plugin_store::game::catalog::{self as game_catalog, GamePlugin, GameVersion};
use orbit_plugin_store::game::package::{PACKAGE_EXTENSION, PackOptions, pack};
use orbit_plugin_store::version::Version;
use serde::{Deserialize, Serialize};

use crate::plugin_store::{KeySource, utc_now};
use crate::{Result, workspace_root};

const HELP: &str = "\
cargo xtask game-plugin pack <crate dir> [--out <dir>] [--id <plugin id>] [--name <display name>]
                             [--min-engine <version>] [--max-engine <version> | none]
                             [--sign <keyfile> | --sign-env <VAR>]
cargo xtask game-plugin index --index <index.json> [--packages <dir>] [--url <pattern>] [--tag <tag>]
                              [--changelog <text> | --changelog-file <file>] [--released <date>]
                              [--source-name <name>] [--allow-unsigned]

pack: packs a game plugin crate (Cargo.toml with [package.metadata.orbit] plugins = [...]) as
<id>-<version>.orbitsrc in --out (default <target>/game-plugins), with <package>.json next to it.
The package records this engine's plugin API version; the engine range defaults to this engine's
version up to the next minor version (excluded); --max-engine none removes the upper bound.
A [lints] workspace = true table is dropped; other workspace inheritance and path dependencies
that are not engine crates (../crates/orbit_*) are refused. --sign / --sign-env sign it with an
Ed25519 key (cargo xtask plugin-key gen).

index: merges the descriptors (*.orbitsrc.json) of --packages (default <target>/game-plugins) into
the game_plugins array of --index (created when missing; its editor plugins are kept). --url is
the download URL pattern, {file} {tag} {id} {version} replaced (default
https://github.com/UZApocalyps/OrbitEnginePlugins/releases/download/{tag}/{file}); --tag defaults
to <id>-v<version>. A version already listed is replaced. Unsigned descriptors are refused unless
--allow-unsigned.
";

/// What `pack` writes next to a package.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameDescriptor {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    pub version: String,
    pub api_version: u32,
    pub min_engine: String,
    #[serde(default)]
    pub max_engine: Option<String>,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    pub signature: Option<String>,
}

fn default_out() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace_root().join("target"), PathBuf::from)
        .join("game-plugins")
}

/// The next breaking engine version after `v`: the next minor before 1.0,
/// the next major after.
pub fn next_breaking(v: &str) -> Option<String> {
    let v = Version::parse(v)?;
    let [major, minor, _] = v.core;
    Some(if major == 0 {
        format!("0.{}.0", minor + 1)
    } else {
        format!("{}.0.0", major + 1)
    })
}

pub fn game_plugin(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("pack") => pack_cmd(&args[1..]),
        Some("index") => index_cmd(&args[1..]),
        _ => {
            print!("{HELP}");
            Ok(())
        }
    }
}

fn pack_cmd(args: &[String]) -> Result {
    let engine = env!("CARGO_PKG_VERSION");
    let mut dir: Option<PathBuf> = None;
    let mut out = default_out();
    let mut opts = PackOptions {
        api_version: orbit_plugin_api::API_VERSION,
        min_engine: engine.to_owned(),
        max_engine: next_breaking(engine),
        id: None,
        name: None,
    };
    let mut key: Option<KeySource> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--out" => out = PathBuf::from(value("--out")?),
            "--id" => opts.id = Some(value("--id")?),
            "--name" => opts.name = Some(value("--name")?),
            "--min-engine" => opts.min_engine = value("--min-engine")?,
            "--max-engine" => {
                let v = value("--max-engine")?;
                opts.max_engine = (v != "none").then_some(v);
            }
            "--sign" => key = Some(KeySource::File(PathBuf::from(value("--sign")?))),
            "--sign-env" => key = Some(KeySource::Env(value("--sign-env")?)),
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option `{other}`\n\n{HELP}").into());
            }
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let dir = dir.ok_or_else(|| format!("the crate folder is required\n\n{HELP}"))?;
    let seed = key.map(|k| k.load()).transpose()?;
    let packed = pack(&dir, &opts).map_err(|e| format!("{}: {e}", dir.display()))?;
    let m = &packed.manifest;
    std::fs::create_dir_all(&out)?;
    let file = format!("{}-{}.{PACKAGE_EXTENSION}", m.id, m.version);
    let path = out.join(&file);
    std::fs::write(&path, &packed.bytes)?;
    let sha256 = orbit_sign::sha256::digest_hex(&packed.bytes);
    let descriptor = GameDescriptor {
        id: m.id.clone(),
        name: m.name.clone(),
        description: m.description.clone(),
        author: m.author.clone(),
        homepage: m.homepage.clone(),
        license: m.license.clone(),
        version: m.version.clone(),
        api_version: m.api_version,
        min_engine: m.min_engine.clone(),
        max_engine: m.max_engine.clone(),
        file: file.clone(),
        size: packed.bytes.len() as u64,
        signature: seed
            .as_ref()
            .map(|s| game_catalog::sign_package(s, &m.id, &m.version, m.api_version, &sha256)),
        sha256,
    };
    let json = PathBuf::from(format!("{}.json", path.display()));
    std::fs::write(&json, serde_json::to_string_pretty(&descriptor)? + "\n")?;
    println!(
        "{} ({} files, {} bytes; plugin API {}, engine {}{}; {})",
        path.display(),
        m.files.len(),
        descriptor.size,
        m.api_version,
        m.min_engine,
        m.max_engine
            .as_deref()
            .map(|v| format!(" to {v} excluded"))
            .unwrap_or_else(|| " or newer".into()),
        if descriptor.signature.is_some() {
            "signed"
        } else {
            "NOT signed"
        }
    );
    if m.build_script || m.proc_macro {
        println!(
            "note: the crate runs code at compile time (build script or proc-macro); the store says so"
        );
    }
    Ok(())
}

/// Merges `descriptors` into `games` (pure, tested).
pub fn merge(
    games: &mut Vec<GamePlugin>,
    descriptors: &[GameDescriptor],
    url_pattern: &str,
    tag: Option<&str>,
    changelog: Option<&str>,
    released: Option<&str>,
) {
    for d in descriptors {
        let plugin = match games.iter().position(|p| p.id == d.id) {
            Some(i) => &mut games[i],
            None => {
                games.push(GamePlugin {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    description: Localized::Text(d.description.clone()),
                    author: d.author.clone(),
                    homepage: d.homepage.clone(),
                    license: d.license.clone(),
                    icon: None,
                    tags: Vec::new(),
                    versions: Vec::new(),
                });
                games.last_mut().expect("just pushed")
            }
        };
        let default_tag = format!("{}-v{}", d.id, d.version);
        let tag = tag.unwrap_or(&default_tag);
        let url = url_pattern
            .replace("{file}", &d.file)
            .replace("{tag}", tag)
            .replace("{id}", &d.id)
            .replace("{version}", &d.version);
        let old = plugin
            .versions
            .iter()
            .position(|v| v.version == d.version)
            .map(|i| plugin.versions.remove(i));
        plugin.versions.push(GameVersion {
            version: d.version.clone(),
            released: released
                .map(str::to_owned)
                .or_else(|| old.as_ref().and_then(|o| o.released.clone())),
            changelog: changelog
                .map(|t| Localized::Text(t.to_owned()))
                .or_else(|| old.map(|o| o.changelog))
                .unwrap_or_default(),
            api_version: d.api_version,
            min_engine: d.min_engine.clone(),
            max_engine: d.max_engine.clone(),
            url,
            sha256: d.sha256.clone(),
            size: d.size,
            signature: d.signature.clone(),
        });
        plugin
            .versions
            .sort_by(|a, b| orbit_plugin_store::version::compare(&b.version, &a.version));
    }
    games.sort_by(|a, b| a.id.cmp(&b.id));
}

fn index_cmd(args: &[String]) -> Result {
    let mut index: Option<PathBuf> = None;
    let mut packages = default_out();
    let mut url =
        "https://github.com/UZApocalyps/OrbitEnginePlugins/releases/download/{tag}/{file}"
            .to_owned();
    let mut tag: Option<String> = None;
    let mut changelog: Option<String> = None;
    let mut released: Option<String> = None;
    let mut source_name = "OrbitEngine official plugins".to_owned();
    let mut allow_unsigned = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--index" => index = Some(PathBuf::from(value("--index")?)),
            "--packages" => packages = PathBuf::from(value("--packages")?),
            "--url" => url = value("--url")?,
            "--tag" => tag = Some(value("--tag")?),
            "--changelog" => changelog = Some(value("--changelog")?),
            "--changelog-file" => {
                changelog = Some(std::fs::read_to_string(value("--changelog-file")?)?)
            }
            "--released" => released = Some(value("--released")?),
            "--source-name" => source_name = value("--source-name")?,
            "--allow-unsigned" => allow_unsigned = true,
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            other => return Err(format!("unknown option `{other}`\n\n{HELP}").into()),
        }
    }
    let index = index.ok_or_else(|| format!("--index is required\n\n{HELP}"))?;
    let suffix = format!(".{PACKAGE_EXTENSION}.json");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&packages)
        .map_err(|e| format!("{}: {e}", packages.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(&suffix))
        .collect();
    files.sort();
    let mut descriptors = Vec::new();
    for f in &files {
        let d: GameDescriptor = serde_json::from_str(&std::fs::read_to_string(f)?)
            .map_err(|e| format!("{}: {e}", f.display()))?;
        if d.signature.is_none() && !allow_unsigned {
            return Err(format!(
                "{} is not signed (cargo xtask game-plugin pack --sign ...)",
                f.display()
            )
            .into());
        }
        descriptors.push(d);
    }
    if descriptors.is_empty() {
        return Err(format!(
            "no package descriptor (*{suffix}) in {}",
            packages.display()
        )
        .into());
    }
    let (catalog, mut games) = match std::fs::read_to_string(&index) {
        Ok(text) => {
            let parsed =
                catalog::parse(&text, true).map_err(|e| format!("{}: {e}", index.display()))?;
            let (games, warnings) = game_catalog::parse_game_plugins(&text, true)
                .map_err(|e| format!("{}: {e}", index.display()))?;
            for w in parsed.warnings.iter().chain(&warnings) {
                println!("warning: {w}");
            }
            (parsed.catalog, games)
        }
        Err(_) => (Catalog::new(&source_name), Vec::new()),
    };
    let released = released.or_else(|| Some(utc_now()[..10].to_owned()));
    merge(
        &mut games,
        &descriptors,
        &url,
        tag.as_deref(),
        changelog.as_deref(),
        released.as_deref(),
    );
    let mut catalog = catalog;
    catalog.generated = Some(utc_now());
    let text = game_catalog::catalog_json(&catalog, &games);
    // What the editor will read.
    let (_, warnings) = game_catalog::parse_game_plugins(&text, true)
        .map_err(|e| format!("the merged catalog is invalid: {e}"))?;
    if !warnings.is_empty() {
        return Err(format!("the merged catalog has problems: {}", warnings.join("; ")).into());
    }
    catalog::parse(&text, true).map_err(|e| format!("the merged catalog is invalid: {e}"))?;
    std::fs::write(&index, text)?;
    println!(
        "{}: {} game plugin package(s) merged",
        index.display(),
        descriptors.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(version: &str) -> GameDescriptor {
        GameDescriptor {
            id: "orbit.dice".into(),
            name: "Dice".into(),
            description: "Rolls dice".into(),
            author: String::new(),
            homepage: None,
            license: Some("MIT".into()),
            version: version.into(),
            api_version: 1,
            min_engine: "0.4.0".into(),
            max_engine: Some("0.5.0".into()),
            file: format!("orbit.dice-{version}.orbitsrc"),
            size: 10,
            sha256: "ab".repeat(32),
            signature: Some("cd".repeat(64)),
        }
    }

    #[test]
    fn descriptors_merge_and_editor_plugins_are_kept() {
        let mut games = Vec::new();
        let pattern = "https://h/{tag}/{file}";
        merge(
            &mut games,
            &[descriptor("0.4.0")],
            pattern,
            None,
            Some("First"),
            Some("2026-11-12"),
        );
        let mut again = descriptor("0.4.0");
        again.sha256 = "ef".repeat(32);
        merge(
            &mut games,
            &[again, descriptor("0.4.1")],
            pattern,
            Some("t"),
            None,
            None,
        );
        let p = &games[0];
        let versions: Vec<&str> = p.versions.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(versions, ["0.4.1", "0.4.0"]);
        let v040 = &p.versions[1];
        assert_eq!(v040.sha256, "ef".repeat(32));
        assert_eq!(
            v040.changelog.get("en"),
            "First",
            "kept when not given again"
        );
        assert_eq!(v040.released.as_deref(), Some("2026-11-12"));
        assert_eq!(v040.url, "https://h/t/orbit.dice-0.4.0.orbitsrc");
        // Written with an editor catalog: both parse, nothing lost.
        let mut c = Catalog::new("Official");
        c.plugins.push(
            serde_json::from_value(serde_json::json!({ "id": "orbit.ai_studio", "name": "AI" }))
                .unwrap(),
        );
        let text = game_catalog::catalog_json(&c, &games);
        assert_eq!(catalog::parse(&text, true).unwrap().catalog, c);
        let (back, warnings) = game_catalog::parse_game_plugins(&text, true).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back, games);
        assert_eq!(next_breaking("0.4.0").as_deref(), Some("0.5.0"));
        assert_eq!(next_breaking("1.2.3").as_deref(), Some("2.0.0"));
    }
}
