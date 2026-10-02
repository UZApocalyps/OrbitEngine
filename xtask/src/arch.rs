//! `cargo xtask check-arch`: architecture rules from ROADMAP 1.5 / 1.6.
//!
//! * `orbit_core`, `orbit_project` and `orbit_vm` must not depend (even transitively, on
//!   any target) on wgpu, winit, egui or our own gfx/platform crates.
//! * They must compile for `wasm32-unknown-unknown`.
//! * `orbit_player` must never depend on the editor, egui or the MCP server.

use std::collections::BTreeSet;

use crate::{Result, cargo, run};

const CORE_CRATES: &[&str] = &["orbit_core", "orbit_project", "orbit_vm"];

/// Package names (or prefixes ending with `*`) forbidden for core crates.
const CORE_FORBIDDEN: &[&str] = &[
    "wgpu",
    "wgpu-*",
    "winit",
    "egui",
    "egui-*",
    "egui_*",
    "eframe",
    "kira",
    "cpal",
    "orbit_gfx",
    "orbit_platform",
    "orbit_render2d",
    "orbit_text",
    "orbit_audio",
    "orbit_player",
    "orbit_editor",
];

const PLAYER_FORBIDDEN: &[&str] = &[
    "egui",
    "egui-*",
    "egui_*",
    "eframe",
    "orbit_editor",
    "orbit_mcp",
    "rmcp",
    "rmcp-*",
];

fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// All transitive dependency names of `package` (all targets, all kinds).
fn dependencies(package: &str) -> Result<BTreeSet<String>> {
    let out = cargo()
        .args([
            "tree",
            "-p",
            package,
            "--edges",
            "normal,build,dev",
            "--target",
            "all",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])
        .output()?;
    if !out.status.success() {
        return Err(format!(
            "cargo tree failed for {package}: {}",
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_owned)
        .collect())
}

fn forbidden_found(deps: &BTreeSet<String>, forbidden: &[&str]) -> Vec<String> {
    deps.iter()
        .filter(|d| forbidden.iter().any(|p| matches(p, d)))
        .cloned()
        .collect()
}

pub fn check() -> Result {
    let mut failures = Vec::new();

    for krate in CORE_CRATES {
        let deps = dependencies(krate)?;
        let bad = forbidden_found(&deps, CORE_FORBIDDEN);
        if bad.is_empty() {
            println!(
                "ok: {krate} has no GPU/window dependency ({} deps checked)",
                deps.len()
            );
        } else {
            failures.push(format!(
                "{krate} depends on forbidden crates: {}",
                bad.join(", ")
            ));
        }
    }

    let player = dependencies("orbit_player")?;
    let bad = forbidden_found(&player, PLAYER_FORBIDDEN);
    if bad.is_empty() {
        println!("ok: orbit_player has no editor/egui/MCP dependency");
    } else {
        failures.push(format!(
            "orbit_player depends on forbidden crates: {}",
            bad.join(", ")
        ));
    }

    if !failures.is_empty() {
        return Err(failures.join("\n").into());
    }

    let mut build = cargo();
    build.args(["build", "--target", "wasm32-unknown-unknown"]);
    for krate in CORE_CRATES {
        build.args(["-p", krate]);
    }
    run(&mut build)?;
    println!("ok: core crates compile for wasm32-unknown-unknown");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_matching() {
        assert!(matches("wgpu-*", "wgpu-core"));
        assert!(!matches("wgpu-*", "wgpu"));
        assert!(matches("winit", "winit"));
        assert!(!matches("winit", "winit-x11"));
    }

    #[test]
    fn detects_forbidden() {
        let deps: BTreeSet<String> = ["serde", "wgpu-hal"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(forbidden_found(&deps, CORE_FORBIDDEN), ["wgpu-hal"]);
    }
}
