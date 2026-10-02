//! Navigation of a screen previewed in the editor: the small stack of open
//! screens, the mock settings and saves, and the feedback shown for actions
//! that would act on the game. No GPU: the runtime owns the drawing.

use std::collections::BTreeMap;

use orbit_core::Value;

use orbit_ui_data::UiAction;
use super::screen::DataContext;
use crate::view::{
    CategoryView, LanguageView, PrefsView, SlotCategory, SlotId, SlotState, SlotView, UiEvent,
};

/// Deepest preview stack (a screen opening itself in a loop stops here).
const MAX_DEPTH: usize = 16;

/// What an event changed in the preview.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavStep {
    /// The screen on top changed: the host builds the new one.
    pub shown_changed: bool,
    /// Short texts to flash ("Action: GameNew").
    pub feedback: Vec<String>,
}

/// The screens the preview has open (top last) and the mock game state the
/// screens read.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewNav {
    stack: Vec<String>,
    /// Settings the controls edit; they survive navigation.
    pub prefs: PrefsView,
    /// Sample saves, changed by saving into / deleting a slot.
    pub saves: Vec<CategoryView>,
    /// Mock story variables (declared initial values, set by the editor);
    /// variable operations change them.
    pub vars: BTreeMap<String, Value>,
}

fn slot(category: SlotCategory, index: u32, state: SlotState) -> SlotView {
    let used = state != SlotState::Empty;
    SlotView {
        id: SlotId { category, index },
        state,
        thumbnail: None,
        timestamp: if used { format!("2026-03-{:02} 21:0{}", 10 + index, index % 10) } else { String::new() },
        preview: if used { "Sample line of the story, as the player left it.".into() } else { String::new() },
    }
}

/// Saves a preview shows: used, empty and incompatible manual slots, a few
/// autosaves and a quicksave.
pub fn sample_saves() -> Vec<CategoryView> {
    let manual = (0..8)
        .map(|i| {
            let state = match i {
                0 | 2 | 6 => SlotState::Used,
                3 => SlotState::Incompatible,
                _ => SlotState::Empty,
            };
            slot(SlotCategory::Manual, i, state)
        })
        .collect();
    vec![
        CategoryView { category: SlotCategory::Manual, slots: manual },
        CategoryView {
            category: SlotCategory::Auto,
            slots: (0..3).map(|i| slot(SlotCategory::Auto, i, SlotState::Used)).collect(),
        },
        CategoryView {
            category: SlotCategory::Quick,
            slots: vec![slot(SlotCategory::Quick, 0, SlotState::Used)],
        },
    ]
}

fn sample_prefs() -> PrefsView {
    PrefsView {
        languages: vec![
            LanguageView { code: "en".into(), name: "English".into() },
            LanguageView { code: "fr".into(), name: "Francais".into() },
        ],
        language: "en".into(),
        ..PrefsView::default()
    }
}

/// Name of the stock screen an `Open*` event stands for.
fn stock_id(event: &UiEvent) -> Option<&'static str> {
    Some(match event {
        UiEvent::OpenSave => "save",
        UiEvent::OpenLoad => "load",
        UiEvent::OpenPreferences => "preferences",
        UiEvent::OpenHistory => "history",
        UiEvent::OpenPause => "pause",
        _ => return None,
    })
}

impl PreviewNav {
    pub fn new(root: &str) -> Self {
        Self {
            stack: vec![root.to_owned()],
            prefs: sample_prefs(),
            saves: sample_saves(),
            vars: BTreeMap::new(),
        }
    }

    /// Declares the story variables (name to initial value): the mock keeps
    /// the current value of those it already has with the same type, adds
    /// new ones and drops the ones no longer declared.
    pub fn set_declared(&mut self, declared: &BTreeMap<String, Value>) {
        let mut vars = BTreeMap::new();
        for (name, initial) in declared {
            let value = self
                .vars
                .get(name)
                .filter(|v| v.value_type() == initial.value_type())
                .unwrap_or(initial);
            vars.insert(name.clone(), value.clone());
        }
        self.vars = vars;
    }

    /// Id of the screen shown.
    pub fn top(&self) -> &str {
        self.stack.last().map_or("", String::as_str)
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Starts over from `id` (the editor picked another screen).
    pub fn reset(&mut self, id: &str) {
        self.stack.clear();
        self.stack.push(id.to_owned());
    }

    /// What the shown screen reads.
    pub fn context(&self, title: &str) -> DataContext {
        DataContext {
            prefs: self.prefs.clone(),
            can_continue: true,
            can_save: true,
            show_quit: true,
            title: title.to_owned(),
            saves: self.saves.clone(),
            vars: self.vars.clone(),
            ..DataContext::default()
        }
    }

    /// Text flashed for an action that has no visible effect in the preview.
    pub fn describe_action(action: &UiAction) -> String {
        format!("Action: {action:?}")
    }

    fn describe(event: &UiEvent) -> Option<String> {
        let name = match event {
            UiEvent::StartGame => "GameNew".to_owned(),
            UiEvent::Continue => "GameContinue".to_owned(),
            UiEvent::Quit => "GameQuit".to_owned(),
            UiEvent::ReturnToMainMenu => "ReturnToTitle".to_owned(),
            UiEvent::QuickSave => "QuickSave".to_owned(),
            UiEvent::QuickLoad => "QuickLoad".to_owned(),
            UiEvent::LoadSlot(id) => format!("LoadSlot({:?} {})", id.category, id.index + 1),
            UiEvent::SaveSlot(id) => format!("SaveSlot({:?} {})", id.category, id.index + 1),
            UiEvent::DeleteSlot(id) => format!("DeleteSlot({:?} {})", id.category, id.index + 1),
            UiEvent::PlaySound(path) => format!("PlaySound({path})"),
            UiEvent::Custom { name, args } if args.is_empty() => format!("Custom({name})"),
            UiEvent::Custom { name, args } => format!("Custom({name}: {})", args.join(", ")),
            UiEvent::ResetPreferences => "ResetPreferences".to_owned(),
            UiEvent::Resume => "Resume".to_owned(),
            UiEvent::Rollback => "Rollback".to_owned(),
            UiEvent::RollbackTo(_) => "RollbackTo".to_owned(),
            UiEvent::ToggleSkip => "ToggleSkip".to_owned(),
            UiEvent::ToggleAuto => "ToggleAuto".to_owned(),
            UiEvent::SetLanguage(code) => format!("SetLanguage({code})"),
            UiEvent::SceneGoto(scene) => format!("SceneGoto({scene})"),
            _ => return None,
        };
        Some(format!("Action: {name}"))
    }

    fn open(&mut self, id: &str, exists: &dyn Fn(&str) -> bool, replace: bool, step: &mut NavStep) {
        if !exists(id) {
            step.feedback.push(format!("No screen `{id}` in the project"));
            return;
        }
        if replace {
            if self.stack.len() > 1 {
                self.stack.pop();
            } else {
                self.stack.clear();
            }
        } else if self.top() == id {
            return;
        }
        if self.stack.len() >= MAX_DEPTH {
            step.feedback.push("Too many screens open in the preview".to_owned());
            return;
        }
        self.stack.push(id.to_owned());
        step.shown_changed = true;
    }

    /// Applies what the shown screen reported. `exists` tells whether a
    /// screen can be shown (the project's, or a default one).
    pub fn apply(&mut self, event: UiEvent, exists: &dyn Fn(&str) -> bool) -> NavStep {
        let mut step = NavStep::default();
        if let Some(id) = stock_id(&event) {
            self.open(id, exists, false, &mut step);
            return step;
        }
        match &event {
            UiEvent::Sound(_) => {}
            UiEvent::OpenScreen(id) => self.open(id, exists, false, &mut step),
            UiEvent::ReplaceScreen(id) => self.open(id, exists, true, &mut step),
            UiEvent::Back => {
                if self.stack.len() > 1 {
                    self.stack.pop();
                    step.shown_changed = true;
                } else {
                    step.feedback.push("Action: Close (no screen below)".to_owned());
                }
            }
            UiEvent::CloseAll | UiEvent::Resume => {
                step.feedback.push("Action: CloseAll".to_owned());
                if self.stack.len() > 1 {
                    self.stack.truncate(1);
                    step.shown_changed = true;
                }
            }
            UiEvent::Toast(text) => step.feedback.push(text.clone()),
            UiEvent::VarOp(op) => match op.apply(&|n| self.vars.get(n).cloned()) {
                Ok(value) => {
                    step.feedback.push(format!("Variable: {} -> {value}", op.var));
                    self.vars.insert(op.var.clone(), value);
                }
                Err(e) => step.feedback.push(format!("Variable `{}` unchanged: {e}", op.var)),
            },
            UiEvent::SetVolume(bus, v) => {
                use crate::view::AudioBus::*;
                match bus {
                    Master => self.prefs.master = *v,
                    Music => self.prefs.music = *v,
                    Sfx => self.prefs.sfx = *v,
                    Voice => self.prefs.voice = *v,
                }
            }
            UiEvent::SetTextSpeed(v) => self.prefs.text_speed = *v,
            UiEvent::SetAutoDelay(v) => self.prefs.auto_delay = *v,
            UiEvent::SetSkipMode(m) => self.prefs.skip_mode = *m,
            UiEvent::SetFullscreen(on) => self.prefs.fullscreen = Some(*on),
            UiEvent::SetLanguage(code) => {
                self.prefs.language.clone_from(code);
                step.feedback.extend(Self::describe(&event));
            }
            UiEvent::ResetPreferences => {
                let languages = std::mem::take(&mut self.prefs.languages);
                self.prefs = PrefsView { languages, ..sample_prefs() };
                step.feedback.extend(Self::describe(&event));
            }
            UiEvent::SaveSlot(id) => {
                self.set_slot(*id, SlotState::Used);
                step.feedback.extend(Self::describe(&event));
            }
            UiEvent::DeleteSlot(id) => {
                self.set_slot(*id, SlotState::Empty);
                step.feedback.extend(Self::describe(&event));
            }
            other => step.feedback.extend(Self::describe(other)),
        }
        step
    }

    fn set_slot(&mut self, id: SlotId, state: SlotState) {
        for cat in &mut self.saves {
            if let Some(s) = cat.slots.iter_mut().find(|s| s.id == id) {
                *s = slot(id.category, id.index, state);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::AudioBus;

    fn all(_: &str) -> bool {
        true
    }

    fn known(id: &str) -> bool {
        matches!(id, "main" | "settings" | "save" | "load" | "pause")
    }

    #[test]
    fn open_replace_back_and_close_all_walk_the_stack() {
        let mut nav = PreviewNav::new("main");
        let step = nav.apply(UiEvent::OpenScreen("settings".into()), &known);
        assert!(step.shown_changed);
        assert_eq!((nav.top(), nav.depth()), ("settings", 2));
        // Opening what is already on top does nothing.
        assert!(!nav.apply(UiEvent::OpenScreen("settings".into()), &known).shown_changed);
        nav.apply(UiEvent::ReplaceScreen("load".into()), &known);
        assert_eq!((nav.top(), nav.depth()), ("load", 2));
        nav.apply(UiEvent::OpenScreen("pause".into()), &known);
        assert_eq!(nav.depth(), 3);
        let step = nav.apply(UiEvent::Back, &known);
        assert!(step.shown_changed);
        assert_eq!(nav.top(), "load");
        let step = nav.apply(UiEvent::CloseAll, &known);
        assert_eq!(nav.top(), "main");
        assert!(step.shown_changed);
        assert_eq!(step.feedback, vec!["Action: CloseAll"]);
        // Nothing below: a message, the screen stays.
        let step = nav.apply(UiEvent::Back, &known);
        assert!(!step.shown_changed && step.feedback.len() == 1);
        assert_eq!(nav.top(), "main");
        // Replacing the only screen replaces the root.
        nav.apply(UiEvent::ReplaceScreen("pause".into()), &known);
        assert_eq!((nav.top(), nav.depth()), ("pause", 1));
    }

    #[test]
    fn stock_open_events_map_to_their_screen_ids_and_unknown_ids_warn() {
        let mut nav = PreviewNav::new("main");
        nav.apply(UiEvent::OpenSave, &known);
        assert_eq!(nav.top(), "save");
        nav.apply(UiEvent::OpenLoad, &known);
        assert_eq!(nav.top(), "load");
        let step = nav.apply(UiEvent::OpenHistory, &known);
        assert_eq!(nav.top(), "load");
        assert!(step.feedback[0].contains("history"), "{step:?}");
        let step = nav.apply(UiEvent::OpenScreen("nope".into()), &known);
        assert!(!step.shown_changed && step.feedback[0].contains("nope"));
    }

    #[test]
    fn the_stack_is_bounded() {
        let mut nav = PreviewNav::new("a");
        let mut i = 0;
        while nav.depth() < MAX_DEPTH {
            i += 1;
            nav.apply(UiEvent::OpenScreen(format!("s{i}")), &all);
        }
        let step = nav.apply(UiEvent::OpenScreen("more".into()), &all);
        assert!(!step.shown_changed);
        assert_eq!(nav.depth(), MAX_DEPTH);
    }

    #[test]
    fn game_actions_give_feedback_and_never_navigate() {
        let mut nav = PreviewNav::new("main");
        for (ev, text) in [
            (UiEvent::StartGame, "Action: GameNew"),
            (UiEvent::Continue, "Action: GameContinue"),
            (UiEvent::Quit, "Action: GameQuit"),
            (UiEvent::ReturnToMainMenu, "Action: ReturnToTitle"),
            (UiEvent::QuickSave, "Action: QuickSave"),
            (UiEvent::QuickLoad, "Action: QuickLoad"),
        ] {
            let step = nav.apply(ev, &known);
            assert!(!step.shown_changed);
            assert_eq!(step.feedback, vec![text.to_owned()]);
        }
        let step = nav.apply(
            UiEvent::Custom { name: "hi".into(), args: vec!["a".into(), "b".into()] },
            &known,
        );
        assert_eq!(step.feedback, vec!["Action: Custom(hi: a, b)"]);
        assert_eq!(
            PreviewNav::describe_action(&UiAction::SceneGoto("x".into())),
            "Action: SceneGoto(\"x\")"
        );
        assert_eq!(nav.depth(), 1);
    }

    #[test]
    fn settings_and_saves_are_mocked_and_survive_navigation() {
        let mut nav = PreviewNav::new("main");
        assert!(nav.apply(UiEvent::SetVolume(AudioBus::Music, 0.25), &known).feedback.is_empty());
        nav.apply(UiEvent::SetFullscreen(true), &known);
        nav.apply(UiEvent::OpenScreen("settings".into()), &known);
        let ctx = nav.context("T");
        assert_eq!(ctx.prefs.music, 0.25);
        assert_eq!(ctx.prefs.fullscreen, Some(true));
        assert_eq!(ctx.prefs.languages.len(), 2);

        let id = SlotId { category: SlotCategory::Manual, index: 1 };
        let state = |nav: &PreviewNav| nav.saves[0].slots[1].state;
        assert_eq!(state(&nav), SlotState::Empty);
        let step = nav.apply(UiEvent::SaveSlot(id), &known);
        assert_eq!(state(&nav), SlotState::Used);
        assert!(step.feedback[0].starts_with("Action: SaveSlot"));
        nav.apply(UiEvent::DeleteSlot(id), &known);
        assert_eq!(state(&nav), SlotState::Empty);
        nav.apply(UiEvent::ResetPreferences, &known);
        assert_eq!(nav.prefs.music, PrefsView::default().music);
        assert_eq!(nav.prefs.languages.len(), 2);
        nav.reset("pause");
        assert_eq!((nav.top(), nav.depth()), ("pause", 1));
    }
    #[test]
    fn variable_operations_change_the_mock_variables() {
        use orbit_core::{OpKind, Operand, VarOp};
        let mut nav = PreviewNav::new("main");
        nav.vars.insert("gold".into(), Value::Int(10));
        let add = VarOp::new("gold", OpKind::Add, Operand::Lit(Value::Int(5)));
        let step = nav.apply(UiEvent::VarOp(add), &known);
        assert_eq!(step.feedback, vec!["Variable: gold -> 15"]);
        assert_eq!(nav.context("T").vars.get("gold"), Some(&Value::Int(15)));
        let div0 = VarOp::new("gold", OpKind::Div, Operand::Lit(Value::Int(0)));
        let step = nav.apply(UiEvent::VarOp(div0), &known);
        assert!(step.feedback[0].contains("unchanged"));
        assert_eq!(nav.vars.get("gold"), Some(&Value::Int(15)));
    }

}
