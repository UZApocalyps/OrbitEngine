//! The web player is built for wasm32-unknown-unknown, where
//! `std::time::Instant::now()` and `SystemTime::now()` panic ("time not
//! implemented on this platform"): the web export stopped loading at the
//! first scene when one slipped into the asset loading. The crates of the
//! player use `web_time` instead; this scan keeps it that way.

use std::path::{Path, PathBuf};

/// Crates compiled into the wasm player (`cargo tree -p orbit_player
/// --target wasm32-unknown-unknown`).
const PLAYER_CRATES: &[&str] = &[
    "orbit_anim", "orbit_assets", "orbit_audio", "orbit_core", "orbit_gfx", "orbit_i18n",
    "orbit_pak", "orbit_platform", "orbit_player", "orbit_profile", "orbit_render2d",
    "orbit_render3d", "orbit_runtime", "orbit_save", "orbit_scene", "orbit_script",
    "orbit_text", "orbit_ui", "orbit_ui_data", "orbit_vm",
];

/// Files whose std time is never compiled for wasm32 (native-only modules
/// or tests), relative to `crates/`.
const NATIVE_ONLY: &[&str] = &[
    "orbit_anim/src/rig/tests.rs",
    "orbit_assets/src/watch.rs",    // `mod native`, cfg(not wasm32)
    "orbit_profile/src/lib.rs",     // `mod imp`, cfg(not wasm32)
    "orbit_save/src/file_storage.rs", // cfg(not wasm32)
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn player_crates_do_not_read_std_time() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut offenders = Vec::new();
    for name in PLAYER_CRATES {
        let mut files = Vec::new();
        rust_files(&crates.join(name).join("src"), &mut files);
        for file in files {
            let rel = file.strip_prefix(crates).unwrap().to_string_lossy().replace('\\', "/");
            if NATIVE_ONLY.contains(&rel.as_str()) {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                let std_time = code.contains("std::time::");
                if std_time && (code.contains("Instant") || code.contains("SystemTime")) {
                    offenders.push(format!("{rel}:{}: {}", n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "std::time::Instant/SystemTime panic in the wasm player, use web_time:\n{}",
        offenders.join("\n")
    );
}
