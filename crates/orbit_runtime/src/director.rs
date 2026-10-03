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
    /// Voice acting of the line (M7-06): the translation's voice for the
    /// selected language, else the one of the `Say` node.
    pub voice: Option<AssetId>,
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
    /// The story waits for the player to close the screen it opened (an
    /// `OpenScreen { wait: true }` node, UI-14): see `Director::close_screen`.
    Screen(String),
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
    /// Plays a voice line on the voice bus, replacing the one playing.
    PlayVoice {
        asset: AssetId,
    },
    /// Cuts the voice line that is playing.
    StopVoice,
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
    /// The blocking `Effect` / `Timeline` the flow is waiting for.
    anim_wait: Option<orbit_core::NodeId>,
    audio: Vec<AudioCommand>,
    /// A voice line was started and the host did not report its end yet
    /// ([`Director::voice_finished`]): auto mode waits for it.
    voice_active: bool,
    /// Achievements unlocked since the last [`Director::take_unlocks`].
    unlocked: Vec<orbit_core::AchievementId>,
    /// The music the host was last asked to play.
    music: Option<MusicState>,
    generation: u64,
    progress: u64,
    /// Reduced motion (M7-07): finite animations jump to their final state,
    /// visual transitions are cuts.
    reduced_motion: bool,
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
            anim_wait: None,
            audio: Vec::new(),
            voice_active: false,
            unlocked: Vec::new(),
            music: None,
            generation: 0,
            progress: 0,
            reduced_motion: false,
        }
    }

    /// Turns reduced motion on or off (accessibility preference). Turning it
    /// on cuts what is playing: transitions end, finite animations reach
    /// their exact final state and a story waiting for one resumes.
    pub fn set_reduced_motion(&mut self, on: bool) {
        if self.reduced_motion == on {
            return;
        }
        self.reduced_motion = on;
        if on {
            self.pending = None;
            self.vm.complete_anims();
            if matches!(self.flow, Flow::Transition { .. }) {
                self.finish_anim_wait();
                self.run();
            }
        }
    }

    pub fn flow(&self) -> &Flow {
        &self.flow
    }

    pub fn stage(&self) -> &StageState {
        &self.stage
    }

    /// What the running animations add on top of the staging. With reduced
    /// motion on, the particles are left out (the other animations already
    /// jump to their final state).
    pub fn stage_fx(&self) -> orbit_core::StageFx {
        let mut fx = self.vm.stage_fx();
        if self.reduced_motion {
            fx.emitters.clear();
        }
        fx
    }

    /// Whether the reduced-motion preference is on.
    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion
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

    /// Achievements unlocked since the last call (M7-05), for the host's toast
    /// and persistence.
    pub fn take_unlocks(&mut self) -> Vec<orbit_core::AchievementId> {
        std::mem::take(&mut self.unlocked)
    }

    /// Whether the voice of the line on screen is still playing (as far as the
    /// host reported): auto mode waits for it to end (M7-06).
    pub fn voice_active(&self) -> bool {
        self.voice_active
    }

    /// The host reports that the voice it was asked to play ended (or could
    /// not play).
    pub fn voice_finished(&mut self) {
        self.voice_active = false;
    }

    /// The voice of the line on screen, if it has one.
    pub fn line_voice(&self) -> Option<AssetId> {
        match &self.flow {
            Flow::Say(line) => line.voice,
            _ => None,
        }
    }

    /// Plays the voice of the line on screen again (the replay button of the
    /// text box and of the history). Returns `false` for a line without
    /// voice.
    pub fn replay_voice(&mut self) -> bool {
        match self.line_voice() {
            Some(asset) => {
                self.audio.push(AudioCommand::PlayVoice { asset });
                self.voice_active = true;
                true
            }
            None => false,
        }
    }

    /// Starts the voice of the line that was just shown going forward (a
    /// rollback or a load stays silent: the player replays it by hand).
    fn start_voice(&mut self) {
        self.replay_voice();
    }

    /// Advances timers (pauses and transitions); resumes the VM when they end.
    pub fn update(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.vm.advance_clock(f64::from(dt));
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
                    self.finish_anim_wait();
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
                self.finish_anim_wait();
                self.run();
            }
            _ => {}
        }
    }

    /// Puts the blocking animation the flow was waiting for (if any) at its
    /// final state: a skip, or the end of the wait (immune to float rounding).
    fn finish_anim_wait(&mut self) {
        if let Some(node) = self.anim_wait.take() {
            self.vm.complete_anim(node);
        }
    }

    /// Cuts a timed pause or a transition short (skip mode). Does nothing
    /// during a line or a choice.
    pub fn skip_timer(&mut self) {
        if matches!(self.flow, Flow::Waiting { .. } | Flow::Transition { .. }) {
            self.pending = None;
            self.finish_anim_wait();
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

    /// Jumps to the start of a scene (id, or name) from outside the story
    /// (`UiAction::SceneGoto`): any pause or transition in progress is
    /// dropped and the scene runs to its first line or choice. Returns
    /// `false` (nothing changed) for an unknown scene.
    pub fn goto_scene(&mut self, scene: &str) -> bool {
        let Some(id) = self.vm.find_scene(scene) else {
            log::warn!("scene jump ignored: no scene `{scene}`");
            return false;
        };
        if let Err(err) = self.vm.goto_scene(id) {
            log::warn!("scene jump ignored: {err}");
            return false;
        }
        self.pending = None;
        self.anim_wait = None;
        self.run();
        true
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
        self.anim_wait = None;
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
                node,
                speaker,
                string_id,
            } => Flow::Say(Line {
                speaker: speaker.and_then(|id| self.speaker(id)),
                text: self.text(*string_id),
                already_read,
                voice: self.loc.voice(*string_id).or_else(|| self.vm.say_voice(*node)),
            }),
            Interaction::Choice { options, .. } => {
                Flow::Choice(options.iter().map(|o| self.text(o.string_id)).collect())
            }
            Interaction::Screen { screen, .. } => Flow::Screen(screen.clone()),
        }
    }

    /// The player closed `screen`, a screen the story opened, with the value
    /// of its `CloseWith` action (`None`: a plain close or Escape). When the
    /// story was waiting for it, it runs on. Returns `false` (nothing
    /// changed) for a screen the story did not open.
    pub fn close_screen(&mut self, screen: &str, result: Option<orbit_core::Value>) -> bool {
        match self.vm.close_screen(screen, result) {
            Ok(waited) => {
                for warning in self.vm.take_warnings() {
                    log::warn!("{warning}");
                }
                if waited && matches!(self.flow, Flow::Screen(_)) {
                    self.run();
                }
                true
            }
            Err(err) => {
                log::warn!("{err}");
                false
            }
        }
    }

    /// Shows a line or a choice. `already_read` is `true` for a line shown
    /// again (rollback, load).
    fn present(&mut self, interaction: Interaction, already_read: bool) {
        if self.voice_active {
            // A new line, choice or screen cuts the voice of the previous one.
            self.audio.push(AudioCommand::StopVoice);
            self.voice_active = false;
        }
        self.pending = None; // a transition not waited for is a cut
        self.anim_wait = None;
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
                PresentationEvent::AchievementUnlocked { achievement } => {
                    self.unlocked.push(*achievement);
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
            if self.reduced_motion {
                // No transition, every finite animation already at its end
                // (endless decorations are left alone).
                self.pending = None;
                self.vm.complete_anims();
            }
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
                    self.start_voice();
                    return;
                }
                StepResult::Choice { node, options } => {
                    self.progress += 1;
                    self.present(Interaction::Choice { node, options }, false);
                    return;
                }
                StepResult::Screen { node, screen } => {
                    self.progress += 1;
                    self.present(Interaction::Screen { node, screen }, false);
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
                StepResult::WaitAnimation { node, duration } => {
                    self.pending = None;
                    if duration.is_finite() && duration > 0.0 && !self.reduced_motion {
                        self.anim_wait = Some(node);
                        self.flow = Flow::Transition {
                            elapsed: 0.0,
                            duration,
                        };
                        return;
                    }
                    self.vm.complete_anim(node);
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
                Flow::Screen(id) => assert!(d.close_screen(&id, None)),
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

    // -------------------------------------------------- effects and timelines

    /// "a", a blocking 2 s move of the background by 0.4, "b", a non-blocking
    /// 4 s move by 1.0, "c", "d".
    fn animated() -> Director {
        use orbit_core::{AnimTarget, Effect, Node, NodeId, NodeKind, Scene, SceneId};
        let mut story = Story::new("fx");
        let mut nodes = Vec::new();
        fn say(story: &mut Story, nodes: &mut Vec<Node>, t: &str) {
            let text_id = story.strings.add(t);
            nodes.push(Node {
                id: NodeId::new(),
                kind: NodeKind::Say { speaker: None, text_id, voice: None },
            });
        }
        say(&mut story, &mut nodes, "a");
        nodes.push(Node {
            id: NodeId::new(),
            kind: NodeKind::Effect(Effect::Move {
                target: AnimTarget::Background,
                dx: 0.4,
                dy: 0.0,
                duration: 2.0,
                ease: None,
                wait: true,
            }),
        });
        say(&mut story, &mut nodes, "b");
        nodes.push(Node {
            id: NodeId::new(),
            kind: NodeKind::Effect(Effect::Move {
                target: AnimTarget::Camera,
                dx: 1.0,
                dy: 0.0,
                duration: 4.0,
                ease: None,
                wait: false,
            }),
        });
        say(&mut story, &mut nodes, "c");
        say(&mut story, &mut nodes, "d");
        // The camera "move" is invalid (validation says so) but harmless: use a
        // background move for the non-blocking one instead.
        if let NodeKind::Effect(Effect::Move { target, .. }) = &mut nodes[3].kind {
            *target = AnimTarget::Background;
        }
        story.scenes.push(Scene { id: SceneId::new(), name: "s".into(), nodes });
        Director::new(Vm::from_start(story).unwrap())
    }

    /// A rain emitter, then "a", "b".
    fn raining() -> Director {
        use orbit_core::{Emitter, Node, NodeId, NodeKind, ParticlePreset, Scene, SceneId};
        let mut story = Story::new("rain");
        let mut nodes = vec![Node {
            id: NodeId::new(),
            kind: NodeKind::Particles(Emitter::preset(ParticlePreset::Rain)),
        }];
        for t in ["a", "b"] {
            let text_id = story.strings.add(t);
            nodes.push(Node { id: NodeId::new(), kind: NodeKind::Say { speaker: None, text_id, voice: None } });
        }
        story.scenes.push(Scene { id: SceneId::new(), name: "s".into(), nodes });
        Director::new(Vm::from_start(story).unwrap())
    }

    #[test]
    fn reduced_motion_draws_no_particles_and_turning_it_off_brings_them_back() {
        let mut d = raining();
        assert_eq!(d.stage_fx().emitters.len(), 1);
        d.set_reduced_motion(true);
        assert!(d.stage_fx().emitters.is_empty());
        assert!(d.reduced_motion());
        // The emitter is still running underneath: nothing was lost.
        assert_eq!(d.vm().stage_fx().emitters.len(), 1);
        d.set_reduced_motion(false);
        assert_eq!(d.stage_fx().emitters.len(), 1);
    }

    #[test]
    fn particles_keep_moving_while_a_line_waits_for_the_click() {
        let mut d = raining();
        assert!(matches!(d.flow(), Flow::Say(_)), "a line is displayed");
        assert!(d.vm().has_running_anims(), "the host must keep drawing");
        let pos = |d: &Director| -> Vec<[f32; 2]> {
            let mut out = Vec::new();
            for e in &d.stage_fx().emitters {
                e.sample(&mut out);
            }
            out.iter().map(|p| p.pos).collect()
        };
        d.update(0.5);
        let a = pos(&d);
        d.update(0.5); // real time passes, no click
        let b = pos(&d);
        assert!(!a.is_empty());
        assert_ne!(a, b, "particles advance without a click");
        assert!(matches!(d.flow(), Flow::Say(_)));
    }

    #[test]
    fn a_non_blocking_effect_keeps_running_under_a_waiting_line() {
        let mut d = animated();
        d.advance();
        d.update(2.1); // the blocking move ends, "b"
        d.advance(); // the 4 s move starts, "c" is shown
        assert!(matches!(d.flow(), Flow::Say(_)));
        assert!(d.vm().has_running_anims());
        let a = bg_x(&d);
        d.update(1.0);
        let b = bg_x(&d);
        assert!(b > a, "the move progresses with the line displayed");
        d.update(10.0);
        assert!(!d.vm().has_running_anims(), "a finished move needs no frames");
    }

    #[test]
    fn a_looping_timeline_keeps_running_under_a_waiting_line() {
        use orbit_core::{
            AnimProp, AnimTarget, Key, Loop, Node, NodeId, NodeKind, Scene, SceneId,
            StoryTrack, Track, TrackData, Vec2,
        };
        let mut story = Story::new("loop");
        let text_id = story.strings.add("a");
        let nodes = vec![
            Node {
                id: NodeId::new(),
                kind: NodeKind::Timeline {
                    duration: 2.0,
                    looping: Loop::Forever,
                    wait: false,
                    tracks: vec![StoryTrack {
                        target: AnimTarget::Background,
                        prop: AnimProp::Offset,
                        data: TrackData::Vec2(Track::new(vec![
                            Key::new(0.0, Vec2::new(0.0, 0.0)),
                            Key::new(2.0, Vec2::new(1.0, 0.0)),
                        ])),
                    }],
                },
            },
            Node { id: NodeId::new(), kind: NodeKind::Say { speaker: None, text_id, voice: None } },
        ];
        story.scenes.push(Scene { id: SceneId::new(), name: "s".into(), nodes });
        let mut d = Director::new(Vm::from_start(story).unwrap());
        assert!(matches!(d.flow(), Flow::Say(_)));
        d.update(0.5);
        let a = bg_x(&d);
        d.update(0.5);
        assert_ne!(a, bg_x(&d));
        d.update(100.0);
        assert!(d.vm().has_running_anims(), "a loop never ends");
    }

    fn bg_x(d: &Director) -> f32 {
        d.vm().stage_fx().background.offset[0]
    }

    #[test]
    fn a_blocking_effect_waits_then_continues_and_a_click_skips_it() {
        let mut d = animated();
        assert!(matches!(d.flow(), Flow::Say(_)));
        d.advance();
        // The story waits for the move; it is not a stage transition.
        assert!(matches!(d.flow(), Flow::Transition { duration, .. } if *duration == 2.0));
        assert!(d.transition().is_none());
        d.update(1.0);
        assert!((bg_x(&d) - 0.2).abs() < 1e-6);
        d.update(1.5);
        assert!(matches!(d.flow(), Flow::Say(_)), "{:?}", d.flow());
        assert_eq!(bg_x(&d), 0.4, "the end of the wait lands exactly on the final value");

        // A click during the wait skips to the same exact state.
        let mut d = animated();
        d.advance();
        d.update(0.3);
        d.advance();
        assert!(matches!(d.flow(), Flow::Say(_)));
        assert_eq!(bg_x(&d), 0.4);
    }

    #[test]
    fn rollback_mid_animation_restores_the_phase_and_keeps_running() {
        let mut d = animated();
        d.advance();
        d.skip_timer(); // blocking move done, "b" shown
        d.advance(); // non-blocking 4 s move starts, "c" shown
        d.update(1.0);
        assert!((bg_x(&d) - 0.4 - 0.25).abs() < 1e-6);
        d.advance(); // "d"
        d.update(2.0);
        assert!((bg_x(&d) - 0.4 - 0.75).abs() < 1e-6);
        // Back to "c": the move is one second old again... no: as it was when
        // "c" appeared (0 s).
        assert!(d.rollback(1));
        assert!((bg_x(&d) - 0.4).abs() < 1e-6);
        d.update(2.0);
        assert!((bg_x(&d) - 0.4 - 0.5).abs() < 1e-6);
        // Back before the move started: nothing of it is left.
        assert!(d.rollback(1));
        assert!((bg_x(&d) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn reduced_motion_cuts_animations_and_lands_on_the_exact_final_state() {
        let mut d = animated();
        d.set_reduced_motion(true);
        d.advance();
        // The blocking move did not make the story wait.
        assert!(matches!(d.flow(), Flow::Say(_)), "{:?}", d.flow());
        assert_eq!(bg_x(&d), 0.4);
        d.advance();
        // The non-blocking 4 s move is finished as soon as it starts.
        assert!(matches!(d.flow(), Flow::Say(_)));
        assert_eq!(bg_x(&d), 1.4);
        d.update(0.5);
        assert_eq!(bg_x(&d), 1.4);
        // The final state is the same as the one reached by playing it out.
        let mut slow = animated();
        slow.advance();
        slow.update(2.5);
        slow.advance();
        slow.update(10.0);
        assert_eq!(bg_x(&slow), bg_x(&d));
    }

    #[test]
    fn turning_reduced_motion_on_cuts_a_wait_and_a_transition_in_progress() {
        let mut d = animated();
        d.advance();
        d.update(0.3);
        assert!(matches!(d.flow(), Flow::Transition { .. }));
        d.set_reduced_motion(true);
        assert!(matches!(d.flow(), Flow::Say(_)));
        assert_eq!(bg_x(&d), 0.4);

        let mut d = demo();
        assert!(matches!(d.flow(), Flow::Transition { .. }));
        d.set_reduced_motion(true);
        assert!(d.transition().is_none());
        assert!(!matches!(d.flow(), Flow::Transition { .. }));
        // Off again: later transitions play as usual.
        d.set_reduced_motion(false);
        let mut played = 0;
        for _ in 0..40 {
            match d.flow() {
                Flow::Transition { duration, .. } => {
                    played += 1;
                    d.update(duration + 0.01);
                }
                Flow::Say(_) => d.advance(),
                Flow::Choice(_) => {
                    d.choose(0);
                }
                _ => break,
            }
        }
        assert!(played >= 1);
    }

    #[test]
    fn a_paused_game_does_not_advance_the_clock() {
        let mut d = animated();
        d.advance();
        d.update(0.5);
        let before = d.vm().scene_clock();
        // The host stops calling `update` while paused: nothing moves.
        assert_eq!(d.vm().scene_clock(), before);
        d.update(f32::NAN);
        d.update(-1.0);
        assert_eq!(d.vm().scene_clock(), before);
    }
}
