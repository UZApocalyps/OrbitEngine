//! OrbitEngine game export.
//!
//! Turns a project into games players can run, without compiling any Rust:
//!
//! 1. **compile** ([`compile`]): the story is validated (errors block the
//!    export), the assets it uses are found (unused ones are left out, with a
//!    warning), the translations are read, and everything is serialized into
//!    a *game pack* (postcard);
//! 2. **pak** ([`build_pak`]): the game pack, the assets, the engine fonts
//!    and the icon go into one versioned file (`orbit_pak`);
//! 3. **package**: for Windows and Linux the pak is glued to a copy of the
//!    precompiled player ([`Templates`]) and archived (zip, tar.gz); for the
//!    web it is written next to the wasm player, with a page and a loader and
//!    optional brotli / gzip versions of the files.
//!
//! The entry points are [`export`] (one target), [`export_many`] (several,
//! compiling once) and [`build`] (just the pak, what `orbit build` does).
//! They report their progress and journal through a callback and can be
//! cancelled, so the editor runs them on a worker thread; the CLI and the MCP
//! server call the same functions.
//!
//! ```no_run
//! use orbit_export::{ExportOptions, ExportProject, export};
//! use orbit_project::ExportTarget;
//!
//! let project = ExportProject::open("my_game".as_ref())?;
//! let profile = project.profile(None)?;
//! let report = export(
//!     &project,
//!     &profile,
//!     ExportTarget::Web,
//!     "dist".as_ref(),
//!     &ExportOptions::default(),
//!     &mut |event| println!("{event:?}"),
//! )?;
//! println!("{}", report.output.display());
//! # Ok::<(), orbit_export::ExportError>(())
//! ```

pub mod android;
mod compile;
mod error;
mod native;
mod pakbuild;
pub mod plugins;
mod project;
mod report;
pub mod scene3d;
pub mod serve;
mod templates;
#[doc(hidden)]
pub mod testing;
mod tool;
mod web;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub use compile::{AssetFile, Compiled, compile};
pub use error::{ExportError, Result};
pub use orbit_project::{
    AndroidFormat, AndroidOrientation, DesktopQuality, ExportProfile, ExportTarget, PakCompression,
};
pub use pakbuild::{BuiltPak, build_pak};
pub use project::ExportProject;
pub use report::{BuildReport, Event, ExportReport, LogLevel, ProgressFn, Reporter};
pub use serve::{Server, open_in_browser, open_path};
pub use templates::{
    AndroidAbi, AndroidTemplate, ENV_VAR as TEMPLATES_ENV_VAR, Templates, WEB3D_DIR, WebTemplate,
};
pub use tool::{ToolFailure, run_request};

/// What an export needs besides the project.
#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    /// Where the player templates are looked for.
    pub templates: Templates,
    /// Set to `true` from another thread to stop the export as soon as
    /// possible ([`ExportError::Cancelled`]).
    pub cancel: Option<Arc<AtomicBool>>,
    /// Android signing and tools (the keystore password is never stored).
    pub android: android::AndroidOptions,
}

impl ExportOptions {
    /// The standard places for the templates (see [`Templates::standard`]).
    pub fn standard() -> Self {
        Self {
            templates: Templates::standard(),
            cancel: None,
            android: android::AndroidOptions::default(),
        }
    }
}

struct Prepared {
    compiled: Compiled,
    pak: BuiltPak,
}

/// Compiles the project and builds its pak, reporting in the first part of
/// the progress (`0.0..=until`).
fn prepare(
    project: &ExportProject,
    profile: &ExportProfile,
    until: f32,
    targets: &[ExportTarget],
    rep: &mut Reporter<'_>,
) -> Result<Prepared> {
    rep.slice(0.0, until * 0.25);
    rep.step(0.0, "Validating the story");
    let compiled = compile(project, profile)?;
    for warning in &compiled.warnings {
        rep.warn(warning.clone());
    }
    rep.info(format!(
        "{}: {} asset(s), {} translation(s)",
        compiled.pack.info.name,
        compiled.assets.len(),
        compiled.pack.translations.len()
    ));
    if scene3d::uses_3d(&compiled.pack.story) {
        rep.step(0.9, "Checking the 3D scenes against the quality profiles");
        let reports = scene3d::analyze(&project.root, &compiled.pack.story);
        for warning in scene3d::warnings(&reports, targets, profile) {
            rep.warn(warning);
        }
    }
    rep.check_cancelled()?;
    rep.slice(until * 0.25, until);
    let pak = build_pak(&compiled, profile, rep)?;
    rep.info(format!(
        "pak: {} file(s), {} -> {} bytes",
        pak.stats.entries, pak.stats.raw_bytes, pak.stats.stored_bytes
    ));
    Ok(Prepared { compiled, pak })
}

/// `orbit build`: compiles the project and writes its pak,
/// `<out_dir>/<game id>.pak`. It can be played with
/// `orbit-player --pak <file>`.
pub fn build(
    project: &ExportProject,
    profile: &ExportProfile,
    out_dir: &Path,
    options: &ExportOptions,
    progress: ProgressFn<'_>,
) -> Result<BuildReport> {
    let mut rep = Reporter::new(progress, options.cancel.clone());
    // A pak alone is played by the desktop player: desktop tier.
    let prepared = prepare(project, profile, 0.9, &[ExportTarget::Windows], &mut rep)?;
    std::fs::create_dir_all(out_dir).map_err(ExportError::io(out_dir))?;
    let pak_path = out_dir.join(format!("{}.pak", profile.game_id));
    rep.slice(0.9, 1.0);
    rep.step(0.0, &format!("Writing {}", pak_path.display()));
    std::fs::write(&pak_path, &prepared.pak.bytes).map_err(ExportError::io(&pak_path))?;
    rep.step(1.0, "Done");
    Ok(BuildReport {
        pak_path,
        pak: prepared.pak.stats,
        unreferenced: prepared.compiled.unreferenced,
        warnings: rep.warnings,
    })
}

/// Exports the project for `target` into `out_dir`.
pub fn export(
    project: &ExportProject,
    profile: &ExportProfile,
    target: ExportTarget,
    out_dir: &Path,
    options: &ExportOptions,
    progress: ProgressFn<'_>,
) -> Result<ExportReport> {
    let mut reports = export_many(project, profile, &[target], out_dir, options, progress)?;
    Ok(reports.remove(0))
}

/// Exports the project for several targets, compiling it once. Stops at the
/// first failure. The templates of every target are checked first, so a
/// missing one fails before any work is done.
pub fn export_many(
    project: &ExportProject,
    profile: &ExportProfile,
    targets: &[ExportTarget],
    out_dir: &Path,
    options: &ExportOptions,
    progress: ProgressFn<'_>,
) -> Result<Vec<ExportReport>> {
    if targets.is_empty() {
        return Err(ExportError::Profile(vec!["no target to export".to_owned()]));
    }
    // Rust plugins (M8-04): a native player is built with them instead of
    // the precompiled template; other targets are not supported yet.
    let plugin_crates = plugins::discover(&project.root)?;
    if !plugin_crates.is_empty() {
        for target in targets {
            if !native::is_host_target(*target) {
                return Err(ExportError::Plugins(format!(
                    "this project has Rust plugins: it can only be exported for the platform \
                     the editor runs on (not {}) for now",
                    target.label()
                )));
            }
        }
        plugins::cargo_version()?;
        plugins::engine_source()?;
    }
    // Fail fast on a missing template, before compiling anything.
    for target in targets {
        if !plugin_crates.is_empty() {
            continue;
        }
        match target {
            ExportTarget::Web => {
                // Which one is needed is known after compiling (M6-13).
                options
                    .templates
                    .web()
                    .or_else(|_| options.templates.web3d())?;
            }
            ExportTarget::Android => {
                android::preflight(&options.templates, &options.android, profile)?;
            }
            native => {
                options.templates.native(*native)?;
            }
        }
    }
    let mut rep = Reporter::new(progress, options.cancel.clone());
    let prepared = prepare(project, profile, 0.4, targets, &mut rep)?;
    let uses_3d = scene3d::uses_3d(&prepared.compiled.pack.story);
    if uses_3d && targets.contains(&ExportTarget::Web) {
        options.templates.web3d()?;
        rep.info("The game shows 3D scenes: the web export uses the 3D player");
    }
    plugins::check_story(&prepared.compiled.pack.story, &plugin_crates)?;
    let plugin_player = if plugin_crates.is_empty() {
        None
    } else {
        rep.info(format!(
            "Building the player with {} plugin crate(s) (cargo, this can take a few minutes)",
            plugin_crates.len()
        ));
        let work = out_dir.join(".orbit-plugin-player");
        Some(plugins::build_player(&plugin_crates, &work)?)
    };
    let share = 0.6 / targets.len() as f32;
    let mut reports = Vec::new();
    for (i, target) in targets.iter().enumerate() {
        rep.check_cancelled()?;
        let from = 0.4 + share * i as f32;
        rep.slice(from, from + share);
        rep.info(format!("Exporting for {}", target.label()));
        let (output, files) = match target {
            ExportTarget::Web => {
                let template = options.templates.web_for(uses_3d)?;
                let written = web::export(
                    &prepared.pak,
                    prepared.compiled.icon.as_deref(),
                    profile,
                    &template,
                    out_dir,
                    &mut rep,
                )?;
                let brotli = written
                    .wasm_brotli_bytes
                    .map(|b| format!(", {b} bytes with brotli"))
                    .unwrap_or_default();
                rep.info(format!("wasm player: {} bytes{brotli}", written.wasm_bytes));
                (written.dir, written.files)
            }
            ExportTarget::Android => {
                let setup =
                    android::preflight(&options.templates, &options.android, profile)?;
                let written = android::export(
                    &prepared.pak,
                    prepared.compiled.icon.as_deref(),
                    profile,
                    &setup,
                    out_dir,
                    &mut android::SystemRunner,
                    &mut rep,
                )?;
                (written.output, written.files)
            }
            native => {
                let template = match &plugin_player {
                    Some(exe) => exe.clone(),
                    None => options.templates.native(*native)?,
                };
                let written = native::export(
                    &prepared.pak,
                    profile,
                    *native,
                    &template,
                    out_dir,
                    &mut rep,
                )?;
                (written.output, written.files)
            }
        };
        let bytes = files
            .iter()
            .filter_map(|f| std::fs::metadata(f).ok())
            .map(|m| m.len())
            .sum();
        rep.info(format!("Written: {}", output.display()));
        reports.push(ExportReport {
            target: *target,
            profile: profile.name.clone(),
            output,
            files,
            bytes,
            pak: prepared.pak.stats,
            warnings: rep.warnings.clone(),
        });
    }
    rep.slice(0.0, 1.0);
    rep.step(1.0, "Export finished");
    Ok(reports)
}
