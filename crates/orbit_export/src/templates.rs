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
//!   web3d/orbit_player.js                    (same, with 3D scenes: used
//!   web3d/orbit_player_bg.wasm                only by games that show one;
//!                                             it also runs scripts)
//!   webscript/orbit_player.js                (2D with scripts: used only by
//!   webscript/orbit_player_bg.wasm            2D games with scripts, M8-01)
//!   android/classes.dex                      (GameActivity, see [`AndroidTemplate`])
//!   android/arm64-v8a/liborbit_player.so
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
        ExportTarget::Android => "liborbit_player.so",
    }
}

/// JavaScript glue of the web template (next to the wasm module).
pub const WEB_JS: &str = "orbit_player.js";

/// Folder of the web template built with 3D scenes (M6-13).
pub const WEB3D_DIR: &str = "web3d";

/// Folder of the 2D web template built with scripts (M8-01).
pub const WEBSCRIPT_DIR: &str = "webscript";

/// The two files of the web template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebTemplate {
    pub js: PathBuf,
    pub wasm: PathBuf,
}

/// An Android CPU architecture (name of the folder in `lib/` of an APK).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AndroidAbi {
    Arm64,
    Arm32,
    X86_64,
}

impl AndroidAbi {
    /// `arm64-v8a` is mandatory in an Android template, the others optional.
    pub const ALL: [AndroidAbi; 3] = [AndroidAbi::Arm64, AndroidAbi::Arm32, AndroidAbi::X86_64];

    pub fn id(self) -> &'static str {
        match self {
            AndroidAbi::Arm64 => "arm64-v8a",
            AndroidAbi::Arm32 => "armeabi-v7a",
            AndroidAbi::X86_64 => "x86_64",
        }
    }

    /// The Rust target triple the player is built for.
    pub fn triple(self) -> &'static str {
        match self {
            AndroidAbi::Arm64 => "aarch64-linux-android",
            AndroidAbi::Arm32 => "armv7-linux-androideabi",
            AndroidAbi::X86_64 => "x86_64-linux-android",
        }
    }
}

/// File name of the dex of the Android template (GameActivity and its
/// dependencies, compiled from the Games Activity AAR with `d8`).
pub const ANDROID_DEX: &str = "classes.dex";

/// What the Android template holds:
///
/// ```text
/// templates/android/classes.dex                       GameActivity (dex)
/// templates/android/arm64-v8a/liborbit_player.so      required
/// templates/android/armeabi-v7a/liborbit_player.so    optional
/// templates/android/x86_64/liborbit_player.so         optional
/// templates/android/bundletool.jar                    optional (AAB)
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AndroidTemplate {
    pub dir: PathBuf,
    pub dex: PathBuf,
    /// Present architectures, `arm64-v8a` first.
    pub libs: Vec<(AndroidAbi, PathBuf)>,
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
        debug_assert!(!matches!(target, ExportTarget::Web | ExportTarget::Android));
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

    /// The web player with 3D scenes (`web3d/`, M6-13): same two files,
    /// built with the `3d` feature. Only games that show a 3D scene use it,
    /// so 2D games keep the smaller module.
    pub fn web3d(&self) -> Result<WebTemplate> {
        self.dirs
            .iter()
            .map(|d| d.join(WEB3D_DIR))
            .find(|d| d.join(WEB_JS).is_file() && d.join(player_file(ExportTarget::Web)).is_file())
            .map(|d| WebTemplate {
                js: d.join(WEB_JS),
                wasm: d.join(player_file(ExportTarget::Web)),
            })
            .ok_or_else(|| ExportError::TemplateMissing {
                target: "web (3D)",
                searched: self.dirs.iter().map(|d| d.join(WEB3D_DIR)).collect(),
            })
    }

    /// The 2D web player with scripts (`webscript/`, M8-01): built with the
    /// `script` feature. Only 2D games with scripts use it.
    pub fn webscript(&self) -> Result<WebTemplate> {
        self.dirs
            .iter()
            .map(|d| d.join(WEBSCRIPT_DIR))
            .find(|d| d.join(WEB_JS).is_file() && d.join(player_file(ExportTarget::Web)).is_file())
            .map(|d| WebTemplate {
                js: d.join(WEB_JS),
                wasm: d.join(player_file(ExportTarget::Web)),
            })
            .ok_or_else(|| ExportError::TemplateMissing {
                target: "web (scripts)",
                searched: self.dirs.iter().map(|d| d.join(WEBSCRIPT_DIR)).collect(),
            })
    }

    /// The web template a game needs: [`Templates::web3d`] when it uses
    /// 3D (it also runs scripts), [`Templates::webscript`] for a 2D game
    /// with scripts, else [`Templates::web`].
    pub fn web_for(&self, uses_3d: bool, uses_scripts: bool) -> Result<WebTemplate> {
        match (uses_3d, uses_scripts) {
            (true, _) => self.web3d(),
            (false, true) => self.webscript(),
            (false, false) => self.web(),
        }
    }

    /// The Android player: the dex and at least the `arm64-v8a` library.
    pub fn android(&self) -> Result<AndroidTemplate> {
        let lib = player_file(ExportTarget::Android);
        self.dirs
            .iter()
            .map(|d| d.join("android"))
            .find(|d| d.join(ANDROID_DEX).is_file() && d.join("arm64-v8a").join(lib).is_file())
            .map(|dir| AndroidTemplate {
                dex: dir.join(ANDROID_DEX),
                libs: AndroidAbi::ALL
                    .into_iter()
                    .map(|abi| (abi, dir.join(abi.id()).join(lib)))
                    .filter(|(_, p)| p.is_file())
                    .collect(),
                dir,
            })
            .ok_or_else(|| self.missing(ExportTarget::Android))
    }

    /// Folders `android/` of the search path (for `bundletool.jar`).
    pub fn android_dirs(&self) -> Vec<PathBuf> {
        self.dirs.iter().map(|d| d.join("android")).collect()
    }

    /// Whether the template of `target` can be found.
    pub fn has(&self, target: ExportTarget) -> bool {
        match target {
            ExportTarget::Web => self.web().is_ok(),
            ExportTarget::Android => self.android().is_ok(),
            native => self.native(native).is_ok(),
        }
    }

    /// The targets whose template is available.
    pub fn available(&self) -> Vec<ExportTarget> {
        ExportTarget::ALL
            .into_iter()
            .filter(|t| self.has(*t))
            .collect()
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
            Err(ExportError::TemplateMissing {
                target: "linux",
                ..
            })
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
    fn the_3d_web_template_is_separate_and_picked_for_3d_games() {
        let dir = TempDir::new("tpl_web3d");
        let templates = Templates::new(vec![dir.path().to_owned()]);
        std::fs::create_dir_all(dir.path().join("web")).unwrap();
        std::fs::write(dir.path().join("web/orbit_player.js"), b"js").unwrap();
        std::fs::write(dir.path().join("web/orbit_player_bg.wasm"), b"\0asm").unwrap();
        assert!(templates.web_for(false, false).is_ok());
        let err = templates.web_for(true, false).unwrap_err().to_string();
        assert!(err.contains("web (3D)") && err.contains("web3d"), "{err}");
        let err = templates.web_for(false, true).unwrap_err().to_string();
        assert!(err.contains("web (scripts)") && err.contains("webscript"), "{err}");
        std::fs::create_dir_all(dir.path().join("web3d")).unwrap();
        std::fs::write(dir.path().join("web3d/orbit_player.js"), b"js").unwrap();
        std::fs::write(dir.path().join("web3d/orbit_player_bg.wasm"), b"\0asm").unwrap();
        let t = templates.web_for(true, false).unwrap();
        assert!(t.wasm.starts_with(dir.path().join("web3d")));
        // A 3D game with scripts uses the 3D player (it runs scripts too).
        assert!(templates.web_for(true, true).unwrap().wasm.starts_with(dir.path().join("web3d")));
        std::fs::create_dir_all(dir.path().join("webscript")).unwrap();
        std::fs::write(dir.path().join("webscript/orbit_player.js"), b"js").unwrap();
        std::fs::write(dir.path().join("webscript/orbit_player_bg.wasm"), b"\0asm").unwrap();
        let t = templates.web_for(false, true).unwrap();
        assert!(t.wasm.starts_with(dir.path().join("webscript")));
        assert!(templates.web_for(false, false).unwrap().wasm.starts_with(dir.path().join("web")));
    }

    #[test]
    fn the_android_template_needs_the_dex_and_the_arm64_library() {
        let dir = TempDir::new("tpl_android");
        let templates = Templates::new(vec![dir.path().to_owned()]);
        assert!(templates.android().is_err());
        let android = dir.path().join("android");
        std::fs::create_dir_all(android.join("arm64-v8a")).unwrap();
        std::fs::write(android.join("arm64-v8a/liborbit_player.so"), b"elf").unwrap();
        assert!(templates.android().is_err(), "no dex yet");
        std::fs::write(android.join("classes.dex"), b"dex").unwrap();
        let t = templates.android().unwrap();
        assert_eq!(t.libs.len(), 1);
        std::fs::create_dir_all(android.join("x86_64")).unwrap();
        std::fs::write(android.join("x86_64/liborbit_player.so"), b"elf").unwrap();
        let t = templates.android().unwrap();
        let abis: Vec<_> = t.libs.iter().map(|(a, _)| a.id()).collect();
        assert_eq!(abis, ["arm64-v8a", "x86_64"]);
        assert!(templates.available().contains(&ExportTarget::Android));
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
