//! The quick menu: a discreet bar of small buttons shown during play.

use super::{Screen, forward};
use crate::input::UiInput;
use crate::texts::Texts;
use crate::theme::Theme;
use crate::view::{QuickMenuView, UiEvent};
use crate::widgets::{Anchor, DrawCtx, Node, Size, Tag, Ui, WidgetEvent};
use orbit_render2d::Layer;

const T_BACK: Tag = 1;
const T_HISTORY: Tag = 2;
const T_SKIP: Tag = 3;
const T_AUTO: Tag = 4;
const T_QSAVE: Tag = 5;
const T_QLOAD: Tag = 6;
const T_PREFS: Tag = 7;
const T_MENU: Tag = 8;
const T_VOICE: Tag = 9;

/// Back (rollback), History, Skip, Auto, Q.Save, Q.Load, Preferences, Menu.
/// Pointer only (keyboard shortcuts belong to the game input); Skip and Auto
/// show their active state. Drawn on the UI layer, under the menus.
pub struct QuickMenu {
    ui: Ui,
    view: QuickMenuView,
    texts: Texts,
    /// Project images the button style may draw.
    images: std::collections::HashMap<String, orbit_render2d::TextureId>,
}

impl QuickMenu {
    pub fn new(theme: &Theme, texts: &Texts, view: QuickMenuView) -> Self {
        let mut s = Self {
            ui: Ui::new(theme.clone()),
            view,
            texts: texts.clone(),
            images: Default::default(),
        };
        s.rebuild();
        s
    }

    pub fn set_view(&mut self, view: QuickMenuView) {
        if view != self.view {
            self.view = view;
            self.rebuild();
        }
    }

    /// Loaded project images (style backgrounds of the buttons).
    pub fn set_images(&mut self, images: std::collections::HashMap<String, orbit_render2d::TextureId>) {
        if images != self.images {
            self.images = images;
            self.rebuild();
        }
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    /// `true` when a click at `p` hits one of the buttons (the game must not
    /// advance the dialogue).
    pub fn wants_pointer(&mut self, p: [f32; 2]) -> bool {
        self.ui.wants_pointer(p)
    }

    /// `[x, y, w, h]` of the bar.
    pub fn bar_rect(&mut self) -> [f32; 4] {
        self.ui.layout();
        self.ui.node(crate::widgets::NodeId(0)).rect()
    }

    fn rebuild(&mut self) {
        let theme = self.ui.theme().clone();
        let t = &self.texts;
        let v = self.view;
        let mut ui = Ui::new(theme.clone());
        ui.set_images(self.images.clone());
        ui.set_layer(Layer::UI);
        let q = &theme.quick_menu;
        let bar = ui.add_root(
            Node::hbox()
                .gap(q.gap.unwrap_or(8.0))
                .anchor(q.anchor.unwrap_or(Anchor::bottom_center(-8.0))),
        );
        let mut items = Vec::from([
            (T_BACK, "ui.quick.back", false, v.can_rollback),
            (T_HISTORY, "ui.quick.history", false, true),
            (T_SKIP, "ui.quick.skip", v.skip_active, true),
            (T_AUTO, "ui.quick.auto", v.auto_active, true),
            (T_QSAVE, "ui.quick.qsave", false, true),
            (T_QLOAD, "ui.quick.qload", false, v.can_quick_load),
            (T_PREFS, "ui.quick.prefs", false, true),
            (T_MENU, "ui.quick.menu", false, true),
        ]);
        if v.can_replay_voice {
            items.insert(2, (T_VOICE, "ui.quick.voice", false, true));
        }
        // A hidden bar keeps its (empty) root: `bar_rect` still answers.
        let items = if q.hidden { &items[..0] } else { &items[..] };
        for &(tag, key, active, enabled) in items {
            let mut node = Node::button(tag, t.get(key))
                .small()
                .w(Size::Fixed(q.button_width.unwrap_or(150.0)))
                .selected(active)
                .enabled(enabled);
            if let Some(style) = &q.button_style {
                node = node.style(style.clone());
            }
            ui.add(bar, node);
        }
        ui.layout();
        self.ui = ui;
    }
}

impl Screen for QuickMenu {
    fn widgets(&mut self) -> Option<&mut Ui> {
        Some(&mut self.ui)
    }

    fn handle(&mut self, input: UiInput) -> Vec<UiEvent> {
        // Pointer only.
        if !matches!(
            input,
            UiInput::PointerMove(_) | UiInput::PointerDown(_) | UiInput::PointerUp(_)
        ) {
            return Vec::new();
        }
        let mut out = Vec::new();
        forward(self.ui.handle(input), &mut out, |ev, out| {
            if let WidgetEvent::Activated(tag) = ev {
                out.push(match tag {
                    T_BACK => UiEvent::Rollback,
                    T_HISTORY => UiEvent::OpenHistory,
                    T_SKIP => UiEvent::ToggleSkip,
                    T_AUTO => UiEvent::ToggleAuto,
                    T_QSAVE => UiEvent::QuickSave,
                    T_QLOAD => UiEvent::QuickLoad,
                    T_PREFS => UiEvent::OpenPreferences,
                    T_MENU => UiEvent::OpenPause,
                    T_VOICE => UiEvent::ReplayVoice,
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

    fn menu(view: QuickMenuView) -> QuickMenu {
        QuickMenu::new(&Theme::default(), &Texts::builtin("en"), view)
    }

    fn real(v: Vec<UiEvent>) -> Vec<UiEvent> {
        v.into_iter()
            .filter(|e| !matches!(e, UiEvent::Sound(_)))
            .collect()
    }

    fn click(m: &mut QuickMenu, tag: Tag) -> Vec<UiEvent> {
        let r = m.ui.rect_of(tag).unwrap();
        let p = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
        m.handle(UiInput::PointerMove(p));
        m.handle(UiInput::PointerDown(p));
        real(m.handle(UiInput::PointerUp(p)))
    }

    #[test]
    fn every_button_maps_to_its_event() {
        let mut m = menu(QuickMenuView {
            can_rollback: true,
            can_quick_load: true,
            ..Default::default()
        });
        let expected = [
            (T_BACK, UiEvent::Rollback),
            (T_HISTORY, UiEvent::OpenHistory),
            (T_SKIP, UiEvent::ToggleSkip),
            (T_AUTO, UiEvent::ToggleAuto),
            (T_QSAVE, UiEvent::QuickSave),
            (T_QLOAD, UiEvent::QuickLoad),
            (T_PREFS, UiEvent::OpenPreferences),
            (T_MENU, UiEvent::OpenPause),
        ];
        for (tag, ev) in expected {
            assert_eq!(click(&mut m, tag), vec![ev], "button {tag}");
        }
    }

    #[test]
    fn disabled_buttons_do_nothing_and_do_not_capture_the_pointer() {
        let mut m = menu(QuickMenuView::default());
        assert!(click(&mut m, T_BACK).is_empty());
        assert!(click(&mut m, T_QLOAD).is_empty());
        let r = m.ui.rect_of(T_BACK).unwrap();
        assert!(!m.wants_pointer([r[0] + 5.0, r[1] + 5.0]));
        let r = m.ui.rect_of(T_HISTORY).unwrap();
        assert!(m.wants_pointer([r[0] + 5.0, r[1] + 5.0]));
        assert!(!m.wants_pointer([10.0, 10.0]));
    }

    #[test]
    fn keyboard_is_ignored_and_the_bar_sits_at_the_bottom() {
        let mut m = menu(QuickMenuView::default());
        assert!(m.handle(UiInput::Confirm).is_empty());
        assert!(m.handle(UiInput::Cancel).is_empty());
        let bar = m.bar_rect();
        assert!(bar[1] + bar[3] <= 1080.0 && bar[1] > 980.0);
        assert!((bar[0] + bar[2] / 2.0 - 960.0).abs() < 1.0);
    }

    #[test]
    fn active_state_is_kept_on_the_button() {
        let m = menu(QuickMenuView {
            skip_active: true,
            ..Default::default()
        });
        let id = m.ui.find(T_SKIP).unwrap();
        assert!(matches!(
            &m.ui.node(id).kind,
            crate::widgets::Kind::Button(b) if b.selected
        ));
    }
}
