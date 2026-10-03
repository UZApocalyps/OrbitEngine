//! The main menu.

use super::{Screen, forward};
use crate::input::UiInput;
use crate::texts::Texts;
use crate::theme::Theme;
use crate::view::{MainMenuView, UiEvent};
use crate::widgets::{Align, DrawCtx, Node, PanelKind, Size, Tag, Ui, WidgetEvent};

const T_NEW: Tag = 1;
const T_CONTINUE: Tag = 2;
const T_LOAD: Tag = 3;
const T_PREFS: Tag = 4;
const T_QUIT: Tag = 5;

/// Title, optional background and the menu buttons. Emits `StartGame`,
/// `Continue`, `OpenLoad`, `OpenPreferences`, `Quit`.
pub struct MainMenu {
    ui: Ui,
    view: MainMenuView,
    texts: Texts,
}

impl MainMenu {
    pub fn new(theme: &Theme, texts: &Texts, view: MainMenuView) -> Self {
        let mut menu = Self {
            ui: Ui::new(theme.clone()),
            view,
            texts: texts.clone(),
        };
        menu.rebuild();
        menu
    }

    /// Replaces the view-model (keeps the focused button when possible).
    pub fn set_view(&mut self, view: MainMenuView) {
        self.view = view;
        self.rebuild();
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn rebuild(&mut self) {
        let focus = self.ui.focused_tag();
        let theme = self.ui.theme().clone();
        let w = &theme.widgets;
        let t = &self.texts;
        let mut ui = Ui::new(theme.clone());
        let root = ui.add_root(Node::stack().fill());
        match self.view.background {
            Some(tex) => {
                ui.add(root, Node::image(Some(tex)).fill());
                ui.add(
                    root,
                    Node::panel(PanelKind::Dim, crate::widgets::Dir::Stack).fill(),
                );
            }
            None => {
                ui.add(
                    root,
                    Node::panel(PanelKind::Solid, crate::widgets::Dir::Stack).fill(),
                );
            }
        }
        let column = ui.add(root, Node::vbox().fill().pad(170.0, 150.0).gap(16.0));
        ui.add(
            column,
            Node::label(self.view.title.clone())
                .font(w.title_size)
                .color(w.accent_color),
        );
        if let Some(sub) = &self.view.subtitle {
            ui.add(column, Node::label(sub.clone()).dimmed());
        }
        ui.add(column, Node::vbox().h(Size::Fixed(60.0)));
        let menu = ui.add(
            column,
            Node::vbox()
                .w(Size::Fixed(560.0))
                .gap(w.spacing)
                .align(Align::Start, Align::Stretch),
        );
        ui.add(menu, Node::button(T_NEW, t.get("ui.menu.new_game")));
        if self.view.can_continue {
            ui.add(menu, Node::button(T_CONTINUE, t.get("ui.menu.continue")));
        }
        ui.add(menu, Node::button(T_LOAD, t.get("ui.menu.load")));
        ui.add(menu, Node::button(T_PREFS, t.get("ui.menu.preferences")));
        if self.view.show_quit {
            ui.add(menu, Node::button(T_QUIT, t.get("ui.menu.quit")));
        }
        ui.layout();
        ui.focus_first();
        if let Some(tag) = focus {
            ui.set_focus_tag(tag);
        }
        self.ui = ui;
    }
}

impl Screen for MainMenu {
    fn widgets(&mut self) -> Option<&mut Ui> {
        Some(&mut self.ui)
    }

    fn handle(&mut self, input: UiInput) -> Vec<UiEvent> {
        let mut out = Vec::new();
        forward(self.ui.handle(input), &mut out, |ev, out| {
            if let WidgetEvent::Activated(tag) = ev {
                out.push(match tag {
                    T_NEW => UiEvent::StartGame,
                    T_CONTINUE => UiEvent::Continue,
                    T_LOAD => UiEvent::OpenLoad,
                    T_PREFS => UiEvent::OpenPreferences,
                    T_QUIT => UiEvent::Quit,
                    _ => return,
                });
            }
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

    fn view(can_continue: bool, show_quit: bool) -> MainMenuView {
        MainMenuView {
            title: "Game".into(),
            subtitle: None,
            background: None,
            can_continue,
            show_quit,
        }
    }

    fn menu(can_continue: bool, show_quit: bool) -> MainMenu {
        MainMenu::new(
            &Theme::default(),
            &Texts::builtin("en"),
            view(can_continue, show_quit),
        )
    }

    #[test]
    fn continue_and_quit_are_optional() {
        let mut full = menu(true, true);
        assert_eq!(
            full.ui.focus_order(),
            vec![T_NEW, T_CONTINUE, T_LOAD, T_PREFS, T_QUIT]
        );
        let mut web = menu(false, false);
        assert_eq!(web.ui.focus_order(), vec![T_NEW, T_LOAD, T_PREFS]);
    }

    #[test]
    fn keyboard_activates_the_right_events() {
        let mut m = menu(true, true);
        let sounds_stripped = |v: Vec<UiEvent>| -> Vec<UiEvent> {
            v.into_iter()
                .filter(|e| !matches!(e, UiEvent::Sound(_)))
                .collect()
        };
        assert_eq!(
            sounds_stripped(m.handle(UiInput::Confirm)),
            vec![UiEvent::StartGame]
        );
        m.handle(UiInput::Down);
        assert_eq!(
            sounds_stripped(m.handle(UiInput::Confirm)),
            vec![UiEvent::Continue]
        );
        m.handle(UiInput::Down);
        assert_eq!(
            sounds_stripped(m.handle(UiInput::Confirm)),
            vec![UiEvent::OpenLoad]
        );
        m.handle(UiInput::Down);
        assert_eq!(
            sounds_stripped(m.handle(UiInput::Confirm)),
            vec![UiEvent::OpenPreferences]
        );
        m.handle(UiInput::Down);
        assert_eq!(
            sounds_stripped(m.handle(UiInput::Confirm)),
            vec![UiEvent::Quit]
        );
    }

    #[test]
    fn clicking_a_button_emits_its_event() {
        let mut m = menu(false, true);
        let r = m.ui.rect_of(T_LOAD).unwrap();
        let p = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
        m.handle(UiInput::PointerMove(p));
        m.handle(UiInput::PointerDown(p));
        let ev = m.handle(UiInput::PointerUp(p));
        assert!(ev.contains(&UiEvent::OpenLoad));
    }

    #[test]
    fn rebuilding_keeps_the_focused_button() {
        let mut m = menu(false, true);
        m.handle(UiInput::Down);
        assert_eq!(m.ui.focused_tag(), Some(T_LOAD));
        m.set_view(view(true, true));
        assert_eq!(m.ui.focused_tag(), Some(T_LOAD));
    }
}
