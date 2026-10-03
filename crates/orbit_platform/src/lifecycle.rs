//! Application lifecycle (ADR 0017): the `resumed` / `suspended` pairs winit
//! reports. On desktop and the web `resumed` comes once at start; on Android
//! it comes again each time the app returns to the foreground, after a
//! `suspended` that took the native window (and so the GPU surface) away.
//!
//! The state machine is pure so it can be tested on the host; the runner
//! executes the [`LifecycleAction`]s.

/// Where the app is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LifecycleState {
    /// Nothing created yet: waiting for the first `resumed`.
    #[default]
    Starting,
    /// In the foreground with a surface.
    Running,
    /// In the background: no surface, no frame.
    Suspended,
}

/// What the runner does after a lifecycle event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleAction {
    /// First `resumed`: create the window and the app.
    Launch,
    /// Back to the foreground: the app recreates its surface.
    ResumeSurface,
    /// Going to the background: the app saves and releases its surface.
    SuspendSurface,
    /// A repeated or early event: nothing to do.
    Nothing,
}

/// The lifecycle state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lifecycle {
    state: LifecycleState,
}

impl Lifecycle {
    pub fn state(&self) -> LifecycleState {
        self.state
    }

    /// Whether frames may be drawn.
    pub fn is_running(&self) -> bool {
        self.state == LifecycleState::Running
    }

    /// winit's `resumed`.
    pub fn on_resumed(&mut self) -> LifecycleAction {
        let action = match self.state {
            LifecycleState::Starting => LifecycleAction::Launch,
            LifecycleState::Suspended => LifecycleAction::ResumeSurface,
            LifecycleState::Running => LifecycleAction::Nothing,
        };
        self.state = LifecycleState::Running;
        action
    }

    /// winit's `suspended`. A `suspended` before the first `resumed` (Android
    /// can start an activity and stop it at once) has nothing to release.
    pub fn on_suspended(&mut self) -> LifecycleAction {
        match self.state {
            LifecycleState::Running => {
                self.state = LifecycleState::Suspended;
                LifecycleAction::SuspendSurface
            }
            LifecycleState::Starting | LifecycleState::Suspended => LifecycleAction::Nothing,
        }
    }
}

/// The part of the window not covered by system bars or a display cutout, as
/// insets in physical pixels from each edge (Android `content_rect`). UI
/// meant to be touched should stay inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SafeArea {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl SafeArea {
    /// The insets of the content rectangle `(left, top, right, bottom)`
    /// (edges, not sizes) inside a `width` x `height` window. A rectangle
    /// partly outside the window is clamped; an empty or inverted one (not
    /// known yet) means no inset.
    pub fn from_content_rect(width: u32, height: u32, rect: (i32, i32, i32, i32)) -> Self {
        let (l, t, r, b) = rect;
        if r <= l || b <= t {
            return Self::default();
        }
        let clamp = |v: i32, max: u32| (v.max(0) as u32).min(max);
        let (l, r) = (clamp(l, width), clamp(r, width));
        let (t, b) = (clamp(t, height), clamp(b, height));
        Self {
            left: l,
            top: t,
            right: width - r,
            bottom: height - b,
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_resumes_once() {
        let mut life = Lifecycle::default();
        assert_eq!(life.on_resumed(), LifecycleAction::Launch);
        assert!(life.is_running());
        // A duplicate `resumed` does not rebuild anything.
        assert_eq!(life.on_resumed(), LifecycleAction::Nothing);
    }

    #[test]
    fn android_background_and_back() {
        let mut life = Lifecycle::default();
        life.on_resumed();
        assert_eq!(life.on_suspended(), LifecycleAction::SuspendSurface);
        assert_eq!(life.state(), LifecycleState::Suspended);
        assert!(!life.is_running());
        assert_eq!(life.on_suspended(), LifecycleAction::Nothing);
        assert_eq!(life.on_resumed(), LifecycleAction::ResumeSurface);
        assert!(life.is_running());
        // Several round trips.
        assert_eq!(life.on_suspended(), LifecycleAction::SuspendSurface);
        assert_eq!(life.on_resumed(), LifecycleAction::ResumeSurface);
    }

    #[test]
    fn suspend_before_start_is_ignored() {
        let mut life = Lifecycle::default();
        assert_eq!(life.on_suspended(), LifecycleAction::Nothing);
        assert_eq!(life.on_resumed(), LifecycleAction::Launch);
    }

    #[test]
    fn safe_area_from_content_rect() {
        // Status bar of 60 px, navigation bar of 120 px on the right.
        let area = SafeArea::from_content_rect(2400, 1080, (0, 60, 2280, 1080));
        assert_eq!(
            area,
            SafeArea {
                left: 0,
                top: 60,
                right: 120,
                bottom: 0
            }
        );
        // Unknown rectangle, full-window rectangle, overflowing rectangle.
        assert!(SafeArea::from_content_rect(800, 600, (0, 0, 0, 0)).is_empty());
        assert!(SafeArea::from_content_rect(800, 600, (0, 0, 800, 600)).is_empty());
        assert!(SafeArea::from_content_rect(800, 600, (-5, -5, 900, 700)).is_empty());
    }
}
