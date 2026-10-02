//! Stock screens. Each one takes a view-model, owns a [`Ui`] and turns
//! [`UiInput`]s into [`UiEvent`]s.

mod history;
mod main_menu;
mod pause;
mod prefs;
mod quick;
mod save_load;
mod toast;

pub use history::HistoryScreen;
pub use main_menu::MainMenu;
pub use pause::PauseMenu;
pub use prefs::PreferencesScreen;
pub use quick::QuickMenu;
pub use save_load::{SaveLoadScreen, page_count, page_range};
pub use toast::ToastStack;

use crate::input::UiInput;
use crate::texts::Texts;
use crate::theme::Theme;
use crate::view::UiEvent;
use crate::widgets::{Align, DrawCtx, Node, NodeId, PanelKind, Size, Tag, Ui, WidgetEvent};
use orbit_text::TextAlign;

/// What the runtime needs from any screen.
pub trait Screen {
    /// Applies an input; returns the events for the runtime.
    fn handle(&mut self, input: UiInput) -> Vec<UiEvent>;
    fn update(&mut self, dt: f32);
    fn draw(&mut self, ctx: &mut DrawCtx);
    /// Re-skins the screen (theme hot reload).
    fn set_theme(&mut self, theme: &Theme);
    /// Re-translates the screen (language change).
    fn set_texts(&mut self, texts: &Texts);
}

/// Forwards sound cues as events and hands the other widget events to `f`.
pub(crate) fn forward(
    events: Vec<WidgetEvent>,
    out: &mut Vec<UiEvent>,
    mut f: impl FnMut(WidgetEvent, &mut Vec<UiEvent>),
) {
    for ev in events {
        match ev {
            WidgetEvent::Sound(s) => out.push(UiEvent::Sound(s)),
            other => f(other, out),
        }
    }
}

/// A full-width screen title.
pub(crate) fn title(theme: &Theme, text: &str) -> Node {
    Node::label(text)
        .font(theme.widgets.heading_size)
        .color(theme.widgets.accent_color)
}

/// Dim layer + a centered bordered panel `width` wide holding a vertical
/// stack. Returns `(root, panel)`.
pub(crate) fn centered_panel(ui: &mut Ui, width: f32, pad: [f32; 2], gap: f32) -> (NodeId, NodeId) {
    let root = ui.add_root(Node::stack().fill().align(Align::Center, Align::Center));
    ui.add(
        root,
        Node::panel(PanelKind::Dim, crate::widgets::Dir::Stack).fill(),
    );
    let panel = ui.add(
        root,
        Node::panel(PanelKind::Normal, crate::widgets::Dir::Vertical)
            .w(Size::Fixed(width))
            .pad(pad[0], pad[1])
            .gap(gap)
            .align(Align::Start, Align::Stretch),
    );
    (root, panel)
}

const T_YES: Tag = 9001;
const T_NO: Tag = 9002;

/// A modal yes/no question owned by a screen.
#[derive(Debug, Default)]
pub(crate) struct Confirm {
    pending: Option<UiEvent>,
}

impl Confirm {
    pub(crate) fn is_open(&self) -> bool {
        self.pending.is_some()
    }

    /// Opens the dialog; "Yes" emits `on_yes`.
    pub(crate) fn open(&mut self, ui: &mut Ui, texts: &Texts, message: &str, on_yes: UiEvent) {
        let theme = ui.theme().clone();
        let (yes, no) = (texts.get("ui.confirm.yes"), texts.get("ui.confirm.no"));
        let message = message.to_owned();
        ui.open_modal(|ui| {
            let root = ui.add_root(Node::stack().fill().align(Align::Center, Align::Center));
            ui.add(
                root,
                Node::panel(PanelKind::Dim, crate::widgets::Dir::Stack).fill(),
            );
            let panel = ui.add(
                root,
                Node::panel(PanelKind::Normal, crate::widgets::Dir::Vertical)
                    .w(Size::Fixed(780.0))
                    .pad(50.0, 44.0)
                    .gap(36.0)
                    .align(Align::Start, Align::Stretch),
            );
            ui.add(
                panel,
                Node::label(message)
                    .font(theme.widgets.text_size + 6.0)
                    .text_align(TextAlign::Center)
                    .lines(2),
            );
            let row = ui.add(
                panel,
                Node::hbox().gap(30.0).align(Align::Center, Align::Start),
            );
            ui.add(row, Node::button(T_YES, yes).w(Size::Fixed(260.0)));
            ui.add(row, Node::button(T_NO, no).w(Size::Fixed(260.0)));
            root
        });
        // Safe default: the question starts on "No".
        ui.set_focus_tag(T_NO);
        self.pending = Some(on_yes);
    }

    /// Consumes `ev` when a dialog is open (returns `true`).
    pub(crate) fn route(&mut self, ui: &mut Ui, ev: &WidgetEvent, out: &mut Vec<UiEvent>) -> bool {
        if self.pending.is_none() {
            return false;
        }
        match ev {
            WidgetEvent::Activated(T_YES) => {
                ui.close_modal();
                out.extend(self.pending.take());
            }
            WidgetEvent::Activated(T_NO) | WidgetEvent::Cancelled => {
                ui.close_modal();
                self.pending = None;
            }
            _ => {}
        }
        true
    }

    pub(crate) fn reset(&mut self) {
        self.pending = None;
    }
}
