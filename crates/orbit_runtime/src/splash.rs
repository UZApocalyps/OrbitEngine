//! Splash screens shown before the main menu: the timeline, GPU free.
//!
//! The sequence is the **engine splash** followed by the project's splashes.
//! The engine splash is built into [`SplashState::new`] from constants of
//! this file: no argument, project file or pak entry can remove, shorten or
//! reorder it, and it is the only way to build a sequence. It stays on
//! screen at least [`ENGINE_SKIP_AFTER`] seconds before it can be skipped.
//!
//! The runtime draws the current step ([`SplashKind`]): the logo (embedded in
//! the binary) for the engine, text or an image for the project's. Video is
//! reserved in the data format (the engine has no decoder): such a splash is
//! dropped.

use orbit_ui::{SplashContent, SplashDef, UiInput};

use crate::input::InputAction;
use crate::shell::Shell;

/// Name of the engine, drawn as text when the logo cannot be decoded.
pub(crate) const ENGINE_NAME: &str = "OrbitEngine";
/// Seconds of the engine splash (fades included).
pub(crate) const ENGINE_DURATION: f32 = 3.0;
/// Fade in and out of the engine splash.
pub(crate) const ENGINE_FADE: f32 = 0.6;
/// The engine splash cannot be skipped before this time.
pub(crate) const ENGINE_SKIP_AFTER: f32 = 1.5;
/// Background of the engine splash (the logo is drawn for a light page).
pub(crate) const ENGINE_BACKGROUND: [u8; 4] = [247, 248, 252, 255];
/// A project splash can be skipped after this time (no accidental skip by
/// the click that started the game).
const PROJECT_SKIP_AFTER: f32 = 0.3;
const MAX_DURATION: f32 = 30.0;

/// What a splash shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SplashKind {
    /// The OrbitEngine logo.
    Engine,
    Text(String),
    /// Asset path.
    Image(String),
}

/// One step of the sequence, with sane values (see [`SplashStep::from_def`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SplashStep {
    pub kind: SplashKind,
    pub duration: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    /// Seconds after which the player may skip; `None`: never.
    pub skip_after: Option<f32>,
    pub background: [u8; 4],
}

fn sane(v: f32, default: f32, max: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, max)
    } else {
        default
    }
}

impl SplashStep {
    fn engine() -> Self {
        Self {
            kind: SplashKind::Engine,
            duration: ENGINE_DURATION,
            fade_in: ENGINE_FADE,
            fade_out: ENGINE_FADE,
            skip_after: Some(ENGINE_SKIP_AFTER),
            background: ENGINE_BACKGROUND,
        }
    }

    /// A project splash with its numbers made safe (the data may come from
    /// anywhere): `None` for video, or when nothing sensible is left.
    fn from_def(def: &SplashDef) -> Option<Self> {
        let kind = match &def.content {
            SplashContent::Text(t) => SplashKind::Text(t.clone()),
            SplashContent::Image(p) => SplashKind::Image(p.clone()),
            SplashContent::Video(_) => return None,
        };
        let duration = sane(def.duration, 0.0, MAX_DURATION);
        if duration <= 0.0 {
            return None;
        }
        let fade_in = sane(def.fade_in, 0.5, duration);
        let fade_out = sane(def.fade_out, 0.5, duration - fade_in);
        Some(Self {
            kind,
            duration,
            fade_in,
            fade_out,
            skip_after: def.skippable.then_some(PROJECT_SKIP_AFTER.min(duration)),
            background: def.background,
        })
    }
}

/// Where the splash sequence is.
#[derive(Clone, Debug)]
pub(crate) struct SplashState {
    steps: Vec<SplashStep>,
    index: usize,
    t: f32,
}

impl SplashState {
    /// The engine splash, then `project`'s splashes in order.
    pub fn new(project: &[SplashDef]) -> Self {
        let mut steps = vec![SplashStep::engine()];
        steps.extend(project.iter().filter_map(SplashStep::from_def));
        Self {
            steps,
            index: 0,
            t: 0.0,
        }
    }

    pub fn steps(&self) -> &[SplashStep] {
        &self.steps
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// Seconds into the current step.
    #[cfg(test)]
    pub fn time(&self) -> f32 {
        self.t
    }

    pub fn finished(&self) -> bool {
        self.index >= self.steps.len()
    }

    pub fn current(&self) -> Option<&SplashStep> {
        self.steps.get(self.index)
    }

    /// Opacity of the content of the current step (fades).
    pub fn alpha(&self) -> f32 {
        let Some(step) = self.current() else {
            return 0.0;
        };
        let mut a = 1.0f32;
        if step.fade_in > 0.0 && self.t < step.fade_in {
            a = a.min(self.t / step.fade_in);
        }
        let out_start = step.duration - step.fade_out;
        if step.fade_out > 0.0 && self.t > out_start {
            a = a.min((step.duration - self.t) / step.fade_out);
        }
        a.clamp(0.0, 1.0)
    }

    /// Advances the timeline; a long frame never skips a whole step.
    pub fn update(&mut self, dt: f32) {
        let Some(duration) = self.current().map(|s| s.duration) else {
            return;
        };
        self.t += dt.max(0.0);
        if self.t >= duration {
            self.index += 1;
            self.t = 0.0;
        }
    }

    /// The player asked to skip: the current step starts fading out when
    /// it may be skipped yet. Returns whether it did.
    pub fn skip(&mut self) -> bool {
        let Some(step) = self.current() else {
            return false;
        };
        let Some(after) = step.skip_after else {
            return false;
        };
        let out_start = step.duration - step.fade_out;
        if self.t < after || self.t >= out_start {
            return false;
        }
        self.t = out_start;
        true
    }
}

/// What follows a freshly loaded story.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AfterLoad {
    /// A restart inside a running session (hot reload): the splashes
    /// already played, the game starts at once.
    RunNow,
    /// Native: no click gate, the splashes play at once (then a new game
    /// when `auto_start`, else the main menu).
    Splash,
    /// Wait for the "click to start" gesture.
    Ready,
}

impl AfterLoad {
    pub fn decide(autostart: bool, native_auto: bool) -> Self {
        if autostart {
            Self::RunNow
        } else if native_auto {
            Self::Splash
        } else {
            Self::Ready
        }
    }
}

/// The splashes playing in front of a shell that is ready.
pub(crate) struct SplashPhase {
    pub shell: Box<Shell>,
    pub state: SplashState,
    /// Start a new game when the splashes end (`auto_start`).
    pub start_game: bool,
}

impl SplashPhase {
    pub fn new(shell: Box<Shell>, state: SplashState, start_game: bool) -> Self {
        Self {
            shell,
            state,
            start_game,
        }
    }

    /// A click, Confirm or Cancel asks to skip; every input is swallowed
    /// (nothing reaches the shell while the splashes play).
    pub fn input(&mut self, action: &InputAction) {
        if matches!(
            action,
            InputAction::Ui(UiInput::Confirm | UiInput::PointerDown(_) | UiInput::Cancel)
        ) {
            self.state.skip();
        }
    }

    /// Advances the timeline; once finished, hands the shell over (with a
    /// new game started when asked).
    pub fn update(mut self, dt: f32) -> Result<Box<Shell>, Self> {
        self.state.update(dt);
        if !self.state.finished() {
            return Err(self);
        }
        if self.start_game {
            self.shell.start_new_game();
        }
        Ok(self.shell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(duration: f32) -> SplashDef {
        SplashDef {
            content: SplashContent::Text("Studio".into()),
            duration,
            fade_in: 0.2,
            fade_out: 0.2,
            skippable: true,
            background: [1, 2, 3, 255],
        }
    }

    fn run_to_end(s: &mut SplashState) -> Vec<usize> {
        let mut seen = vec![s.index()];
        for _ in 0..10_000 {
            if s.finished() {
                break;
            }
            s.update(0.05);
            if seen.last() != Some(&s.index()) {
                seen.push(s.index());
            }
        }
        seen
    }

    #[test]
    fn the_engine_splash_is_first_even_with_an_empty_bundle() {
        let s = SplashState::new(&[]);
        assert_eq!(s.steps().len(), 1);
        assert_eq!(s.steps()[0].kind, SplashKind::Engine);
        assert!(!s.finished());
        assert_eq!(s.current().unwrap().kind, SplashKind::Engine);
    }

    #[test]
    fn project_splashes_follow_in_order() {
        let mut image = text(1.0);
        image.content = SplashContent::Image("logo.png".into());
        let s = SplashState::new(&[text(1.0), image]);
        let kinds: Vec<_> = s.steps().iter().map(|x| x.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                SplashKind::Engine,
                SplashKind::Text("Studio".into()),
                SplashKind::Image("logo.png".into())
            ]
        );
        let mut s = s;
        assert_eq!(run_to_end(&mut s), vec![0, 1, 2, 3]);
        assert!(s.finished());
        assert_eq!(s.alpha(), 0.0);
    }

    #[test]
    fn tampered_data_cannot_remove_or_alter_the_engine_splash() {
        let hostile = [
            text(0.0),
            text(f32::NAN),
            text(-5.0),
            text(f32::INFINITY),
            SplashDef {
                content: SplashContent::Video("a.mp4".into()),
                ..text(3.0)
            },
        ];
        let s = SplashState::new(&hostile);
        assert_eq!(s.steps()[0], SplashStep::engine());
        assert_eq!(s.steps()[0].duration, ENGINE_DURATION);
        // Zero, NaN, negative, infinite and video steps are all dropped.
        assert_eq!(s.steps().len(), 1);
        let long = SplashState::new(&[text(1000.0)]);
        assert_eq!(long.steps()[1].duration, MAX_DURATION);
    }

    #[test]
    fn fades_never_exceed_the_duration() {
        let mut d = text(1.0);
        d.fade_in = 5.0;
        d.fade_out = 5.0;
        let step = SplashStep::from_def(&d).unwrap();
        assert!(step.fade_in + step.fade_out <= step.duration + 1e-6);
    }

    #[test]
    fn the_engine_splash_is_not_skippable_at_first_then_fades_out() {
        let mut s = SplashState::new(&[]);
        s.update(ENGINE_SKIP_AFTER - 0.1);
        assert!(!s.skip(), "too early");
        assert_eq!(s.index(), 0);
        s.update(0.2);
        assert!(s.skip());
        assert!((s.time() - (ENGINE_DURATION - ENGINE_FADE)).abs() < 1e-6);
        assert!(!s.skip(), "already leaving");
        s.update(ENGINE_FADE + 0.01);
        assert!(s.finished());
    }

    #[test]
    fn a_non_skippable_project_splash_plays_to_the_end() {
        let mut d = text(1.0);
        d.skippable = false;
        let mut s = SplashState::new(&[d]);
        s.update(ENGINE_DURATION + 0.01);
        assert_eq!(s.index(), 1);
        s.update(0.5);
        assert!(!s.skip());
    }

    #[test]
    fn alpha_ramps_in_holds_and_ramps_out() {
        let mut s = SplashState::new(&[]);
        assert_eq!(s.alpha(), 0.0);
        s.update(ENGINE_FADE / 2.0);
        assert!((s.alpha() - 0.5).abs() < 1e-5);
        s.update(ENGINE_DURATION / 2.0 - ENGINE_FADE / 2.0);
        assert_eq!(s.alpha(), 1.0);
        s.update(ENGINE_DURATION / 2.0 - ENGINE_FADE / 2.0 - 0.001 + ENGINE_FADE / 2.0);
        assert!(s.alpha() < 0.55 && s.alpha() > 0.0);
    }

    #[test]
    fn a_huge_frame_advances_one_step_at_a_time() {
        let mut s = SplashState::new(&[text(1.0)]);
        s.update(100.0);
        assert_eq!(s.index(), 1, "the next splash still shows");
    }
}
