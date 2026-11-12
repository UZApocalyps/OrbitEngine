//! Plugin store tooling (see `docs/plugin-store.md`):
//!
//! * `cargo xtask plugin-key gen [--out <file>] [--force]`: a new Ed25519
//!   signing key, written OUTSIDE any git work tree; `plugin-key pub <file>`
//!   prints the public key of a key file;
//! * the signing step of `cargo xtask plugins --sign <file>` /
//!   `--sign-env <VAR>`: a `<package>.json` descriptor next to each package
//!   (sha256, size, editor build id, signature);
//! * `cargo xtask plugin-index`: merges those descriptors into a catalog
//!   (`index.json`).

use std::path::{Path, PathBuf};

use orbit_plugin_store::catalog::{self, Build, Catalog, CatalogPlugin, CatalogVersion, Localized};
use orbit_plugin_store::signing;
use serde::{Deserialize, Serialize};

use crate::{Result, workspace_root};

const KEY_HELP: &str = "\
cargo xtask plugin-key gen [--out <file>] [--force]
cargo xtask plugin-key pub <file>

gen: creates an Ed25519 signing key for plugin builds. The PRIVATE key is written to
--out (default: ../OrbitEngine-secrets/plugin_signing_key next to the repository); a path
inside a git work tree is refused. Put the file's content in the GitHub secret
ORBIT_PLUGIN_SIGNING_KEY; never commit it. pub: prints the public key of a key file.
";

const INDEX_HELP: &str = "\
cargo xtask plugin-index --index <index.json> [--builds <dir>] [--url <pattern>] [--tag <tag>]
                         [--changelog <text> | --changelog-file <file>] [--released <date>]
                         [--source-name <name>]

Merges the build descriptors (<package>.orbitplugin.json, written by `cargo xtask plugins`)
of --builds (default <target>/plugins) into the catalog --index (created when missing).
--url is the download URL pattern, {file} {tag} {id} {version} replaced
(default https://github.com/UZApocalyps/OrbitEnginePlugins/releases/download/{tag}/{file});
--tag defaults to <id>-v<version>. A build with the same plugin, version and editor build
id is replaced. Unsigned descriptors are refused unless --allow-unsigned.
";

/// What `cargo xtask plugins` writes next to each package.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildDescriptor {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub homepage: Option<String>,
    pub version: String,
    pub editor_version: String,
    pub fingerprint: String,
    pub target: String,
    pub profile: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    pub signature: Option<String>,
}

fn inside_git_work_tree(path: &Path) -> bool {
    let mut dir = Some(path);
    while let Some(d) = dir {
        if d.join(".git").exists() {
            return true;
        }
        dir = d.parent();
    }
    false
}

/// Absolute form of `path` (it may not exist yet).
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    }
}

/// Normalizes `..` without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn random_seed() -> Result<[u8; 32]> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| format!("no system randomness: {e}"))?;
    Ok(seed)
}

pub fn plugin_key(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("gen") => {
            let mut out: Option<PathBuf> = None;
            let mut force = false;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a file")?)),
                    "--force" => force = true,
                    other => return Err(format!("unknown option `{other}`\n\n{KEY_HELP}").into()),
                }
            }
            let out = normalize(&absolute(&out.unwrap_or_else(|| {
                workspace_root()
                    .join("..")
                    .join("OrbitEngine-secrets")
                    .join("plugin_signing_key")
            })));
            let parent = out.parent().ok_or("the key file needs a folder")?;
            if inside_git_work_tree(parent) {
                return Err(format!(
                    "{} is inside a git work tree: the private key must live outside any repository (pass --out)",
                    out.display()
                )
                .into());
            }
            if out.exists() && !force {
                return Err(format!(
                    "{} exists (--force replaces it: the old key is lost)",
                    out.display()
                )
                .into());
            }
            let seed = random_seed()?;
            std::fs::create_dir_all(parent)?;
            std::fs::write(&out, signing::private_key_file(&seed))?;
            println!("private key written to {}", out.display());
            println!("public key: {}", signing::public_key_hex(&seed));
            println!(
                "\nNext steps:\n  1. Keep the file safe and NEVER commit it (it is outside the repository).\n  \
                 2. GitHub > UZApocalyps/OrbitEnginePlugins > Settings > Secrets and variables > Actions >\n     \
                 New repository secret: name ORBIT_PLUGIN_SIGNING_KEY, value = the content of the file.\n  \
                 3. For the official key: set orbit_plugin_store::signing::OFFICIAL_PUBLIC_KEY to the public key\n     \
                 (it ships with the next editor release)."
            );
            Ok(())
        }
        Some("pub") => {
            let file = args.get(1).ok_or("plugin-key pub needs a key file")?;
            let seed = signing::parse_private_key(&std::fs::read_to_string(file)?)
                .ok_or("no key in that file")?;
            println!("{}", signing::public_key_hex(&seed));
            Ok(())
        }
        _ => {
            print!("{KEY_HELP}");
            Ok(())
        }
    }
}

/// Where `cargo xtask plugins` gets the signing key.
pub enum KeySource {
    File(PathBuf),
    Env(String),
}

impl KeySource {
    pub fn load(&self) -> Result<[u8; 32]> {
        let text = match self {
            Self::File(p) => {
                std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?
            }
            Self::Env(var) => std::env::var(var)
                .map_err(|_| format!("the environment variable {var} is not set"))?,
        };
        Ok(signing::parse_private_key(&text).ok_or("no signing key (64 hex characters) found")?)
    }
}

/// Writes `<package>.json` next to `package` (and signs it with `seed`).
#[allow(clippy::too_many_arguments)]
pub fn write_descriptor(
    package: &Path,
    manifest_id: &str,
    name: &str,
    description: &str,
    author: &str,
    homepage: Option<String>,
    version: &str,
    editor_version: &str,
    fingerprint_json: &serde_json::Value,
    seed: Option<&[u8; 32]>,
) -> Result<BuildDescriptor> {
    let key = orbit_plugin_store::fingerprint_key_from_json(fingerprint_json)
        .ok_or("the probe did not return a complete fingerprint")?;
    let fingerprint = orbit_plugin_store::fingerprint_id(&key);
    let (sha256, size) = orbit_sign::sha256::file_hex(package, &|| false)?.unwrap_or_default();
    let signature =
        seed.map(|s| signing::sign_build(s, manifest_id, version, &fingerprint, &sha256));
    let descriptor = BuildDescriptor {
        id: manifest_id.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
        author: author.to_owned(),
        homepage,
        version: version.to_owned(),
        editor_version: editor_version.to_owned(),
        fingerprint,
        target: fingerprint_json["target"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        profile: fingerprint_json["profile"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        file: package
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size,
        sha256,
        signature,
    };
    let path = PathBuf::from(format!("{}.json", package.display()));
    std::fs::write(&path, serde_json::to_string_pretty(&descriptor)? + "\n")?;
    println!(
        "  {} ({}; build id {})",
        path.display(),
        if descriptor.signature.is_some() {
            "signed"
        } else {
            "NOT signed"
        },
        descriptor.fingerprint
    );
    Ok(descriptor)
}

/// Merges `descriptors` into `catalog` (pure, tested).
pub fn merge(
    catalog: &mut Catalog,
    descriptors: &[BuildDescriptor],
    url_pattern: &str,
    tag: Option<&str>,
    changelog: Option<&str>,
    released: Option<&str>,
) {
    for d in descriptors {
        let plugin = match catalog.plugins.iter().position(|p| p.id == d.id) {
            Some(i) => &mut catalog.plugins[i],
            None => {
                catalog.plugins.push(CatalogPlugin {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    description: Localized::Text(d.description.clone()),
                    author: d.author.clone(),
                    homepage: d.homepage.clone(),
                    license: None,
                    icon: None,
                    tags: Vec::new(),
                    versions: Vec::new(),
                });
                catalog.plugins.last_mut().expect("just pushed")
            }
        };
        let version = match plugin.versions.iter().position(|v| v.version == d.version) {
            Some(i) => &mut plugin.versions[i],
            None => {
                plugin.versions.push(CatalogVersion {
                    version: d.version.clone(),
                    released: released.map(str::to_owned),
                    changelog: Localized::default(),
                    builds: Vec::new(),
                });
                plugin.versions.last_mut().expect("just pushed")
            }
        };
        if let Some(text) = changelog {
            version.changelog = Localized::Text(text.to_owned());
        }
        if released.is_some() {
            version.released = released.map(str::to_owned);
        }
        let default_tag = format!("{}-v{}", d.id, d.version);
        let tag = tag.unwrap_or(&default_tag);
        let url = url_pattern
            .replace("{file}", &d.file)
            .replace("{tag}", tag)
            .replace("{id}", &d.id)
            .replace("{version}", &d.version);
        let build = Build {
            editor_version: d.editor_version.clone(),
            fingerprint: d.fingerprint.clone(),
            target: d.target.clone(),
            profile: d.profile.clone(),
            url,
            sha256: d.sha256.clone(),
            size: d.size,
            signature: d.signature.clone(),
        };
        version
            .builds
            .retain(|b| b.fingerprint != build.fingerprint);
        version.builds.push(build);
        version.builds.sort_by(|a, b| a.target.cmp(&b.target));
        plugin
            .versions
            .sort_by(|a, b| orbit_plugin_store::version::compare(&b.version, &a.version));
    }
    catalog.plugins.sort_by(|a, b| a.id.cmp(&b.id));
}

pub(crate) fn utc_now() -> String {
    // Days to civil date (Howard Hinnant's algorithm), no time crate.
    let secs = orbit_plugin_store::now_secs();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub fn plugin_index(args: &[String], default_builds: PathBuf) -> Result {
    let mut index: Option<PathBuf> = None;
    let mut builds = default_builds;
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
            "--builds" => builds = PathBuf::from(value("--builds")?),
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
                print!("{INDEX_HELP}");
                return Ok(());
            }
            other => return Err(format!("unknown option `{other}`\n\n{INDEX_HELP}").into()),
        }
    }
    let index = index.ok_or_else(|| format!("--index is required\n\n{INDEX_HELP}"))?;
    let mut descriptors = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&builds)
        .map_err(|e| format!("{}: {e}", builds.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".orbitplugin.json"))
        .collect();
    files.sort();
    for f in &files {
        let d: BuildDescriptor = serde_json::from_str(&std::fs::read_to_string(f)?)
            .map_err(|e| format!("{}: {e}", f.display()))?;
        if d.signature.is_none() && !allow_unsigned {
            return Err(format!(
                "{} is not signed (cargo xtask plugins --sign ...)",
                f.display()
            )
            .into());
        }
        descriptors.push(d);
    }
    if descriptors.is_empty() {
        return Err(format!(
            "no build descriptor (*.orbitplugin.json) in {}",
            builds.display()
        )
        .into());
    }
    // The catalog's game plugins (`cargo xtask game-plugin index`) are kept.
    let mut games = Vec::new();
    let mut catalog = match std::fs::read_to_string(&index) {
        Ok(text) => {
            let parsed =
                catalog::parse(&text, true).map_err(|e| format!("{}: {e}", index.display()))?;
            for w in &parsed.warnings {
                println!("warning: {w}");
            }
            games = orbit_plugin_store::game::catalog::parse_game_plugins(&text, true)
                .map(|(g, _)| g)
                .unwrap_or_default();
            parsed.catalog
        }
        Err(_) => Catalog::new(&source_name),
    };
    let released = released.or_else(|| Some(utc_now()[..10].to_owned()));
    merge(
        &mut catalog,
        &descriptors,
        &url,
        tag.as_deref(),
        changelog.as_deref(),
        released.as_deref(),
    );
    catalog.generated = Some(utc_now());
    let text = orbit_plugin_store::game::catalog::catalog_json(&catalog, &games);
    // What the editor will read.
    let check =
        catalog::parse(&text, true).map_err(|e| format!("the merged catalog is invalid: {e}"))?;
    if !check.warnings.is_empty() {
        return Err(format!(
            "the merged catalog has problems: {}",
            check.warnings.join("; ")
        )
        .into());
    }
    std::fs::write(&index, text)?;
    println!("{}: {} build(s) merged", index.display(), descriptors.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(version: &str, fp: &str, target: &str) -> BuildDescriptor {
        BuildDescriptor {
            id: "orbit.ai_studio".into(),
            name: "AI Studio".into(),
            description: "Wizard".into(),
            author: "OrbitEngine".into(),
            homepage: None,
            version: version.into(),
            editor_version: "0.4.0".into(),
            fingerprint: fp.into(),
            target: target.into(),
            profile: "release".into(),
            file: format!("orbit.ai_studio-{version}-{target}.orbitplugin"),
            size: 3,
            sha256: "ab".repeat(32),
            signature: Some("cd".repeat(64)),
        }
    }

    #[test]
    fn descriptors_merge_into_a_valid_catalog() {
        let mut c = Catalog::new("Official");
        let a = "a".repeat(32);
        let b = "b".repeat(32);
        let pattern = "https://github.com/o/r/releases/download/{tag}/{file}";
        merge(
            &mut c,
            &[
                descriptor("0.4.0", &a, "x86_64-pc-windows-msvc"),
                descriptor("0.4.0", &b, "x86_64-unknown-linux-gnu"),
            ],
            pattern,
            None,
            Some("First"),
            Some("2026-11-12"),
        );
        // Rebuilt Windows package: replaced, not duplicated.
        let mut again = descriptor("0.4.0", &a, "x86_64-pc-windows-msvc");
        again.sha256 = "ef".repeat(32);
        merge(
            &mut c,
            &[again, descriptor("0.5.0", &a, "x86_64-pc-windows-msvc")],
            pattern,
            Some("editor-v0.4.0"),
            None,
            None,
        );
        let p = c.plugin("orbit.ai_studio").unwrap();
        assert_eq!(
            p.versions
                .iter()
                .map(|v| v.version.as_str())
                .collect::<Vec<_>>(),
            ["0.5.0", "0.4.0"]
        );
        let v040 = &p.versions[1];
        assert_eq!(v040.builds.len(), 2);
        assert_eq!(v040.changelog.get("en"), "First");
        assert_eq!(v040.builds[0].sha256, "ef".repeat(32));
        assert_eq!(
            v040.builds[0].url,
            "https://github.com/o/r/releases/download/editor-v0.4.0/orbit.ai_studio-0.4.0-x86_64-pc-windows-msvc.orbitplugin"
        );
        assert_eq!(
            v040.builds[1].url,
            "https://github.com/o/r/releases/download/orbit.ai_studio-v0.4.0/orbit.ai_studio-0.4.0-x86_64-unknown-linux-gnu.orbitplugin"
        );
        let parsed = catalog::parse(&c.to_json(), true).unwrap();
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        assert_eq!(utc_now().len(), 20);
    }

    #[test]
    fn keys_are_refused_inside_a_repository() {
        assert!(inside_git_work_tree(&workspace_root()));
        assert_eq!(
            normalize(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
    }
}
