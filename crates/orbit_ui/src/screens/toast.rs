//! Light notifications ("Saved", "Incompatible save").

use crate::theme::Theme;
use crate::widgets::{Align, Anchor, Dir, DrawCtx, Node, PanelKind, Ui};
use orbit_text::TextAlign;

const FADE_IN: f32 = 0.15;
const FADE_OUT: f32 = 0.4;
const MAX_TOASTS: usize = 3;

struct Toast {
    text: String,
    /// A picture left of the text (an achievement icon).
    icon: Option<orbit_render2d::TextureId>,
    age: f32,
}

/// Up to three stacked toasts at the top of the canvas, each shown for
/// `theme.widgets.list.toast_seconds`.
pub struct ToastStack {
    ui: Ui,
    toasts: Vec<Toast>,
    seconds: f32,
}

impl ToastStack {
    pub fn new(theme: &Theme) -> Self {
        Self {
            ui: Ui::new(theme.clone()),
            toasts: Vec::new(),
            seconds: theme.widgets.list.toast_seconds,
        }
    }

    pub fn set_theme(&mut self, theme: &Theme) {
        self.ui.set_theme(theme.clone());
        self.seconds = theme.widgets.list.toast_seconds;
        self.rebuild();
    }

    pub fn push(&mut self, text: impl Into<String>) {
        self.push_with_icon(text, None);
    }

    /// A toast with a picture left of its text (the icon of an unlocked
    /// achievement); None is a plain toast.
    pub fn push_with_icon(
        &mut self,
        text: impl Into<String>,
        icon: Option<orbit_render2d::TextureId>,
    ) {
        if self.toasts.len() == MAX_TOASTS {
            self.toasts.remove(0);
        }
        self.toasts.push(Toast {
            text: text.into(),
            icon,
            age: 0.0,
        });
        self.rebuild();
    }

    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.toasts.len()
    }

    pub fn update(&mut self, dt: f32) {
        for t in &mut self.toasts {
            t.age += dt;
        }
        let before = self.toasts.len();
        let seconds = self.seconds;
        self.toasts.retain(|t| t.age < seconds);
        if self.toasts.len() != before {
            self.rebuild();
        }
        // Fade of the stack follows its newest toast (fade in) and its
        // oldest (fade out).
        let alpha = self.toasts.iter().fold(1.0_f32, |a, t| {
            let fade_in = (t.age / FADE_IN).min(1.0);
            let fade_out = ((seconds - t.age) / FADE_OUT).min(1.0);
            a.min(fade_in.min(fade_out).max(0.0)).max(0.0)
        });
        let alpha = if self.toasts.len() > 1 {
            alpha.max(0.6)
        } else {
            alpha
        };
        self.ui.set_alpha(alpha);
    }

    pub fn draw(&mut self, ctx: &mut DrawCtx) {
        if !self.toasts.is_empty() {
            self.ui.draw(ctx);
        }
    }

    fn rebuild(&mut self) {
        let theme = self.ui.theme().clone();
        let mut ui = Ui::new(theme.clone());
        ui.set_layer(orbit_render2d::Layer(40));
        let column = ui.add_root(
            Node::vbox()
                .gap(10.0)
                .align(Align::Start, Align::Center)
                .anchor(Anchor::top_center(36.0)),
        );
        for t in &self.toasts {
            let panel = ui.add(
                column,
                Node::panel(PanelKind::Normal, Dir::Stack).pad(40.0, 14.0),
            );
            let panel = match t.icon {
                Some(icon) => {
                    let row = ui.add(panel, Node::hbox().gap(16.0).align(Align::Start, Align::Center));
                    ui.add(row, Node::image(Some(icon)).fixed(64.0, 64.0));
                    row
                }
                None => panel,
            };
            ui.add(
                panel,
                Node::label(t.text.clone()).text_align(TextAlign::Center),
            );
        }
        ui.layout();
        self.ui = ui;
        self.ui.set_alpha(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toasts_expire_and_are_capped() {
        let mut t = ToastStack::new(&Theme::default());
        assert!(t.is_empty());
        t.push("a");
        t.push("b");
        t.push("c");
        t.push("d");
        assert_eq!(t.len(), 3, "oldest dropped");
        t.update(1.0);
        assert_eq!(t.len(), 3);
        t.update(2.0);
        assert!(t.is_empty());
    }

    #[test]
    fn a_single_toast_fades_in_and_out() {
        let mut t = ToastStack::new(&Theme::default());
        t.push("Saved");
        t.update(0.01);
        assert!(t.ui.alpha() < 0.2);
        t.update(0.5);
        assert_eq!(t.ui.alpha(), 1.0);
        t.update(1.8); // 2.31 s: inside the fade-out of a 2.5 s toast
        assert!(t.ui.alpha() < 1.0 && t.ui.alpha() > 0.0);
    }
}
