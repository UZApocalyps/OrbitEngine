//! Touch gestures (ADR 0017): turns raw touch points into the input events
//! the app already understands, so the runtime needs no touch-specific code.
//!
//! * tap: cursor at the finger then a left click (advance, press a button);
//! * long press (finger held still): a right click (Cancel: menu);
//! * swipe down: a wheel step up (history); swipe up: a wheel step down.
//!
//! Only the first finger is followed; others are ignored. The long press is
//! recognized when the finger lifts. Pure, so it is tested on the host.

use std::time::Duration;

use web_time::Instant;
use winit::event::{MouseButton, TouchPhase};

use crate::InputEvent;

/// Thresholds, in density-independent pixels (dp) and time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchConfig {
    /// Held at least this long without moving: a long press.
    pub long_press: Duration,
    /// Moving less than this (dp) is still a tap.
    pub slop_dp: f64,
    /// A vertical travel of at least this (dp) is a swipe.
    pub swipe_dp: f64,
}

impl Default for TouchConfig {
    fn default() -> Self {
        Self {
            long_press: Duration::from_millis(500),
            slop_dp: 12.0,
            swipe_dp: 64.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Track {
    id: u64,
    started: Instant,
    start: (f64, f64),
    /// Moved beyond the slop at some point: no tap, no long press.
    moved: bool,
}

/// The gesture recognizer of one window.
#[derive(Debug, Default)]
pub struct TouchRecognizer {
    config: TouchConfig,
    track: Option<Track>,
}

impl TouchRecognizer {
    pub fn new(config: TouchConfig) -> Self {
        Self {
            config,
            track: None,
        }
    }

    /// Feeds one touch point (`x`, `y` in physical pixels; `scale` is the
    /// window scale factor, physical pixels per dp) and returns the input
    /// events it produces.
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        (x, y): (f64, f64),
        now: Instant,
        scale: f64,
    ) -> Vec<InputEvent> {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let cursor = InputEvent::CursorMoved { x, y };
        match phase {
            TouchPhase::Started => {
                if self.track.is_some() {
                    return Vec::new(); // a second finger
                }
                self.track = Some(Track {
                    id,
                    started: now,
                    start: (x, y),
                    moved: false,
                });
                vec![cursor]
            }
            TouchPhase::Moved => match &mut self.track {
                Some(track) if track.id == id => {
                    let (dx, dy) = (x - track.start.0, y - track.start.1);
                    if dx.hypot(dy) > self.config.slop_dp * scale {
                        track.moved = true;
                    }
                    vec![cursor]
                }
                _ => Vec::new(),
            },
            TouchPhase::Cancelled => {
                if self.track.is_some_and(|t| t.id == id) {
                    self.track = None;
                }
                Vec::new()
            }
            TouchPhase::Ended => {
                let Some(track) = self.track.filter(|t| t.id == id) else {
                    return Vec::new();
                };
                self.track = None;
                let (dx, dy) = (x - track.start.0, y - track.start.1);
                if dy.abs() >= self.config.swipe_dp * scale && dy.abs() > dx.abs() {
                    // Finger down = content down = wheel up (history).
                    let step = if dy > 0.0 { 1.0 } else { -1.0 };
                    return vec![InputEvent::Scroll { dx: 0.0, dy: step }];
                }
                if track.moved {
                    return Vec::new();
                }
                let held = now.saturating_duration_since(track.started);
                let button = if held >= self.config.long_press {
                    MouseButton::Right
                } else {
                    MouseButton::Left
                };
                vec![
                    cursor,
                    InputEvent::MouseButton {
                        button,
                        pressed: true,
                    },
                    InputEvent::MouseButton {
                        button,
                        pressed: false,
                    },
                ]
            }
        }
    }
}

/// Whether a key event is the Android back button (or a keyboard's
/// "browser back" key): the shell treats it as `Cancel`, like Escape.
pub fn is_back_key(
    physical: &winit::keyboard::PhysicalKey,
    logical: &winit::keyboard::Key,
) -> bool {
    use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
    matches!(physical, PhysicalKey::Code(KeyCode::BrowserBack))
        || matches!(
            logical,
            Key::Named(NamedKey::GoBack | NamedKey::BrowserBack)
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn click(button: MouseButton, x: f64, y: f64) -> Vec<InputEvent> {
        vec![
            InputEvent::CursorMoved { x, y },
            InputEvent::MouseButton {
                button,
                pressed: true,
            },
            InputEvent::MouseButton {
                button,
                pressed: false,
            },
        ]
    }

    #[test]
    fn a_tap_is_a_left_click_where_the_finger_lifts() {
        let mut r = TouchRecognizer::default();
        let t0 = Instant::now();
        let start = r.touch(1, TouchPhase::Started, (100.0, 200.0), t0, 2.0);
        assert_eq!(start, vec![InputEvent::CursorMoved { x: 100.0, y: 200.0 }]);
        // A small wobble (under 12 dp = 24 px at scale 2) is still a tap.
        r.touch(1, TouchPhase::Moved, (110.0, 205.0), t0 + 50 * MS, 2.0);
        let end = r.touch(1, TouchPhase::Ended, (110.0, 205.0), t0 + 120 * MS, 2.0);
        assert_eq!(end, click(MouseButton::Left, 110.0, 205.0));
    }

    #[test]
    fn a_long_press_is_a_right_click() {
        let mut r = TouchRecognizer::default();
        let t0 = Instant::now();
        r.touch(7, TouchPhase::Started, (10.0, 10.0), t0, 1.0);
        let end = r.touch(7, TouchPhase::Ended, (10.0, 10.0), t0 + 600 * MS, 1.0);
        assert_eq!(end, click(MouseButton::Right, 10.0, 10.0));
    }

    #[test]
    fn vertical_swipes_scroll() {
        let mut r = TouchRecognizer::default();
        let t0 = Instant::now();
        r.touch(1, TouchPhase::Started, (300.0, 100.0), t0, 1.5);
        r.touch(1, TouchPhase::Moved, (305.0, 200.0), t0 + 50 * MS, 1.5);
        let down = r.touch(1, TouchPhase::Ended, (305.0, 220.0), t0 + 100 * MS, 1.5);
        assert_eq!(down, vec![InputEvent::Scroll { dx: 0.0, dy: 1.0 }]);

        r.touch(2, TouchPhase::Started, (300.0, 400.0), t0, 1.0);
        let up = r.touch(2, TouchPhase::Ended, (300.0, 300.0), t0 + 100 * MS, 1.0);
        assert_eq!(up, vec![InputEvent::Scroll { dx: 0.0, dy: -1.0 }]);
    }

    #[test]
    fn a_drag_that_is_no_swipe_does_nothing() {
        let mut r = TouchRecognizer::default();
        let t0 = Instant::now();
        r.touch(1, TouchPhase::Started, (0.0, 0.0), t0, 1.0);
        // Horizontal travel: no swipe, too far for a tap.
        r.touch(1, TouchPhase::Moved, (200.0, 10.0), t0 + 50 * MS, 1.0);
        assert!(
            r.touch(1, TouchPhase::Ended, (200.0, 10.0), t0 + 900 * MS, 1.0)
                .is_empty()
        );
    }

    #[test]
    fn other_fingers_and_cancellations_are_ignored() {
        let mut r = TouchRecognizer::default();
        let t0 = Instant::now();
        r.touch(1, TouchPhase::Started, (0.0, 0.0), t0, 1.0);
        assert!(
            r.touch(2, TouchPhase::Started, (50.0, 50.0), t0, 1.0)
                .is_empty()
        );
        assert!(
            r.touch(2, TouchPhase::Ended, (50.0, 50.0), t0, 1.0)
                .is_empty()
        );
        assert!(
            r.touch(1, TouchPhase::Cancelled, (0.0, 0.0), t0, 1.0)
                .is_empty()
        );
        // The cancelled touch produces no click when it "ends".
        assert!(
            r.touch(1, TouchPhase::Ended, (0.0, 0.0), t0, 1.0)
                .is_empty()
        );
    }

    #[test]
    fn back_key_detection() {
        use winit::keyboard::{Key, KeyCode, NamedKey, NativeKeyCode, PhysicalKey};
        let unknown = PhysicalKey::Unidentified(NativeKeyCode::Unidentified);
        assert!(is_back_key(&unknown, &Key::Named(NamedKey::GoBack)));
        assert!(is_back_key(&unknown, &Key::Named(NamedKey::BrowserBack)));
        assert!(is_back_key(
            &PhysicalKey::Code(KeyCode::BrowserBack),
            &Key::Named(NamedKey::Escape)
        ));
        assert!(!is_back_key(
            &PhysicalKey::Code(KeyCode::Escape),
            &Key::Named(NamedKey::Escape)
        ));
    }
}
