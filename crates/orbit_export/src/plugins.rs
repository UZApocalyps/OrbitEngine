//! Export "with plugins" (ADR 0015 §4, M8-04).
//!
//! Plugins are Rust crates linked **statically** into the player (no dylib,
//! ADR 0005). A project keeps them in `plugins/<crate>/`; each crate:
//!
//! * declares the plugin ids it provides in its `Cargo.toml`:
//!   ```toml
//!   [package.metadata.orbit]
//!   plugins = ["acme.dice"]
//!   ```
//! * exports `pub fn orbit_plugins() -> Vec<Box<dyn orbit_plugin_api::RuntimePlugin>>`.
//!
//! When a project has such crates, a native export does not use the
//! precompiled player template: it generates a small crate (`Cargo.toml`,
//! `src/main.rs`, `src/plugins.rs`) depending on `orbit_player` and on the
//! plugin crates, builds it with `cargo build --release`, and uses the
//! result as the template. This needs a Rust toolchain (`cargo` on the
//! `PATH`) and the engine sources (`ORBIT_ENGINE_SRC`, or the tree this
//! exporter was built from); a clear error says what is missing.
//!
//! A story whose custom nodes need a plugin no crate provides is refused.

use std::path::{Path, PathBuf};
use std::process::Command;

use orbit_core::{NodeKind, Story};

use crate::error::{ExportError, Result};

/// Folder of the plugin crates, relative to the project root.
pub const PLUGINS_DIR: &str = "plugins";
/// Environment variable naming the engine source tree (holding
/// `crates/orbit_player`).
pub const ENGINE_SRC_ENV: &str = "ORBIT_ENGINE_SRC";

/// A plugin crate found in the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginCrate {
    /// Package name (`Cargo.toml` `[package] name`).
    pub name: String,
    pub dir: PathBuf,
    /// Plugin ids the crate provides (`[package.metadata.orbit] plugins`).
    pub plugin_ids: Vec<String>,
}

impl PluginCrate {
    /// The crate name as a Rust path (`-` becomes `_`).
    pub fn lib_name(&self) -> String {
        self.name.replace('-', "_")
    }
}

fn plugin_error(message: impl Into<String>) -> ExportError {
    ExportError::Plugins(message.into())
}

/// Reads the bits of a plugin crate's `Cargo.toml` the exporter needs. A
/// tiny line parser: `name = "…"` in `[package]` and `plugins = […]` in
/// `[package.metadata.orbit]` (on one line).
pub fn parse_manifest(text: &str, dir: &Path) -> Result<PluginCrate> {
    let mut section = String::new();
    let mut name = None;
    let mut ids = None;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').trim().to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        match (section.as_str(), key) {
            ("package", "name") => name = Some(value.trim_matches('"').to_owned()),
            ("package.metadata.orbit", "plugins") => {
                let inner = value.trim_start_matches('[').trim_end_matches(']');
                ids = Some(
                    inner
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').to_owned())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>(),
                );
            }
            _ => {}
        }
    }
    let name = name.ok_or_else(|| {
        plugin_error(format!("{}: no `[package] name`", dir.join("Cargo.toml").display()))
    })?;
    let plugin_ids = ids.ok_or_else(|| {
        plugin_error(format!(
            "plugin crate `{name}` does not declare its plugin ids: add \
             `[package.metadata.orbit]` `plugins = [\"your.plugin.id\"]` to its Cargo.toml"
        ))
    })?;
    Ok(PluginCrate {
        name,
        dir: dir.to_owned(),
        plugin_ids,
    })
}

/// The plugin crates of the project (`plugins/*/Cargo.toml`), sorted by
/// name. No `plugins` folder: none.
pub fn discover(project_root: &Path) -> Result<Vec<PluginCrate>> {
    let dir = project_root.join(PLUGINS_DIR);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut crates = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(ExportError::io(&dir))?;
    for entry in entries {
        let entry = entry.map_err(ExportError::io(&dir))?;
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest).map_err(ExportError::io(&manifest))?;
        crates.push(parse_manifest(&text, &entry.path())?);
    }
    crates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(crates)
}

/// Plugin ids the story's custom nodes need, sorted, deduplicated.
pub fn needed_plugins(story: &Story) -> Vec<String> {
    let mut ids: Vec<String> = story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| match &n.kind {
            NodeKind::Custom { plugin_id, .. } => Some(plugin_id.clone()),
            _ => None,
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Refuses a story that needs plugins the project does not provide.
pub fn check_story(story: &Story, crates: &[PluginCrate]) -> Result<()> {
    let missing: Vec<String> = needed_plugins(story)
        .into_iter()
        .filter(|id| !crates.iter().any(|c| c.plugin_ids.contains(id)))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(plugin_error(format!(
            "the story uses plugin(s) the project does not provide: {}. Put each plugin \
             crate in `{PLUGINS_DIR}/<name>/` (declaring its ids in \
             `[package.metadata.orbit] plugins`), or remove its nodes.",
            missing.join(", ")
        )))
    }
}

/// The engine source tree: `ORBIT_ENGINE_SRC`, else the tree this crate
/// was built from (when it is still there).
pub fn engine_source() -> Result<PathBuf> {
    let candidates = std::env::var_os(ENGINE_SRC_ENV)
        .map(PathBuf::from)
        .into_iter()
        .chain(std::iter::once(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(".."),
        ));
    for dir in candidates {
        if dir.join("crates").join("orbit_player").join("Cargo.toml").is_file() {
            return Ok(dir.canonicalize().unwrap_or(dir));
        }
    }
    Err(plugin_error(format!(
        "exporting with plugins builds a custom player and needs the OrbitEngine sources: \
         set {ENGINE_SRC_ENV} to the engine folder (the one holding `crates/orbit_player`)"
    )))
}

/// `cargo --version`, or a clear error when there is no Rust toolchain.
pub fn cargo_version() -> Result<String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(&cargo).arg("--version").output().map_err(|_| {
        plugin_error(
            "this project has Rust plugins (`plugins/`): exporting it builds a custom player \
             and needs a Rust toolchain. Install it from https://rustup.rs, then export again.",
        )
    })?;
    if !out.status.success() {
        return Err(plugin_error("`cargo --version` failed: is the Rust toolchain installed?"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn toml_path(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Text of the generated `src/plugins.rs`: installs every plugin of every
/// crate in a registry.
pub fn plugins_rs(crates: &[PluginCrate]) -> String {
    let mut s = String::from(
        "//! Generated by orbit_export: the project's plugins, linked statically.\n\n\
         pub fn registry() -> orbit_plugin_api::RuntimeRegistry {\n    \
         let mut reg = orbit_plugin_api::RuntimeRegistry::new();\n",
    );
    for c in crates {
        s.push_str(&format!(
            "    for plugin in {}::orbit_plugins() {{\n        \
             if let Err(e) = reg.install(plugin.as_ref()) {{\n            \
             log::error!(\"plugin crate `{}`: {{e}}\");\n        }}\n    }}\n",
            c.lib_name(),
            c.name
        ));
    }
    s.push_str("    reg\n}\n");
    s
}

const MAIN_RS: &str = "//! Generated by orbit_export: the player with the project's plugins.\n\n\
mod plugins;\n\n\
fn main() {\n    \
orbit_player::init_logging();\n    \
let host = plugins::registry().into_host();\n    \
if let Err(err) = orbit_player::run_with_plugins(host) {\n        \
log::error!(\"player failed: {err}\");\n        \
std::process::exit(1);\n    }\n}\n";

/// Text of the generated `Cargo.toml`. Its own `[workspace]` keeps it out
/// of any enclosing workspace.
pub fn cargo_toml(crates: &[PluginCrate], engine: &Path, with_3d: bool) -> String {
    let crates_dir = engine.join("crates");
    let features = if with_3d { "features = [\"3d\"], " } else { "" };
    let mut s = format!(
        "[package]\nname = \"orbit-game-player\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n\
         [[bin]]\nname = \"orbit-player\"\npath = \"src/main.rs\"\n\n\
         [dependencies]\n\
         orbit_player = {{ path = \"{}\", default-features = false, {features}}}\n\
         orbit_plugin_api = {{ path = \"{}\" }}\n\
         log = \"0.4\"\n",
        toml_path(&crates_dir.join("orbit_player")),
        toml_path(&crates_dir.join("orbit_plugin_api")),
    );
    for c in crates {
        s.push_str(&format!(
            "{} = {{ path = \"{}\" }}\n",
            c.name,
            toml_path(&c.dir)
        ));
    }
    // Plugin crates must depend on `orbit_plugin_api` from this same tree
    // (by path), or the trait types would not match.
    s.push_str("\n[workspace]\n\n[profile.release]\nlto = \"thin\"\nstrip = true\n");
    s
}

/// Writes the generated crate into `dir`.
pub fn generate(dir: &Path, crates: &[PluginCrate], engine: &Path, with_3d: bool) -> Result<()> {
    let src = dir.join("src");
    std::fs::create_dir_all(&src).map_err(ExportError::io(&src))?;
    let files = [
        (dir.join("Cargo.toml"), cargo_toml(crates, engine, with_3d)),
        (src.join("main.rs"), MAIN_RS.to_owned()),
        (src.join("plugins.rs"), plugins_rs(crates)),
    ];
    for (path, text) in files {
        std::fs::write(&path, text).map_err(ExportError::io(&path))?;
    }
    Ok(())
}

/// Builds the player with the project's plugins for the host platform and
/// returns the executable. `work_dir` holds the generated crate and its
/// `target/` (kept between exports: rebuilds are incremental).
pub fn build_player(crates: &[PluginCrate], work_dir: &Path) -> Result<PathBuf> {
    cargo_version()?;
    let engine = engine_source()?;
    generate(work_dir, crates, &engine, true)?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(&cargo)
        .args(["build", "--release", "--bin", "orbit-player"])
        .current_dir(work_dir)
        .output()
        .map_err(|e| plugin_error(format!("cannot run cargo: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(30).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(plugin_error(format!(
            "building the player with plugins failed:\n{}",
            tail.join("\n")
        )));
    }
    let exe = work_dir
        .join("target")
        .join("release")
        .join(if cfg!(windows) { "orbit-player.exe" } else { "orbit-player" });
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(plugin_error(format!("cargo did not produce {}", exe.display())))
    }
}

#[cfg(test)]
mod tests {
    use orbit_core::{CustomData, Node, NodeId, Scene, SceneId};

    use super::*;

    const DICE: &str = r#"
[package]
name = "orbit-dice"   # the crate
version = "0.1.0"

[package.metadata.orbit]
plugins = ["orbit.dice", "orbit.extra"]

[dependencies]
name = "not this one"
"#;

    #[test]
    fn manifests_are_parsed() {
        let c = parse_manifest(DICE, Path::new("plugins/dice")).unwrap();
        assert_eq!(c.name, "orbit-dice");
        assert_eq!(c.lib_name(), "orbit_dice");
        assert_eq!(c.plugin_ids, vec!["orbit.dice", "orbit.extra"]);
        let err = parse_manifest("[package]\nname = \"x\"\n", Path::new("p")).unwrap_err();
        assert!(err.to_string().contains("package.metadata.orbit"));
    }

    fn story_using(plugin: &str) -> Story {
        let mut story = Story::new("t");
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes: vec![Node {
                id: NodeId::new(),
                kind: NodeKind::Custom {
                    plugin_id: plugin.into(),
                    kind: "roll".into(),
                    version: 1,
                    data: CustomData::Null,
                },
            }],
        });
        story
    }

    #[test]
    fn stories_needing_absent_plugins_are_refused() {
        let c = parse_manifest(DICE, Path::new("plugins/dice")).unwrap();
        assert!(check_story(&story_using("orbit.dice"), std::slice::from_ref(&c)).is_ok());
        let err = check_story(&story_using("acme.cards"), &[c]).unwrap_err();
        assert!(err.to_string().contains("acme.cards"));
        assert!(check_story(&Story::new("t"), &[]).is_ok());
    }

    #[test]
    fn the_generated_crate_installs_every_plugin() {
        let c = parse_manifest(DICE, Path::new("/p/plugins/dice")).unwrap();
        let rs = plugins_rs(std::slice::from_ref(&c));
        assert!(rs.contains("orbit_dice::orbit_plugins()"));
        let toml = cargo_toml(&[c], Path::new("/engine"), true);
        assert!(toml.contains("orbit-dice = { path = \"/p/plugins/dice\" }"));
        assert!(toml.contains("/engine/crates/orbit_player"));
        assert!(toml.contains("[workspace]"));
        assert!(MAIN_RS.contains("run_with_plugins"));
    }

    #[test]
    fn discovery_reads_the_plugins_folder() {
        let root = std::env::temp_dir().join(format!("orbit-plugins-{}", std::process::id()));
        let dir = root.join(PLUGINS_DIR).join("dice");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), DICE).unwrap();
        std::fs::create_dir_all(root.join(PLUGINS_DIR).join("not_a_crate")).unwrap();
        let found = discover(&root).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "orbit-dice");
        assert!(discover(&root.join("nowhere")).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_engine_tree_is_found_from_the_build() {
        assert!(engine_source().is_ok());
    }

    /// End to end: builds a release player with `examples/plugins/dice`.
    /// Slow (a full release build): `cargo test -p orbit_export -- --ignored`.
    #[test]
    #[ignore]
    fn builds_a_player_with_the_dice_plugin() {
        let engine = engine_source().unwrap();
        let project = std::env::temp_dir().join(format!("orbit-dice-proj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&project);
        std::fs::create_dir_all(project.join(PLUGINS_DIR)).unwrap();
        // The example depends on the engine by a relative path: point the
        // copy at the engine tree.
        let src = engine.join("examples").join("plugins").join("dice");
        let dst = project.join(PLUGINS_DIR).join("dice");
        std::fs::create_dir_all(dst.join("src")).unwrap();
        let manifest = std::fs::read_to_string(src.join("Cargo.toml"))
            .unwrap()
            .replace("../../../crates", &toml_path(&engine.join("crates")))
            .replace("[lints]\nworkspace = true\n", "")
            .replace("[lints]\r\nworkspace = true\r\n", "");
        std::fs::write(dst.join("Cargo.toml"), manifest).unwrap();
        std::fs::copy(src.join("src").join("lib.rs"), dst.join("src").join("lib.rs")).unwrap();
        let crates = discover(&project).unwrap();
        assert_eq!(crates[0].plugin_ids, vec!["orbit.dice"]);
        let exe = build_player(&crates, &project.join("build")).unwrap();
        assert!(exe.is_file());
    }
}
