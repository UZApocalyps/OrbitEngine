//! Android export (ADR 0017, M7-11): an APK or an AAB assembled around the
//! precompiled player, without Gradle and without Rust on the author's
//! machine.
//!
//! ```text
//! templates/android/classes.dex  + <abi>/liborbit_player.so   (the template)
//!        |
//!  aapt2 compile/link  ->  manifest + icon resources (binary XML, or proto for an AAB)
//!        |
//!  assembly (here)     ->  classes.dex, lib/<abi>/*.so, assets/game.pak (STORED)
//!        |
//!  APK: zipalign -p, apksigner sign      AAB: bundletool build-bundle, jarsigner
//! ```
//!
//! **The pak is stored uncompressed** in `assets/game.pak`: the player reads
//! it in place through `AAssetManager` (M7-10), which cannot seek in a
//! compressed entry. For an AAB, `BundleConfig.json` lists it in
//! `uncompressedGlob` so that bundletool keeps it stored too.
//!
//! **Keystore**: its path is an editor preference (never in the project), its
//! password is asked at every export, lives in a [`Secret`] (never printed,
//! never stored) and reaches the tools through stdin (`apksigner`) or the
//! environment of the child process (`jarsigner`), never through the command
//! line. The *debug* keystore is the public Android one (password `android`),
//! for tests on a device only.
//!
//! The tools are run through the [`Runner`] trait, so the command lines and
//! the file layout are unit-tested without any Android tool installed.

pub mod manifest;
pub mod toolchain;

use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use orbit_project::{AndroidFormat, ExportProfile};

use crate::error::{ExportError, Result};
use crate::native::file_safe;
use crate::pakbuild::BuiltPak;
use crate::report::Reporter;
use crate::templates::{AndroidTemplate, Templates};
use manifest::{ManifestInput, manifest_xml};
use toolchain::{Bundletool, Toolchain};

/// Name of the pak inside `assets/` (what `orbit_player` opens).
pub const GAME_PAK: &str = "game.pak";
/// Alias and password of the debug keystore Android tools create.
pub const DEBUG_ALIAS: &str = "androiddebugkey";
const DEBUG_PASSWORD: &str = "android";
/// Environment variable carrying the keystore password to `jarsigner`.
const STOREPASS_ENV: &str = "ORBIT_ANDROID_STOREPASS";

/// A password: it never shows in `Debug` and has no `Display`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<hidden>)")
    }
}

/// How the package is signed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Signing {
    /// Nothing chosen: the export refuses to start.
    #[default]
    Unset,
    /// The Android debug keystore (created on first use): to try the game on
    /// a device, never to publish.
    Debug,
    /// The author's keystore. The key password must be the keystore's.
    Keystore {
        path: PathBuf,
        alias: String,
        password: Secret,
    },
}

/// Android-only export options (not saved in the project).
#[derive(Clone, Debug, Default)]
pub struct AndroidOptions {
    pub signing: Signing,
    /// Tools to use instead of detecting them (tests, custom setups).
    pub toolchain: Option<Toolchain>,
    /// Where the debug keystore lives; default `~/.android/debug.keystore`.
    pub debug_keystore: Option<PathBuf>,
}

/// The debug keystore path: `~/.android/debug.keystore`.
pub fn default_debug_keystore(env: toolchain::EnvFn<'_>) -> Option<PathBuf> {
    env("USERPROFILE")
        .or_else(|| env("HOME"))
        .map(|h| PathBuf::from(h).join(".android").join("debug.keystore"))
}

// ------------------------------------------------------------------ commands

/// One external command, as data.
#[derive(Clone, Debug)]
pub struct Cmd {
    /// What it does, for the journal and the errors.
    pub label: &'static str,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(String, String)>,
    /// Environment variables holding a password.
    pub secret_env: Vec<(String, Secret)>,
    /// Written to the standard input of the command.
    pub stdin: Option<Secret>,
}

impl Cmd {
    fn new(label: &'static str, program: impl Into<PathBuf>) -> Self {
        Self {
            label,
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            secret_env: Vec::new(),
            stdin: None,
        }
    }

    fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    fn java_home(mut self, jdk: &toolchain::Jdk) -> Self {
        if let Some(home) = &jdk.home {
            self.env
                .push(("JAVA_HOME".to_owned(), home.to_string_lossy().into_owned()));
        }
        self
    }

    /// The command line as shown in the journal (no secret in it).
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_os_str())
            .chain(self.args.iter().map(OsString::as_os_str))
            .map(|a| {
                let text = a.to_string_lossy();
                if text.contains(' ') {
                    format!("\"{text}\"")
                } else {
                    text.into_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Runs the external tools.
pub trait Runner {
    fn run(&mut self, cmd: &Cmd) -> Result<()>;
}

/// Runs the commands for real.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&mut self, cmd: &Cmd) -> Result<()> {
        let mut command = Command::new(&cmd.program);
        command
            .args(&cmd.args)
            .envs(cmd.env.iter().map(|(k, v)| (k, v)))
            .envs(cmd.secret_env.iter().map(|(k, v)| (k, v.expose())))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if cmd.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        let mut child = command.spawn().map_err(|e| {
            let why = if e.kind() == std::io::ErrorKind::NotFound {
                "the program does not exist".to_owned()
            } else {
                e.to_string()
            };
            ExportError::Package(format!(
                "cannot run {} ({}): {why}",
                cmd.label,
                cmd.program.display()
            ))
        })?;
        if let (Some(secret), Some(mut stdin)) = (&cmd.stdin, child.stdin.take()) {
            // A tool that exits early closes the pipe: its error is reported below.
            let _ = writeln!(stdin, "{}", secret.expose());
        }
        let output = child
            .wait_with_output()
            .map_err(|e| ExportError::Package(format!("{} did not finish: {e}", cmd.label)))?;
        if output.status.success() {
            return Ok(());
        }
        let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stdout));
        let tail: Vec<&str> = text.lines().rev().take(12).collect();
        let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
        Err(ExportError::Package(format!(
            "{} failed ({}):\n{tail}",
            cmd.label, output.status
        )))
    }
}

// ------------------------------------------------------------------ preflight

/// The key that signs the package, once checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub path: PathBuf,
    pub alias: String,
    pub password: Secret,
    /// The keystore does not exist yet and must be created (debug only).
    pub create: bool,
}

/// What is needed before anything is compiled.
pub(crate) struct Setup {
    pub template: AndroidTemplate,
    pub toolchain: Toolchain,
    pub key: Key,
}

fn resolve_key(options: &AndroidOptions) -> Result<Key> {
    match &options.signing {
        Signing::Unset => Err(ExportError::Package(
            "Android export: choose how to sign the package (your keystore, or the debug keystore for tests)"
                .to_owned(),
        )),
        Signing::Debug => {
            let path = options
                .debug_keystore
                .clone()
                .or_else(|| default_debug_keystore(&toolchain::process_env))
                .ok_or_else(|| {
                    ExportError::Package("cannot find the home folder for the debug keystore".into())
                })?;
            Ok(Key {
                create: !path.is_file(),
                path,
                alias: DEBUG_ALIAS.to_owned(),
                password: Secret::new(DEBUG_PASSWORD),
            })
        }
        Signing::Keystore {
            path,
            alias,
            password,
        } => {
            if !path.is_file() {
                return Err(ExportError::Package(format!(
                    "the keystore {} does not exist",
                    path.display()
                )));
            }
            if alias.trim().is_empty() {
                return Err(ExportError::Package("the key alias is empty".into()));
            }
            if password.is_empty() {
                return Err(ExportError::Package(
                    "the keystore password is needed to sign the package".into(),
                ));
            }
            Ok(Key {
                path: path.clone(),
                alias: alias.trim().to_owned(),
                password: password.clone(),
                create: false,
            })
        }
    }
}

/// Checks the template, the tools and the signing before doing any work.
pub(crate) fn preflight(
    templates: &Templates,
    options: &AndroidOptions,
    profile: &ExportProfile,
) -> Result<Setup> {
    let template = templates.android()?;
    let toolchain = match &options.toolchain {
        Some(t) => t.clone(),
        None => {
            let detected = toolchain::detect(&toolchain::process_env, &templates.android_dirs());
            toolchain::resolve(&detected, profile.android_format == AndroidFormat::Aab)?
        }
    };
    if profile.android_format == AndroidFormat::Aab && toolchain.bundletool.is_none() {
        return Err(ExportError::Package(
            "bundletool is needed for an AAB: set BUNDLETOOL_JAR or export an APK".into(),
        ));
    }
    let key = resolve_key(options)?;
    Ok(Setup {
        template,
        toolchain,
        key,
    })
}

// ------------------------------------------------------------------ assembly

/// Which package is being assembled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Apk,
    /// The `base` module of an app bundle.
    BundleModule,
}

fn zip_err(path: &Path, e: impl std::fmt::Display) -> ExportError {
    ExportError::Package(format!("cannot write {}: {e}", path.display()))
}

/// Writes the package zip: what `aapt2 link` produced (manifest, resources),
/// the dex, the native libraries and the pak. Libraries and pak are STORED
/// (the pak is read in place, the libraries are mapped from the APK).
pub(crate) fn write_package(
    kind: Kind,
    linked: &Path,
    template: &AndroidTemplate,
    pak: &[u8],
    output: &Path,
) -> Result<()> {
    use zip::CompressionMethod::{Deflated, Stored};
    use zip::write::SimpleFileOptions;

    let file = File::create(output).map_err(ExportError::io(output))?;
    let mut writer = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let err = |e: zip::result::ZipError| zip_err(output, e);
    let io = |e: std::io::Error| zip_err(output, e);

    let linked_file = File::open(linked).map_err(ExportError::io(linked))?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(linked_file))
        .map_err(|e| zip_err(linked, e))?;
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|e| zip_err(linked, e))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        if kind == Kind::BundleModule && name == "AndroidManifest.xml" {
            writer
                .raw_copy_file_rename(entry, "manifest/AndroidManifest.xml")
                .map_err(err)?;
        } else {
            writer.raw_copy_file(entry).map_err(err)?;
        }
    }

    let dex_name = match kind {
        Kind::Apk => "classes.dex",
        Kind::BundleModule => "dex/classes.dex",
    };
    let mut dex = Vec::new();
    File::open(&template.dex)
        .and_then(|mut f| f.read_to_end(&mut dex))
        .map_err(ExportError::io(&template.dex))?;
    writer
        .start_file(dex_name, SimpleFileOptions::default().compression_method(Deflated))
        .map_err(err)?;
    writer.write_all(&dex).map_err(io)?;

    let stored = SimpleFileOptions::default().compression_method(Stored);
    for (abi, lib) in &template.libs {
        let data = std::fs::read(lib).map_err(ExportError::io(lib))?;
        writer
            .start_file(
                format!("lib/{}/liborbit_player.so", abi.id()),
                stored.large_file(data.len() as u64 > u32::MAX as u64),
            )
            .map_err(err)?;
        writer.write_all(&data).map_err(io)?;
    }
    writer
        .start_file(
            format!("assets/{GAME_PAK}"),
            stored.large_file(pak.len() as u64 > u32::MAX as u64),
        )
        .map_err(err)?;
    writer.write_all(pak).map_err(io)?;
    let mut inner = writer.finish().map_err(err)?;
    inner.flush().map_err(io)?;
    Ok(())
}

/// `BundleConfig.json`: keeps the pak stored in the bundle.
pub(crate) fn bundle_config() -> String {
    serde_json::json!({
        "compression": { "uncompressedGlob": [format!("assets/{GAME_PAK}")] },
        "optimizations": {
            "splitsConfig": {
                "splitDimension": [{ "value": "LANGUAGE", "negate": true }]
            }
        }
    })
    .to_string()
}

/// Name of the file: `<id>-<version>-android.apk` (or `.aab`).
pub(crate) fn output_name(profile: &ExportProfile) -> String {
    let ext = match profile.android_format {
        AndroidFormat::Apk => "apk",
        AndroidFormat::Aab => "aab",
    };
    format!(
        "{}-{}-android.{ext}",
        profile.game_id,
        file_safe(&profile.version).replace(' ', "_")
    )
}

// ------------------------------------------------------------------ the commands

/// `aapt2 compile` of the icon resources.
pub(crate) fn compile_resources_cmd(tc: &Toolchain, res_dir: &Path, out: &Path) -> Cmd {
    Cmd::new("aapt2 compile", tc.aapt2())
        .args(["compile", "--dir"])
        .args([res_dir.as_os_str(), "-o".as_ref(), out.as_os_str()])
}

/// `aapt2 link`: the binary manifest and resources (proto for an AAB).
pub(crate) fn link_cmd(
    tc: &Toolchain,
    kind: Kind,
    manifest: &Path,
    compiled_res: Option<&Path>,
    out: &Path,
) -> Cmd {
    let mut cmd = Cmd::new("aapt2 link", tc.aapt2())
        .args(["link", "-o"])
        .args([out.as_os_str(), "-I".as_ref(), tc.android_jar.as_os_str()])
        .args(["--manifest".as_ref(), manifest.as_os_str()])
        .args([
            "--min-sdk-version",
            &manifest::MIN_SDK.to_string(),
            "--target-sdk-version",
            &manifest::TARGET_SDK.to_string(),
        ]);
    if let Some(res) = compiled_res {
        cmd = cmd.args(["-R".as_ref(), res.as_os_str()]);
    }
    if kind == Kind::BundleModule {
        cmd = cmd.args(["--proto-format"]);
    }
    cmd.java_home(&tc.jdk)
}

pub(crate) fn zipalign_cmd(tc: &Toolchain, input: &Path, out: &Path) -> Cmd {
    // `-p`: page-align the stored native libraries (extractNativeLibs=false).
    Cmd::new("zipalign", tc.zipalign())
        .args(["-p", "-f", "4"])
        .args([input.as_os_str(), out.as_os_str()])
}

pub(crate) fn apksigner_sign_cmd(tc: &Toolchain, key: &Key, input: &Path, out: &Path) -> Cmd {
    let mut cmd = Cmd::new("apksigner sign", tc.apksigner())
        .args(["sign", "--ks"])
        .args([key.path.as_os_str()])
        .args(["--ks-key-alias", key.alias.as_str(), "--ks-pass", "stdin", "--out"])
        .args([out.as_os_str(), input.as_os_str()])
        .java_home(&tc.jdk);
    cmd.stdin = Some(key.password.clone());
    cmd
}

pub(crate) fn apksigner_verify_cmd(tc: &Toolchain, apk: &Path) -> Cmd {
    Cmd::new("apksigner verify", tc.apksigner())
        .args(["verify", "--min-sdk-version", &manifest::MIN_SDK.to_string()])
        .args([apk.as_os_str()])
        .java_home(&tc.jdk)
}

pub(crate) fn bundletool_cmd(tc: &Toolchain, module: &Path, config: &Path, out: &Path) -> Cmd {
    let args = |cmd: Cmd| {
        let mut modules = OsString::from("--modules=");
        modules.push(module);
        let mut output = OsString::from("--output=");
        output.push(out);
        let mut cfg = OsString::from("--config=");
        cfg.push(config);
        cmd.args([
            OsString::from("build-bundle"),
            modules,
            output,
            cfg,
            OsString::from("--overwrite"),
        ])
    };
    match &tc.bundletool {
        Some(Bundletool::Jar(jar)) => args(
            Cmd::new("bundletool", tc.jdk.tool("java"))
                .args(["-jar".as_ref(), jar.as_os_str()])
                .java_home(&tc.jdk),
        ),
        Some(Bundletool::Program(program)) => {
            args(Cmd::new("bundletool", program).java_home(&tc.jdk))
        }
        None => args(Cmd::new("bundletool", "bundletool")),
    }
}

pub(crate) fn jarsigner_cmd(tc: &Toolchain, key: &Key, aab: &Path) -> Cmd {
    let mut cmd = Cmd::new("jarsigner", tc.jdk.tool("jarsigner"))
        .args(["-keystore"])
        .args([key.path.as_os_str()])
        .args([
            "-storepass:env".to_owned(),
            STOREPASS_ENV.to_owned(),
            "-sigalg".to_owned(),
            "SHA256withRSA".to_owned(),
            "-digestalg".to_owned(),
            "SHA-256".to_owned(),
        ])
        .args([aab.as_os_str()])
        .args([key.alias.as_str()])
        .java_home(&tc.jdk);
    cmd.secret_env
        .push((STOREPASS_ENV.to_owned(), key.password.clone()));
    cmd
}

/// Creates the debug keystore, as Android Studio does.
pub(crate) fn keytool_debug_cmd(tc: &Toolchain, key: &Key) -> Cmd {
    Cmd::new("keytool", tc.jdk.tool("keytool"))
        .args(["-genkeypair", "-keystore"])
        .args([key.path.as_os_str()])
        .args([
            "-storepass",
            DEBUG_PASSWORD,
            "-keypass",
            DEBUG_PASSWORD,
            "-alias",
            DEBUG_ALIAS,
            "-keyalg",
            "RSA",
            "-keysize",
            "2048",
            "-validity",
            "10000",
            "-dname",
            "CN=Android Debug,O=Android,C=US",
        ])
        .java_home(&tc.jdk)
}

// ------------------------------------------------------------------ export

/// Removes the working folder when done, success or not.
struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) struct Written {
    pub output: PathBuf,
    pub files: Vec<PathBuf>,
}

fn run(runner: &mut dyn Runner, rep: &mut Reporter<'_>, cmd: &Cmd) -> Result<()> {
    rep.check_cancelled()?;
    rep.info(format!("$ {}", cmd.display()));
    runner.run(cmd)
}

/// Builds `<out_dir>/<id>-<version>-android.apk|aab`.
pub(crate) fn export(
    pak: &BuiltPak,
    icon: Option<&[u8]>,
    profile: &ExportProfile,
    setup: &Setup,
    out_dir: &Path,
    runner: &mut dyn Runner,
    rep: &mut Reporter<'_>,
) -> Result<Written> {
    let tc = &setup.toolchain;
    let kind = match profile.android_format {
        AndroidFormat::Apk => Kind::Apk,
        AndroidFormat::Aab => Kind::BundleModule,
    };
    std::fs::create_dir_all(out_dir).map_err(ExportError::io(out_dir))?;
    let work = WorkDir(out_dir.join(format!(".orbit-android-{}", profile.game_id)));
    let _ = std::fs::remove_dir_all(&work.0);
    std::fs::create_dir_all(&work.0).map_err(ExportError::io(&work.0))?;
    let w = &work.0;
    let output = out_dir.join(output_name(profile));

    rep.step(0.05, "Android: writing the manifest");
    let input = ManifestInput::from_profile(profile, icon.is_some());
    rep.info(format!(
        "package {} version {} ({})",
        input.package, input.version_name, input.version_code
    ));
    let manifest_path = w.join("AndroidManifest.xml");
    std::fs::write(&manifest_path, manifest_xml(&input)).map_err(ExportError::io(&manifest_path))?;

    let mut compiled_res = None;
    if let Some(png) = icon {
        rep.step(0.15, "Android: compiling the icon");
        let dir = w.join("res").join("mipmap-xxxhdpi");
        std::fs::create_dir_all(&dir).map_err(ExportError::io(&dir))?;
        std::fs::write(dir.join("ic_launcher.png"), png).map_err(ExportError::io(&dir))?;
        let out = w.join("res.zip");
        run(runner, rep, &compile_resources_cmd(tc, &w.join("res"), &out))?;
        compiled_res = Some(out);
    }

    rep.step(0.3, "Android: linking the resources");
    let linked = w.join("linked.apk");
    run(
        runner,
        rep,
        &link_cmd(tc, kind, &manifest_path, compiled_res.as_deref(), &linked),
    )?;

    rep.step(0.5, "Android: assembling the package (pak stored uncompressed)");
    let assembled = w.join(match kind {
        Kind::Apk => "unaligned.apk",
        Kind::BundleModule => "base.zip",
    });
    write_package(kind, &linked, &setup.template, &pak.bytes, &assembled)?;
    for (abi, _) in &setup.template.libs {
        rep.info(format!("native library: {}", abi.id()));
    }

    if setup.key.create {
        rep.step(0.6, "Android: creating the debug keystore");
        if let Some(parent) = setup.key.path.parent() {
            std::fs::create_dir_all(parent).map_err(ExportError::io(parent))?;
        }
        run(runner, rep, &keytool_debug_cmd(tc, &setup.key))?;
    }

    match kind {
        Kind::Apk => {
            rep.step(0.7, "Android: aligning");
            let aligned = w.join("aligned.apk");
            run(runner, rep, &zipalign_cmd(tc, &assembled, &aligned))?;
            rep.step(0.8, "Android: signing");
            run(
                runner,
                rep,
                &apksigner_sign_cmd(tc, &setup.key, &aligned, &output),
            )?;
            rep.step(0.95, "Android: verifying the signature");
            run(runner, rep, &apksigner_verify_cmd(tc, &output))?;
        }
        Kind::BundleModule => {
            rep.step(0.7, "Android: building the bundle");
            let config = w.join("BundleConfig.json");
            std::fs::write(&config, bundle_config()).map_err(ExportError::io(&config))?;
            run(runner, rep, &bundletool_cmd(tc, &assembled, &config, &output))?;
            rep.step(0.9, "Android: signing");
            run(runner, rep, &jarsigner_cmd(tc, &setup.key, &output))?;
        }
    }
    rep.step(1.0, "Android: package written");
    Ok(Written {
        files: vec![output.clone()],
        output,
    })
}

#[cfg(test)]
mod tests;
