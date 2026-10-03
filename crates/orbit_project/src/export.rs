//! Export profiles: how a project is turned into a distributable game.
//!
//! They live in the project manifest (`orbit.project.ron`) so that they are
//! versioned with the game and shared by the editor, the CLI and the MCP
//! server. The export itself is in `orbit_export`.

use serde::{Deserialize, Serialize};

/// A platform a game can be exported for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExportTarget {
    Windows,
    Linux,
    Web,
    /// APK or AAB (`orbit_export`).
    Android,
}

impl ExportTarget {
    pub const ALL: [ExportTarget; 4] = [
        ExportTarget::Windows,
        ExportTarget::Linux,
        ExportTarget::Web,
        ExportTarget::Android,
    ];

    /// Name used on the command line and in folder names.
    pub fn id(self) -> &'static str {
        match self {
            ExportTarget::Windows => "windows",
            ExportTarget::Linux => "linux",
            ExportTarget::Web => "web",
            ExportTarget::Android => "android",
        }
    }

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            ExportTarget::Windows => "Windows (zip)",
            ExportTarget::Linux => "Linux (tar.gz)",
            ExportTarget::Web => "Web (navigateur)",
            ExportTarget::Android => "Android (APK / AAB)",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.id().eq_ignore_ascii_case(text.trim()))
    }

    /// The target of the machine this build runs on (for a quick test).
    pub fn host() -> Option<Self> {
        if cfg!(target_os = "windows") {
            Some(ExportTarget::Windows)
        } else if cfg!(target_os = "linux") {
            Some(ExportTarget::Linux)
        } else {
            None
        }
    }
}

/// Compression of the entries of the asset pak.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PakCompression {
    /// Nothing is compressed (fastest to load).
    None,
    /// LZ4 for the files that gain at least 10 %; images and sounds are
    /// already compressed and stay as they are.
    #[default]
    Lz4,
}

/// Screen orientation of the Android game.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AndroidOrientation {
    #[default]
    Landscape,
    Portrait,
    /// Follows the sensor.
    Sensor,
}

/// 3D rendering quality of the desktop players (M6-12, ADR 0014 section 3).
/// The web and Android players always use the web profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesktopQuality {
    /// 8 lights, one shadow, bloom: modest PCs and laptops.
    #[default]
    Low,
    /// 16 lights, 4 shadows, softer shadows.
    High,
}

impl DesktopQuality {
    /// The quality tier id the player reads (`orbit_scene::quality`).
    pub fn tier_id(self) -> &'static str {
        match self {
            DesktopQuality::Low => "desktop-low",
            DesktopQuality::High => "desktop-high",
        }
    }
}

/// What the Android export produces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AndroidFormat {
    /// Installable file (tests, direct distribution).
    #[default]
    Apk,
    /// App bundle (Play Store).
    Aab,
}

/// Whether `id` is a valid Android application id (`com.example.game`: at
/// least two segments, each starting with a letter).
pub fn is_valid_android_package(id: &str) -> bool {
    let segments: Vec<&str> = id.split('.').collect();
    segments.len() >= 2
        && segments.iter().all(|s| {
            s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// Android is exported on request (it needs the Android SDK): the default
/// profile covers the desktop and the web.
fn default_targets() -> Vec<ExportTarget> {
    vec![ExportTarget::Windows, ExportTarget::Linux, ExportTarget::Web]
}

fn default_true() -> bool {
    true
}

/// One way to export the game: identity, targets and options.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportProfile {
    /// Name of the profile (`release`, `demo`...), unique in the project.
    pub name: String,
    /// Display name of the game: window title, folder and file names.
    pub game_name: String,
    /// Stable identifier (`a-z`, `0-9`, `-`, `_`): where the saves of the
    /// players live. Never change it once the game is published.
    pub game_id: String,
    /// Version of the game, free text (`1.0.0`).
    #[serde(default = "default_version")]
    pub version: String,
    /// PNG icon, relative to the project root (window and browser tab icon).
    #[serde(default)]
    pub icon: Option<String>,
    /// Targets exported by default.
    #[serde(default = "default_targets")]
    pub targets: Vec<ExportTarget>,
    #[serde(default)]
    pub pak_compression: PakCompression,
    /// Web: also write brotli (`.br`) and gzip (`.gz`) versions of the files.
    #[serde(default = "default_true")]
    pub web_precompress: bool,
    /// Language used when the player has no saved preference (BCP 47);
    /// empty follows the system language.
    #[serde(default)]
    pub default_language: Option<String>,
    /// Android application id (`com.studio.game`); empty derives
    /// `com.orbit.<game id>`.
    #[serde(default)]
    pub android_package: Option<String>,
    #[serde(default)]
    pub android_orientation: AndroidOrientation,
    #[serde(default)]
    pub android_format: AndroidFormat,
    /// Android `versionCode`; empty derives it from the version (`1.2.3`
    /// gives 10203).
    #[serde(default)]
    pub android_version_code: Option<u32>,
    /// 3D quality of the Windows and Linux players.
    #[serde(default)]
    pub desktop_quality: DesktopQuality,
}

fn default_version() -> String {
    "1.0.0".to_owned()
}

/// Lowercase identifier made of letters, digits, `-` and `_` ("Mon jeu !"
/// becomes `mon-jeu`); `game` when nothing is left.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "game".into() } else { out }
}

impl ExportProfile {
    /// The profile used when a project defines none.
    pub fn new(profile_name: &str, game_name: &str) -> Self {
        Self {
            name: profile_name.to_owned(),
            game_name: game_name.to_owned(),
            game_id: slugify(game_name),
            version: default_version(),
            icon: None,
            targets: default_targets(),
            pak_compression: PakCompression::default(),
            web_precompress: true,
            default_language: None,
            android_package: None,
            android_orientation: AndroidOrientation::default(),
            android_format: AndroidFormat::default(),
            desktop_quality: DesktopQuality::default(),
            android_version_code: None,
        }
    }

    /// Problems that make the profile unusable (empty means valid).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.name.trim().is_empty() {
            out.push("the profile has no name".to_owned());
        }
        if self.game_name.trim().is_empty() {
            out.push("the game name is empty".to_owned());
        }
        if self.game_id.is_empty()
            || !self
                .game_id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            out.push(format!(
                "the game identifier `{}` must be made of a-z, 0-9, `-` and `_`",
                self.game_id
            ));
        }
        if self.targets.is_empty() {
            out.push("the profile has no target".to_owned());
        }
        if let Some(icon) = &self.icon
            && (icon.starts_with('/') || icon.contains("..") || icon.contains('\\'))
        {
            out.push(format!(
                "the icon path `{icon}` must be relative to the project"
            ));
        }
        if let Some(package) = &self.android_package
            && !package.trim().is_empty()
            && !is_valid_android_package(package.trim())
        {
            out.push(format!(
                "the Android package `{package}` must look like com.studio.game"
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Mon jeu !"), "mon-jeu");
        assert_eq!(slugify("  "), "game");
        assert_eq!(slugify("A_b-C"), "a-b-c");
    }

    #[test]
    fn default_profile_is_valid_and_targets_everything() {
        let p = ExportProfile::new("release", "Mon jeu");
        assert!(p.problems().is_empty(), "{:?}", p.problems());
        assert_eq!(p.game_id, "mon-jeu");
        assert_eq!(p.targets.len(), 3);
    }

    #[test]
    fn problems_are_listed() {
        let mut p = ExportProfile::new("", " ");
        p.game_id = "Bad Id".into();
        p.targets.clear();
        p.icon = Some("../x.png".into());
        assert_eq!(p.problems().len(), 5);
    }

    #[test]
    fn android_package_ids() {
        assert!(is_valid_android_package("com.orbit.mon_jeu"));
        assert!(!is_valid_android_package("game"));
        assert!(!is_valid_android_package("com.1game"));
        assert!(!is_valid_android_package("com..x"));
        let mut p = ExportProfile::new("r", "G");
        p.android_package = Some("nope".into());
        assert_eq!(p.problems().len(), 1);
        assert_eq!(ExportTarget::parse("android"), Some(ExportTarget::Android));
    }

    #[test]
    fn targets_parse() {
        assert_eq!(ExportTarget::parse("Web"), Some(ExportTarget::Web));
        assert_eq!(ExportTarget::parse("mac"), None);
        assert_eq!(ExportTarget::Linux.id(), "linux");
    }
}
