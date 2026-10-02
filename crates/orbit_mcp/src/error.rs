//! Structured tool errors: what an AI client sees when a call is refused.

use std::fmt;

use orbit_project::{CommandError, ProjectError};

use crate::sandbox::SandboxError;

/// Coarse category of a refusal, stable for clients to branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The arguments do not match the tool's input schema or make no sense.
    InvalidArguments,
    /// A scene, node, character, asset... does not exist (or is ambiguous).
    NotFound,
    /// The node was modified since the version the caller read.
    VersionConflict,
    /// The command was refused by the project (still referenced, in use...).
    Refused,
    /// The path is outside the project folder.
    OutsideProject,
    /// File system failure.
    Io,
    /// A plugin tool failed.
    Plugin,
    /// Bug in the server (a panic was caught).
    Internal,
    /// The feature needs something this editor does not have right now (no
    /// GPU for the preview, no export plugin, no project open...).
    Unavailable,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::InvalidArguments => "invalid_arguments",
            ErrorKind::NotFound => "not_found",
            ErrorKind::VersionConflict => "version_conflict",
            ErrorKind::Refused => "refused",
            ErrorKind::OutsideProject => "outside_project",
            ErrorKind::Io => "io_error",
            ErrorKind::Plugin => "plugin_error",
            ErrorKind::Internal => "internal_error",
            ErrorKind::Unavailable => "unavailable",
        }
    }
}

/// A failed tool call. The project is left unchanged (calls are rolled back
/// as a whole).
#[derive(Debug, Clone)]
pub struct ToolError {
    pub kind: ErrorKind,
    pub message: String,
}

impl ToolError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidArguments, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    pub fn refused(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Refused, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unavailable, message)
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)
    }
}

impl std::error::Error for ToolError {}

impl From<ProjectError> for ToolError {
    fn from(err: ProjectError) -> Self {
        let cmd = match &err {
            ProjectError::Command(cmd) => cmd,
            ProjectError::Rejected(message) => {
                return Self::new(ErrorKind::Refused, message.message.clone());
            }
        };
        let kind = match cmd {
            CommandError::VersionConflict { .. } => ErrorKind::VersionConflict,
            CommandError::UnknownScene(_)
            | CommandError::UnknownNode(_)
            | CommandError::UnknownString(_)
            | CommandError::UnknownCharacter(_)
            | CommandError::UnknownVariable(_)
            | CommandError::UnknownAsset(_) => ErrorKind::NotFound,
            _ => ErrorKind::Refused,
        };
        let mut message = cmd.to_string();
        if kind == ErrorKind::VersionConflict {
            message.push_str(
                ". The node was changed since you read it (possibly by the user in the editor).                  Re-read the scene (read_scene) to get the current version, then retry.",
            );
        }
        Self::new(kind, message)
    }
}

impl From<SandboxError> for ToolError {
    fn from(err: SandboxError) -> Self {
        let kind = match err {
            SandboxError::Invalid(_) => ErrorKind::InvalidArguments,
            SandboxError::Escapes(_) => ErrorKind::OutsideProject,
            SandboxError::Io(_) => ErrorKind::Io,
        };
        Self::new(kind, err.to_string())
    }
}

impl From<std::io::Error> for ToolError {
    fn from(err: std::io::Error) -> Self {
        Self::new(ErrorKind::Io, err.to_string())
    }
}
