//! Progress reporting, logging and cancellation of an export.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use orbit_pak::WriteStats;
use orbit_project::ExportTarget;

use crate::error::{ExportError, Result};

/// Severity of a log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
}

/// What the callback of an export is told.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The export is at `fraction` (`0.0..=1.0`, of the whole call) and is
    /// now doing `label`.
    Step { fraction: f32, label: String },
    /// A line for the journal.
    Log { level: LogLevel, text: String },
}

/// Receives the events of an export.
pub type ProgressFn<'a> = &'a mut dyn FnMut(&Event);

/// Sends events to the callback, remembers the warnings and checks the
/// cancellation flag. The fractions it is given are mapped into the slice of
/// the whole export it works for ([`Reporter::slice`]).
pub struct Reporter<'a> {
    callback: Option<ProgressFn<'a>>,
    cancel: Option<Arc<AtomicBool>>,
    base: f32,
    span: f32,
    /// Warnings seen so far, for the report.
    pub warnings: Vec<String>,
}

impl<'a> Reporter<'a> {
    pub fn new(callback: ProgressFn<'a>, cancel: Option<Arc<AtomicBool>>) -> Self {
        Self {
            callback: Some(callback),
            cancel,
            base: 0.0,
            span: 1.0,
            warnings: Vec::new(),
        }
    }

    /// A reporter that reports nothing (tests).
    pub fn silent() -> Reporter<'static> {
        Reporter {
            callback: None,
            cancel: None,
            base: 0.0,
            span: 1.0,
            warnings: Vec::new(),
        }
    }

    /// Restricts the following steps to `from..to` of the whole.
    pub fn slice(&mut self, from: f32, to: f32) {
        self.base = from;
        self.span = (to - from).max(0.0);
    }

    fn emit(&mut self, event: Event) {
        if let Some(callback) = self.callback.as_mut() {
            callback(&event);
        }
    }

    /// Progress: `local` is `0..=1` within the current slice.
    pub fn step(&mut self, local: f32, label: &str) {
        let fraction = (self.base + self.span * local.clamp(0.0, 1.0)).clamp(0.0, 1.0);
        self.emit(Event::Step {
            fraction,
            label: label.to_owned(),
        });
    }

    pub fn info(&mut self, text: impl Into<String>) {
        let text = text.into();
        log::info!("export: {text}");
        self.emit(Event::Log {
            level: LogLevel::Info,
            text,
        });
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        let text = text.into();
        log::warn!("export: {text}");
        self.warnings.push(text.clone());
        self.emit(Event::Log {
            level: LogLevel::Warning,
            text,
        });
    }

    pub fn check_cancelled(&self) -> Result<()> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            Err(ExportError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// What an export produced.
#[derive(Clone, Debug)]
pub struct ExportReport {
    pub target: ExportTarget,
    /// Name of the profile used.
    pub profile: String,
    /// The main output: the `.zip` or `.tar.gz` file, or the folder of the
    /// web bundle (open `index.html` through a web server).
    pub output: PathBuf,
    /// Every file written.
    pub files: Vec<PathBuf>,
    /// Total size of the files written.
    pub bytes: u64,
    /// Content of the pak.
    pub pak: WriteStats,
    /// Everything worth the author's attention (not blocking).
    pub warnings: Vec<String>,
}

/// What `build` produced.
#[derive(Clone, Debug)]
pub struct BuildReport {
    /// The pak file written.
    pub pak_path: PathBuf,
    pub pak: WriteStats,
    /// Assets that are declared but used by nothing (not in the pak).
    pub unreferenced: Vec<String>,
    pub warnings: Vec<String>,
}
