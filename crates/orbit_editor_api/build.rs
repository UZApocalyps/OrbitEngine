//! Captures the build facts a native editor plugin must share with the editor
//! (see `src/native.rs`): the exact compiler, target and profile, and a hash
//! of the sources of every crate whose types cross the plugin boundary.
//!
//! Both the editor and each plugin compile this crate, so both get these
//! values from their own build; the plugin manager compares them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Crates whose types a plugin sees (`EditorContext`, `Project`, `Story`...),
/// relative to this crate's folder.
const API_CRATES: &[&str] = &[
    ".",
    "../orbit_plugin_api",
    "../orbit_core",
    "../orbit_anim",
    "../orbit_project",
    "../orbit_script",
    "../orbit_vm",
];

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let version = Command::new(&rustc)
        .arg("-vV")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    // "rustc 1.95.0 (59807616e 2026-04-14)" plus the full commit hash.
    let first = version.lines().next().unwrap_or("rustc ?").trim().to_owned();
    let commit = version
        .lines()
        .find_map(|l| l.strip_prefix("commit-hash: "))
        .unwrap_or("")
        .trim()
        .to_owned();
    let rustc_id = if commit.is_empty() { first } else { format!("{first} [{commit}]") };
    println!("cargo:rustc-env=ORBIT_PLUGIN_RUSTC={rustc_id}");

    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo:rustc-env=ORBIT_PLUGIN_TARGET={target}");
    // `debug` or `release` (custom profiles report the one they inherit).
    let profile = std::env::var("PROFILE").unwrap_or_default();
    println!("cargo:rustc-env=ORBIT_PLUGIN_PROFILE={profile}");

    let mut hash = Fnv::new();
    for krate in API_CRATES {
        let dir = manifest_dir.join(krate);
        let name = dir
            .canonicalize()
            .ok()
            .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| (*krate).to_owned());
        let mut files = Vec::new();
        files.push(dir.join("Cargo.toml"));
        collect(&dir.join("src"), &mut files);
        files.sort();
        for file in &files {
            let rel = file
                .strip_prefix(&dir)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            hash.write(name.as_bytes());
            hash.write(b"/");
            hash.write(rel.as_bytes());
            hash.write(b"\0");
            if let Ok(bytes) = std::fs::read(file) {
                // Line endings depend on the checkout (git autocrlf): ignored.
                let text: Vec<u8> = bytes.into_iter().filter(|b| *b != b'\r').collect();
                hash.write(&text);
            }
            hash.write(b"\0");
        }
        println!("cargo:rerun-if-changed={}", dir.join("src").display());
        println!("cargo:rerun-if-changed={}", dir.join("Cargo.toml").display());
    }
    println!("cargo:rustc-env=ORBIT_PLUGIN_SRC_HASH={:016x}", hash.0);
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RUSTC");
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// 64-bit FNV-1a (no dependency for a build script).
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}
