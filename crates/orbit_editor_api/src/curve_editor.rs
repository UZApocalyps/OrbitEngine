//! Reusable easing-curve editor (ADR 0012, M5A-02): a preset picker, the four
//! CSS-style Bézier numbers, a graph with draggable Bézier handles and an
//! animated preview. Like the other field widgets it never mutates anything:
//! it returns `Some(new_curve)` only when the user changed the curve.
//!
//! It is meant to be reused by the project curves panel, the transition
//! inspector and, later, the timeline key inspector.

use std::collections::BTreeMap;

use egui::{
    Color32, ComboBox, CornerRadius, DragValue, Pos2, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use orbit_core::{Curve, Ease};

use crate::UiText;
use crate::fields::Salt;

/// Number of segments used to draw a curve.
const SAMPLES: usize = 64;
/// Handles can be dragged within this vertical range (overshoot allowed).
pub const Y_RANGE: std::ops::RangeInclusive<f32> = -2.0..=3.0;
/// Length (seconds) of one preview run, then a pause of [`PREVIEW_PAUSE`].
const PREVIEW_RUN: f64 = 1.2;
const PREVIEW_PAUSE: f64 = 0.5;

/// What the editor lets the user pick for a curve.
#[derive(Clone, Copy, Debug)]
pub struct CurveEditorOptions {
    /// Offer the story's named curves in the picker.
    pub allow_named: bool,
    /// Draw the animated preview below the graph.
    pub preview: bool,
    /// Edge of the square graph, in points.
    pub graph_size: f32,
}

impl Default for CurveEditorOptions {
    fn default() -> Self {
        Self {
            allow_named: true,
            preview: true,
            graph_size: 140.0,
        }
    }
}

/// The CSS curves offered as one-click Bézier starting points.
pub fn bezier_presets() -> [(&'static str, Curve); 4] {
    [
        ("curve.css.ease", Curve::css_ease()),
        ("curve.css.ease_in", Curve::css_ease_in()),
        ("curve.css.ease_out", Curve::css_ease_out()),
        ("curve.css.ease_in_out", Curve::css_ease_in_out()),
    ]
}

/// Short human name of a curve kind, for the picker.
pub fn curve_label(curve: &Curve) -> String {
    match curve {
        Curve::Preset(e) => e.name().to_owned(),
        Curve::Bezier { p1, p2 } => {
            format!("cubic-bezier({:.2}, {:.2}, {:.2}, {:.2})", p1[0], p1[1], p2[0], p2[1])
        }
        Curve::Named(n) => n.clone(),
    }
}

/// Resolves `Named` through `named`; anything unresolved is linear.
pub fn resolve<'a>(curve: &'a Curve, named: &'a BTreeMap<String, Curve>) -> &'a Curve {
    match curve {
        Curve::Named(n) => match named.get(n) {
            Some(c) if !matches!(c, Curve::Named(_)) => c,
            _ => &Curve::LINEAR,
        },
        other => other,
    }
}

/// Samples `curve` at `SAMPLES + 1` evenly spaced progress values.
pub fn sample(curve: &Curve, named: &BTreeMap<String, Curve>) -> Vec<f32> {
    (0..=SAMPLES)
        .map(|i| curve.eval_with(i as f32 / SAMPLES as f32, |n| named.get(n)))
        .collect()
}

/// The vertical range (`min`, `max`) of the graph: at least 0..1, extended to
/// show overshoots with a small margin.
pub fn graph_range(samples: &[f32]) -> (f32, f32) {
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for v in samples.iter().copied().filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if lo < 0.0 || hi > 1.0 {
        let pad = (hi - lo) * 0.08;
        (lo - pad, hi + pad)
    } else {
        (-0.05, 1.05)
    }
}

/// Maps curve space (`x` in 0..1, `y` in `range`) to a screen position inside
/// `rect` (y grows upwards in curve space).
pub fn to_screen(rect: Rect, range: (f32, f32), x: f32, y: f32) -> Pos2 {
    let ty = (y - range.0) / (range.1 - range.0);
    pos2(
        rect.left() + x * rect.width(),
        rect.bottom() - ty * rect.height(),
    )
}

/// Inverse of [`to_screen`].
pub fn from_screen(rect: Rect, range: (f32, f32), p: Pos2) -> (f32, f32) {
    let x = (p.x - rect.left()) / rect.width();
    let ty = (rect.bottom() - p.y) / rect.height();
    (x, range.0 + ty * (range.1 - range.0))
}

/// A Bézier moved to `handle` (0 or 1) at curve-space `(x, y)`: x stays in
/// 0..1 and y in [`Y_RANGE`], so the result is always a valid curve.
pub fn with_handle(curve: &Curve, handle: usize, x: f32, y: f32) -> Curve {
    let (mut p1, mut p2) = match curve {
        Curve::Bezier { p1, p2 } => (*p1, *p2),
        _ => return curve.clone(),
    };
    let p = [x.clamp(0.0, 1.0), y.clamp(*Y_RANGE.start(), *Y_RANGE.end())];
    if handle == 0 {
        p1 = p;
    } else {
        p2 = p;
    }
    Curve::Bezier { p1, p2 }
}

/// Progress (0..1) of the animated preview at wall-clock `time` seconds:
/// a run of [`PREVIEW_RUN`] then a pause on the end value.
pub fn preview_progress(time: f64) -> f32 {
    let cycle = PREVIEW_RUN + PREVIEW_PAUSE;
    let local = time.rem_euclid(cycle);
    (local / PREVIEW_RUN).min(1.0) as f32
}

/// The editor. `named` is the story's curve table (to resolve and to offer
/// `Named` curves). Returns the new curve when the user edited it.
pub fn curve_editor(
    ui: &mut Ui,
    salt: impl Salt,
    curve: &Curve,
    named: &BTreeMap<String, Curve>,
    options: CurveEditorOptions,
) -> Option<Curve> {
    let mut out: Option<Curve> = None;
    let current = out.as_ref().unwrap_or(curve).clone();

    // ---- picker + CSS presets
    ComboBox::from_id_salt((salt, "pick"))
        .selected_text(curve_label(&current))
        .width(ui.available_width().min(260.0))
        .show_ui(ui, |ui| {
            for e in Ease::ALL {
                let c = Curve::Preset(e);
                if ui.selectable_label(current == c, e.name()).clicked() {
                    out = Some(c);
                }
            }
            ui.separator();
            let is_bezier = matches!(current, Curve::Bezier { .. });
            if ui
                .selectable_label(is_bezier, ui.t("curve.custom"))
                .clicked()
                && !is_bezier
            {
                out = Some(match &current {
                    Curve::Preset(Ease::Linear) | Curve::Named(_) => Curve::bezier(0.25, 0.25, 0.75, 0.75),
                    _ => Curve::css_ease(),
                });
            }
            if options.allow_named && !named.is_empty() {
                ui.separator();
                for name in named.keys() {
                    let c = Curve::Named(name.clone());
                    if ui.selectable_label(current == c, name).clicked() {
                        out = Some(c);
                    }
                }
            }
        });
    ui.horizontal_wrapped(|ui| {
        for (key, preset) in bezier_presets() {
            if ui.small_button(ui.t(key)).clicked() {
                out = Some(preset);
            }
        }
    });

    let current = out.as_ref().unwrap_or(curve).clone();

    // ---- numeric fields for a Bézier
    if let Curve::Bezier { p1, p2 } = &current {
        let mut v = [p1[0], p1[1], p2[0], p2[1]];
        ui.horizontal(|ui| {
            for (i, label) in ["x1", "y1", "x2", "y2"].into_iter().enumerate() {
                let range = if i % 2 == 0 { 0.0..=1.0 } else { Y_RANGE };
                ui.label(RichText::new(label).small().color(ui.visuals().weak_text_color()));
                ui.add(DragValue::new(&mut v[i]).speed(0.01).range(range).max_decimals(3));
            }
        });
        if v != [p1[0], p1[1], p2[0], p2[1]] {
            out = Some(Curve::Bezier { p1: [v[0], v[1]], p2: [v[2], v[3]] });
        }
    }

    // ---- graph
    let current = out.as_ref().unwrap_or(curve).clone();
    let samples = sample(&current, named);
    let range = graph_range(&samples);
    let size = options.graph_size.max(60.0);
    let (outer, _) = ui.allocate_exact_size(vec2(size + 24.0, size + 24.0), Sense::hover());
    let rect = Rect::from_center_size(outer.center(), Vec2::splat(size));
    let painter = ui.painter_at(outer);
    let visuals = ui.visuals().clone();
    painter.rect_filled(rect, CornerRadius::same(4), visuals.extreme_bg_color);
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
        StrokeKind::Inside,
    );
    let grid = Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color.gamma_multiply(0.5));
    for v in [0.0f32, 1.0] {
        painter.line_segment(
            [to_screen(rect, range, 0.0, v), to_screen(rect, range, 1.0, v)],
            grid,
        );
    }
    let accent = visuals.selection.bg_fill;
    let line = Stroke::new(2.0, visuals.hyperlink_color);
    let points: Vec<Pos2> = samples
        .iter()
        .enumerate()
        .map(|(i, v)| to_screen(rect, range, i as f32 / SAMPLES as f32, *v))
        .collect();
    painter.add(Shape::line(points, line));

    if let Curve::Bezier { p1, p2 } = &current {
        let c0 = to_screen(rect, range, 0.0, 0.0);
        let c1 = to_screen(rect, range, 1.0, 1.0);
        let h = [
            to_screen(rect, range, p1[0], p1[1]),
            to_screen(rect, range, p2[0], p2[1]),
        ];
        let stem = Stroke::new(1.0, visuals.weak_text_color());
        painter.line_segment([c0, h[0]], stem);
        painter.line_segment([c1, h[1]], stem);
        for (i, pos) in h.iter().enumerate() {
            let id = ui.make_persistent_id((salt, "handle", i));
            let hit = Rect::from_center_size(*pos, Vec2::splat(18.0));
            let resp = ui.interact(hit, id, Sense::drag());
            let active = resp.hovered() || resp.dragged();
            painter.circle_filled(*pos, if active { 6.0 } else { 5.0 }, accent);
            painter.circle_stroke(*pos, 6.0, Stroke::new(1.5, visuals.strong_text_color()));
            if resp.dragged()
                && let Some(p) = resp.interact_pointer_pos()
            {
                let (x, y) = from_screen(rect, range, p);
                let next = with_handle(&current, i, x, y);
                if next != current {
                    out = Some(next);
                }
            }
        }
    }

    // ---- animated preview
    if options.preview {
        let width = ui.available_width().clamp(120.0, 300.0);
        let (bar, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
        let track = Rect::from_center_size(bar.center(), vec2(bar.width() - 16.0, 4.0));
        let p = ui.painter_at(bar);
        p.rect_filled(track, CornerRadius::same(2), visuals.extreme_bg_color);
        let time = ui.ctx().input(|i| i.time);
        let t = preview_progress(time);
        let v = current.eval_with(t, |n| named.get(n));
        let x = track.left() + v * track.width();
        p.circle_filled(pos2(x, bar.center().y), 7.0, Color32::from_rgb(236, 168, 70));
        ui.ctx().request_repaint();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_mapping_round_trips() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 80.0));
        let range = (-0.3, 1.4);
        let p = to_screen(rect, range, 0.25, 0.8);
        let (x, y) = from_screen(rect, range, p);
        assert!((x - 0.25).abs() < 1e-5 && (y - 0.8).abs() < 1e-5);
        // Curve y = 0 is the bottom edge when the range starts at 0.
        assert_eq!(to_screen(rect, (0.0, 1.0), 0.0, 0.0).y, rect.bottom());
        assert_eq!(to_screen(rect, (0.0, 1.0), 1.0, 1.0).y, rect.top());
    }

    #[test]
    fn dragging_a_handle_keeps_the_curve_valid() {
        let c = Curve::css_ease();
        let Curve::Bezier { p1, p2 } = with_handle(&c, 0, -5.0, 99.0) else {
            panic!("still a bezier")
        };
        assert_eq!(p1, [0.0, *Y_RANGE.end()]);
        assert_eq!(p2, [0.25, 1.0]);
        // A preset has no handles: unchanged.
        assert_eq!(with_handle(&Curve::LINEAR, 0, 0.5, 0.5), Curve::LINEAR);
    }

    #[test]
    fn the_graph_shows_overshoot() {
        let plain = sample(&Curve::LINEAR, &BTreeMap::new());
        assert_eq!(graph_range(&plain), (-0.05, 1.05));
        let back = sample(&Curve::Preset(Ease::OutBack), &BTreeMap::new());
        let (lo, hi) = graph_range(&back);
        assert!(hi > 1.05 && lo <= 0.0);
    }

    #[test]
    fn preview_runs_then_pauses_on_the_end() {
        assert_eq!(preview_progress(0.0), 0.0);
        assert!((preview_progress(PREVIEW_RUN / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(preview_progress(PREVIEW_RUN + 0.1), 1.0);
        assert_eq!(preview_progress(PREVIEW_RUN + PREVIEW_PAUSE), 0.0);
    }

    #[test]
    fn named_curves_resolve_without_chains() {
        let mut named = BTreeMap::new();
        named.insert("a".to_owned(), Curve::css_ease_in());
        named.insert("loop".to_owned(), Curve::Named("a".into()));
        assert_eq!(resolve(&Curve::Named("a".into()), &named), &Curve::css_ease_in());
        assert_eq!(resolve(&Curve::Named("loop".into()), &named), &Curve::LINEAR);
        assert_eq!(resolve(&Curve::Named("nope".into()), &named), &Curve::LINEAR);
    }

    #[test]
    fn the_widget_draws_and_returns_a_picked_preset() {
        let ctx = egui::Context::default();
        let named = BTreeMap::new();
        let mut result = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            result = curve_editor(
                ui,
                "t",
                &Curve::css_ease(),
                &named,
                CurveEditorOptions::default(),
            );
        });
        output.textures_delta.clear();
        assert_eq!(result, None, "no interaction, no change");
    }
}
