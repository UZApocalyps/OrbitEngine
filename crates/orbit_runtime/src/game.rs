//! A playthrough: the director plus the in-game widgets that reflect it.

use std::sync::Arc;

use orbit_core::Story;
use orbit_ui::{Advance, ChoiceMenu, DialogueBox, MenuInput, Theme};
use orbit_vm::Vm;

use crate::director::{Director, Flow};
use crate::input::UiAction;

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
    /// The ended story was restarted from the beginning.
    Restarted,
}

pub(crate) struct Game {
    pub story: Arc<Story>,
    pub director: Director,
    pub dialogue: DialogueBox,
    pub menu: Option<ChoiceMenu>,
    seen_generation: u64,
}

impl Game {
    pub fn new(story: Arc<Story>, vm: Vm, theme: &Theme) -> Self {
        let mut game = Self {
            story,
            director: Director::new(vm),
            dialogue: DialogueBox::new(theme.textbox.clone()),
            menu: None,
            seen_generation: u64::MAX,
        };
        game.sync(theme);
        game
    }

    pub fn kind(&self) -> FlowKind {
        FlowKind::of(self.director.flow())
    }

    pub fn update(&mut self, dt: f32, theme: &Theme) {
        self.director.update(dt);
        if self.kind() == FlowKind::Say {
            self.dialogue.update(dt);
        }
        self.sync(theme);
    }

    /// Rebuilds the widgets when the director presented something new.
    fn sync(&mut self, theme: &Theme) {
        if self.director.generation() != self.seen_generation {
            self.seen_generation = self.director.generation();
            match self.director.flow() {
                Flow::Say(line) => {
                    self.dialogue.set_line(
                        line.speaker.as_ref().map(|(n, c)| (n.as_str(), *c)),
                        &line.text,
                    );
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

    /// Handles one decoded input.
    pub fn on_action(&mut self, action: UiAction, theme: &Theme) -> InputOutcome {
        let confirm = matches!(action, UiAction::Confirm | UiAction::Click(_));
        let mut outcome = InputOutcome::Nothing;
        match self.kind() {
            FlowKind::Say if confirm => {
                // First press completes the typewriter, the next one goes on.
                if self.dialogue.advance() == Advance::Next {
                    self.director.advance();
                }
            }
            FlowKind::Choice => {
                let input = match action {
                    UiAction::Confirm => MenuInput::Confirm,
                    UiAction::Click(p) => MenuInput::Click(p),
                    UiAction::Hover(p) => MenuInput::Hover(p),
                    UiAction::Up => MenuInput::Up,
                    UiAction::Down => MenuInput::Down,
                    UiAction::Number(n) => MenuInput::Number(n),
                };
                if let Some(index) = self.menu.as_mut().and_then(|m| m.handle(input)) {
                    self.director.choose(index);
                }
            }
            // Clicks skip the transition instead of being ignored.
            FlowKind::Transition if confirm => self.director.advance(),
            FlowKind::Ended if confirm => outcome = self.restart(theme),
            _ => {}
        }
        self.sync(theme);
        outcome
    }

    fn restart(&mut self, theme: &Theme) -> InputOutcome {
        match Vm::from_start(self.story.clone()) {
            Ok(vm) => {
                *self = Game::new(self.story.clone(), vm, theme);
                InputOutcome::Restarted
            }
            Err(err) => {
                log::error!("cannot restart the story: {err}");
                InputOutcome::Nothing
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = include_str!("../../../examples/stories/demo.ron");

    fn game() -> (Game, Theme) {
        let theme = Theme::default();
        let story = Arc::new(Story::from_ron(DEMO).unwrap());
        let vm = Vm::from_start(story.clone()).unwrap();
        (Game::new(story, vm, &theme), theme)
    }

    #[test]
    fn click_skips_the_opening_transition_then_shows_the_first_line() {
        let (mut g, theme) = game();
        assert_eq!(g.kind(), FlowKind::Transition);
        g.on_action(UiAction::Click([10.0, 10.0]), &theme);
        assert_eq!(g.kind(), FlowKind::Say);
        assert!(g.dialogue.has_line());
        assert!(!g.dialogue.is_complete()); // typewriter just started
    }

    #[test]
    fn first_press_completes_the_line_second_goes_next() {
        let (mut g, theme) = game();
        g.on_action(UiAction::Confirm, &theme); // skip transition
        let before = g.director.generation();
        g.on_action(UiAction::Confirm, &theme);
        assert!(g.dialogue.is_complete());
        assert_eq!(g.director.generation(), before, "still the same line");
        g.on_action(UiAction::Confirm, &theme);
        // Next node is a dissolve-in of Alice: a transition, not a new line yet.
        assert!(matches!(g.kind(), FlowKind::Transition | FlowKind::Say));
    }

    #[test]
    fn menu_appears_for_choices_and_digits_pick_options() {
        let (mut g, theme) = game();
        for _ in 0..500 {
            match g.kind() {
                FlowKind::Choice => break,
                FlowKind::Waiting => g.update(1.0, &theme),
                _ => {
                    g.on_action(UiAction::Confirm, &theme);
                    g.on_action(UiAction::Confirm, &theme);
                }
            }
        }
        assert_eq!(g.kind(), FlowKind::Choice);
        let menu = g.menu.as_ref().expect("menu is shown");
        assert_eq!(menu.len(), 2);
        assert!(!g.dialogue.has_line());
        // Digit 9 is out of range: still choosing. Digit 2 picks "Stay home".
        g.on_action(UiAction::Number(9), &theme);
        assert_eq!(g.kind(), FlowKind::Choice);
        g.on_action(UiAction::Number(2), &theme);
        assert_ne!(g.kind(), FlowKind::Choice);
        assert!(g.menu.is_none());
    }

    #[test]
    fn story_end_then_click_restarts() {
        let (mut g, theme) = game();
        let mut restarted = false;
        for _ in 0..3000 {
            match g.kind() {
                FlowKind::Ended => {
                    restarted =
                        g.on_action(UiAction::Click([1.0, 1.0]), &theme) == InputOutcome::Restarted;
                    break;
                }
                FlowKind::Choice => {
                    g.on_action(UiAction::Number(1), &theme);
                }
                FlowKind::Waiting => g.update(1.0, &theme),
                FlowKind::Failed => panic!("story failed"),
                _ => {
                    g.on_action(UiAction::Confirm, &theme);
                }
            }
        }
        assert!(restarted, "story never ended");
        assert_eq!(g.kind(), FlowKind::Transition, "back at the opening fade");
    }

    #[test]
    fn hover_and_other_inputs_never_advance_a_line() {
        let (mut g, theme) = game();
        g.on_action(UiAction::Confirm, &theme);
        let generation = g.director.generation();
        g.on_action(UiAction::Hover([5.0, 5.0]), &theme);
        g.on_action(UiAction::Up, &theme);
        g.on_action(UiAction::Number(1), &theme);
        assert_eq!(g.director.generation(), generation);
        assert!(!g.dialogue.is_complete());
    }
}
