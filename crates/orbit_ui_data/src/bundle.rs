//! Everything a project can customize of its UI, in one value: the theme,
//! the screens, which screens replace the stock ones and the splash screens
//! shown before the main menu.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::validate::{Issue, Severity, validate};
use crate::widget::{ScreenAsset, from_ron};
use crate::theme::Theme;

/// What a project splash shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SplashContent {
    /// A line of text, centered.
    Text(String),
    /// An image asset (path), scaled to fit the canvas.
    Image(String),
    /// Reserved: the engine has no video decoder yet, such a splash is
    /// skipped.
    Video(String),
}

fn yes() -> bool {
    true
}

fn black() -> [u8; 4] {
    [0, 0, 0, 255]
}

fn half() -> f32 {
    0.5
}

/// One project splash screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplashDef {
    pub content: SplashContent,
    /// Seconds on screen, fades included.
    pub duration: f32,
    #[serde(default = "half")]
    pub fade_in: f32,
    #[serde(default = "half")]
    pub fade_out: f32,
    #[serde(default = "yes")]
    pub skippable: bool,
    #[serde(default = "black")]
    pub background: [u8; 4],
}

/// Theme, screens and splashes of a project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiBundle {
    /// Replaces the theme of the game when present.
    pub theme: Option<Theme>,
    pub screens: Vec<ScreenAsset>,
    /// System screen (`main_menu`, `pause`, `preferences`) -> id of the
    /// screen standing in for it. A screen whose id is a system name does so
    /// without an entry here. Missing system screens keep the built-in
    /// Rust ones.
    pub system_screens: BTreeMap<String, String>,
    /// Shown in order after the engine splash, before the main menu.
    pub splashes: Vec<SplashDef>,
}

/// Names of the screens the shell can replace.
pub const SYSTEM_SCREENS: [&str; 5] = ["main_menu", "pause", "preferences", "save", "load"];

impl UiBundle {
    pub fn from_ron(source: &str) -> Result<Self, String> {
        ron::from_str(source).map_err(|e| e.to_string())
    }

    pub fn to_ron(&self) -> Result<String, String> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new()).map_err(|e| e.to_string())
    }

    /// Builds a bundle from the texts of its files (the theme is optional).
    pub fn from_parts(theme: Option<&str>, screens: &[&str]) -> Result<Self, String> {
        let theme = theme
            .map(|t| Theme::from_ron(t).map_err(|e| format!("theme: {e}")))
            .transpose()?;
        let screens = screens
            .iter()
            .map(|s| from_ron(s))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            theme,
            screens,
            ..Self::default()
        })
    }

    /// The stock screens (main menu, pause, preferences) and theme as data:
    /// what a project starts from when it opts into data-driven screens.
    pub fn builtin() -> Self {
        Self::from_parts(
            Some(include_str!("../../../assets/ui/default/theme.ron")),
            &[
                include_str!("../../../assets/ui/default/main_menu.screen.ron"),
                include_str!("../../../assets/ui/default/pause.screen.ron"),
                include_str!("../../../assets/ui/default/preferences.screen.ron"),
                include_str!("../../../assets/ui/default/save.screen.ron"),
                include_str!("../../../assets/ui/default/load.screen.ron"),
            ],
        )
        .expect("the built-in UI assets are valid")
    }

    /// Reads a project's `ui/` folder: `theme.ron` (optional), every
    /// `*.screen.ron`, and `ui.ron` (optional: a bundle holding only the
    /// `system_screens` and `splashes`). Native only (no file access on the
    /// web: the bundle comes from the pak there).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_dir(dir: &std::path::Path) -> Result<Self, String> {
        let read = |p: &std::path::Path| {
            std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
        };
        let mut bundle = match dir.join("ui.ron") {
            p if p.is_file() => Self::from_ron(&read(&p)?).map_err(|e| format!("ui.ron: {e}"))?,
            _ => Self::default(),
        };
        let theme = dir.join("theme.ron");
        if theme.is_file() {
            bundle.theme = Some(
                Theme::from_ron(&read(&theme)?).map_err(|e| format!("theme.ron: {e}"))?,
            );
        }
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".screen.ron"))
            })
            .collect();
        files.sort();
        for file in files {
            let name = file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            bundle
                .screens
                .push(from_ron(&read(&file)?).map_err(|e| format!("{name}: {e}"))?);
        }
        Ok(bundle)
    }

    pub fn screen(&self, id: &str) -> Option<&ScreenAsset> {
        self.screens.iter().find(|s| s.id == id)
    }

    /// The screen replacing the system screen `name`, if any.
    pub fn system_screen(&self, name: &str) -> Option<&ScreenAsset> {
        let id = self.system_screens.get(name).map_or(name, String::as_str);
        self.screen(id)
    }

    /// Warnings about story variables in every screen (unknown variable,
    /// operation or value of the wrong type), given the declared types.
    pub fn variable_issues(
        &self,
        ty_of: &dyn Fn(&str) -> Option<orbit_core::ValueType>,
    ) -> Vec<Issue> {
        self.screens
            .iter()
            .flat_map(|s| crate::validate::variable_issues(s, ty_of))
            .collect()
    }

    /// Warnings for `SceneGoto` actions to scenes the story does not have
    /// (`known` says whether a scene id or name exists).
    pub fn scene_issues(&self, known: &dyn Fn(&str) -> bool) -> Vec<Issue> {
        self.screens
            .iter()
            .flat_map(|s| crate::validate::scene_issues(s, known))
            .collect()
    }

    /// Problems of every screen and splash (errors make a bundle unusable).
    pub fn validate(&self) -> Vec<Issue> {
        let mut out = Vec::new();
        let theme = self.theme.as_ref();
        for (i, screen) in self.screens.iter().enumerate() {
            if self.screens[..i].iter().any(|s| s.id == screen.id) {
                out.push(Issue::error(&screen.id, "", "duplicate screen id"));
            }
            out.extend(validate(screen, theme));
        }
        for (system, id) in &self.system_screens {
            if !SYSTEM_SCREENS.contains(&system.as_str()) {
                out.push(Issue::error(id, "", format!("`{system}` is not a system screen")));
            }
            if self.screen(id).is_none() {
                out.push(Issue::error(id, "", format!("system screen `{system}` points to a missing screen")));
            }
        }
        for (i, splash) in self.splashes.iter().enumerate() {
            let at = format!("splash {i}");
            if splash.duration <= 0.0 {
                out.push(Issue::error(&at, "", "duration must be positive"));
            }
            if splash.fade_in < 0.0 || splash.fade_out < 0.0 {
                out.push(Issue::error(&at, "", "fades cannot be negative"));
            }
            if splash.fade_in + splash.fade_out > splash.duration {
                out.push(Issue::warning(&at, "", "the fades are longer than the splash"));
            }
            if matches!(splash.content, SplashContent::Video(_)) {
                out.push(Issue::warning(&at, "", "video splashes are not supported yet and are skipped"));
            }
        }
        out
    }

    /// `true` when [`UiBundle::validate`] found no error.
    pub fn is_valid(&self) -> bool {
        !self.validate().iter().any(|i| i.severity == Severity::Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_round_trips_and_finds_system_screens() {
        let mut bundle = UiBundle::from_parts(
            None,
            &["(id: \"main_menu\", root: (kind: Stack(dir: Vertical)))",
              "(id: \"fancy\", root: (kind: Stack(dir: Vertical)))"],
        )
        .unwrap();
        bundle.system_screens.insert("pause".into(), "fancy".into());
        bundle.splashes.push(SplashDef {
            content: SplashContent::Text("Studio".into()),
            duration: 2.0,
            fade_in: 0.5,
            fade_out: 0.5,
            skippable: true,
            background: [0, 0, 0, 255],
        });
        let back = UiBundle::from_ron(&bundle.to_ron().unwrap()).unwrap();
        assert_eq!(back, bundle);
        assert_eq!(back.system_screen("main_menu").unwrap().id, "main_menu");
        assert_eq!(back.system_screen("pause").unwrap().id, "fancy");
        assert!(back.system_screen("preferences").is_none());
        assert!(back.is_valid(), "{:?}", back.validate());
    }

    #[test]
    fn the_builtin_bundle_is_valid_and_replaces_the_system_screens() {
        let b = UiBundle::builtin();
        assert!(b.validate().is_empty(), "{:?}", b.validate());
        for name in SYSTEM_SCREENS {
            assert!(b.system_screen(name).is_some(), "{name}");
        }
        assert!(b.splashes.is_empty());
    }

    #[test]
    fn a_ui_folder_loads_into_a_bundle() {
        let dir = std::env::temp_dir().join(format!("orbit_ui_dir_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let w = |n: &str, t: &str| std::fs::write(dir.join(n), t).unwrap();
        w("theme.ron", "(styles: {})");
        w("b.screen.ron", "(id: \"b\", root: (kind: Stack(dir: Vertical)))");
        w("a.screen.ron", "(id: \"a\", root: (kind: Stack(dir: Vertical)))");
        w("ui.ron", "(splashes: [(content: Text(\"Hi\"), duration: 1.0)])");
        w("notes.txt", "ignored");
        let b = UiBundle::load_dir(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(b.theme.is_some());
        assert_eq!(b.screens.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(b.splashes.len(), 1);
        assert!(UiBundle::load_dir(&dir).is_err(), "missing folder");
    }

    #[test]
    fn splash_fields_default() {
        let b = UiBundle::from_ron("(splashes: [(content: Text(\"x\"), duration: 2.0)])").unwrap();
        let s = &b.splashes[0];
        assert!(s.skippable);
        assert_eq!(s.background, [0, 0, 0, 255]);
        assert_eq!((s.fade_in, s.fade_out), (0.5, 0.5));
    }

    #[test]
    fn bad_splashes_and_system_links_are_reported() {
        let mut b = UiBundle::default();
        b.splashes.push(SplashDef {
            content: SplashContent::Video("a.mp4".into()),
            duration: 0.0,
            fade_in: 0.5,
            fade_out: 0.5,
            skippable: true,
            background: [0; 4],
        });
        b.system_screens.insert("pause".into(), "ghost".into());
        b.system_screens.insert("menu".into(), "ghost".into());
        let issues = b.validate();
        assert!(issues.iter().filter(|i| i.severity == Severity::Error).count() >= 3);
        assert!(!b.is_valid());
    }
}
