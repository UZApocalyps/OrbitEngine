//! Opening and closing transitions of a screen (UI-16, ADR 0016 §5).
//!
//! Purely visual: the screen is logically open (or closed) at once, the
//! transition only changes how it is drawn for `duration` seconds. Rollback
//! and skip cut it, reduced motion turns it off.

use orbit_core::Curve;
use serde::{Deserialize, Serialize};

/// How the screen moves in or out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScreenTransitionKind {
    /// Opacity from 0 to 1 (in) or 1 to 0 (out).
    #[default]
    Fade,
    /// Comes from (goes to) the bottom edge.
    SlideUp,
    /// Comes from (goes to) the top edge.
    SlideDown,
    /// Comes from (goes to) the right edge.
    SlideLeft,
    /// Comes from (goes to) the left edge.
    SlideRight,
    /// Grows from (shrinks to) 80 % of its size around the canvas center,
    /// fading at the same time.
    Scale,
}

impl ScreenTransitionKind {
    pub const ALL: [ScreenTransitionKind; 6] = [
        Self::Fade,
        Self::SlideUp,
        Self::SlideDown,
        Self::SlideLeft,
        Self::SlideRight,
        Self::Scale,
    ];

    /// Stable name (editor, MCP).
    pub fn key(self) -> &'static str {
        match self {
            Self::Fade => "fade",
            Self::SlideUp => "slide_up",
            Self::SlideDown => "slide_down",
            Self::SlideLeft => "slide_left",
            Self::SlideRight => "slide_right",
            Self::Scale => "scale",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }
}

fn default_duration() -> f32 {
    0.3
}

/// A screen transition: kind, length and easing curve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenTransition {
    #[serde(default)]
    pub kind: ScreenTransitionKind,
    /// Seconds (`0` = a cut).
    #[serde(default = "default_duration")]
    pub duration: f32,
    /// Easing of the progress (`None`: linear). A `Named` curve is looked up
    /// in the story's named curves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ease: Option<Curve>,
}

impl Default for ScreenTransition {
    fn default() -> Self {
        Self {
            kind: ScreenTransitionKind::Fade,
            duration: default_duration(),
            ease: None,
        }
    }
}

/// What a transition does to a screen at one moment: multiply the opacity by
/// `alpha`, move it by `offset` (virtual pixels), scale it by `scale` around
/// the canvas center.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenFx {
    pub alpha: f32,
    pub offset: [f32; 2],
    pub scale: f32,
}

impl ScreenFx {
    pub const NONE: ScreenFx = ScreenFx {
        alpha: 1.0,
        offset: [0.0, 0.0],
        scale: 1.0,
    };

    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }
}

impl ScreenTransition {
    /// The look of the screen when `shown` (0 = hidden, 1 = fully shown) of
    /// it is shown, on a canvas of `canvas` virtual pixels. `shown` is
    /// already eased.
    pub fn fx(&self, shown: f32, canvas: [f32; 2]) -> ScreenFx {
        let t = if shown.is_finite() { shown.clamp(0.0, 1.0) } else { 1.0 };
        let hidden = 1.0 - t;
        let mut fx = ScreenFx::NONE;
        match self.kind {
            ScreenTransitionKind::Fade => fx.alpha = t,
            ScreenTransitionKind::SlideUp => fx.offset = [0.0, hidden * canvas[1]],
            ScreenTransitionKind::SlideDown => fx.offset = [0.0, -hidden * canvas[1]],
            ScreenTransitionKind::SlideLeft => fx.offset = [hidden * canvas[0], 0.0],
            ScreenTransitionKind::SlideRight => fx.offset = [-hidden * canvas[0], 0.0],
            ScreenTransitionKind::Scale => {
                fx.scale = 0.8 + 0.2 * t;
                fx.alpha = t;
            }
        }
        fx
    }

    /// Seconds the transition lasts (`0` for a non-finite or negative
    /// duration).
    pub fn seconds(&self) -> f32 {
        if self.duration.is_finite() {
            self.duration.max(0.0)
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_are_exact() {
        for kind in ScreenTransitionKind::ALL {
            let t = ScreenTransition {
                kind,
                ..Default::default()
            };
            assert!(t.fx(1.0, [1920.0, 1080.0]).is_none(), "{kind:?} fully shown");
            assert_eq!(ScreenTransitionKind::from_key(kind.key()), Some(kind));
        }
        let fade = ScreenTransition::default();
        assert_eq!(fade.fx(0.0, [1.0, 1.0]).alpha, 0.0);
        let slide = ScreenTransition {
            kind: ScreenTransitionKind::SlideUp,
            ..Default::default()
        };
        assert_eq!(slide.fx(0.5, [100.0, 200.0]).offset, [0.0, 100.0]);
    }

    #[test]
    fn ron_has_defaults() {
        let t: ScreenTransition = ron::from_str("(kind: Scale)").unwrap();
        assert_eq!(t.duration, 0.3);
        assert_eq!(t.ease, None);
        let back: ScreenTransition = ron::from_str(&ron::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
    }
}
