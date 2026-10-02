//! Export errors.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

fn bullets(items: &[String]) -> String {
    items
        .iter()
        .map(|i| format!("  - {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Error)]
pub enum ExportError {
    /// The project cannot be opened (no story, unreadable manifest...).
    #[error("{0}")]
    Project(String),
    /// The export profile is unusable.
    #[error("invalid export profile: {}", .0.join("; "))]
    Profile(Vec<String>),
    /// The story has validation errors: nothing is exported.
    #[error("the story has {} error(s), fix them before exporting:\n{}", .0.len(), bullets(.0))]
    Invalid(Vec<String>),
    /// Assets the story uses are not on disk.
    #[error("{} asset file(s) used by the story are missing:\n{}", .0.len(), bullets(.0))]
    MissingAssets(Vec<String>),
    #[error("translation `{file}`: {message}")]
    Translation { file: String, message: String },
    #[error("the icon `{path}` cannot be used: {message}")]
    Icon { path: String, message: String },
    /// No player template for the target.
    #[error(
        "no player template for {target} (looked in: {}).\n\
         Templates are built by `cargo xtask templates` and ship in the `templates` folder next to the editor; \
         set ORBIT_TEMPLATES or the editor's templates folder to point elsewhere.",
        .searched.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )]
    TemplateMissing {
        target: &'static str,
        searched: Vec<PathBuf>,
    },
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("pak: {0}")]
    Pak(#[from] orbit_pak::PakError),
    #[error("game pack: {0}")]
    Pack(#[from] orbit_pak::PackError),
    /// An external tool or a compression step failed.
    #[error("{0}")]
    Package(String),
    #[error("the export was cancelled")]
    Cancelled,
}

impl ExportError {
    pub(crate) fn io(path: &Path) -> impl FnOnce(io::Error) -> ExportError + '_ {
        move |source| ExportError::Io {
            path: path.to_owned(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, ExportError>;
