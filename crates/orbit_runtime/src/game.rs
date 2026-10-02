//! A playthrough: the director plus the in-game widgets that reflect it, and
//! the skip / auto modes. No GPU: everything here is unit tested.

use std::sync::Arc;

use orbit_core::Story;
use orbit_i18n::Localizer;
use orbit_ui::{Advance, ChoiceMenu, DialogueBox, MenuInput, TextBoxTheme, Theme};

use crate::director::{Director, Flow};

/// Seconds between two lines while skipping (about 14 lines per second).
const SKIP_INTERVAL: f32 = 0.07;
/// Auto mode waits this much longer per character of the line, on top of the
/// preference delay.
const AUTO_SECONDS_PER_CHAR: f32 = 0.03;

/// Coarse state of the flow, to branch on without holding a borrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowKind {
    Say,
    Choice,
    Transition,
    Waiting,
    Ended,
    Failed,
}

impl FlowKind {
    pub(crate) fn of(flow: &Flow) -> Self {
        match flow {
            Flow::Say(_) => Self::Say,
            Flow::Choice(_) => Self::Choice,
            Flow::Transition { .. } => Self::Transition,
            Flow::Waiting { .. } => Self::Waiting,
            Flow::Ended => Self::Ended,
            Flow::Failed(_) => Self::Failed,
        }
    }
}

/// What an input did to the playthrough.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputOutcome {
    Nothing,
    /// The player confirmed on the end screen: back to the main menu.
    Finished,
}

/// The player's reading preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlaySettings {
    /// Typewriter speed in characters per second; `0` shows lines at once.
    pub text_speed: f32,
    /// Base delay of auto mode, in seconds.
    pub auto_delay: f32,
    /// Skip everything (`true`) or only text already read.
    pub skip_all: bool,
}

impl Default for PlaySettings {
    fn default() -> Self {
        Self {
            text_speed: 40.0,
            auto_delay: 2.0,
            skip_all: false,
        }
    }
}

fn textbox_theme(base: &TextBoxTheme, settings: &PlaySettings) -> TextBoxTheme {
    TextBoxTheme {
        chars_per_second: if settings.text_speed > 0.0 {
            settings.text_speed
        } else {
            f32::INFINITY
        },
        ..base.clone()
    }
}

pub(crate) struct Game {
    pub story: Arc<Story>,
    pub director: Director,
    pub dialogue: DialogueBox,
    pub menu: Option<ChoiceMenu>,
    settings: PlaySettings,
    seen_generation: u64,
    seen_progress: u64,
    /// The Skip button is on.
    skip_toggled: bool,
    /// The temporary skip key (Ctrl) is down.
    skip_held: bool,
    auto: bool,
    skip_timer: f32,
    auto_timer: f32,
}

impl Game {
    pub fn new(
        story: Arc<Story>,
        director: Director,
        theme: &Theme,
        settings: PlaySettings,
    ) -> Self {
        let mut game = Self {
            story,
            director,
            dialogue: DialogueBox::new(textbox_theme(&theme.textbox, &settings)),
            menu: None,
            settings,
            seen_generation: u64::MAX,
            seen_progress: 0,
            skip_toggled: false,
            skip_held: false,
            auto: false,
            skip_timer: 0.0,
            auto_timer: 0.0,
        };
        game.sync(theme);
        game
    }

    pub fn kind(&self) -> FlowKind {
        FlowKind::of(self.director.flow())
    }

    // ------------------------------------------------------------- settings

    pub fn set_settings(&mut self, settings: PlaySettings, theme: &Theme) {
        self.settings = settings;
        self.dialogue
            .set_theme(textbox_theme(&theme.textbox, &settings));
    }

    /// Switches the language: the line or choice on screen is translated and
    /// laid out again (shown whole, the typewriter does not restart).
    pub fn set_localizer(&mut self, loc: Arc<Localizer>, theme: &Theme) {
        self.director.set_localizer(loc);
        self.seen_generation = u64::MAX;
        self.seen_progress = self.director.progress();
        self.sync(theme);
    }

    // ------------------------------------------------------------ skip / auto

    /// Skip is active (button or Ctrl).
    pub fn skipping(&self) -> bool {
        self.skip_toggled || self.skip_held
    }

    #[cfg(test)]
    pub fn skip_button_on(&self) -> bool {
        self.skip_toggled
    }

    pub fn auto_on(&self) -> bool {
        self.auto
    }

    /// Whether anything moves without input: a typewriter, a timed pause, a
    /// transition, skip or auto mode.
    pub fn is_animating(&self) -> bool {
        let by_flow = match self.kind() {
            FlowKind::Waiting | FlowKind::Transition => true,
            FlowKind::Say => !self.dialogue.is_complete() || self.skipping() || self.auto,
            FlowKind::Choice | FlowKind::Ended | FlowKind::Failed => false,
        };
        by_flow || self.stage_is_animated()
    }

    /// Whether a character on stage plays a frame animation.
    pub fn stage_is_animated(&self) -> bool {
        self.director.stage().characters.iter().any(|c| {
            self.story
                .character(c.character)
                .and_then(|ch| ch.expression(&c.expression))
                .and_then(|e| e.animation())
                .is_some_and(orbit_core::Animation::is_animated)
        })
    }

    /// Sets the Skip button state (it still stops by itself at a choice).
    pub fn set_skip(&mut self, on: bool) {
        if self.skip_toggled != on {
            self.toggle_skip();
        }
    }

    pub fn toggle_skip(&mut self) {
        self.skip_toggled = !self.skip_toggled;
        self.auto = false;
        self.skip_timer = 0.0;
    }

    pub fn toggle_auto(&mut self) {
        self.auto = !self.auto;
        self.skip_toggled = false;
        self.auto_timer = 0.0;
    }

    pub fn set_skip_held(&mut self, held: bool) {
        self.skip_held = held;
        self.skip_timer = 0.0;
    }

    /// Any relevant user action (advance, pick, rewind...) ends skip and auto
    /// (the held key keeps skipping while it is down).
    fn interrupt(&mut self) {
        self.skip_toggled = false;
        self.auto = false;
    }

    /// Can the current line be skipped under the preference?
    fn line_skippable(&self) -> bool {
        match self.director.flow() {
            Flow::Say(line) => self.settings.skip_all || line.already_read,
            _ => false,
        }
    }

    fn auto_wait(&self) -> f32 {
        let chars = match self.director.flow() {
            Flow::Say(line) => line.text.chars().count() as f32,
            _ => 0.0,
        };
        self.settings.auto_delay + AUTO_SECONDS_PER_CHAR * chars
    }

    // --------------------------------------------------------------- updating

    pub fn update(&mut self, dt: f32, theme: &Theme) {
        self.director.update(dt);
        match self.kind() {
            FlowKind::Say => {
                self.dialogue.update(dt);
                self.update_say(dt);
            }
            FlowKind::Transition | FlowKind::Waiting => {
                if self.skipping() {
                    self.director.skip_timer();
                }
            }
            FlowKind::Choice | FlowKind::Ended | FlowKind::Failed => {}
        }
        // Skip stops at every choice and at the end; auto only at the end
        // (it waits on a choice, it never decides).
        match self.kind() {
            FlowKind::Choice => self.skip_toggled = false,
            FlowKind::Ended | FlowKind::Failed => {
                self.skip_toggled = false;
                self.auto = false;
            }
            _ => {}
        }
        self.sync(theme);
    }

    fn update_say(&mut self, dt: f32) {
        if self.skipping() {
            if !self.line_skippable() {
                // Unread line (or "read only" mode): stop the toggled skip,
                // keep waiting for the key to be released.
                self.skip_toggled = false;
                return;
            }
            self.skip_timer += dt;
            if self.skip_timer >= SKIP_INTERVAL {
                self.skip_timer = 0.0;
                self.director.advance();
            }
            return;
        }
        if self.auto && self.dialogue.is_complete() {
            self.auto_timer += dt;
            if self.auto_timer >= self.auto_wait() {
                self.auto_timer = 0.0;
                self.director.advance();
            }
        }
    }

    /// Rebuilds the widgets when the director presented something new.
    fn sync(&mut self, theme: &Theme) {
        if self.director.generation() != self.seen_generation {
            self.seen_generation = self.director.generation();
            // Going forward the line is typed out; after a rollback, a load or
            // a language change it is shown whole.
            let forward = self.director.progress() != self.seen_progress;
            self.seen_progress = self.director.progress();
            self.auto_timer = 0.0;
            self.skip_timer = 0.0;
            match self.director.flow() {
                Flow::Say(line) => {
                    self.dialogue.set_line(
                        line.speaker.as_ref().map(|(n, c)| (n.as_str(), *c)),
                        &line.text,
                    );
                    if !forward {
                        self.dialogue.advance();
                    }
                    self.menu = None;
                }
                Flow::Choice(labels) => {
                    self.dialogue.clear();
                    self.menu = Some(ChoiceMenu::new(theme.choice.clone(), labels));
                }
                _ => {}
            }
        }
        if self.kind() != FlowKind::Choice {
            self.menu = None;
        }
        if matches!(self.kind(), FlowKind::Ended | FlowKind::Failed) {
            self.dialogue.clear();
        }
    }

    // ------------------------------------------------------------------ input

    /// Advance / confirm (Enter, Space, or a click that is not on a button).
    pub fn confirm(&mut self, theme: &Theme) -> InputOutcome {
        self.interrupt();
        let mut outcome = InputOutcome::Nothing;
        match self.kind() {
            FlowKind::Say => {
                // First press completes the typewriter, the next one goes on.
                if self.dialogue.advance() == Advance::Next {
                    self.director.advance();
                }
            }
            FlowKind::Choice => {
                if let Some(index) = self
                    .menu
                    .as_mut()
                    .and_then(|m| m.handle(MenuInput::Confirm))
                {
                    self.director.choose(index);
                }
            }
            // Clicks skip the transition instead of being ignored.
            FlowKind::Transition => self.director.advance(),
            FlowKind::Ended => outcome = InputOutcome::Finished,
            FlowKind::Waiting | FlowKind::Failed => {}
        }
        self.sync(theme);
        outcome
    }

    /// Primary click on the game view (the quick menu already declined it).
    pub fn click(&mut self, point: [f32; 2], theme: &Theme) -> InputOutcome {
        if self.kind() == FlowKind::Choice {
            self.interrupt();
            if let Some(index) = self
                .menu
                .as_mut()
                .and_then(|m| m.handle(MenuInput::Click(point)))
            {
                self.director.choose(index);
            }
            self.sync(theme);
            return InputOutcome::Nothing;
        }
        self.confirm(theme)
    }

    /// Picks option `index` of the choice on screen (the preview's MCP tools).
    /// `false`, and nothing changes, when no choice is shown or the index is
    /// out of range.
    pub fn choose_index(&mut self, index: usize, theme: &Theme) -> bool {
        self.interrupt();
        let done = self.kind() == FlowKind::Choice && self.director.choose(index);
        self.sync(theme);
        done
    }

    /// Pointer moved (selects a hovered choice).
    pub fn hover(&mut self, point: [f32; 2]) {
        if let Some(menu) = &mut self.menu {
            menu.handle(MenuInput::Hover(point));
        }
    }

    /// Up / Down / digit on a choice menu.
    pub fn menu_input(&mut self, input: MenuInput, theme: &Theme) {
        if self.kind() != FlowKind::Choice {
            return;
        }
        if matches!(input, MenuInput::Number(_)) {
            self.interrupt();
        }
        if let Some(index) = self.menu.as_mut().and_then(|m| m.handle(input)) {
            self.interrupt();
            self.director.choose(index);
        }
        self.sync(theme);
    }

    /// Jumps to a scene (`UiAction::SceneGoto`). Like a choice, it stops
    /// skip and auto. Returns whether the jump happened.
    pub fn goto_scene(&mut self, scene: &str, theme: &Theme) -> bool {
        self.interrupt();
        let moved = self.director.goto_scene(scene);
        self.sync(theme);
        moved
    }

    /// One step back in time. Returns whether anything changed.
    pub fn rollback(&mut self, theme: &Theme) -> bool {
        self.interrupt();
        let moved = self.director.rollback(1);
        self.sync(theme);
        moved
    }

    pub fn roll_forward(&mut self, theme: &Theme) -> bool {
        self.interrupt();
        let moved = self.director.roll_forward(1);
        self.sync(theme);
        moved
    }

    pub fn rollback_to(&mut self, seq: u64, theme: &Theme) -> bool {
        self.interrupt();
        let moved = self.director.rollback_to(seq);
        self.sync(theme);
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_vm::Vm;

    const DEMO: &str = include_str!("../../../examples/stories/test_minimal.ron");

    pub(crate) fn story() -> Arc<Story> {
        Arc::new(Story::from_ron(DEMO).unwrap())
    }

    fn game_with(settings: PlaySettings) -> (Game, Theme) {
        let theme = Theme::default();
        let story = story();
        let vm = Vm::from_start(story.clone()).unwrap();
        (Game::new(story, Director::new(vm), &theme, settings), theme)
    }

    fn game() -> (Game, Theme) {
        game_with(PlaySettings::default())
    }

    #[test]
    fn click_skips_the_opening_transition_then_shows_the_first_line() {
        let (mut g, theme) = game();
        assert_eq!(g.kind(), FlowKind::Transition);
        g.click([10.0, 10.0], &theme);
        assert_eq!(g.kind(), FlowKind::Say);
        assert!(g.dialogue.has_line());
        assert!(!g.dialogue.is_complete()); // typewriter just started
    }

    #[test]
    fn first_press_completes_the_line_second_goes_next() {
        let (mut g, theme) = game();
        g.confirm(&theme); // skip transition
        let before = g.director.generation();
        g.confirm(&theme);
        assert!(g.dialogue.is_complete());
        assert_eq!(g.director.generation(), before, "still the same line");
        g.confirm(&theme);
        // Next node is a dissolve-in of Alice: a transition, not a new line yet.
        assert!(matches!(g.kind(), FlowKind::Transition | FlowKind::Say));
    }

    /// Plays until a choice is shown.
    fn play_to_choice(g: &mut Game, theme: &Theme) {
        for _ in 0..500 {
            match g.kind() {
                FlowKind::Choice => return,
                FlowKind::Waiting => g.update(1.0, theme),
                _ => {
                    g.confirm(theme);
                    if g.kind() != FlowKind::Choice {
                        g.confirm(theme);
                    }
                }
            }
        }
        panic!("no choice reached");
    }

    #[test]
    fn menu_appears_for_choices_and_digits_pick_options() {
        let (mut g, theme) = game();
        play_to_choice(&mut g, &theme);
        let menu = g.menu.as_ref().expect("menu is shown");
        assert_eq!(menu.len(), 2);
        assert!(!g.dialogue.has_line());
        // Digit 9 is out of range: still choosing. Digit 2 picks "Haggle".
        g.menu_input(MenuInput::Number(9), &theme);
        assert_eq!(g.kind(), FlowKind::Choice);
        g.menu_input(MenuInput::Number(2), &theme);
        assert_ne!(g.kind(), FlowKind::Choice);
        assert!(g.menu.is_none());
    }

    #[test]
    fn confirming_the_end_asks_to_leave() {
        let (mut g, theme) = game();
        let mut outcome = InputOutcome::Nothing;
        for _ in 0..3000 {
            match g.kind() {
                FlowKind::Ended => {
                    outcome = g.confirm(&theme);
                    break;
                }
                FlowKind::Choice => g.menu_input(MenuInput::Number(1), &theme),
                FlowKind::Waiting => g.update(1.0, &theme),
                FlowKind::Failed => panic!("story failed"),
                _ => {
                    g.confirm(&theme);
                }
            }
        }
        assert_eq!(outcome, InputOutcome::Finished);
    }

    #[test]
    fn hover_and_other_inputs_never_advance_a_line() {
        let (mut g, theme) = game();
        g.confirm(&theme);
        let generation = g.director.generation();
        g.hover([5.0, 5.0]);
        g.menu_input(MenuInput::Up, &theme);
        g.menu_input(MenuInput::Number(1), &theme);
        assert_eq!(g.director.generation(), generation);
        assert!(!g.dialogue.is_complete());
    }

    // ------------------------------------------------------------ skip / auto

    /// Advances to the first line and reads it, then goes one more.
    fn read_first_lines(g: &mut Game, theme: &Theme, lines: usize) {
        g.confirm(theme); // opening transition
        for _ in 0..lines {
            g.confirm(theme);
            g.confirm(theme);
            g.update(1.0, theme); // let a dissolve / wait end
        }
    }

    #[test]
    fn skip_all_runs_through_text_and_stops_at_the_choice() {
        let (mut g, theme) = game_with(PlaySettings {
            skip_all: true,
            ..Default::default()
        });
        g.toggle_skip();
        assert!(g.skipping());
        let mut seen_lines = 0;
        for _ in 0..4000 {
            let before = g.director.generation();
            g.update(SKIP_INTERVAL + 0.01, &theme);
            if g.director.generation() != before && g.kind() == FlowKind::Say {
                seen_lines += 1;
            }
            if g.kind() == FlowKind::Choice {
                break;
            }
        }
        assert_eq!(g.kind(), FlowKind::Choice, "skip must reach the choice");
        assert!(seen_lines >= 6, "only {seen_lines} lines skipped");
        assert!(!g.skip_button_on(), "skip is switched off at a choice");
        // Time passing on the choice changes nothing.
        g.update(5.0, &theme);
        assert_eq!(g.kind(), FlowKind::Choice);
    }

    #[test]
    fn read_only_skip_stops_at_the_first_unread_line() {
        let (mut g, theme) = game(); // skip_all = false, nothing read yet
        g.toggle_skip();
        for _ in 0..50 {
            g.update(SKIP_INTERVAL + 0.01, &theme);
        }
        // The very first line is unread: the toggle is released at once and
        // nothing advanced past it (only the opening transition was cut).
        assert_eq!(g.kind(), FlowKind::Say);
        assert!(!g.skip_button_on());
        let Flow::Say(line) = g.director.flow() else {
            unreachable!()
        };
        assert!(!line.already_read);
    }

    #[test]
    fn read_only_skip_passes_lines_read_before_and_stops_at_new_ones() {
        // First playthrough: read the first lines (marks them read in the VM).
        let theme = Theme::default();
        let story = story();
        let mut vm = Vm::from_start(story.clone()).unwrap();
        let mut director = Director::new(vm);
        // Read 6 lines by hand.
        for _ in 0..200 {
            match director.flow().clone() {
                Flow::Say(_) => {
                    if director.vm().read_nodes().len() >= 6 {
                        break;
                    }
                    director.advance();
                }
                Flow::Transition { duration, .. } => director.update(duration + 0.01),
                Flow::Waiting { remaining } => director.update(remaining + 0.01),
                _ => break,
            }
        }
        let read = director.vm().read_nodes().clone();
        assert!(read.len() >= 6);
        // Second playthrough with the same read set.
        vm = Vm::from_start(story.clone()).unwrap();
        vm.set_read_nodes(read.clone());
        let mut g = Game::new(story, Director::new(vm), &theme, PlaySettings::default());
        g.toggle_skip();
        for _ in 0..400 {
            g.update(SKIP_INTERVAL + 0.01, &theme);
            if !g.skip_button_on() {
                break;
            }
        }
        assert!(
            !g.skip_button_on(),
            "skip must stop on the first unread line"
        );
        let Flow::Say(line) = g.director.flow() else {
            panic!("expected a line, got {:?}", g.director.flow());
        };
        assert!(!line.already_read);
        // It stopped right after the read part: every earlier line was read.
        assert!(g.director.vm().read_nodes().len() > read.len());
    }

    #[test]
    fn ctrl_skips_while_held_and_pauses_on_unread_lines() {
        let (mut g, theme) = game_with(PlaySettings {
            skip_all: true,
            ..Default::default()
        });
        g.set_skip_held(true);
        assert!(g.skipping());
        for _ in 0..30 {
            g.update(SKIP_INTERVAL + 0.01, &theme);
        }
        assert!(g.director.vm().read_nodes().len() >= 3, "held Ctrl skipped");
        g.set_skip_held(false);
        let generation = g.director.generation();
        for _ in 0..30 {
            g.update(SKIP_INTERVAL + 0.01, &theme);
        }
        assert_eq!(g.director.generation(), generation, "released: stops");
    }

    #[test]
    fn auto_advances_after_the_typewriter_and_the_delay() {
        let (mut g, theme) = game_with(PlaySettings {
            auto_delay: 1.0,
            text_speed: 1000.0,
            ..Default::default()
        });
        g.confirm(&theme); // opening transition
        g.toggle_auto();
        let generation = g.director.generation();
        g.update(0.2, &theme); // typewriter completes
        assert!(g.dialogue.is_complete());
        g.update(0.5, &theme);
        assert_eq!(g.director.generation(), generation, "delay not elapsed");
        let wait = g.auto_wait();
        assert!(wait > 1.0, "delay grows with the line length");
        g.update(wait, &theme);
        assert_ne!(
            g.director.generation(),
            generation,
            "auto advanced past the line"
        );
    }

    #[test]
    fn auto_does_not_pick_choices() {
        let (mut g, theme) = game_with(PlaySettings {
            auto_delay: 0.1,
            text_speed: 10_000.0,
            ..Default::default()
        });
        g.toggle_auto();
        for _ in 0..8000 {
            g.update(0.1, &theme);
            if g.kind() == FlowKind::Choice {
                break;
            }
        }
        assert_eq!(g.kind(), FlowKind::Choice);
        for _ in 0..50 {
            g.update(1.0, &theme);
        }
        assert_eq!(g.kind(), FlowKind::Choice, "auto never decides");
    }

    #[test]
    fn user_input_cuts_skip_and_auto() {
        let (mut g, theme) = game_with(PlaySettings {
            skip_all: true,
            ..Default::default()
        });
        g.toggle_auto();
        assert!(g.auto_on());
        g.confirm(&theme);
        assert!(!g.auto_on());
        g.toggle_skip();
        assert!(g.skip_button_on());
        g.confirm(&theme);
        assert!(!g.skip_button_on());
        g.toggle_skip();
        g.rollback(&theme);
        assert!(!g.skip_button_on());
        // The two buttons are exclusive.
        g.toggle_auto();
        g.toggle_skip();
        assert!(g.skip_button_on() && !g.auto_on());
    }

    #[test]
    fn instant_text_speed_shows_the_line_whole() {
        let (mut g, theme) = game_with(PlaySettings {
            text_speed: 0.0,
            ..Default::default()
        });
        g.confirm(&theme);
        g.update(0.05, &theme);
        assert!(g.dialogue.is_complete());
    }

    // ---------------------------------------------------------------- rollback

    #[test]
    fn rollback_shows_the_previous_line_whole_and_forward_returns() {
        let (mut g, theme) = game();
        read_first_lines(&mut g, &theme, 3);
        let Flow::Say(current) = g.director.flow().clone() else {
            panic!("expected a line")
        };
        assert!(g.rollback(&theme));
        let Flow::Say(previous) = g.director.flow().clone() else {
            panic!("expected a line")
        };
        assert_ne!(previous.text, current.text);
        assert!(g.dialogue.is_complete(), "no typewriter after a rollback");
        assert!(previous.already_read);
        assert!(g.roll_forward(&theme));
        let Flow::Say(again) = g.director.flow().clone() else {
            panic!("expected a line")
        };
        assert_eq!(again.text, current.text);
    }

    #[test]
    fn rollback_at_the_very_start_does_nothing() {
        let (mut g, theme) = game();
        g.confirm(&theme);
        assert!(!g.rollback(&theme));
        assert_eq!(g.kind(), FlowKind::Say);
    }

    #[test]
    fn language_change_translates_the_current_line_in_place() {
        let (mut g, theme) = game();
        g.confirm(&theme);
        let mut loc = Localizer::from_story(&g.story);
        loc.add_translation_ron(include_str!("../../../examples/stories/i18n/test_minimal.fr.ron"))
            .unwrap();
        assert!(loc.set_language("fr"));
        g.set_localizer(Arc::new(loc), &theme);
        let Flow::Say(line) = g.director.flow() else {
            panic!("expected a line")
        };
        assert!(line.text.contains("pluie"), "{}", line.text);
        assert!(g.dialogue.is_complete());
    }
}
