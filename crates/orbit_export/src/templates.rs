//! Player templates: the precompiled player of each target.
//!
//! Exporting a game never compiles Rust: the player (native executables and
//! the web module) is built once, by `cargo xtask templates` or by the CI,
//! and an export glues the game's pak to a copy of it. A **templates
//! folder** holds one subfolder per target:
//!
//! ```text
//! templates/
//!   windows/orbit-player.exe
//!   linux/orbit-player
//!   web/orbit_player.js
//!   web/orbit_player_bg.wasm
//! ```
//!
//! Where the folder is looked for ([`Templates::standard`]), in order:
//! 1. the `ORBIT_TEMPLATES` environment variable;
//! 2. `templates/` next to the running executable (the installed editor and
//!    its portable archive ship it there);
//! 3. `../templates` of the executable (a build of the repository:
//!    `target/templates`, where `cargo xtask templates` writes).
//!
//! The editor adds the folder chosen in its settings in front of those.

use std::path::{Path, PathBuf};

use orbit_project::ExportTarget;

use crate::error::{ExportError, Result};

/// Environment variable naming a templates folder.
pub const ENV_VAR: &str = "ORBIT_TEMPLATES";

/// File name of the native player template.
pub fn player_file(target: ExportTarget) -> &'static str {
    match target {
        ExportTarget::Windows => "orbit-player.exe",
        ExportTarget::Linux => "orbit-player",
        ExportTarget::Web => "orbit_player_bg.wasm",
    }
}

/// JavaScript glue of the web template (next to the wasm module).
pub const WEB_JS: &str = "orbit_player.js";

/// The two files of the web template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebTemplate {
    pub js: PathBuf,
    pub wasm: PathBuf,
}

/// Where player templates are looked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Templates {
    dirs: Vec<PathBuf>,
}

impl Templates {
    /// Looks in exactly these folders, in order.
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Self { dirs }
    }

    /// The standard places (see the module documentation).
    pub fn standard() -> Self {
        let mut dirs = Vec::new();
        if let Some(dir) = std::env::var_os(ENV_VAR).filter(|v| !v.is_empty()) {
            dirs.push(PathBuf::from(dir));
        }
        if let Some(exe_dir) = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_owned))
        {
            dirs.push(exe_dir.join("templates"));
            dirs.push(exe_dir.join("..").join("templates"));
        }
        Self { dirs }
    }

    /// Looks in `dir` first.
    pub fn with_dir_first(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dirs.insert(0, dir.into());
        self
    }

    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    fn missing(&self, target: ExportTarget) -> ExportError {
        ExportError::TemplateMissing {
            target: target.id(),
            searched: self.dirs.iter().map(|d| d.join(target.id())).collect(),
        }
    }

    /// The player executable of a native target.
    pub fn native(&self, target: ExportTarget) -> Result<PathBuf> {
        debug_assert_ne!(target, ExportTarget::Web);
        self.dirs
            .iter()
            .map(|d| d.join(target.id()).join(player_file(target)))
            .find(|p| p.is_file())
            .ok_or_else(|| self.missing(target))
    }

    /// The web module and its glue.
    pub fn web(&self) -> Result<WebTemplate> {
        self.dirs
            .iter()
            .map(|d| d.join("web"))
            .find(|d| d.join(WEB_JS).is_file() && d.join(player_file(ExportTarget::Web)).is_file())
            .map(|d| WebTemplate {
                js: d.join(WEB_JS),
                wasm: d.join(player_file(ExportTarget::Web)),
            })
            .ok_or_else(|| self.missing(ExportTarget::Web))
    }

    /// Whether the template of `target` can be found.
    pub fn has(&self, target: ExportTarget) -> bool {
        match target {
            ExportTarget::Web => self.web().is_ok(),
            native => self.native(native).is_ok(),
        }
    }

    /// The targets whose template is available.
    pub fn available(&self) -> Vec<ExportTarget> {
        ExportTarget::ALL.into_iter().filter(|t| self.has(*t)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn finds_templates_in_the_first_folder_that_has_them() {
        let a = TempDir::new("tpl_a");
        let b = TempDir::new("tpl_b");
        std::fs::create_dir_all(b.path().join("windows")).unwrap();
        std::fs::write(b.path().join("windows/orbit-player.exe"), b"MZ").unwrap();
        std::fs::create_dir_all(b.path().join("web")).unwrap();
        std::fs::write(b.path().join("web/orbit_player.js"), b"js").unwrap();
        let templates = Templates::new(vec![a.path().to_owned(), b.path().to_owned()]);
        assert!(templates.native(ExportTarget::Windows).is_ok());
        assert!(matches!(
            templates.native(ExportTarget::Linux),
            Err(ExportError::TemplateMissing { target: "linux", .. })
        ));
        // The web template needs both files.
        assert!(templates.web().is_err());
        std::fs::write(b.path().join("web/orbit_player_bg.wasm"), b"\0asm").unwrap();
        assert!(templates.web().is_ok());
        assert_eq!(
            templates.available(),
            [ExportTarget::Windows, ExportTarget::Web]
        );
        let first = Templates::default().with_dir_first(a.path());
        assert_eq!(first.dirs(), [a.path().to_owned()]);
    }

    #[test]
    fn the_error_says_where_it_looked_and_what_to_do() {
        let message = Templates::new(vec![PathBuf::from("nowhere")])
            .native(ExportTarget::Linux)
            .unwrap_err()
            .to_string();
        assert!(message.contains("nowhere"), "{message}");
        assert!(message.contains("cargo xtask templates"), "{message}");
    }
}
