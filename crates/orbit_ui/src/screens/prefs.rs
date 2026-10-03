//! The preferences screen.

use super::{Screen, forward, title};
use crate::input::UiInput;
use crate::texts::Texts;
use crate::theme::Theme;
use crate::view::{AUTO_DELAY_RANGE, AudioBus, PrefsView, SkipMode, TEXT_SPEED_RANGE, UiEvent};
use orbit_ui_data::TEXT_SCALE_RANGE;
use crate::widgets::{Align, Dir, DrawCtx, Node, PanelKind, Size, Tag, Ui, WidgetEvent};
use orbit_text::TextAlign;

const T_MASTER: Tag = 1;
const T_MUSIC: Tag = 2;
const T_SFX: Tag = 3;
const T_VOICE: Tag = 4;
const T_TEXT_SPEED: Tag = 5;
const T_AUTO_DELAY: Tag = 6;
const T_SKIP: Tag = 7;
const T_FULLSCREEN: Tag = 8;
const T_LANGUAGE: Tag = 9;
const T_DEFAULTS: Tag = 10;
const T_BACK: Tag = 11;
const T_SCREEN_READER: Tag = 12;
const T_TEXT_SCALE: Tag = 13;
const T_HIGH_CONTRAST: Tag = 14;
const T_REDUCED_MOTION: Tag = 15;
/// Value readouts are tagged `T_VALUE + slider tag`.
const T_VALUE: Tag = 50;

fn to_unit(v: f32, range: [f32; 2]) -> f32 {
    ((v - range[0]) / (range[1] - range[0])).clamp(0.0, 1.0)
}

fn from_unit(u: f32, range: [f32; 2]) -> f32 {
    range[0] + u.clamp(0.0, 1.0) * (range[1] - range[0])
}

/// Sliders (volumes, text speed, auto delay), selectors (skip mode,
/// language), fullscreen toggle, reset and back. Changes are emitted live
/// (`SetVolume`, `SetTextSpeed`...).
pub struct PreferencesScreen {
    ui: Ui,
    view: PrefsView,
    texts: Texts,
}

impl PreferencesScreen {
    pub fn new(theme: &Theme, texts: &Texts, view: PrefsView) -> Self {
        let mut s = Self {
            ui: Ui::new(theme.clone()),
            view,
            texts: texts.clone(),
        };
        s.rebuild();
        s
    }

    pub fn set_view(&mut self, view: PrefsView) {
        self.view = view;
        self.rebuild();
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn value_text(&self, tag: Tag, unit: f32) -> String {
        match tag {
            T_TEXT_SPEED => self.texts.format(
                "ui.prefs.chars_per_second",
                &[("v", &format!("{:.0}", from_unit(unit, TEXT_SPEED_RANGE)))],
            ),
            T_AUTO_DELAY => self.texts.format(
                "ui.prefs.seconds",
                &[("v", &format!("{:.1}", from_unit(unit, AUTO_DELAY_RANGE)))],
            ),
            T_TEXT_SCALE => format!("{:.0}%", from_unit(unit, TEXT_SCALE_RANGE) * 100.0),
            _ => format!("{:.0}%", unit * 100.0),
        }
    }

    fn rebuild(&mut self) {
        let focus = self.ui.focused_tag();
        let theme = self.ui.theme().clone();
        let w = &theme.widgets;
        let v = self.view.clone();
        let mut ui = Ui::new(theme.clone());
        let root = ui.add_root(Node::stack().fill().align(Align::Center, Align::Center));
        ui.add(root, Node::panel(PanelKind::Dim, Dir::Stack).fill());
        let panel = ui.add(
            root,
            Node::panel(PanelKind::Normal, Dir::Vertical)
                .w(Size::Fixed(1500.0))
                .h(Size::Fit)
                .pad(50.0, 30.0)
                .gap(12.0)
                .align(Align::Start, Align::Stretch),
        );
        ui.add(panel, title(&theme, &self.texts.get("ui.prefs.title")));

        let sliders = [
            (T_MASTER, "ui.prefs.volume_master", v.master),
            (T_MUSIC, "ui.prefs.volume_music", v.music),
            (T_SFX, "ui.prefs.volume_sfx", v.sfx),
            (T_VOICE, "ui.prefs.volume_voice", v.voice),
            (
                T_TEXT_SPEED,
                "ui.prefs.text_speed",
                to_unit(v.text_speed, TEXT_SPEED_RANGE),
            ),
            (
                T_AUTO_DELAY,
                "ui.prefs.auto_delay",
                to_unit(v.auto_delay, AUTO_DELAY_RANGE),
            ),
            (
                T_TEXT_SCALE,
                "ui.prefs.text_scale",
                to_unit(v.text_scale, TEXT_SCALE_RANGE),
            ),
        ];
        for (tag, key, unit) in sliders {
            let row = row(&mut ui, panel);
            ui.add(row, Node::label(self.texts.get(key)).w(Size::Fixed(460.0)));
            let mut slider = Node::slider(tag, unit).w(Size::Fixed(620.0));
            if tag == T_TEXT_SCALE {
                // Steps of 10 %.
                slider = slider.step(0.1 / (TEXT_SCALE_RANGE[1] - TEXT_SCALE_RANGE[0]));
            }
            ui.add(row, slider);
            ui.add(
                row,
                Node::label(self.value_text(tag, unit))
                    .tag(T_VALUE + tag)
                    .w(Size::Fixed(180.0))
                    .text_align(TextAlign::Right)
                    .dimmed(),
            );
        }

        let skip_options = vec![
            self.texts.get("ui.prefs.skip_read"),
            self.texts.get("ui.prefs.skip_all"),
        ];
        let skip_index = match v.skip_mode {
            SkipMode::ReadOnly => 0,
            SkipMode::All => 1,
        };
        let r = row(&mut ui, panel);
        ui.add(
            r,
            Node::label(self.texts.get("ui.prefs.skip_mode")).w(Size::Fixed(460.0)),
        );
        ui.add(
            r,
            Node::selector(T_SKIP, skip_options, skip_index).w(Size::Fixed(620.0)),
        );

        // Toggles go two per row: the screen stays inside the canvas even
        // at the largest text scale.
        let toggles = [
            (T_FULLSCREEN, "ui.prefs.fullscreen", v.fullscreen),
            (T_SCREEN_READER, "ui.prefs.screen_reader", v.screen_reader),
            (T_HIGH_CONTRAST, "ui.prefs.high_contrast", Some(v.high_contrast)),
            (T_REDUCED_MOTION, "ui.prefs.reduced_motion", Some(v.reduced_motion)),
        ];
        for pair in toggles.chunks(2) {
            let shown: Vec<_> = pair.iter().filter_map(|(t, k, on)| on.map(|on| (*t, *k, on))).collect();
            if shown.is_empty() {
                continue;
            }
            let r = ui.add(panel, Node::hbox().gap(20.0).align(Align::Start, Align::Center));
            for (tag, key, on) in shown {
                ui.add(
                    r,
                    Node::toggle(tag, self.texts.get(key), on).w(Size::Fixed(640.0)),
                );
            }
        }
        if v.languages.len() > 1 {
            let r = row(&mut ui, panel);
            ui.add(
                r,
                Node::label(self.texts.get("ui.prefs.language")).w(Size::Fixed(460.0)),
            );
            let names = v.languages.iter().map(|l| l.name.clone()).collect();
            let index = v
                .languages
                .iter()
                .position(|l| l.code == v.language)
                .unwrap_or(0);
            ui.add(
                r,
                Node::selector(T_LANGUAGE, names, index).w(Size::Fixed(620.0)),
            );
        }

        ui.add(panel, Node::spacer());
        let footer = ui.add(panel, Node::hbox().gap(24.0));
        ui.add(
            footer,
            Node::button(T_DEFAULTS, self.texts.get("ui.prefs.defaults")).w(Size::Fixed(420.0)),
        );
        ui.add(footer, Node::spacer());
        ui.add(
            footer,
            Node::button(T_BACK, self.texts.get("ui.menu.back")).w(Size::Fixed(300.0)),
        );
        let _ = w;
        ui.layout();
        ui.focus_first();
        if let Some(tag) = focus {
            ui.set_focus_tag(tag);
        }
        self.ui = ui;
    }
}

fn row(ui: &mut Ui, panel: crate::widgets::NodeId) -> crate::widgets::NodeId {
    ui.add(
        panel,
        Node::hbox().gap(24.0).align(Align::Start, Align::Center),
    )
}

impl Screen for PreferencesScreen {
    fn widgets(&mut self) -> Option<&mut Ui> {
        Some(&mut self.ui)
    }

    fn handle(&mut self, input: UiInput) -> Vec<UiEvent> {
        let mut out = Vec::new();
        let mut changed: Vec<(Tag, f32)> = Vec::new();
        forward(self.ui.handle(input), &mut out, |ev, out| match ev {
            WidgetEvent::Cancelled => out.push(UiEvent::Back),
            WidgetEvent::Activated(T_BACK) => out.push(UiEvent::Back),
            WidgetEvent::Activated(T_DEFAULTS) => out.push(UiEvent::ResetPreferences),
            WidgetEvent::SliderChanged(tag, unit) => {
                changed.push((tag, unit));
                out.push(match tag {
                    T_MASTER => UiEvent::SetVolume(AudioBus::Master, unit),
                    T_MUSIC => UiEvent::SetVolume(AudioBus::Music, unit),
                    T_SFX => UiEvent::SetVolume(AudioBus::Sfx, unit),
                    T_VOICE => UiEvent::SetVolume(AudioBus::Voice, unit),
                    T_TEXT_SPEED => UiEvent::SetTextSpeed(from_unit(unit, TEXT_SPEED_RANGE)),
                    T_AUTO_DELAY => UiEvent::SetAutoDelay(from_unit(unit, AUTO_DELAY_RANGE)),
                    T_TEXT_SCALE => UiEvent::SetTextScale(
                        (from_unit(unit, TEXT_SCALE_RANGE) * 10.0).round() / 10.0,
                    ),
                    _ => return,
                });
            }
            WidgetEvent::Selected(T_SKIP, i) => out.push(UiEvent::SetSkipMode(if i == 0 {
                SkipMode::ReadOnly
            } else {
                SkipMode::All
            })),
            WidgetEvent::Selected(T_LANGUAGE, i) => {
                if let Some(l) = self.view.languages.get(i) {
                    out.push(UiEvent::SetLanguage(l.code.clone()));
                }
            }
            WidgetEvent::Toggled(T_FULLSCREEN, on) => out.push(UiEvent::SetFullscreen(on)),
            WidgetEvent::Toggled(T_SCREEN_READER, on) => out.push(UiEvent::SetScreenReader(on)),
            WidgetEvent::Toggled(T_HIGH_CONTRAST, on) => out.push(UiEvent::SetHighContrast(on)),
            WidgetEvent::Toggled(T_REDUCED_MOTION, on) => out.push(UiEvent::SetReducedMotion(on)),
            _ => {}
        });
        for (tag, unit) in changed {
            let text = self.value_text(tag, unit);
            self.ui.set_text(T_VALUE + tag, text);
        }
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
    use crate::view::LanguageView;

    fn view() -> PrefsView {
        PrefsView {
            languages: vec![
                LanguageView {
                    code: "en".into(),
                    name: "English".into(),
                },
                LanguageView {
                    code: "fr".into(),
                    name: "Fran\u{e7}ais".into(),
                },
            ],
            language: "en".into(),
            ..Default::default()
        }
    }

    fn screen(v: PrefsView) -> PreferencesScreen {
        PreferencesScreen::new(&Theme::default(), &Texts::builtin("en"), v)
    }

    fn real(v: Vec<UiEvent>) -> Vec<UiEvent> {
        v.into_iter()
            .filter(|e| !matches!(e, UiEvent::Sound(_)))
            .collect()
    }

    #[test]
    fn fits_the_canvas_and_orders_widgets_top_to_bottom() {
        let mut s = screen(view());
        let order = s.ui.focus_order();
        assert_eq!(
            order,
            vec![
                T_MASTER,
                T_MUSIC,
                T_SFX,
                T_VOICE,
                T_TEXT_SPEED,
                T_AUTO_DELAY,
                T_TEXT_SCALE,
                T_SKIP,
                T_FULLSCREEN,
                T_SCREEN_READER,
                T_HIGH_CONTRAST,
                T_REDUCED_MOTION,
                T_LANGUAGE,
                T_DEFAULTS,
                T_BACK
            ]
        );
        for tag in &order {
            let r = s.ui.rect_of(*tag).unwrap();
            assert!(
                r[1] >= 0.0 && r[1] + r[3] <= 1080.0 && r[0] + r[2] <= 1920.0,
                "{tag}: {r:?}"
            );
        }
    }

    #[test]
    fn stays_inside_the_canvas_at_the_largest_text_scale_and_in_high_contrast() {
        let theme = Theme::default().accessible(1.6, true);
        let mut s = PreferencesScreen::new(&theme, &Texts::builtin("fr"), view());
        for tag in s.ui.focus_order() {
            let r = s.ui.rect_of(tag).unwrap();
            assert!(
                r[1] >= 0.0 && r[1] + r[3] <= 1080.0 && r[0] >= 0.0 && r[0] + r[2] <= 1920.0,
                "{tag}: {r:?}"
            );
        }
    }

    #[test]
    fn accessibility_controls_emit_their_events() {
        let mut s = screen(view());
        s.ui.set_focus_tag(T_HIGH_CONTRAST);
        assert_eq!(real(s.handle(UiInput::Confirm)), vec![UiEvent::SetHighContrast(true)]);
        s.ui.set_focus_tag(T_REDUCED_MOTION);
        assert_eq!(real(s.handle(UiInput::Confirm)), vec![UiEvent::SetReducedMotion(true)]);
        s.ui.set_focus_tag(T_TEXT_SCALE);
        assert_eq!(real(s.handle(UiInput::Right)), vec![UiEvent::SetTextScale(1.1)]);
        assert_eq!(real(s.handle(UiInput::Right)), vec![UiEvent::SetTextScale(1.2)]);
        let mut v = view();
        v.text_scale = 1.6;
        let mut s = screen(v);
        s.ui.set_focus_tag(T_TEXT_SCALE);
        assert_eq!(real(s.handle(UiInput::Right)), Vec::<UiEvent>::new());
    }

    #[test]
    fn fullscreen_and_language_can_be_hidden() {
        let mut v = view();
        v.fullscreen = None;
        v.languages.clear();
        let mut s = screen(v);
        let order = s.ui.focus_order();
        assert!(!order.contains(&T_FULLSCREEN) && !order.contains(&T_LANGUAGE));
    }

    #[test]
    fn keyboard_adjusts_a_volume_and_emits_the_event() {
        let mut s = screen(view());
        let ev = real(s.handle(UiInput::Left));
        assert_eq!(ev.len(), 1);
        match &ev[0] {
            UiEvent::SetVolume(AudioBus::Master, v) => assert!((v - 0.95).abs() < 1e-5),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            s.ui.node(s.ui.find(T_VALUE + T_MASTER).unwrap()).rect()[2],
            180.0
        );
    }

    #[test]
    fn text_speed_is_reported_in_chars_per_second() {
        let mut s = screen(view());
        for _ in 0..4 {
            s.handle(UiInput::Down);
        }
        assert_eq!(s.ui.focused_tag(), Some(T_TEXT_SPEED));
        s.ui.set_slider(T_TEXT_SPEED, 0.0);
        let ev = real(s.handle(UiInput::Right));
        match &ev[0] {
            UiEvent::SetTextSpeed(v) => {
                assert!(
                    (v - (TEXT_SPEED_RANGE[0]
                        + 0.05 * (TEXT_SPEED_RANGE[1] - TEXT_SPEED_RANGE[0])))
                        .abs()
                        < 1e-3
                )
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn selectors_toggle_and_buttons_emit_events() {
        let mut s = screen(view());
        s.ui.set_focus_tag(T_SKIP);
        assert_eq!(
            real(s.handle(UiInput::Right)),
            vec![UiEvent::SetSkipMode(SkipMode::All)]
        );
        s.ui.set_focus_tag(T_FULLSCREEN);
        assert_eq!(
            real(s.handle(UiInput::Confirm)),
            vec![UiEvent::SetFullscreen(true)]
        );
        s.ui.set_focus_tag(T_LANGUAGE);
        assert_eq!(
            real(s.handle(UiInput::Right)),
            vec![UiEvent::SetLanguage("fr".into())]
        );
        s.ui.set_focus_tag(T_DEFAULTS);
        assert_eq!(
            real(s.handle(UiInput::Confirm)),
            vec![UiEvent::ResetPreferences]
        );
        s.ui.set_focus_tag(T_BACK);
        assert_eq!(real(s.handle(UiInput::Confirm)), vec![UiEvent::Back]);
        assert_eq!(real(s.handle(UiInput::Cancel)), vec![UiEvent::Back]);
    }

    #[test]
    fn dragging_a_slider_with_the_mouse_sets_its_value() {
        let mut s = screen(view());
        let r = s.ui.rect_of(T_MUSIC).unwrap();
        let y = r[1] + r[3] / 2.0;
        s.handle(UiInput::PointerMove([r[0] + 5.0, y]));
        let ev = real(s.handle(UiInput::PointerDown([r[0] + r[2] / 2.0, y])));
        assert!(
            matches!(ev.as_slice(), [UiEvent::SetVolume(AudioBus::Music, v)] if (v - 0.5).abs() < 0.02)
        );
        let ev = real(s.handle(UiInput::PointerMove([r[0] + r[2] + 400.0, y])));
        assert!(matches!(ev.as_slice(), [UiEvent::SetVolume(AudioBus::Music, v)] if *v == 1.0));
        s.handle(UiInput::PointerUp([0.0, 0.0]));
        assert!(real(s.handle(UiInput::PointerMove([r[0], y]))).is_empty());
    }
}
