//! `orbit build` and `orbit export`: thin wrappers over `orbit_export`.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use orbit_export::{
    Event, ExportError, ExportOptions, ExportProject, ExportTarget, LogLevel, Server, Templates,
    build, export_many, open_in_browser,
};

pub const USAGE: &str = "\
    orbit build --project <dir> [--out <dir>] [--profile <name>]
        Compile the project: the story is validated (errors stop the build),
        compiled to a binary game pack with its translations, and the assets
        it uses are packed into <out>/<game id>.pak (default out: <project>/build).
        Play it with: orbit-player --pak <file>
    orbit export <windows|linux|web|android|all> --project <dir> --out <dir>
                 [--profile <name>] [--templates <dir>] [--serve [--port <n>] [--no-open]]
        Export the game for a platform: a zip (Windows), a tar.gz (Linux) or a
        folder for a web server (web). `all` uses the targets of the profile.
        The player templates come from --templates, $ORBIT_TEMPLATES, or the
        `templates` folder next to this program (see `cargo xtask templates`).
        --serve   web only: serve the exported folder on 127.0.0.1 to try it
                  (opens the browser unless --no-open; stop with Ctrl+C)
        android: an APK or AAB (profile option), needs the Android SDK build-tools
                  and a JDK. Sign with --android-keystore <file> --android-alias <name>
                  (password in $ORBIT_ANDROID_KEYSTORE_PASSWORD, never stored) or
                  --android-debug (the Android debug keystore, for tests only).
";

struct Common {
    project: Option<PathBuf>,
    out: Option<PathBuf>,
    profile: Option<String>,
    templates: Option<PathBuf>,
    serve: bool,
    port: Option<u16>,
    no_open: bool,
    android_keystore: Option<PathBuf>,
    android_alias: Option<String>,
    android_debug: bool,
}

fn parse(args: &[String]) -> Result<(Vec<String>, Common), String> {
    let mut common = Common {
        project: None,
        out: None,
        profile: None,
        templates: None,
        serve: false,
        port: None,
        no_open: false,
        android_keystore: None,
        android_alias: None,
        android_debug: false,
    };
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--project" => common.project = Some(PathBuf::from(value("--project")?)),
            "--out" => common.out = Some(PathBuf::from(value("--out")?)),
            "--profile" => common.profile = Some(value("--profile")?),
            "--templates" => common.templates = Some(PathBuf::from(value("--templates")?)),
            "--port" => {
                let text = value("--port")?;
                common.port = Some(text.parse().map_err(|_| format!("invalid port `{text}`"))?);
            }
            "--serve" => common.serve = true,
            "--no-open" => common.no_open = true,
            "--android-keystore" => {
                common.android_keystore = Some(PathBuf::from(value("--android-keystore")?));
            }
            "--android-alias" => common.android_alias = Some(value("--android-alias")?),
            "--android-debug" => common.android_debug = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            other => positional.push(other.to_owned()),
        }
    }
    Ok((positional, common))
}

/// Prints the events of an export: steps (when the label changes), journal
/// lines. Progress and warnings go to `err`, so `out` only has results.
fn printer<'a>(err: &'a mut dyn Write) -> impl FnMut(&Event) + 'a {
    let mut last = String::new();
    move |event| match event {
        Event::Step { fraction, label } => {
            if *label != last {
                let _ = writeln!(err, "[{:>3.0}%] {label}", fraction * 100.0);
                last.clone_from(label);
            }
        }
        Event::Log {
            level: LogLevel::Warning,
            text,
        } => {
            let _ = writeln!(err, "warning: {text}");
        }
        Event::Log { text, .. } => {
            let _ = writeln!(err, "{text}");
        }
    }
}

fn options(common: &Common) -> ExportOptions {
    let mut templates = Templates::standard();
    if let Some(dir) = &common.templates {
        templates = templates.with_dir_first(dir);
    }
    let mut android = orbit_export::android::AndroidOptions::default();
    if common.android_debug {
        android.signing = orbit_export::android::Signing::Debug;
    } else if let Some(path) = &common.android_keystore {
        android.signing = orbit_export::android::Signing::Keystore {
            path: path.clone(),
            alias: common.android_alias.clone().unwrap_or_default(),
            password: orbit_export::android::Secret::new(
                std::env::var("ORBIT_ANDROID_KEYSTORE_PASSWORD").unwrap_or_default(),
            ),
        };
    }
    ExportOptions {
        templates,
        cancel: None,
        android,
    }
}

/// Content problems (invalid story, missing assets) exit with 1, like
/// `orbit validate`; anything else is an error of use or of the environment.
fn exit_code(error: ExportError, err: &mut dyn Write) -> Result<u8, String> {
    match error {
        ExportError::Invalid(_)
        | ExportError::MissingAssets(_)
        | ExportError::Translation { .. }
        | ExportError::Icon { .. } => {
            let _ = writeln!(err, "error: {error}");
            Ok(1)
        }
        other => Err(other.to_string()),
    }
}

pub fn build_cmd(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<u8, String> {
    let (positional, common) = parse(args)?;
    if !positional.is_empty() {
        return Err("usage: orbit build --project <dir> [--out <dir>] [--profile <name>]".into());
    }
    let project_path = common
        .project
        .clone()
        .ok_or("usage: orbit build --project <dir> [--out <dir>] [--profile <name>]")?;
    let project = ExportProject::open(&project_path).map_err(|e| e.to_string())?;
    let profile = project
        .profile(common.profile.as_deref())
        .map_err(|e| e.to_string())?;
    let out_dir = common
        .out
        .clone()
        .unwrap_or_else(|| project.root.join("build"));
    let result = {
        let mut progress = printer(err);
        build(
            &project,
            &profile,
            &out_dir,
            &options(&common),
            &mut progress,
        )
    };
    match result {
        Ok(report) => {
            let _ = writeln!(
                out,
                "{}: {} file(s), {} bytes ({} bytes before compression)",
                report.pak_path.display(),
                report.pak.entries,
                report.pak.pak_bytes,
                report.pak.raw_bytes
            );
            Ok(0)
        }
        Err(error) => exit_code(error, err),
    }
}

pub fn export_cmd(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<u8, String> {
    const USAGE_LINE: &str = "usage: orbit export <windows|linux|web|android|all> --project <dir> --out <dir> [--profile <name>]";
    let (positional, common) = parse(args)?;
    let [target] = positional.as_slice() else {
        return Err(USAGE_LINE.into());
    };
    let project_path = common.project.clone().ok_or(USAGE_LINE)?;
    let out_dir = common.out.clone().ok_or(USAGE_LINE)?;
    let project = ExportProject::open(&project_path).map_err(|e| e.to_string())?;
    let profile = project
        .profile(common.profile.as_deref())
        .map_err(|e| e.to_string())?;
    let targets: Vec<ExportTarget> = if target.eq_ignore_ascii_case("all") {
        profile.targets.clone()
    } else {
        vec![ExportTarget::parse(target).ok_or(format!(
            "unknown target `{target}` (windows, linux, web, android or all)"
        ))?]
    };
    if common.serve && targets != [ExportTarget::Web] {
        return Err("--serve only works with the web target".into());
    }
    let result = {
        let mut progress = printer(err);
        export_many(
            &project,
            &profile,
            &targets,
            &out_dir,
            &options(&common),
            &mut progress,
        )
    };
    let reports = match result {
        Ok(reports) => reports,
        Err(error) => return exit_code(error, err),
    };
    for report in &reports {
        let _ = writeln!(
            out,
            "{}: {} ({} bytes)",
            report.target.id(),
            report.output.display(),
            report.bytes
        );
    }
    if common.serve
        && let Some(report) = reports.first()
    {
        let log: orbit_export::serve::RequestLog = Arc::new(|line| eprintln!("  {line}"));
        let server = Server::start(report.output.clone(), common.port, Some(log))
            .map_err(|e| format!("cannot start the server: {e}"))?;
        let _ = writeln!(
            out,
            "Serving {} on {} (Ctrl+C to stop)",
            report.output.display(),
            server.url()
        );
        let _ = out.flush();
        if !common.no_open {
            let _ = open_in_browser(&server.url());
        }
        server.wait();
    }
    Ok(0)
}
