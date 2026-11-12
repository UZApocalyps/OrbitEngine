//! `cargo xtask plugins`: builds the official native editor plugins
//! (`plugins/*`, and the SDK examples with `--examples`) with this
//! workspace's toolchain and profile, and packages each one as a
//! `.orbitplugin` (zip: `manifest.ron`, the library, `locales/`, `assets/`)
//! into `<target>/plugins/`.
//!
//! A native plugin only loads in an editor built from the same sources, by
//! the same compiler, with the same profile (see `orbit_editor_api::native`):
//! build the editor with the same `--release` / `--debug` choice. Before
//! building, the features each plugin resolves for the crates it shares with
//! the editor (everything `orbit_editor_api` depends on, egui included) are
//! compared with the editor's; after, when the editor binary of that profile
//! exists, its
//! fingerprint (`orbit-editor --plugin-abi`) is compared with the plugin's
//! (`orbit-editor --probe-plugin`).

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Result, cargo, run, workspace_root};

const HELP: &str = "\
cargo xtask plugins [--debug] [--examples] [--only <package>] [--out <dir>] [--no-check]
                    [--sign <keyfile> | --sign-env <VAR>] [--bundle [<ids>]] [--bundle-dir <dir>]

Builds the official native editor plugins (plugins/*) in release (or --debug), with the
toolchain and profile of the editor, and writes <target>/plugins/<id>-<version>-<os>-<arch>.orbitplugin.
--examples also builds examples/editor_plugins/*; --only builds one package; --no-check skips the
comparison of the features resolved for the crates shared with the editor. Build the editor with
the same profile: a plugin only loads in it.
When the editor of that profile is built, <package>.orbitplugin.json (sha256, size, editor build
id: what `cargo xtask plugin-index` puts in a catalog) is written next to each package; --sign
signs it with the Ed25519 key of a file (cargo xtask plugin-key gen), --sign-env with the key in an
environment variable (CI secret ORBIT_PLUGIN_SIGNING_KEY). Signing needs the editor binary.
--bundle copies the packages of the listed plugin ids (comma separated; default: DEFAULT_BUNDLED,
orbit.renpy_import and orbit.mcp) into <bundle-dir> (default packaging/bundled-plugins), the folder the
installers ship as bundled-plugins/ next to the editor: the editor installs those packages on its
first start. Older packages of the same id are removed from that folder; build the plugins in the
same profile as the editor you package.
";

/// The plugins shipped with the editor by default (`--bundle` without ids):
/// change this list to ship others. Keep it short: every shipped plugin
/// adds its size to the installer.
const DEFAULT_BUNDLED: &[&str] = &["orbit.renpy_import", "orbit.mcp"];

/// The package manifest (`manifest.ron`), as the editor reads it.
#[derive(Serialize, Deserialize)]
struct Manifest {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    library: String,
    #[serde(default)]
    min_editor: Option<String>,
    #[serde(default)]
    max_editor: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
}

struct PluginCrate {
    name: String,
    version: String,
    dir: PathBuf,
}

fn metadata() -> Result<serde_json::Value> {
    let out = cargo()
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

/// The cdylib packages under `plugins/` (and `examples/editor_plugins/`).
fn plugin_crates(meta: &serde_json::Value, examples: bool) -> Vec<PluginCrate> {
    let root = workspace_root();
    let mut dirs = vec![root.join("plugins")];
    if examples {
        dirs.push(root.join("examples").join("editor_plugins"));
    }
    let mut out = Vec::new();
    for package in meta["packages"].as_array().into_iter().flatten() {
        let manifest = PathBuf::from(package["manifest_path"].as_str().unwrap_or_default());
        let Some(dir) = manifest.parent() else { continue };
        let inside = dirs.iter().any(|d| {
            dir.canonicalize()
                .ok()
                .zip(d.canonicalize().ok())
                .is_some_and(|(a, b)| a.starts_with(b))
        });
        let cdylib = package["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|t| t["crate_types"].as_array().into_iter().flatten().any(|c| c == "cdylib"));
        if inside && cdylib && dir.join("manifest.ron").is_file() {
            out.push(PluginCrate {
                name: package["name"].as_str().unwrap_or_default().to_owned(),
                version: package["version"].as_str().unwrap_or_default().to_owned(),
                dir: dir.to_path_buf(),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `package version|features` of every crate `package`'s build resolves
/// (normal and build dependencies, proc macros included).
fn resolved(package: &str) -> Result<BTreeSet<String>> {
    let out = cargo()
        .args(["tree", "-p", package, "-e", "normal,build", "--prefix", "none", "-f", "{p}|{f}"])
        .output()?;
    if !out.status.success() {
        return Err(format!("cargo tree failed for {package}: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.replace(" (*)", ""))
        .map(|l| {
            // Drop the path of workspace crates: `name vX (path)|features`.
            match (l.find(" ("), l.find('|')) {
                (Some(a), Some(b)) if a < b => format!("{}{}", &l[..a], &l[b..]),
                _ => l,
            }
        })
        .collect())
}

/// The crates whose build must be the editor's: everything
/// `orbit_editor_api` depends on (egui included). Their resolved features
/// decide their `TypeId`s, which the plugin fingerprint compares.
fn shared_crates() -> Result<BTreeSet<String>> {
    let mut shared: BTreeSet<String> = resolved("orbit_editor_api")?
        .into_iter()
        .filter_map(|l| l.split('|').next().map(str::to_owned))
        .collect();
    // `orbit_editor_api` alone does not enable what the editor enables on
    // those crates: `orbit_core/schema` adds `schemars`, and its features
    // (and those of its own dependencies) are part of `orbit_core`'s
    // identity. Add everything the editor's tree puts below a shared
    // workspace crate.
    let out = cargo()
        .args(["tree", "-p", "orbit_editor", "-e", "normal,build", "--no-dedupe", "--prefix", "depth", "-f", "{p}"])
        .output()?;
    if !out.status.success() {
        return Err(format!("cargo tree failed for orbit_editor: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    // (depth of the shared workspace crate whose subtree we are in, if any)
    let mut under: Option<usize> = None;
    let mut extra = BTreeSet::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        let (Ok(depth), rest) = (line[..digits].parse::<usize>(), &line[digits..]) else { continue };
        if under.is_some_and(|d| depth <= d) {
            under = None;
        }
        // `name vX.Y.Z (path)` for workspace crates, `name vX.Y.Z (proc-macro)` for proc macros.
        // Same form as `shared`: `name vX.Y.Z`.
        let mut words = rest.split(' ');
        let name = format!("{} {}", words.next().unwrap_or_default(), words.next().unwrap_or_default());
        let workspace = rest.contains(" (") && !rest.contains("(proc-macro)") && !rest.contains("(*)");
        if under.is_none() && workspace && shared.contains(&name) {
            under = Some(depth);
        }
        if under.is_some() {
            extra.insert(name);
        }
    }
    shared.extend(extra);
    Ok(shared)
}

/// What `package` resolves differently from the editor among `shared`.
fn differences(editor: &BTreeSet<String>, package: &str, shared: &BTreeSet<String>) -> Result<Vec<String>> {
    let keep = |l: &String| l.split('|').next().is_some_and(|p| shared.contains(p));
    let plugin: BTreeSet<String> = resolved(package)?.into_iter().filter(keep).collect();
    let editor: BTreeSet<String> = editor.iter().filter(|l| keep(l)).cloned().collect();
    Ok(editor
        .symmetric_difference(&plugin)
        .map(|l| {
            let side = if editor.contains(l) { "editor" } else { "plugin" };
            format!("{side}: {l}")
        })
        .collect())
}

fn library_file(name: &str) -> String {
    format!(
        "{}{}{}",
        std::env::consts::DLL_PREFIX,
        name.replace('-', "_"),
        std::env::consts::DLL_SUFFIX
    )
}

fn add_dir(zip: &mut zip::ZipWriter<std::fs::File>, base: &Path, dir: &Path) -> Result {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            add_dir(zip, base, &path)?;
        } else {
            let rel = path.strip_prefix(base)?.to_string_lossy().replace('\\', "/");
            zip.start_file(rel, zip::write::SimpleFileOptions::default())?;
            zip.write_all(&std::fs::read(&path)?)?;
        }
    }
    Ok(())
}

/// Runs `editor <args>` and returns its last JSON line.
fn editor_json(editor: &Path, args: &[&std::ffi::OsStr]) -> Option<serde_json::Value> {
    let out = std::process::Command::new(editor).args(args).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .and_then(|l| serde_json::from_str(l).ok())
}

pub fn plugins(args: &[String]) -> Result {
    let mut release = true;
    let mut examples = false;
    let mut only: Option<String> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut check = true;
    let mut key: Option<crate::plugin_store::KeySource> = None;
    let mut bundle: Option<Vec<String>> = None;
    let mut bundle_dir: Option<PathBuf> = None;
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--debug" => release = false,
            "--release" => release = true,
            "--examples" => examples = true,
            "--no-check" => check = false,
            "--only" => only = Some(it.next().ok_or("--only needs a package")?.clone()),
            "--out" => out_dir = Some(PathBuf::from(it.next().ok_or("--out needs a folder")?)),
            "--bundle" => {
                // Optional list of ids; a following option is not one.
                let ids = match it.peek() {
                    Some(next) if !next.starts_with("--") => it.next().cloned().unwrap_or_default(),
                    _ => DEFAULT_BUNDLED.join(","),
                };
                bundle = Some(ids.split(',').map(str::trim).filter(|i| !i.is_empty()).map(str::to_owned).collect());
            }
            "--bundle-dir" => bundle_dir = Some(PathBuf::from(it.next().ok_or("--bundle-dir needs a folder")?)),
            "--sign" => {
                key = Some(crate::plugin_store::KeySource::File(PathBuf::from(
                    it.next().ok_or("--sign needs a key file")?,
                )))
            }
            "--sign-env" => {
                key = Some(crate::plugin_store::KeySource::Env(it.next().ok_or("--sign-env needs a variable")?.clone()))
            }
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            other => return Err(format!("unknown option `{other}`\n\n{HELP}").into()),
        }
    }
    // Read the key first: a bad key fails before the long builds.
    let seed = key.as_ref().map(|k| k.load()).transpose()?;
    if let Some(seed) = &seed {
        println!("signing with the key {}", orbit_plugin_store::signing::public_key_hex(seed));
    }
    let meta = metadata()?;
    let target = PathBuf::from(meta["target_directory"].as_str().unwrap_or("target"));
    let editor_version = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["name"] == "orbit_editor")
        .and_then(|p| p["version"].as_str())
        .ok_or("orbit_editor not found in the workspace")?
        .to_owned();
    let mut crates = plugin_crates(&meta, examples);
    if let Some(name) = &only {
        crates.retain(|c| &c.name == name);
        if crates.is_empty() {
            return Err(format!("no plugin package `{name}`").into());
        }
    }
    if crates.is_empty() {
        println!("no plugin to build");
        return Ok(());
    }

    if check {
        let shared = shared_crates()?;
        let editor = resolved("orbit_editor")?;
        for c in &crates {
            let diff = differences(&editor, &c.name, &shared)?;
            if !diff.is_empty() {
                return Err(format!(
                    "{} resolves crates shared with the editor differently (their TypeIds would differ, \
                     the editor would refuse it); depend on orbit_editor_sdk and do not enable other \
                     features of these crates:\n  {}",
                    c.name,
                    diff.join("\n  ")
                )
                .into());
            }
        }
        println!(
            "ok: the plugins build the {} crates shared with the editor exactly like the editor",
            shared.len()
        );
    }

    let profile = if release { "release" } else { "debug" };
    let out_dir = out_dir.unwrap_or_else(|| target.join("plugins"));
    std::fs::create_dir_all(&out_dir)?;
    let editor_exe = target.join(profile).join(format!("orbit-editor{}", std::env::consts::EXE_SUFFIX));
    let editor_abi = editor_exe
        .is_file()
        .then(|| editor_json(&editor_exe, &["--plugin-abi".as_ref()]))
        .flatten();
    if seed.is_some() && editor_abi.is_none() {
        return Err(format!(
            "signing needs the editor of this profile to read the plugins' fingerprint: build it first \
             (cargo build -p orbit_editor{}); {} not found",
            if release { " --release" } else { "" },
            editor_exe.display()
        )
        .into());
    }

    let bundle_dir = bundle_dir.unwrap_or_else(|| workspace_root().join("packaging").join("bundled-plugins"));
    let mut bundled: BTreeSet<String> = BTreeSet::new();
    for c in &crates {
        let mut build = cargo();
        build.args(["build", "-p", &c.name]);
        if release {
            build.arg("--release");
        }
        run(&mut build)?;
        let lib_name = library_file(&c.name);
        let lib = target.join(profile).join(&lib_name);
        if !lib.is_file() {
            return Err(format!("{} was not produced", lib.display()).into());
        }
        let text = std::fs::read_to_string(c.dir.join("manifest.ron"))?;
        let mut manifest: Manifest = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(&text)
            .map_err(|e| format!("{}: manifest.ron: {e}", c.name))?;
        manifest.library = lib_name.clone();
        manifest.version = c.version.clone();
        // A native plugin is tied to the editor build it was compiled with.
        manifest.min_editor = Some(editor_version.clone());
        manifest.max_editor = Some(editor_version.clone());

        let file = out_dir.join(format!(
            "{}-{}-{}-{}.orbitplugin",
            manifest.id,
            manifest.version,
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&file)?);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.ron", options)?;
        zip.write_all(
            ron::ser::to_string_pretty(&manifest, ron::ser::PrettyConfig::default())?.as_bytes(),
        )?;
        zip.start_file(lib_name.as_str(), options)?;
        zip.write_all(&std::fs::read(&lib)?)?;
        for sub in ["locales", "assets"] {
            add_dir(&mut zip, &c.dir, &c.dir.join(sub))?;
        }
        zip.finish()?;
        let size = std::fs::metadata(&file)?.len();
        println!("{} ({:.1} MB)", file.display(), size as f64 / 1e6);
        if bundle.as_ref().is_some_and(|ids| ids.contains(&manifest.id)) {
            std::fs::create_dir_all(&bundle_dir)?;
            // One package per id: drop the other versions.
            let prefix = format!("{}-", manifest.id);
            for old in std::fs::read_dir(&bundle_dir)?.flatten() {
                let name = old.file_name().to_string_lossy().into_owned();
                if name.starts_with(&prefix) && name.ends_with(".orbitplugin") {
                    std::fs::remove_file(old.path())?;
                }
            }
            let dest = bundle_dir.join(file.file_name().ok_or("package without a name")?);
            std::fs::copy(&file, &dest)?;
            println!("  bundled: {}", dest.display());
            bundled.insert(manifest.id.clone());
        }

        if let Some(editor) = &editor_abi {
            let probe = editor_json(&editor_exe, &["--probe-plugin".as_ref(), lib.as_os_str()]);
            match probe.as_ref().map(|p| (&p["ok"], &p["info"]["fingerprint"])) {
                Some((serde_json::Value::Bool(true), fp)) if fp == editor => {
                    println!("  compatible with {}", editor_exe.display());
                    crate::plugin_store::write_descriptor(
                        &file,
                        &manifest.id,
                        &manifest.name,
                        &manifest.description,
                        &manifest.author,
                        manifest.homepage.clone(),
                        &manifest.version,
                        &editor_version,
                        fp,
                        seed.as_ref(),
                    )?;
                }
                Some((serde_json::Value::Bool(true), fp)) => {
                    let text = format!(
                        "NOT compatible with {} (rebuild the editor with this profile)\n  plugin: {fp}\n  editor: {editor}",
                        editor_exe.display()
                    );
                    if seed.is_some() {
                        return Err(format!("{}: {text}; not signed", c.name).into());
                    }
                    println!("  warning: {text}");
                }
                _ if seed.is_some() => {
                    return Err(format!("the probe of {} failed: {probe:?}; not signed", lib.display()).into());
                }
                _ => println!("  warning: the probe of {} failed: {probe:?}", lib.display()),
            }
        }
    }
    if let Some(ids) = &bundle {
        let missing: Vec<&String> = ids.iter().filter(|i| !bundled.contains(*i)).collect();
        if !missing.is_empty() {
            return Err(format!(
                "no package built for the bundled plugin id(s) {} (check --only and the ids)",
                missing.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", ")
            )
            .into());
        }
        println!("{} plugin package(s) in {}", bundled.len(), bundle_dir.display());
    }
    Ok(())
}
