//! `cargo xtask web`: wasm32 release build + wasm-bindgen + index.html.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::{Result, cargo, run, workspace_root};

const INDEX_HTML: &str = include_str!("../web/index.html");
const FAVICON_PNG: &[u8] = include_bytes!("../../assets/brand/icon-64.png");
const ICON_PNG: &[u8] = include_bytes!("../../assets/brand/icon-256.png");

/// Version of the `wasm-bindgen` crate resolved in Cargo.lock.
pub(crate) fn locked_wasm_bindgen_version() -> Result<String> {
    let lock = fs::read_to_string(workspace_root().join("Cargo.lock"))?;
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line == "name = \"wasm-bindgen\"" {
            let version = lines.next().unwrap_or_default();
            return version
                .strip_prefix("version = \"")
                .and_then(|v| v.strip_suffix('"'))
                .map(str::to_owned)
                .ok_or_else(|| "malformed Cargo.lock entry for wasm-bindgen".into());
        }
    }
    Err("wasm-bindgen is not in Cargo.lock".into())
}

pub(crate) fn ensure_wasm_bindgen_cli(expected: &str) -> Result {
    let out = Command::new("wasm-bindgen").arg("--version").output().map_err(|_| {
        format!(
            "wasm-bindgen CLI not found. Install it with:\n  cargo install wasm-bindgen-cli --version {expected} --locked"
        )
    })?;
    let found = String::from_utf8_lossy(&out.stdout);
    if found.split_whitespace().nth(1) != Some(expected) {
        return Err(format!(
            "wasm-bindgen CLI version mismatch: Cargo.lock has {expected}, installed is `{}`.\n\
             Install the matching one with:\n  cargo install wasm-bindgen-cli --version {expected} --locked --force",
            found.trim()
        )
        .into());
    }
    Ok(())
}

/// Copies `src` into `dst` recursively, skipping directories named in `skip`.
fn copy_dir(src: &Path, dst: &Path, skip: &[&str]) -> Result {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if skip.iter().any(|s| name == *s) {
                continue;
            }
            copy_dir(&path, &dst.join(&name), skip)?;
        } else {
            fs::copy(&path, dst.join(&name))?;
        }
    }
    Ok(())
}

/// Copies the demo assets, the web-safe fonts and the example stories into
/// the bundle (stories keep their repository-relative paths, so the manifest
/// paths `assets/...` resolve against the bundle root). The CJK
/// test fonts (`assets/fonts/cjk`) are deliberately left out: they are several
/// MB and only used by native tests.
fn copy_assets(root: &Path, out_dir: &Path) -> Result {
    let assets = root.join("assets");
    let target = out_dir.join("assets");
    for sub in ["demo", "fonts"] {
        let src = assets.join(sub);
        if src.exists() {
            copy_dir(&src, &target.join(sub), &["cjk"])?;
        }
    }
    let stories = root.join("examples").join("stories");
    if stories.exists() {
        copy_dir(&stories, &out_dir.join("examples").join("stories"), &[])?;
    }
    Ok(())
}

pub fn build() -> Result {
    let version = locked_wasm_bindgen_version()?;
    ensure_wasm_bindgen_cli(&version)?;

    run(cargo().args([
        "build",
        "-p",
        "orbit_player",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
    ]))?;

    let root = workspace_root();
    let wasm = root.join("target/wasm32-unknown-unknown/release/orbit_player.wasm");
    let out_dir = root.join("target/web");
    if out_dir.exists() {
        fs::remove_dir_all(&out_dir)?;
    }
    fs::create_dir_all(&out_dir)?;

    run(Command::new("wasm-bindgen")
        .args([
            "--target",
            "web",
            "--no-typescript",
            "--out-name",
            "orbit_player",
            "--out-dir",
        ])
        .arg(&out_dir)
        .arg(&wasm))?;

    fs::write(out_dir.join("index.html"), INDEX_HTML)?;
    fs::write(out_dir.join("favicon.png"), FAVICON_PNG)?;
    fs::write(out_dir.join("icon-256.png"), ICON_PNG)?;
    copy_assets(&root, &out_dir)?;

    let size = fs::metadata(out_dir.join("orbit_player_bg.wasm"))?.len();
    println!(
        "web bundle ready in {} (wasm: {:.2} MiB). Run `cargo xtask serve`, then open / (demo story), /?story=examples/stories/demo.ron or /?demo (renderer demo).",
        out_dir.display(),
        size as f64 / (1024.0 * 1024.0)
    );
    Ok(())
}
