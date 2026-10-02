//! The director: drives the VM and turns its output into a presentation
//! state machine. No GPU, no audio device, no clock: time comes from
//! [`Director::update`], so everything here is unit tested.
//!
//! Besides playing forward it can go back in time: [`Director::rollback`],
//! [`Director::roll_forward`] and [`Director::rollback_to`] ask the VM for a
//! [`Presentation`] and rebuild the stage from it with cuts (no transition),
//! decide what the music must do (nothing if the right track already plays) and
//! show the line or choice again. [`Director::resumed`] does the same for a VM
//! that was just restored from a save.

use std::sync::Arc;

use orbit_core::{AssetId, CharacterId, StringId, TransitionKind};
use orbit_i18n::Localizer;
use orbit_vm::{Interaction, MusicState, Presentation, PresentationEvent, StepResult, Vm};

use crate::stage::{StageEffect, StageState};

/// A VM step loop never runs more than this many times per call (guards
/// against stories made only of zero-length waits).
const MAX_STEPS_PER_RUN: usize = 64;
/// Fade used when a rollback or a load has to change the music.
const MUSIC_SWAP_FADE: f32 = 0.5;

/// A line to display.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Speaker name and sRGB color; `None` for the narrator.
    pub speaker: Option<(String, [u8; 3])>,
    pub text: String,
    /// The line was already shown before (this or another playthrough, or a
    /// rollback): candidates for skipping.
    pub already_read: bool,
}

/// What the story is doing right now.
#[derive(Clone, Debug, PartialEq)]
pub enum Flow {
    /// A line is shown; waiting for the player.
    Say(Line),
    /// Options are shown; waiting for the player's pick.
    Choice(Vec<String>),
    /// A timed pause (`Wait` node).
    Waiting { remaining: f32 },
    /// A visual transition is playing.
    Transition { elapsed: f32, duration: f32 },
    /// The story is over.
    Ended,
    /// The VM failed; the message is meant for the player/author.
    Failed(String),
}

/// Audio request for the host, in execution order.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioCommand {
    PlayMusic {
        asset: AssetId,
        fade_in: f32,
        looping: bool,
    },
    StopMusic {
        fade_out: f32,
    },
    PlaySound {
        asset: AssetId,
    },
}

/// The visual change being animated: the stage before it, and how to blend.
#[derive(Clone, Debug, PartialEq)]
pub struct StageTransition {
    /// The stage as it was before the change (the "to" side is the live
    /// stage).
    pub from: StageState,
    pub kind: TransitionKind,
    /// `0..=1`.
    pub progress: f32,
}

#[derive(Clone, Debug)]
struct PendingTransition {
    ease: Option<orbit_core::Curve>,
    from: StageState,
    kind: TransitionKind,
}

/// Runs a story and exposes what to show.
pub struct Director {
    vm: Vm,
    loc: Arc<Localizer>,
    stage: StageState,
    flow: Flow,
    /// The line or choice on screen (to re-translate it without the VM).
    shown: Option<Interaction>,
    pending: Option<PendingTransition>,
    audio: Vec<AudioCommand>,
    /// The music the host was last asked to play.
    music: Option<MusicState>,
    generation: u64,
    progress: u64,
}

impl Director {
    /// Starts the story in its source language: runs the VM up to its first
    /// blocking point.
    pub fn new(vm: Vm) -> Self {
        let loc = Arc::new(Localizer::from_story(vm.story()));
        Self::with_localizer(vm, loc)
    }

    /// Like [`Director::new`], showing texts through `loc`.
    pub fn with_localizer(vm: Vm, loc: Arc<Localizer>) -> Self {
        let mut director = Self::bare(vm, loc);
        director.run();
        director
    }

    /// Continues a VM that was just restored (a save was applied): shows what
    /// was on screen, with the scene and the music, without replaying any
    /// transition. A VM that shows nothing yet simply starts running.
    pub fn resumed(vm: Vm, loc: Arc<Localizer>) -> Self {
        let mut director = Self::bare(vm, loc);
        let presentation = director.vm.presentation();
        director.show(presentation);
        director
    }

    fn bare(vm: Vm, loc: Arc<Localizer>) -> Self {
        Self {
            vm,
            loc,
            stage: StageState::default(),
            flow: Flow::Ended,
            shown: None,
            pending: None,
            audio: Vec::new(),
            music: None,
            generation: 0,
            progress: 0,
        }
    }

    pub fn flow(&self) -> &Flow {
        &self.flow
    }

    pub fn stage(&self) -> &StageState {
        &self.stage
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn vm_mut(&mut self) -> &mut Vm {
        &mut self.vm
    }

    pub fn localizer(&self) -> &Arc<Localizer> {
        &self.loc
    }

    /// Incremented every time a new line or choice is presented (going
    /// forward, back, or after a load), so the UI knows when to rebuild its
    /// widgets.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Incremented only when the story moves *forward* to a new line or
    /// choice (not on rollback or load): what autosave counts.
    pub fn progress(&self) -> u64 {
        self.progress
    }

    /// The music the director last asked the host to play.
    pub fn music(&self) -> Option<&MusicState> {
        self.music.as_ref()
    }

    /// The line or choice on screen.
    pub fn shown(&self) -> Option<&Interaction> {
        self.shown.as_ref()
    }

    /// The transition being played, if any.
    pub fn transition(&self) -> Option<StageTransition> {
        let Flow::Transition { elapsed, duration } = &self.flow else {
            return None;
        };
        let pending = self.pending.as_ref()?;
        Some(StageTransition {
            from: pending.from.clone(),
            kind: pending.kind,
            progress: {
                let raw = if *duration > 0.0 {
                    (elapsed / duration).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                // The easing only reshapes the way; both ends stay exact
                // (`eval(0) = 0`, `eval(1) = 1`), so skipping still lands on
                // the final image.
                match &pending.ease {
                    Some(curve) => {
                        let story = self.vm.story();
                        curve.eval_with(raw, |n| story.curves.get(n))
                    }
                    None => raw,
                }
            },
        })
    }

    /// Audio requests accumulated since the last call.
    pub fn take_audio(&mut self) -> Vec<AudioCommand> {
        std::mem::take(&mut self.audio)
    }

    /// Advances timers (pauses and transitions); resumes the VM when they end.
    pub fn update(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        match &mut self.flow {
            Flow::Waiting { remaining } => {
                *remaining -= dt;
                if *remaining <= 0.0 {
                    self.run();
                }
            }
            Flow::Transition { elapsed, duration } => {
                *elapsed += dt;
                if *elapsed >= *duration {
                    self.pending = None;
                    self.run();
                }
            }
            _ => {}
        }
    }

    /// The player pressed "advance". On a line: next step. During a
    /// transition: completes it immediately (clicks are not ignored, they
    /// skip). Ignored while pausing, choosing or after the end.
    pub fn advance(&mut self) {
        match self.flow {
            Flow::Say(_) => self.run(),
            Flow::Transition { .. } => {
                self.pending = None;
                self.run();
            }
            _ => {}
        }
    }

    /// Cuts a timed pause or a transition short (skip mode). Does nothing
    /// during a line or a choice.
    pub fn skip_timer(&mut self) {
        if matches!(self.flow, Flow::Waiting { .. } | Flow::Transition { .. }) {
            self.pending = None;
            self.run();
        }
    }

    /// The player picked option `index` of the current [`Flow::Choice`].
    /// Returns `false` (and changes nothing) if that is not possible.
    pub fn choose(&mut self, index: usize) -> bool {
        if !matches!(self.flow, Flow::Choice(_)) {
            return false;
        }
        match self.vm.choose(index) {
            Ok(()) => {
                self.run();
                true
            }
            Err(err) => {
                log::warn!("invalid choice {index}: {err}");
                false
            }
        }
    }

    // ------------------------------------------------------------ time travel

    /// `true` if [`Director::rollback`] has somewhere to go.
    pub fn can_rollback(&self) -> bool {
        self.vm.can_rollback()
    }

    /// `true` if [`Director::roll_forward`] has somewhere to go.
    pub fn can_roll_forward(&self) -> bool {
        self.vm.can_roll_forward()
    }

    /// Goes back `steps` lines or choices. Returns `false` (nothing changed)
    /// when already at the oldest point.
    pub fn rollback(&mut self, steps: usize) -> bool {
        let presentation = self.vm.rollback(steps);
        self.show_moved(presentation)
    }

    /// Goes forward through the history a rollback kept.
    pub fn roll_forward(&mut self, steps: usize) -> bool {
        let presentation = self.vm.roll_forward(steps);
        self.show_moved(presentation)
    }

    /// Goes back to the history entry `seq` (see `Vm::backlog`).
    pub fn rollback_to(&mut self, seq: u64) -> bool {
        let presentation = self.vm.rollback_to(seq);
        self.show_moved(presentation)
    }

    fn show_moved(&mut self, presentation: Option<Presentation>) -> bool {
        match presentation {
            Some(p) => {
                self.show(p);
                true
            }
            None => false,
        }
    }

    /// Rebuilds everything from a [`Presentation`]: stage (cut), music,
    /// line or choice. No transition is replayed and no sound effect is
    /// repeated.
    fn show(&mut self, presentation: Presentation) {
        self.pending = None;
        self.stage = StageState::default();
        for event in &presentation.events {
            self.stage.apply(event);
        }
        self.sync_music(presentation.scene.music.as_ref());
        match presentation.interaction {
            Some(interaction) => self.present(interaction, true),
            None => self.run(),
        }
    }

    /// Makes the music match `target`: nothing if it already is, else a swap
    /// (or a stop).
    fn sync_music(&mut self, target: Option<&MusicState>) {
        if self.music.as_ref() == target {
            return;
        }
        match target {
            Some(music) => self.audio.push(AudioCommand::PlayMusic {
                asset: music.asset,
                fade_in: MUSIC_SWAP_FADE,
                looping: music.looping,
            }),
            None => self.audio.push(AudioCommand::StopMusic {
                fade_out: MUSIC_SWAP_FADE,
            }),
        }
        self.music = target.cloned();
    }

    // ------------------------------------------------------------ localization

    /// Switches the language of the texts: the line or choice on screen is
    /// translated in place.
    pub fn set_localizer(&mut self, loc: Arc<Localizer>) {
        self.loc = loc;
        if matches!(self.flow, Flow::Say(_) | Flow::Choice(_))
            && let Some(interaction) = self.shown.clone()
        {
            let already_read = matches!(&self.flow, Flow::Say(l) if l.already_read);
            self.flow = self.flow_for(&interaction, already_read);
        }
    }

    // ------------------------------------------------------------------

    fn text(&self, id: StringId) -> String {
        self.loc.text(id).to_owned()
    }

    /// Display name (translated) and sRGB color of a character.
    pub fn speaker(&self, id: CharacterId) -> Option<(String, [u8; 3])> {
        let story = self.vm.story();
        let def = story.characters.iter().find(|c| c.id == id)?;
        let color = def.color;
        Some((self.text(def.name_id), [color.r, color.g, color.b]))
    }

    fn flow_for(&self, interaction: &Interaction, already_read: bool) -> Flow {
        match interaction {
            Interaction::Say {
                speaker, string_id, ..
            } => Flow::Say(Line {
                speaker: speaker.and_then(|id| self.speaker(id)),
                text: self.text(*string_id),
                already_read,
            }),
            Interaction::Choice { options, .. } => {
                Flow::Choice(options.iter().map(|o| self.text(o.string_id)).collect())
            }
        }
    }

    /// Shows a line or a choice. `already_read` is `true` for a line shown
    /// again (rollback, load).
    fn present(&mut self, interaction: Interaction, already_read: bool) {
        self.pending = None; // a transition not waited for is a cut
        self.flow = self.flow_for(&interaction, already_read);
        self.shown = Some(interaction);
        self.generation += 1;
    }

    fn apply_events(&mut self) {
        for event in self.vm.take_events() {
            match &event {
                PresentationEvent::PlayMusic {
                    asset,
                    fade_in,
                    looping,
                } => {
                    self.music = Some(MusicState {
                        asset: *asset,
                        looping: *looping,
                    });
                    self.audio.push(AudioCommand::PlayMusic {
                        asset: *asset,
                        fade_in: *fade_in,
                        looping: *looping,
                    });
                }
                PresentationEvent::StopMusic { fade_out } => {
                    self.music = None;
                    self.audio.push(AudioCommand::StopMusic {
                        fade_out: *fade_out,
                    })
                }
                PresentationEvent::PlaySound { asset } => {
                    self.audio.push(AudioCommand::PlaySound { asset: *asset })
                }
                _ => {}
            }
            let before = self.stage.clone();
            if let StageEffect::Visual(Some(t)) = self.stage.apply(&event)
                && t.duration > 0.0
                && t.duration.is_finite()
                && self.pending.is_none()
            {
                self.pending = Some(PendingTransition {
                    from: before,
                    kind: t.kind,
                    ease: t.ease,
                });
            }
        }
    }

    /// Steps the VM until it blocks on something the host must handle.
    fn run(&mut self) {
        for _ in 0..MAX_STEPS_PER_RUN {
            let result = self.vm.step();
            self.apply_events();
            match result {
                StepResult::Say {
                    node,
                    speaker,
                    string_id,
                    already_read,
                } => {
                    self.progress += 1;
                    self.present(
                        Interaction::Say {
                            node,
                            speaker,
                            string_id,
                        },
                        already_read,
                    );
                    return;
                }
                StepResult::Choice { node, options } => {
                    self.progress += 1;
                    self.present(Interaction::Choice { node, options }, false);
                    return;
                }
                StepResult::WaitTime(seconds) => {
                    self.pending = None;
                    if seconds.is_finite() && seconds > 0.0 {
                        self.flow = Flow::Waiting { remaining: seconds };
                        return;
                    }
                    // Zero-length wait: keep going.
                }
                StepResult::WaitTransition { duration } => {
                    if self.pending.is_some() && duration > 0.0 && duration.is_finite() {
                        self.flow = Flow::Transition {
                            elapsed: 0.0,
                            duration,
                        };
                        return;
                    }
                    self.pending = None; // nothing to animate: continue
                }
                StepResult::End => {
                    self.pending = None;
                    self.flow = Flow::Ended;
                    self.shown = None;
                    return;
                }
                StepResult::Error(err) => {
                    self.pending = None;
                    log::error!("story error: {err}");
                    self.flow = Flow::Failed(err.to_string());
                    return;
                }
            }
        }
        // Too many consecutive instant steps: yield and resume next frame.
        self.flow = Flow::Waiting { remaining: 0.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::Story;

    const DEMO: &str = include_str!("../../../examples/stories/test_minimal.ron");

    fn demo() -> Director {
        let story = Story::from_ron(DEMO).expect("demo story is valid");
        Director::new(Vm::from_start(story).unwrap())
    }

    /// Plays on (advancing lines, finishing transitions, choosing option 0)
    /// until the story ends; returns what happened.
    struct Playthrough {
        lines: usize,
        choices: usize,
        transitions: usize,
        audio: Vec<AudioCommand>,
    }

    fn play_to_end(d: &mut Director) -> Playthrough {
        let mut p = Playthrough {
            lines: 0,
            choices: 0,
            transitions: 0,
            audio: Vec::new(),
        };
        for _ in 0..1000 {
            p.audio.extend(d.take_audio());
            match d.flow().clone() {
                Flow::Say(_) => {
                    p.lines += 1;
                    d.advance();
                }
                Flow::Choice(_) => {
                    p.choices += 1;
                    assert!(d.choose(0));
                }
                Flow::Transition { duration, .. } => {
                    p.transitions += 1;
                    d.update(duration + 0.01);
                }
                Flow::Waiting { remaining } => d.update(remaining + 0.01),
                Flow::Ended => return p,
                Flow::Failed(msg) => panic!("story failed: {msg}"),
            }
        }
        panic!("story did not end");
    }

    #[test]
    fn demo_starts_with_a_fade_transition_over_the_new_background() {
        let d = demo();
        assert!(matches!(d.flow(), Flow::Transition { duration, .. } if *duration == 1.0));
        let tr = d.transition().expect("a transition is active");
        assert_eq!(tr.progress, 0.0);
        assert_eq!(tr.from, StageState::default()); // black before
        assert!(d.stage().background.is_some()); // room after
        assert!(matches!(tr.kind, TransitionKind::Fade(_)));
    }

    /// The demo with `ease` on its first transition (the opening background).
    fn demo_eased(ease: orbit_core::Curve, named: &[(&str, orbit_core::Curve)]) -> Director {
        let mut story = Story::from_ron(DEMO).expect("demo story is valid");
        for (n, c) in named {
            story.curves.insert((*n).to_owned(), c.clone());
        }
        let node = story
            .scenes
            .iter_mut()
            .flat_map(|s| &mut s.nodes)
            .find_map(|n| match &mut n.kind {
                orbit_core::NodeKind::Background {
                    transition: Some(t),
                    ..
                } => Some(t),
                _ => None,
            })
            .expect("the demo opens with a transition");
        node.ease = Some(ease);
        Director::new(Vm::from_start(story).unwrap())
    }

    #[test]
    fn a_transition_progress_follows_its_easing_curve() {
        use orbit_core::{Curve, Ease};
        let mut d = demo_eased(Curve::Preset(Ease::InQuad), &[]);
        d.update(0.5); // 1.0s transition: raw progress 0.5
        let tr = d.transition().unwrap();
        assert!((tr.progress - 0.25).abs() < 1e-5, "InQuad(0.5) = 0.25");
        // A named curve resolves through the story; an unknown one is linear.
        let mut d = demo_eased(Curve::Named("late".into()), &[("late", Curve::Preset(Ease::InQuad))]);
        d.update(0.5);
        assert!((d.transition().unwrap().progress - 0.25).abs() < 1e-5);
        let mut d = demo_eased(Curve::Named("missing".into()), &[]);
        d.update(0.5);
        assert!((d.transition().unwrap().progress - 0.5).abs() < 1e-5);
        // Both ends are exact whatever the curve: skipping lands on the end.
        let d = demo_eased(Curve::Preset(Ease::OutBack), &[]);
        assert_eq!(d.transition().unwrap().progress, 0.0);
    }

    #[test]
    fn transition_progresses_then_the_story_resumes() {
        let mut d = demo();
        d.update(0.5);
        let tr = d.transition().unwrap();
        assert!((tr.progress - 0.5).abs() < 1e-5);
        assert!(matches!(d.flow(), Flow::Transition { .. }));
        d.update(0.6);
        assert!(d.transition().is_none());
        match d.flow() {
            Flow::Say(line) => {
                assert!(line.speaker.is_none()); // narrator
                assert!(line.text.contains("Neo-Kyoto"));
            }
            other => panic!("expected the first line, got {other:?}"),
        }
        // Music was requested by the story (fade-in 2s, looping).
        let audio = d.take_audio();
        assert!(
            audio
                .iter()
                .any(|a| matches!(a, AudioCommand::PlayMusic { looping: true, .. }))
        );
        assert!(d.take_audio().is_empty());
    }

    #[test]
    fn advance_skips_a_running_transition() {
        let mut d = demo();
        d.advance();
        assert!(d.transition().is_none());
        assert!(matches!(d.flow(), Flow::Say(_)));
    }

    #[test]
    fn lines_carry_speaker_name_and_color() {
        let mut d = demo();
        d.advance(); // skip the transition
        d.advance(); // first narrator line
        d.advance(); // second narrator line -> Show alice (dissolve)
        d.update(10.0);
        let Flow::Say(line) = d.flow() else {
            panic!("expected a line, got {:?}", d.flow());
        };
        let (name, color) = line.speaker.clone().expect("Alice speaks");
        assert_eq!(name, "Alice");
        assert_eq!(color, [232, 120, 160]);
        assert_eq!(d.stage().characters.len(), 1);
    }

    #[test]
    fn generation_changes_with_each_presented_line() {
        let mut d = demo();
        d.advance();
        let g = d.generation();
        d.advance();
        d.update(10.0);
        assert!(d.generation() > g);
    }

    #[test]
    fn ineffective_inputs_are_ignored() {
        let mut d = demo();
        d.advance();
        assert!(!d.choose(0)); // not a choice
        let before = d.flow().clone();
        d.update(0.0);
        assert_eq!(*d.flow(), before);
        d.update(f32::NAN); // garbage dt must not corrupt the state
        assert_eq!(*d.flow(), before);
    }

    #[test]
    fn demo_plays_to_the_end_with_a_choice_and_audio() {
        let mut d = demo();
        let p = play_to_end(&mut d);
        assert!(p.lines >= 6, "only {} lines", p.lines);
        assert!(p.choices >= 1, "only {} choices", p.choices);
        assert!(p.transitions >= 3);
        assert!(
            p.audio
                .iter()
                .any(|a| matches!(a, AudioCommand::PlaySound { .. }))
        );
        assert!(
            p.audio
                .iter()
                .any(|a| matches!(a, AudioCommand::StopMusic { .. }))
        );
        assert_eq!(*d.flow(), Flow::Ended);
        // Further input after the end is harmless.
        d.advance();
        d.update(1.0);
        assert!(!d.choose(0));
        assert_eq!(*d.flow(), Flow::Ended);
    }

    #[test]
    fn choice_exposes_labels_and_rejects_bad_index() {
        let mut d = demo();
        for _ in 0..1000 {
            match d.flow().clone() {
                Flow::Choice(labels) => {
                    assert_eq!(labels.len(), 2);
                    assert!(labels[0].contains("Take the job"));
                    assert!(!d.choose(5)); // out of range: unchanged
                    assert!(matches!(d.flow(), Flow::Choice(_)));
                    assert!(d.choose(1));
                    return;
                }
                Flow::Say(_) => d.advance(),
                Flow::Transition { duration, .. } => d.update(duration + 0.01),
                Flow::Waiting { remaining } => d.update(remaining + 0.01),
                other => panic!("unexpected {other:?}"),
            }
        }
        panic!("no choice reached");
    }

    // ------------------------------------------------------------ time travel

    /// Plays on until the line `text` (a part of it) is on screen.
    fn play_until_line(d: &mut Director, text: &str) {
        for _ in 0..2000 {
            match d.flow().clone() {
                Flow::Say(line) if line.text.contains(text) => return,
                Flow::Say(_) => d.advance(),
                Flow::Choice(_) => {
                    assert!(d.choose(0));
                }
                Flow::Transition { duration, .. } => d.update(duration + 0.01),
                Flow::Waiting { remaining } => d.update(remaining + 0.01),
                other => panic!("never reached `{text}`: {other:?}"),
            }
        }
        panic!("never reached `{text}`");
    }

    #[test]
    fn rollback_rebuilds_the_stage_with_cuts() {
        let mut d = demo();
        play_until_line(&mut d, "Sorry to wake you"); // Bob has just appeared
        assert_eq!(d.stage().characters.len(), 2);
        // Back to the line before Bob entered: only Alice on stage.
        assert!(d.rollback(1));
        assert_eq!(d.stage().characters.len(), 1, "Bob is gone");
        assert!(d.transition().is_none(), "a rollback never animates");
        assert!(matches!(d.flow(), Flow::Say(l) if l.already_read));
        // Forward again: Bob is back, still without a transition.
        assert!(d.roll_forward(1));
        assert_eq!(d.stage().characters.len(), 2);
        assert!(d.transition().is_none());
        // Stepping on replays the future.
        d.advance();
        assert!(matches!(d.flow(), Flow::Say(_) | Flow::Transition { .. }));
    }

    #[test]
    fn rollback_within_the_same_music_does_not_touch_the_audio() {
        let mut d = demo();
        play_until_line(&mut d, "Sorry to wake you");
        d.take_audio(); // the music started at the beginning
        assert!(d.music().is_some());
        assert!(d.rollback(1));
        assert!(d.rollback(1));
        assert!(d.roll_forward(1));
        assert!(d.take_audio().is_empty(), "same track: nothing to do");
    }

    #[test]
    fn rollback_across_a_music_change_restarts_and_stops_it() {
        let mut d = demo();
        play_until_line(&mut d, "The music fades away."); // music stopped just before
        assert!(d.music().is_none());
        d.take_audio();
        // Back before the stop: the music must come back.
        assert!(d.rollback(1));
        let audio = d.take_audio();
        assert!(
            audio
                .iter()
                .any(|a| matches!(a, AudioCommand::PlayMusic { looping: true, .. })),
            "{audio:?}"
        );
        assert!(d.music().is_some());
        // Forward over the stop: the music is cut again.
        assert!(d.roll_forward(1));
        let audio = d.take_audio();
        assert!(
            audio
                .iter()
                .any(|a| matches!(a, AudioCommand::StopMusic { .. })),
            "{audio:?}"
        );
        assert!(d.music().is_none());
    }

    #[test]
    fn rollback_to_the_start_stops_at_the_oldest_point() {
        let mut d = demo();
        play_until_line(&mut d, "Another night");
        while d.rollback(1) {}
        assert!(!d.can_rollback());
        assert!(matches!(d.flow(), Flow::Say(l) if l.text.contains("Neo-Kyoto")));
        assert!(d.can_roll_forward());
    }

    #[test]
    fn rollback_to_a_backlog_entry() {
        let mut d = demo();
        play_until_line(&mut d, "Sorry to wake you");
        let backlog = d.vm().backlog();
        let first = backlog.first().unwrap();
        let seq = first.checkpoint.expect("rollback is possible");
        assert!(d.rollback_to(seq));
        assert!(matches!(d.flow(), Flow::Say(l) if l.text.contains("Neo-Kyoto")));
        assert!(!d.rollback_to(99_999), "unknown entry: nothing happens");
    }

    #[test]
    fn rolled_back_choices_can_be_answered_differently() {
        let mut d = demo();
        for _ in 0..1000 {
            if matches!(d.flow(), Flow::Choice(_)) {
                break;
            }
            match d.flow().clone() {
                Flow::Say(_) => d.advance(),
                Flow::Transition { duration, .. } => d.update(duration + 0.01),
                Flow::Waiting { remaining } => d.update(remaining + 0.01),
                other => panic!("{other:?}"),
            }
        }
        assert!(d.choose(0)); // take the job
        play_until_line(&mut d, "Fifty credits up front");
        // Back to the choice, take the other branch.
        while !matches!(d.flow(), Flow::Choice(_)) {
            assert!(d.rollback(1));
        }
        assert!(d.choose(1)); // haggle
        play_until_line(&mut d, "You are pushing it");
    }

    #[test]
    fn resumed_shows_the_saved_screen_without_transition() {
        let mut d = demo();
        play_until_line(&mut d, "Sorry to wake you");
        let vm_state = d.vm().snapshot();
        let stage = d.stage().clone();
        let story = d.vm().story().clone();
        let mut vm = Vm::from_start(story.clone()).unwrap();
        vm.restore(vm_state).unwrap();
        let mut resumed = Director::resumed(vm, d.localizer().clone());
        assert!(matches!(resumed.flow(), Flow::Say(l) if l.text.contains("Sorry to wake")));
        assert_eq!(resumed.stage(), &stage);
        assert!(resumed.transition().is_none());
        // The music of the scene is asked for (no fade-in surprise).
        assert!(
            resumed
                .take_audio()
                .iter()
                .any(|a| matches!(a, AudioCommand::PlayMusic { .. }))
        );
        assert_eq!(resumed.progress(), 0, "loading is not progress");
    }

    #[test]
    fn set_localizer_translates_the_line_on_screen() {
        let mut d = demo();
        play_until_line(&mut d, "Another night");
        let mut loc = Localizer::from_story(d.vm().story());
        loc.add_translation_ron(include_str!("../../../examples/stories/i18n/test_minimal.fr.ron"))
            .unwrap();
        loc.set_language("fr");
        d.set_localizer(Arc::new(loc));
        let Flow::Say(line) = d.flow() else {
            panic!("a line is expected")
        };
        assert!(line.text.contains("Encore une nuit"), "{}", line.text);
        assert_eq!(line.speaker.as_ref().map(|s| s.0.as_str()), Some("Alice"));
    }

    #[test]
    fn skip_timer_cuts_transitions_but_not_lines() {
        let mut d = demo();
        assert!(matches!(d.flow(), Flow::Transition { .. }));
        d.skip_timer();
        assert!(matches!(d.flow(), Flow::Say(_)));
        d.skip_timer();
        assert!(
            matches!(d.flow(), Flow::Say(_)),
            "a line is left to the player"
        );
    }
}
