//! `cargo xtask templates` and `cargo xtask wasm-budget`.
//!
//! A *template* is a precompiled player an export glues a game to (see
//! `orbit_export::templates`). They are written to `target/templates/`:
//!
//! ```text
//! target/templates/windows/orbit-player.exe     (built on Windows)
//! target/templates/linux/orbit-player           (built on Linux, i.e. by the CI)
//! target/templates/web/orbit_player.js          (wasm-bindgen glue)
//! target/templates/web/orbit_player_bg.wasm     (release-wasm profile, wasm-opt -Oz)
//! ```
//!
//! The player is built without the `showcase` feature (no renderer demo, no
//! UI gallery): those are development tools.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::web::{ensure_wasm_bindgen_cli, locked_wasm_bindgen_version};
use crate::{Result, cargo, run, workspace_root};

/// Largest accepted size of the web player (`orbit_player_bg.wasm` after
/// wasm-opt), in bytes. The CI fails above it (`cargo xtask wasm-budget`).
/// Raise it on purpose, with a line in docs/adr/0010-export.md, never to
/// silence a regression.
pub const WASM_BUDGET_BYTES: u64 = 4_700_000;

/// The same, compressed with brotli (what a visitor downloads).
pub const WASM_BUDGET_BROTLI_BYTES: u64 = 1_630_000;

/// Size of the web player before this work (release profile, no wasm-opt),
/// for the record: 10 167 000 bytes.
pub const WASM_BASELINE_BYTES: u64 = 10_167_000;

struct Options {
    out: PathBuf,
    native: bool,
    web: bool,
    wasm_opt: Option<PathBuf>,
    skip_wasm_opt: bool,
    /// Extra native targets (`--target <triple>`), besides the host.
    targets: Vec<String>,
    check_budget: bool,
}

fn parse(args: &[String]) -> Result<Options> {
    let mut o = Options {
        out: workspace_root().join("target/templates"),
        native: true,
        web: true,
        wasm_opt: None,
        skip_wasm_opt: false,
        targets: Vec::new(),
        check_budget: false,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--out" => o.out = PathBuf::from(it.next().ok_or("--out needs a folder")?),
            "--no-native" => o.native = false,
            "--no-web" => o.web = false,
            "--wasm-opt" => {
                o.wasm_opt = Some(PathBuf::from(it.next().ok_or("--wasm-opt needs a path")?))
            }
            "--skip-wasm-opt" => o.skip_wasm_opt = true,
            "--target" => o
                .targets
                .push(it.next().ok_or("--target needs a triple")?.clone()),
            "--check-budget" => o.check_budget = true,
            other => return Err(format!("unknown option `{other}`").into()),
        }
    }
    Ok(o)
}

fn rustc_host() -> Result<String> {
    let out = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("-vV")
        .output()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: ").map(str::to_owned))
        .ok_or_else(|| "cannot read the host triple from `rustc -vV`".into())
}

/// `windows` / `linux` for a target triple.
fn os_of(triple: &str) -> Option<&'static str> {
    if triple.contains("windows") {
        Some("windows")
    } else if triple.contains("linux") {
        Some("linux")
    } else {
        None
    }
}

/// Builds the native player for `triple` (`None` = the host) and copies it
/// into the templates folder.
fn native_template(out: &Path, triple: Option<&str>, host: &str) -> Result {
    let used = triple.unwrap_or(host);
    let Some(os) = os_of(used) else {
        println!("skipping {used}: no export target for it");
        return Ok(());
    };
    let mut build = cargo();
    build.args([
        "build",
        "--release",
        "-p",
        "orbit_player",
        "--no-default-features",
        // Desktop players show 3D scenes (ADR 0014); the web template stays
        // 2D only so that 2D games keep the same wasm.
        "--features",
        "3d",
    ]);
    if let Some(triple) = triple {
        build.args(["--target", triple]);
    }
    if let Err(err) = run(&mut build) {
        if triple.is_some() {
            println!(
                "could not build the {os} player for {used}: {err}\n\
                 (cross-compiling needs that target's linker; the {os} template is produced on a {os} machine, e.g. by the CI)"
            );
            return Ok(());
        }
        return Err(err);
    }
    let exe = if os == "windows" {
        "orbit_player.exe"
    } else {
        "orbit_player"
    };
    let release = match triple {
        Some(t) => workspace_root().join("target").join(t).join("release"),
        None => workspace_root().join("target/release"),
    };
    let dest_dir = out.join(os);
    fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(if os == "windows" {
        "orbit-player.exe"
    } else {
        "orbit-player"
    });
    fs::copy(release.join(exe), &dest)?;
    println!(
        "{os} template: {} ({:.1} MiB)",
        dest.display(),
        mib(fs::metadata(&dest)?.len())
    );
    Ok(())
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// Finds `wasm-opt`: `--wasm-opt`, `$WASM_OPT`, the `PATH`, `~/.cargo/bin`.
fn find_wasm_opt(explicit: Option<&Path>) -> Option<PathBuf> {
    let works = |p: &Path| {
        Command::new(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.extend(explicit.map(Path::to_owned));
    candidates.extend(std::env::var_os("WASM_OPT").map(PathBuf::from));
    candidates.push(PathBuf::from("wasm-opt"));
    if let Some(home) = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|h| PathBuf::from(h).join(".cargo"))
        })
    {
        candidates.push(home.join("bin").join(if cfg!(windows) {
            "wasm-opt.exe"
        } else {
            "wasm-opt"
        }));
    }
    candidates.into_iter().find(|p| works(p))
}

/// Size of `data` compressed with brotli (quality 9: about 12 % above what a
/// quality 11 export produces, in a fraction of the time).
pub fn brotli_size(data: &[u8]) -> Result<u64> {
    let params = brotli::enc::BrotliEncoderParams {
        quality: 9,
        lgwin: 24,
        ..Default::default()
    };
    let mut out = Vec::new();
    brotli::BrotliCompress(&mut &data[..], &mut out, &params)?;
    Ok(out.len() as u64)
}

fn report_size(label: &str, path: &Path) -> Result<u64> {
    let bytes = fs::metadata(path)?.len();
    println!("  {label}: {bytes} bytes ({:.2} MiB)", mib(bytes));
    Ok(bytes)
}

fn web_template(out: &Path, o: &Options) -> Result {
    let version = locked_wasm_bindgen_version()?;
    ensure_wasm_bindgen_cli(&version)?;
    run(cargo().args([
        "build",
        "-p",
        "orbit_player",
        "--target",
        "wasm32-unknown-unknown",
        "--profile",
        "release-wasm",
        "--no-default-features",
    ]))?;
    let root = workspace_root();
    let wasm = root.join("target/wasm32-unknown-unknown/release-wasm/orbit_player.wasm");
    let work = root.join("target/templates-work");
    if work.exists() {
        fs::remove_dir_all(&work)?;
    }
    fs::create_dir_all(&work)?;
    run(Command::new("wasm-bindgen")
        .args([
            "--target",
            "web",
            "--no-typescript",
            "--remove-name-section",
            "--remove-producers-section",
            "--out-name",
            "orbit_player",
            "--out-dir",
        ])
        .arg(&work)
        .arg(&wasm))?;
    let bg = work.join("orbit_player_bg.wasm");
    println!("web player size:");
    report_size("release-wasm + wasm-bindgen", &bg)?;

    if o.skip_wasm_opt {
        println!("  wasm-opt skipped (--skip-wasm-opt)");
    } else if let Some(tool) = find_wasm_opt(o.wasm_opt.as_deref()) {
        let optimized = work.join("optimized.wasm");
        run(Command::new(&tool)
            .args([
                "-Oz",
                "--converge",
                "--low-memory-unused",
                "--zero-filled-memory",
                "--enable-bulk-memory",
                "--enable-nontrapping-float-to-int",
                "--enable-sign-ext",
                "--enable-mutable-globals",
                "--enable-reference-types",
                "--enable-multivalue",
                "--strip-debug",
                "--strip-producers",
            ])
            .arg(&bg)
            .arg("-o")
            .arg(&optimized))?;
        fs::rename(&optimized, &bg)?;
        report_size("after wasm-opt -Oz --converge", &bg)?;
    } else {
        println!(
            "  warning: wasm-opt not found, the web player is left unoptimized (about 15 % bigger).\n  \
             Install binaryen: `cargo install wasm-opt --locked` (or put a binaryen release's wasm-opt on the PATH,\n  \
             or pass --wasm-opt <path>), then run `cargo xtask templates` again."
        );
    }

    let dest = out.join("web");
    fs::create_dir_all(&dest)?;
    fs::copy(work.join("orbit_player.js"), dest.join("orbit_player.js"))?;
    fs::copy(&bg, dest.join("orbit_player_bg.wasm"))?;
    let bytes = fs::metadata(dest.join("orbit_player_bg.wasm"))?.len();
    println!(
        "web template: {} (baseline {} bytes: {:+.1} %)",
        dest.display(),
        WASM_BASELINE_BYTES,
        (bytes as f64 / WASM_BASELINE_BYTES as f64 - 1.0) * 100.0
    );
    Ok(())
}

pub fn templates(args: &[String]) -> Result {
    let o = parse(args)?;
    fs::create_dir_all(&o.out)?;
    if o.native {
        let host = rustc_host()?;
        native_template(&o.out, None, &host)?;
        for triple in &o.targets {
            native_template(&o.out, Some(triple), &host)?;
        }
    }
    if o.web {
        web_template(&o.out, &o)?;
    }
    println!("templates ready in {}", o.out.display());
    if o.check_budget && o.web {
        check_budget(&o.out.join("web/orbit_player_bg.wasm"))?;
    }
    Ok(())
}

/// What is over budget, given the plain and the brotli size.
fn budget_errors(size: u64, packed: u64) -> Vec<String> {
    let mut errors = Vec::new();
    if size > WASM_BUDGET_BYTES {
        errors.push(format!(
            "the web player is {size} bytes, over the budget of {WASM_BUDGET_BYTES}"
        ));
    }
    if packed > WASM_BUDGET_BROTLI_BYTES {
        errors.push(format!(
            "the web player is {packed} bytes with brotli, over the budget of {WASM_BUDGET_BROTLI_BYTES}"
        ));
    }
    errors
}

/// Fails when the web player is over the documented budget.
pub fn check_budget(wasm: &Path) -> Result {
    let data = fs::read(wasm).map_err(|e| format!("cannot read {}: {e}", wasm.display()))?;
    let size = data.len() as u64;
    let packed = brotli_size(&data)?;
    println!(
        "wasm budget: {size} / {WASM_BUDGET_BYTES} bytes, brotli {packed} / {WASM_BUDGET_BROTLI_BYTES} bytes"
    );
    let errors = budget_errors(size, packed);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{}\nFind what grew (cargo bloat, twiggy) or, if the growth is deliberate, raise the budget in xtask/src/templates.rs and explain it in docs/adr/0010-export.md.",
            errors.join("\n")
        )
        .into())
    }
}

/// `cargo xtask wasm-budget [--file <wasm>]`: the check alone.
pub fn budget(args: &[String]) -> Result {
    let mut file = workspace_root().join("target/templates/web/orbit_player_bg.wasm");
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--file" => file = PathBuf::from(it.next().ok_or("--file needs a path")?),
            other => return Err(format!("unknown option `{other}`").into()),
        }
    }
    check_budget(&file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options() {
        let args = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let o = parse(&args(&[
            "--no-web",
            "--target",
            "x86_64-unknown-linux-gnu",
            "--skip-wasm-opt",
        ]))
        .unwrap();
        assert!(!o.web && o.native && o.skip_wasm_opt);
        assert_eq!(o.targets, ["x86_64-unknown-linux-gnu"]);
        assert!(parse(&args(&["--bogus"])).is_err());
        assert!(parse(&args(&["--out"])).is_err());
    }

    #[test]
    fn target_triples_map_to_export_targets() {
        assert_eq!(os_of("x86_64-pc-windows-msvc"), Some("windows"));
        assert_eq!(os_of("x86_64-unknown-linux-gnu"), Some("linux"));
        assert_eq!(os_of("aarch64-apple-darwin"), None);
    }

    #[test]
    fn the_budget_fails_when_either_size_is_over() {
        assert!(budget_errors(WASM_BUDGET_BYTES, WASM_BUDGET_BROTLI_BYTES).is_empty());
        assert_eq!(budget_errors(WASM_BUDGET_BYTES + 1, 0).len(), 1);
        assert_eq!(budget_errors(0, WASM_BUDGET_BROTLI_BYTES + 1).len(), 1);
        assert_eq!(budget_errors(u64::MAX, u64::MAX).len(), 2);
    }

    #[test]
    fn a_tiny_file_is_within_budget_and_a_missing_one_is_an_error() {
        let dir = std::env::temp_dir().join(format!("orbit_xtask_budget_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let small = dir.join("small.wasm");
        fs::write(&small, b"\0asm\x01\0\0\0").unwrap();
        assert!(check_budget(&small).is_ok());
        assert!(check_budget(&dir.join("missing.wasm")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
