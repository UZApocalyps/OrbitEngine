//! The dialogue history screen.

use super::{Screen, forward, title};
use crate::input::UiInput;
use crate::texts::Texts;
use crate::theme::Theme;
use crate::view::{HistoryView, UiEvent};
use crate::widgets::{
    Align, Dir, DrawCtx, Node, PanelKind, Size, Tag, Ui, WidgetEvent, estimate_lines,
};

const T_LIST: Tag = 1;
const T_BACK: Tag = 2;
/// Rewind buttons are tagged `T_REWIND + entry index`.
const T_REWIND: Tag = 1000;

const PANEL: [f32; 2] = [1760.0, 980.0];
const REWIND_W: f32 = 240.0;

/// Scrollable list of past lines (colored speaker + text) with a "rewind
/// here" button where the view-model allows it. Opens at the latest line.
pub struct HistoryScreen {
    ui: Ui,
    view: HistoryView,
    texts: Texts,
}

impl HistoryScreen {
    pub fn new(theme: &Theme, texts: &Texts, view: HistoryView) -> Self {
        let mut s = Self {
            ui: Ui::new(theme.clone()),
            view,
            texts: texts.clone(),
        };
        s.rebuild();
        s
    }

    pub fn set_view(&mut self, view: HistoryView) {
        self.view = view;
        self.rebuild();
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn rebuild(&mut self) {
        let theme = self.ui.theme().clone();
        let w = &theme.widgets;
        let mut ui = Ui::new(theme.clone());
        let root = ui.add_root(Node::stack().fill().align(Align::Center, Align::Center));
        ui.add(root, Node::panel(PanelKind::Dim, Dir::Stack).fill());
        let panel = ui.add(
            root,
            Node::panel(PanelKind::Normal, Dir::Vertical)
                .fixed(PANEL[0], PANEL[1])
                .pad(40.0, 28.0)
                .gap(16.0)
                .align(Align::Start, Align::Stretch),
        );
        ui.add(panel, title(&theme, &self.texts.get("ui.history.title")));
        if self.view.entries.is_empty() {
            ui.add(
                panel,
                Node::label(self.texts.get("ui.history.empty")).dimmed(),
            );
            ui.add(panel, Node::spacer());
        } else {
            let list = ui.add(
                panel,
                Node::scroll_list(T_LIST)
                    .size(Size::Fill(1.0), Size::Fill(1.0))
                    .gap(6.0),
            );
            // Width left to the text column: panel inner width minus bar,
            // paddings and the button column.
            let text_w = PANEL[0] - 80.0 - w.list.bar_width - 8.0 - 2.0 * 20.0 - REWIND_W - 24.0;
            for (i, e) in self.view.entries.iter().enumerate() {
                let row = ui.add(
                    list,
                    Node::hbox()
                        .size(Size::Fill(1.0), Size::Fit)
                        .pad(20.0, 8.0)
                        .gap(24.0)
                        .align(Align::Start, Align::Center),
                );
                let col = ui.add(
                    row,
                    Node::vbox()
                        .w(Size::Fixed(text_w))
                        .gap(2.0)
                        .align(Align::Start, Align::Stretch),
                );
                if let Some((name, [r, g, b])) = &e.speaker {
                    ui.add(
                        col,
                        Node::label(name.clone())
                            .font(w.text_size)
                            .color([*r, *g, *b, 255]),
                    );
                }
                let lines = estimate_lines(&e.text, w.text_size, text_w);
                ui.add(col, Node::label(e.text.clone()).lines(lines));
                if e.can_rollback {
                    ui.add(
                        row,
                        Node::button(T_REWIND + i as Tag, self.texts.get("ui.history.rewind"))
                            .small()
                            .w(Size::Fixed(REWIND_W)),
                    );
                } else {
                    ui.add(row, Node::vbox().w(Size::Fixed(REWIND_W)));
                }
            }
        }
        let footer = ui.add(panel, Node::hbox());
        ui.add(footer, Node::spacer());
        ui.add(
            footer,
            Node::button(T_BACK, self.texts.get("ui.menu.back")).w(Size::Fixed(300.0)),
        );
        ui.layout();
        ui.scroll_to_end(T_LIST);
        // Start on the latest rewind button, else the list, else Back.
        let last = self
            .view
            .entries
            .iter()
            .rposition(|e| e.can_rollback)
            .map(|i| T_REWIND + i as Tag);
        ui.focus_first();
        match last {
            Some(tag) => {
                ui.set_focus_tag(tag);
            }
            None => {
                ui.set_focus_tag(T_LIST);
            }
        }
        self.ui = ui;
    }
}

impl Screen for HistoryScreen {
    fn handle(&mut self, input: UiInput) -> Vec<UiEvent> {
        let mut out = Vec::new();
        forward(self.ui.handle(input), &mut out, |ev, out| match ev {
            WidgetEvent::Cancelled | WidgetEvent::Activated(T_BACK) => out.push(UiEvent::Back),
            WidgetEvent::Activated(tag) if tag >= T_REWIND => {
                let index = (tag - T_REWIND) as usize;
                if let Some(entry) = self.view.entries.get(index) {
                    out.push(UiEvent::RollbackTo(entry.id));
                }
            }
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
    use crate::view::HistoryEntryView;

    fn entry(i: usize, can_rollback: bool) -> HistoryEntryView {
        HistoryEntryView {
            id: 100 + i,
            speaker: Some(("Alice".into(), [255, 150, 190])),
            text: format!("Line number {i} of the dialogue"),
            can_rollback,
        }
    }

    fn screen(n: usize, rollback: impl Fn(usize) -> bool) -> HistoryScreen {
        HistoryScreen::new(
            &Theme::default(),
            &Texts::builtin("en"),
            HistoryView {
                entries: (0..n).map(|i| entry(i, rollback(i))).collect(),
            },
        )
    }

    fn real(v: Vec<UiEvent>) -> Vec<UiEvent> {
        v.into_iter()
            .filter(|e| !matches!(e, UiEvent::Sound(_)))
            .collect()
    }

    #[test]
    fn opens_on_the_latest_lines_and_scrolls_with_the_wheel() {
        let mut s = screen(40, |_| false);
        let first = s.ui.scroll_first(T_LIST).unwrap();
        assert!(first > 20, "scrolled to the end, first = {first}");
        s.handle(UiInput::Scroll(1.0));
        assert_eq!(s.ui.scroll_first(T_LIST), Some(first - 1));
        for _ in 0..100 {
            s.handle(UiInput::Scroll(1.0));
        }
        assert_eq!(s.ui.scroll_first(T_LIST), Some(0));
        for _ in 0..100 {
            s.handle(UiInput::Scroll(-1.0));
        }
        assert_eq!(
            s.ui.scroll_first(T_LIST),
            Some(first),
            "cannot scroll past the end"
        );
    }

    #[test]
    fn keyboard_scrolls_a_list_without_buttons() {
        let mut s = screen(40, |_| false);
        assert_eq!(s.ui.focused_tag(), Some(T_LIST));
        let first = s.ui.scroll_first(T_LIST).unwrap();
        s.handle(UiInput::Up);
        assert_eq!(s.ui.scroll_first(T_LIST), Some(first - 1));
        s.handle(UiInput::PageUp);
        assert!(s.ui.scroll_first(T_LIST).unwrap() < first - 1);
    }

    #[test]
    fn rewind_buttons_emit_the_entry_id_and_follow_the_focus() {
        let mut s = screen(40, |i| i % 2 == 0);
        // Starts on the last rollback-able entry (38).
        assert_eq!(s.ui.focused_tag(), Some(T_REWIND + 38));
        assert_eq!(
            real(s.handle(UiInput::Confirm)),
            vec![UiEvent::RollbackTo(138)]
        );
        // Walk up: the list scrolls so the focused row stays visible.
        for _ in 0..15 {
            s.handle(UiInput::Up);
        }
        let tag = s.ui.focused_tag().unwrap();
        assert_eq!(tag, T_REWIND + 8);
        let id = s.ui.find(tag).unwrap();
        assert!(s.ui.node(id).is_visible());
        let r = s.ui.node(id).rect();
        assert!(r[1] >= 0.0 && r[1] + r[3] <= 1080.0);
    }

    #[test]
    fn entries_without_permission_have_no_button() {
        let s = screen(3, |i| i == 1);
        assert!(s.ui.find(T_REWIND + 1).is_some());
        assert!(s.ui.find(T_REWIND).is_none());
        assert!(s.ui.find(T_REWIND + 2).is_none());
    }

    #[test]
    fn empty_history_shows_a_message_and_back_works() {
        let mut s = screen(0, |_| false);
        assert!(s.ui.find(T_LIST).is_none());
        assert_eq!(real(s.handle(UiInput::Cancel)), vec![UiEvent::Back]);
    }
}
