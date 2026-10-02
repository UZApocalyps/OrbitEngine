//! The in-game pause menu.

use super::{Screen, centered_panel, forward, title};
use crate::input::UiInput;
use crate::texts::Texts;
use crate::theme::Theme;
use crate::view::{PauseView, UiEvent};
use crate::widgets::{DrawCtx, Node, Tag, Ui, WidgetEvent};
use orbit_text::TextAlign;

const T_RESUME: Tag = 1;
const T_SAVE: Tag = 2;
const T_LOAD: Tag = 3;
const T_HISTORY: Tag = 4;
const T_PREFS: Tag = 5;
const T_MAIN: Tag = 6;
const T_QUIT: Tag = 7;

/// Resume, Save, Load, History, Preferences, Main menu, Quit. Cancel
/// (Escape / right click) resumes.
pub struct PauseMenu {
    ui: Ui,
    view: PauseView,
    texts: Texts,
}

impl PauseMenu {
    pub fn new(theme: &Theme, texts: &Texts, view: PauseView) -> Self {
        let mut menu = Self {
            ui: Ui::new(theme.clone()),
            view,
            texts: texts.clone(),
        };
        menu.rebuild();
        menu
    }

    pub fn set_view(&mut self, view: PauseView) {
        self.view = view;
        self.rebuild();
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn rebuild(&mut self) {
        let focus = self.ui.focused_tag();
        let theme = self.ui.theme().clone();
        let t = &self.texts;
        let mut ui = Ui::new(theme.clone());
        let (_, panel) = centered_panel(&mut ui, 640.0, [60.0, 46.0], theme.widgets.spacing);
        ui.add(
            panel,
            title(&theme, &t.get("ui.pause.title")).text_align(TextAlign::Center),
        );
        ui.add(panel, Node::button(T_RESUME, t.get("ui.menu.resume")));
        ui.add(
            panel,
            Node::button(T_SAVE, t.get("ui.menu.save")).enabled(self.view.can_save),
        );
        ui.add(panel, Node::button(T_LOAD, t.get("ui.menu.load")));
        ui.add(panel, Node::button(T_HISTORY, t.get("ui.menu.history")));
        ui.add(panel, Node::button(T_PREFS, t.get("ui.menu.preferences")));
        ui.add(panel, Node::button(T_MAIN, t.get("ui.menu.main_menu")));
        if self.view.show_quit {
            ui.add(panel, Node::button(T_QUIT, t.get("ui.menu.quit")));
        }
        ui.layout();
        ui.focus_first();
        if let Some(tag) = focus {
            ui.set_focus_tag(tag);
        }
        self.ui = ui;
    }
}

impl Screen for PauseMenu {
    fn handle(&mut self, input: UiInput) -> Vec<UiEvent> {
        let mut out = Vec::new();
        forward(self.ui.handle(input), &mut out, |ev, out| match ev {
            WidgetEvent::Cancelled => out.push(UiEvent::Resume),
            WidgetEvent::Activated(tag) => out.push(match tag {
                T_RESUME => UiEvent::Resume,
                T_SAVE => UiEvent::OpenSave,
                T_LOAD => UiEvent::OpenLoad,
                T_HISTORY => UiEvent::OpenHistory,
                T_PREFS => UiEvent::OpenPreferences,
                T_MAIN => UiEvent::ReturnToMainMenu,
                T_QUIT => UiEvent::Quit,
                _ => return,
            }),
            _ => {}
        });
        out
    }

    fn update(&mut self, dt: f32) {
        self.ui.update(dt);
    }

    fn draw(&mut self, ctx: &mut DrawCtx) {
        self.ui.draw(ctx);
    }

    fn set_theme(&mut self, theme: &Theme) {
        self.ui.set_theme(theme.clone());
        self.rebuild();
    }

    fn set_texts(&mut self, texts: &Texts) {
        self.texts = texts.clone();
        self.rebuild();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real(v: Vec<UiEvent>) -> Vec<UiEvent> {
        v.into_iter()
            .filter(|e| !matches!(e, UiEvent::Sound(_)))
            .collect()
    }

    fn menu(can_save: bool, show_quit: bool) -> PauseMenu {
        PauseMenu::new(
            &Theme::default(),
            &Texts::builtin("en"),
            PauseView {
                can_save,
                show_quit,
            },
        )
    }

    #[test]
    fn cancel_resumes_and_entries_map_to_events() {
        let mut m = menu(true, true);
        assert_eq!(real(m.handle(UiInput::Cancel)), vec![UiEvent::Resume]);
        let expected = [
            UiEvent::Resume,
            UiEvent::OpenSave,
            UiEvent::OpenLoad,
            UiEvent::OpenHistory,
            UiEvent::OpenPreferences,
            UiEvent::ReturnToMainMenu,
            UiEvent::Quit,
        ];
        for (i, want) in expected.iter().enumerate() {
            if i > 0 {
                m.handle(UiInput::Down);
            }
            assert_eq!(
                real(m.handle(UiInput::Confirm)),
                vec![want.clone()],
                "entry {i}"
            );
        }
    }

    #[test]
    fn save_can_be_disabled_and_quit_hidden() {
        let mut m = menu(false, false);
        assert_eq!(
            m.ui.focus_order(),
            vec![T_RESUME, T_LOAD, T_HISTORY, T_PREFS, T_MAIN]
        );
    }
}
