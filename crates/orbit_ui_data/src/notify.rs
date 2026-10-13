//! In-game notifications ("toasts", `ui_version` 11): placement, timing,
//! animation and named looks (`Theme::notifications`), and the
//! `UiAction::Notify` request.
//!
//! Every field has a default and the whole section is left out of the file
//! when it is the stock one, so an older theme reads unchanged and draws the
//! toasts of earlier versions (the panel look of the theme).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::theme::Border;
use crate::widget::TextRef;

/// The style a notification uses when it names none (or an unknown one).
pub const DEFAULT_NOTIFY_STYLE: &str = "default";
/// The style of the achievement unlock toasts.
pub const ACHIEVEMENT_NOTIFY_STYLE: &str = "achievement";
/// The style of the engine messages ("Saved", "Incompatible save"...).
pub const SYSTEM_NOTIFY_STYLE: &str = "system";

/// The corner or edge of the screen the toasts stack from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotifyAnchor {
    TopLeft,
    TopCenter,
    #[default]
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl NotifyAnchor {
    pub const ALL: [NotifyAnchor; 6] = [
        NotifyAnchor::TopLeft,
        NotifyAnchor::TopCenter,
        NotifyAnchor::TopRight,
        NotifyAnchor::BottomLeft,
        NotifyAnchor::BottomCenter,
        NotifyAnchor::BottomRight,
    ];

    /// `[x, y]` fraction of the canvas (and of the toast: its pivot).
    pub fn fraction(self) -> [f32; 2] {
        match self {
            NotifyAnchor::TopLeft => [0.0, 0.0],
            NotifyAnchor::TopCenter => [0.5, 0.0],
            NotifyAnchor::TopRight => [1.0, 0.0],
            NotifyAnchor::BottomLeft => [0.0, 1.0],
            NotifyAnchor::BottomCenter => [0.5, 1.0],
            NotifyAnchor::BottomRight => [1.0, 1.0],
        }
    }

    pub fn is_top(self) -> bool {
        self.fraction()[1] == 0.0
    }
}

/// Which way the stack grows from the anchor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StackDirection {
    /// Away from the anchored edge (down from the top, up from the bottom).
    #[default]
    Auto,
    Down,
    Up,
}

/// How a toast comes in and goes out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotifyAnimation {
    /// Slides in from the nearest side edge (from above / below when
    /// centered) while fading.
    #[default]
    Slide,
    Fade,
    /// Appears and disappears at once.
    None,
}

/// What toasts do while the player skips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotifySkip {
    /// Shown as usual.
    Show,
    /// Shown for a short time ([`BRIEF_SECONDS`] at most).
    #[default]
    Brief,
    /// Not shown at all (achievements are still unlocked).
    Suppress,
}

/// Longest time a toast stays on screen while skipping in `Brief` mode.
pub const BRIEF_SECONDS: f32 = 1.0;

/// The look of one kind of toast. Every field is optional: `None` keeps the
/// look of the `default` style, then the stock toast (the theme's panel,
/// widget text size and color).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationStyle {
    /// A named style of the theme ([`crate::Theme::styles`]) drawn as the
    /// background, border and text (any fill: color, image, nine-slice,
    /// gradient). The fields below win over it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<[u8; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f32>,
    /// Inner margin `[x, y]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<[f32; 2]>,
    /// Side of the square the icon is drawn in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_size: Option<f32>,
    /// An icon shown when the notification has none (project image path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_color: Option<[u8; 4]>,
    /// Font of the text (a name of `Theme::fonts` or a family).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Width of the toast (`0`: fit the text). Else the theme's width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// Seconds on screen. Else the theme's duration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f32>,
    /// Sound played when it shows (project path). Else the theme's sound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
}

impl NotificationStyle {
    /// `self` on top of `base`.
    pub fn over(&self, base: &NotificationStyle) -> NotificationStyle {
        NotificationStyle {
            style: self.style.clone().or_else(|| base.style.clone()),
            background: self.background.or(base.background),
            border: self.border.clone().or_else(|| base.border.clone()),
            corner_radius: self.corner_radius.or(base.corner_radius),
            padding: self.padding.or(base.padding),
            icon_size: self.icon_size.or(base.icon_size),
            icon: self.icon.clone().or_else(|| base.icon.clone()),
            text_size: self.text_size.or(base.text_size),
            text_color: self.text_color.or(base.text_color),
            font: self.font.clone().or_else(|| base.font.clone()),
            width: self.width.or(base.width),
            duration: self.duration.or(base.duration),
            sound: self.sound.clone().or_else(|| base.sound.clone()),
        }
    }
}

/// Placement, timing and looks of the in-game notifications.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationTheme {
    pub anchor: NotifyAnchor,
    /// Distance `[x, y]` from the anchored corner (x is ignored when
    /// centered).
    pub offset: [f32; 2],
    /// Width of a toast (`0`: fit its text).
    pub width: f32,
    /// Toasts on screen at once; the next ones wait their turn.
    pub max_visible: usize,
    /// Space between two toasts.
    pub gap: f32,
    pub direction: StackDirection,
    /// Seconds a toast stays (`None`: `widgets.list.toast_seconds`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f32>,
    pub animation: NotifyAnimation,
    /// Seconds of the enter and exit animations.
    pub animation_seconds: f32,
    /// Sound played when a toast shows (project path; a style or the
    /// notification itself can change it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    /// A click (or tap) on a toast dismisses it.
    pub click_to_dismiss: bool,
    pub skip: NotifySkip,
    /// Named looks (`default`, `achievement`, `system`, or any name a
    /// `Notify` picks).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub styles: BTreeMap<String, NotificationStyle>,
}

impl Default for NotificationTheme {
    fn default() -> Self {
        Self {
            anchor: NotifyAnchor::TopRight,
            offset: [32.0, 32.0],
            width: 560.0,
            max_visible: 4,
            gap: 12.0,
            direction: StackDirection::Auto,
            duration: None,
            animation: NotifyAnimation::Slide,
            animation_seconds: 0.25,
            sound: None,
            click_to_dismiss: true,
            skip: NotifySkip::Brief,
            styles: BTreeMap::new(),
        }
    }
}

impl NotificationTheme {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Whether the stack grows downwards.
    pub fn grows_down(&self) -> bool {
        match self.direction {
            StackDirection::Down => true,
            StackDirection::Up => false,
            StackDirection::Auto => self.anchor.is_top(),
        }
    }

    /// The look of the style `name`: its own fields over `default`'s (an
    /// unknown name is `default`).
    pub fn style(&self, name: Option<&str>) -> NotificationStyle {
        let base = self.styles.get(DEFAULT_NOTIFY_STYLE).cloned().unwrap_or_default();
        match name.filter(|n| *n != DEFAULT_NOTIFY_STYLE).and_then(|n| self.styles.get(n)) {
            Some(s) => s.over(&base),
            None => base,
        }
    }

    /// The style names a toast can pick: the built-in ones first, then the
    /// theme's own.
    pub fn style_names(&self) -> Vec<String> {
        let mut out: Vec<String> = [DEFAULT_NOTIFY_STYLE, ACHIEVEMENT_NOTIFY_STYLE, SYSTEM_NOTIFY_STYLE]
            .into_iter()
            .map(str::to_owned)
            .collect();
        for name in self.styles.keys() {
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    }

    /// Image paths the notifications draw (default icons).
    pub fn image_paths(&self) -> Vec<String> {
        self.styles.values().filter_map(|s| s.icon.clone()).collect()
    }

    /// Sound paths the notifications play.
    pub fn sound_paths(&self) -> Vec<String> {
        self.sound
            .iter()
            .cloned()
            .chain(self.styles.values().filter_map(|s| s.sound.clone()))
            .collect()
    }
}

/// A notification asked by a button (`UiAction::Notify`, `ui_version` 11).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiNotify {
    /// Text shown (a key of the UI texts, a literal or a binding; markup and
    /// `[variable]` allowed).
    pub text: TextRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Project path of an image shown left of the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Project path of a sound played with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f32>,
}

impl UiNotify {
    /// A plain notification with a literal text.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: TextRef::Literal(text.into()),
            style: None,
            icon: None,
            sound: None,
            duration: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn an_old_theme_has_the_stock_notifications_and_writes_none() {
        let theme = Theme::from_ron("(textbox: (text_size: 36.0))").unwrap();
        assert!(theme.notifications.is_default());
        assert!(!theme.to_ron().unwrap().contains("notifications"));
    }

    #[test]
    fn notification_settings_round_trip() {
        let mut theme = Theme::default();
        let n = &mut theme.notifications;
        n.anchor = NotifyAnchor::BottomLeft;
        n.offset = [10.0, 20.0];
        n.max_visible = 2;
        n.direction = StackDirection::Down;
        n.duration = Some(5.0);
        n.animation = NotifyAnimation::Fade;
        n.sound = Some("assets/ding.ogg".into());
        n.skip = NotifySkip::Suppress;
        n.styles.insert(
            ACHIEVEMENT_NOTIFY_STYLE.into(),
            NotificationStyle {
                style: Some("gold".into()),
                corner_radius: Some(12.0),
                border: Some(Border { color: [1, 2, 3, 4], width: 2.0 }),
                icon_size: Some(80.0),
                ..Default::default()
            },
        );
        let ron = theme.to_ron().unwrap();
        assert_eq!(Theme::from_ron(&ron).unwrap(), theme);
        // A partial section keeps the other defaults.
        let partial = Theme::from_ron("(notifications: (max_visible: 7))").unwrap();
        assert_eq!(partial.notifications.max_visible, 7);
        assert_eq!(partial.notifications.anchor, NotifyAnchor::TopRight);
    }

    #[test]
    fn a_style_falls_back_to_default() {
        let mut n = NotificationTheme::default();
        n.styles.insert(
            DEFAULT_NOTIFY_STYLE.into(),
            NotificationStyle { text_size: Some(30.0), background: Some([1, 1, 1, 255]), ..Default::default() },
        );
        n.styles.insert(
            "quest".into(),
            NotificationStyle { background: Some([9, 9, 9, 255]), ..Default::default() },
        );
        let q = n.style(Some("quest"));
        assert_eq!(q.background, Some([9, 9, 9, 255]));
        assert_eq!(q.text_size, Some(30.0));
        assert_eq!(n.style(Some("unknown")).background, Some([1, 1, 1, 255]));
        assert!(n.style_names().starts_with(&["default".into(), "achievement".into(), "system".into()]));
        assert!(n.grows_down());
    }
}
