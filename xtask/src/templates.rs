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
//! target/templates/web3d/orbit_player.js        (the same with the `3d` feature:
//! target/templates/web3d/orbit_player_bg.wasm    games that show a 3D scene, M6-13)
//! target/templates/android/<abi>/liborbit_player.so + android/classes.dex
//!                                               (when cargo-ndk, the NDK and the rust targets exist)
//! ```
//!
//! The player is built without the `showcase` feature (no renderer demo, no
//! UI gallery): those are development tools.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use orbit_export::AndroidAbi;
use orbit_export::android::toolchain::{Detected, detect, process_env};

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

/// Budget of the 3D web player (`web3d/`, M6-13): the 2D player plus
/// `orbit_render3d` and `orbit_scene` (PBR shaders, shadows, post-process).
/// Same rule as [`WASM_BUDGET_BYTES`]; documented in docs/adr/0014.
pub const WASM3D_BUDGET_BYTES: u64 = 5_400_000;
pub const WASM3D_BUDGET_BROTLI_BYTES: u64 = 1_850_000;

/// One web player variant: where it goes and what it is built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebVariant {
    pub name: &'static str,
    /// Folder under the templates folder.
    pub dir: &'static str,
    /// Cargo features of `orbit_player` (on top of `--no-default-features`).
    pub features: &'static str,
    pub budget: u64,
    pub budget_brotli: u64,
}

/// The 2D web player: what every 2D game ships.
pub const WEB_2D: WebVariant = WebVariant {
    name: "web player",
    dir: "web",
    features: "",
    budget: WASM_BUDGET_BYTES,
    budget_brotli: WASM_BUDGET_BROTLI_BYTES,
};

/// The web player with 3D scenes, used only by games that show one.
pub const WEB_3D: WebVariant = WebVariant {
    name: "3D web player",
    dir: "web3d",
    features: "3d",
    budget: WASM3D_BUDGET_BYTES,
    budget_brotli: WASM3D_BUDGET_BROTLI_BYTES,
};

struct Options {
    out: PathBuf,
    native: bool,
    web: bool,
    /// Also build the 3D web player (`--no-web3d` skips it).
    web3d: bool,
    wasm_opt: Option<PathBuf>,
    skip_wasm_opt: bool,
    /// Extra native targets (`--target <triple>`), besides the host.
    targets: Vec<String>,
    check_budget: bool,
    /// Build the Android player when its tools are there (`--no-android` skips).
    android: bool,
    /// Inputs of `d8` for `classes.dex`: the Games Activity AAR and its
    /// dependencies (`--android-dex-input <file>`, repeatable).
    android_dex_inputs: Vec<PathBuf>,
}

fn parse(args: &[String]) -> Result<Options> {
    let mut o = Options {
        out: workspace_root().join("target/templates"),
        native: true,
        web: true,
        web3d: true,
        wasm_opt: None,
        skip_wasm_opt: false,
        targets: Vec::new(),
        check_budget: false,
        android: true,
        android_dex_inputs: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--out" => o.out = PathBuf::from(it.next().ok_or("--out needs a folder")?),
            "--no-native" => o.native = false,
            "--no-web" => o.web = false,
            "--no-web3d" => o.web3d = false,
            "--wasm-opt" => {
                o.wasm_opt = Some(PathBuf::from(it.next().ok_or("--wasm-opt needs a path")?))
            }
            "--skip-wasm-opt" => o.skip_wasm_opt = true,
            "--target" => o
                .targets
                .push(it.next().ok_or("--target needs a triple")?.clone()),
            "--check-budget" => o.check_budget = true,
            "--no-android" => o.android = false,
            "--android-dex-input" => o.android_dex_inputs.push(PathBuf::from(
                it.next().ok_or("--android-dex-input needs a file")?,
            )),
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

/// What `cargo xtask templates` can do for Android, given what is installed:
/// the ABIs to build, or why to skip.
fn android_plan(
    installed_targets: &[String],
    cargo_ndk: bool,
    ndk: bool,
) -> std::result::Result<Vec<AndroidAbi>, String> {
    if !cargo_ndk {
        return Err("cargo-ndk is not installed (`cargo install cargo-ndk --locked`)".to_owned());
    }
    if !ndk {
        return Err(
            "the Android NDK was not found (install it with sdkmanager `ndk;26.1.10909125`, \
             then set ANDROID_NDK_HOME or ANDROID_HOME)"
                .to_owned(),
        );
    }
    let abis: Vec<AndroidAbi> = AndroidAbi::ALL
        .into_iter()
        .filter(|abi| installed_targets.iter().any(|t| t == abi.triple()))
        .collect();
    if !abis.contains(&AndroidAbi::Arm64) {
        return Err(
            "the rust target aarch64-linux-android is not installed \
             (`rustup target add aarch64-linux-android`)"
                .to_owned(),
        );
    }
    Ok(abis)
}

fn command_works(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn installed_rust_targets() -> Vec<String> {
    Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Builds `liborbit_player.so` for each possible ABI with cargo-ndk, and
/// `classes.dex` (GameActivity) with d8 when its inputs are given. Anything
/// missing skips the Android template with a message, it never fails the
/// whole command.
fn android_template(out: &Path, o: &Options) -> Result {
    let detected = detect(&process_env, &[]);
    let plan = android_plan(
        &installed_rust_targets(),
        command_works("cargo", &["ndk", "--version"]),
        detected.ndk.is_some(),
    );
    let abis = match plan {
        Ok(abis) => abis,
        Err(why) => {
            println!("skipping the Android template: {why}");
            return Ok(());
        }
    };
    let dest = out.join("android");
    for abi in &abis {
        let mut build = cargo();
        build.args(["ndk", "-t", abi.id(), "rustc"]).args([
            "-p",
            "orbit_player",
            "--lib",
            "--release",
            "--no-default-features",
            "--crate-type",
            "cdylib",
        ]);
        if let Some(ndk) = &detected.ndk {
            build.env("ANDROID_NDK_HOME", ndk);
        }
        if let Err(err) = run(&mut build) {
            if *abi == AndroidAbi::Arm64 {
                println!("skipping the Android template: the arm64-v8a build failed: {err}");
                return Ok(());
            }
            println!("no {} library: {err}", abi.id());
            continue;
        }
        let so = workspace_root()
            .join("target")
            .join(abi.triple())
            .join("release")
            .join("liborbit_player.so");
        let dir = dest.join(abi.id());
        fs::create_dir_all(&dir)?;
        fs::copy(&so, dir.join("liborbit_player.so"))
            .map_err(|e| format!("cannot copy {}: {e}", so.display()))?;
        println!("android {}: {}", abi.id(), dir.join("liborbit_player.so").display());
    }
    android_dex(&dest, o, &detected)
}

/// `classes.dex` from the Games Activity AAR (and dependencies) with `d8`.
fn android_dex(dest: &Path, o: &Options, detected: &Detected) -> Result {
    if o.android_dex_inputs.is_empty() {
        println!(
            "android: no classes.dex produced (GameActivity is Java): pass the games-activity AAR and its \
             dependencies with --android-dex-input <file> (repeatable), or copy a prebuilt classes.dex to {}",
            dest.join("classes.dex").display()
        );
        return Ok(());
    }
    let Some(build_tools) = &detected.build_tools else {
        println!("android: d8 needs the SDK build-tools (sdkmanager \"build-tools;34.0.0\"): classes.dex skipped");
        return Ok(());
    };
    let work = workspace_root().join("target/templates-work/android-dex");
    if work.exists() {
        fs::remove_dir_all(&work)?;
    }
    fs::create_dir_all(&work)?;
    let d8 = ["d8", "d8.bat", "d8.exe"]
        .iter()
        .map(|n| build_tools.join(n))
        .find(|p| p.is_file())
        .ok_or_else(|| format!("no d8 in {}", build_tools.display()))?;
    let mut cmd = Command::new(d8);
    cmd.args(["--release", "--min-api", "26", "--output"])
        .arg(&work)
        .args(&o.android_dex_inputs);
    if let Some(jdk_home) = detected.jdk.as_ref().and_then(|j| j.home.as_ref()) {
        cmd.env("JAVA_HOME", jdk_home);
    }
    run(&mut cmd)?;
    let extra = work.join("classes2.dex");
    if extra.exists() {
        return Err("d8 produced several dex files: the Android template holds a single classes.dex".into());
    }
    fs::create_dir_all(dest)?;
    fs::copy(work.join("classes.dex"), dest.join("classes.dex"))?;
    println!("android classes.dex: {}", dest.join("classes.dex").display());
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

fn web_template(out: &Path, o: &Options, variant: WebVariant) -> Result {
    let version = locked_wasm_bindgen_version()?;
    ensure_wasm_bindgen_cli(&version)?;
    let mut build = cargo();
    build.args([
        "build",
        "-p",
        "orbit_player",
        "--target",
        "wasm32-unknown-unknown",
        "--profile",
        "release-wasm",
        "--no-default-features",
    ]);
    if !variant.features.is_empty() {
        build.args(["--features", variant.features]);
    }
    run(&mut build)?;
    let root = workspace_root();
    let wasm = root.join("target/wasm32-unknown-unknown/release-wasm/orbit_player.wasm");
    let work = root.join("target/templates-work").join(variant.dir);
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
    println!("{} size:", variant.name);
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

    let dest = out.join(variant.dir);
    fs::create_dir_all(&dest)?;
    fs::copy(work.join("orbit_player.js"), dest.join("orbit_player.js"))?;
    fs::copy(&bg, dest.join("orbit_player_bg.wasm"))?;
    let bytes = fs::metadata(dest.join("orbit_player_bg.wasm"))?.len();
    println!(
        "{} template: {} (baseline {} bytes: {:+.1} %)",
        variant.name,
        dest.display(),
        WASM_BASELINE_BYTES,
        (bytes as f64 / WASM_BASELINE_BYTES as f64 - 1.0) * 100.0
    );
    Ok(())
}

/// The web variants `o` builds.
fn web_variants(o: &Options) -> Vec<WebVariant> {
    let mut out = Vec::new();
    if o.web {
        out.push(WEB_2D);
        if o.web3d {
            out.push(WEB_3D);
        }
    }
    out
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
    for variant in web_variants(&o) {
        web_template(&o.out, &o, variant)?;
    }
    if o.android {
        android_template(&o.out, &o)?;
    }
    println!("templates ready in {}", o.out.display());
    if o.check_budget {
        for variant in web_variants(&o) {
            check_budget(&o.out.join(variant.dir).join("orbit_player_bg.wasm"), variant)?;
        }
    }
    Ok(())
}

/// What is over budget, given the plain and the brotli size.
fn budget_errors(size: u64, packed: u64, variant: WebVariant) -> Vec<String> {
    let (name, budget, budget_brotli) = (variant.name, variant.budget, variant.budget_brotli);
    let mut errors = Vec::new();
    if size > budget {
        errors.push(format!(
            "the {name} is {size} bytes, over the budget of {budget}"
        ));
    }
    if packed > budget_brotli {
        errors.push(format!(
            "the {name} is {packed} bytes with brotli, over the budget of {budget_brotli}"
        ));
    }
    errors
}

/// Fails when a web player is over its documented budget.
pub fn check_budget(wasm: &Path, variant: WebVariant) -> Result {
    let data = fs::read(wasm).map_err(|e| format!("cannot read {}: {e}", wasm.display()))?;
    let size = data.len() as u64;
    let packed = brotli_size(&data)?;
    println!(
        "wasm budget ({}): {size} / {} bytes, brotli {packed} / {} bytes",
        variant.name, variant.budget, variant.budget_brotli
    );
    let errors = budget_errors(size, packed, variant);
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

/// `cargo xtask wasm-budget [--3d] [--file <wasm>]`: the check alone (the
/// 2D player, or the 3D one with `--3d`).
pub fn budget(args: &[String]) -> Result {
    let mut file = None;
    let mut variant = WEB_2D;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--file" => file = Some(PathBuf::from(it.next().ok_or("--file needs a path")?)),
            "--3d" => variant = WEB_3D,
            other => return Err(format!("unknown option `{other}`").into()),
        }
    }
    let file = file.unwrap_or_else(|| {
        workspace_root()
            .join("target/templates")
            .join(variant.dir)
            .join("orbit_player_bg.wasm")
    });
    check_budget(&file, variant)
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
        assert!(web_variants(&o).is_empty());
        let all = parse(&args(&[])).unwrap();
        assert_eq!(web_variants(&all), [WEB_2D, WEB_3D]);
        let no3d = parse(&args(&["--no-web3d"])).unwrap();
        assert_eq!(web_variants(&no3d), [WEB_2D]);
        assert_eq!(o.targets, ["x86_64-unknown-linux-gnu"]);
        assert!(parse(&args(&["--bogus"])).is_err());
        assert!(parse(&args(&["--out"])).is_err());
    }

    #[test]
    fn android_is_skipped_with_a_reason_when_tools_are_missing() {
        let rust = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let all = rust(&["aarch64-linux-android", "x86_64-linux-android"]);
        let why = android_plan(&all, false, true).unwrap_err();
        assert!(why.contains("cargo-ndk"), "{why}");
        let why = android_plan(&all, true, false).unwrap_err();
        assert!(why.contains("NDK"), "{why}");
        let why = android_plan(&rust(&["x86_64-linux-android"]), true, true).unwrap_err();
        assert!(why.contains("rustup target add aarch64-linux-android"), "{why}");
        assert_eq!(
            android_plan(&all, true, true).unwrap(),
            [AndroidAbi::Arm64, AndroidAbi::X86_64]
        );
        let args = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let o = parse(&args(&["--no-android", "--android-dex-input", "a.aar"])).unwrap();
        assert!(!o.android);
        assert_eq!(o.android_dex_inputs, [PathBuf::from("a.aar")]);
    }

    #[test]
    fn target_triples_map_to_export_targets() {
        assert_eq!(os_of("x86_64-pc-windows-msvc"), Some("windows"));
        assert_eq!(os_of("x86_64-unknown-linux-gnu"), Some("linux"));
        assert_eq!(os_of("aarch64-apple-darwin"), None);
    }

    #[test]
    fn the_budget_fails_when_either_size_is_over() {
        let v = WEB_2D;
        assert!(budget_errors(WASM_BUDGET_BYTES, WASM_BUDGET_BROTLI_BYTES, v).is_empty());
        assert_eq!(budget_errors(WASM_BUDGET_BYTES + 1, 0, v).len(), 1);
        assert_eq!(budget_errors(0, WASM_BUDGET_BROTLI_BYTES + 1, v).len(), 1);
        assert_eq!(budget_errors(u64::MAX, u64::MAX, v).len(), 2);
        // The 3D player has its own, larger budget.
        const { assert!(WASM3D_BUDGET_BYTES > WASM_BUDGET_BYTES) };
        assert!(budget_errors(WASM_BUDGET_BYTES + 1, 0, WEB_3D).is_empty());
        let e = budget_errors(WASM3D_BUDGET_BYTES + 1, 0, WEB_3D);
        assert!(e[0].contains("3D web player"), "{e:?}");
    }

    #[test]
    fn a_tiny_file_is_within_budget_and_a_missing_one_is_an_error() {
        let dir = std::env::temp_dir().join(format!("orbit_xtask_budget_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let small = dir.join("small.wasm");
        fs::write(&small, b"\0asm\x01\0\0\0").unwrap();
        assert!(check_budget(&small, WEB_2D).is_ok());
        assert!(check_budget(&small, WEB_3D).is_ok());
        assert!(check_budget(&dir.join("missing.wasm"), WEB_2D).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
