//! egui pieces shared by the story timeline panel ([`super::timeline`]) and
//! the rig clip editor ([`super::rigedit`]): the ruler, the key lanes, the
//! playhead, wheel navigation and key picking. Neither editor's data model is
//! known here: they hand over plain key times (rows of `f32`) and get back
//! times and `(row, key)` references, so both stay on the same look and feel
//! without duplicating the drawing.

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2,
};

use super::timeline_model::{self as model, KeyRef, TimeView};
use super::ui_util::{accent, danger, weak};

pub const LABEL_W: f32 = 190.0;
pub const RULER_H: f32 = 24.0;
pub const ROW_H: f32 = 28.0;
pub const KEY_R: f32 = 6.0;

pub fn fmt_time(t: f32) -> String {
    let s = format!("{t:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s}s")
}

pub fn diamond(center: Pos2, r: f32) -> Vec<Pos2> {
    vec![
        pos2(center.x, center.y - r),
        pos2(center.x + r, center.y),
        pos2(center.x, center.y + r),
        pos2(center.x - r, center.y),
    ]
}

pub fn row_rect(area: Rect, row: usize) -> Rect {
    Rect::from_min_size(pos2(area.left(), area.top() + ROW_H * row as f32), vec2(area.width(), ROW_H))
}

pub fn key_pos(canvas: Rect, view: &TimeView, row: usize, area: Rect, t: f32) -> Pos2 {
    let r = row_rect(area, row);
    pos2(canvas.left() + view.x_of(t), r.center().y)
}

/// The ruler: ticks, labels, and the shaded part past the end.
pub fn draw_ruler(ui: &Ui, ruler: Rect, view: &TimeView, duration: f32) {
    let p = ui.painter();
    p.rect_filled(ruler, 0.0, ui.visuals().extreme_bg_color);
    let step = model::grid_step(view.pps, 70.0);
    let minor = step / 5.0;
    let t0 = view.time_at(0.0).max(0.0);
    let t1 = view.time_at(ruler.width());
    let first = (t0 / minor).floor() as i64;
    let last = (t1 / minor).ceil() as i64;
    for n in first..=last.min(first + 4000) {
        let t = n as f32 * minor;
        let x = ruler.left() + view.x_of(t);
        if x < ruler.left() || x > ruler.right() {
            continue;
        }
        let major = n % 5 == 0;
        let h = if major { 10.0 } else { 5.0 };
        p.line_segment([pos2(x, ruler.bottom() - h), pos2(x, ruler.bottom())], Stroke::new(1.0, weak(ui)));
        if major {
            p.text(pos2(x + 3.0, ruler.top() + 2.0), Align2::LEFT_TOP, fmt_time(t), FontId::proportional(10.0), weak(ui));
        }
    }
    let end_x = ruler.left() + view.x_of(duration);
    if end_x < ruler.right() {
        let from = end_x.max(ruler.left());
        p.rect_filled(Rect::from_min_max(pos2(from, ruler.top()), ruler.right_bottom()), 0.0, Color32::from_black_alpha(70));
    }
}

/// Click / drag on the ruler: the time under the pointer (clamped to the
/// clip), `None` when nothing happened.
pub fn scrub_ruler(ui: &Ui, ruler: Rect, id: Id, view: &TimeView, duration: f32) -> Option<f32> {
    let resp = ui.interact(ruler, id, Sense::click_and_drag());
    let p = resp.interact_pointer_pos()?;
    (resp.dragged() || resp.clicked() || resp.drag_started())
        .then(|| view.time_at(p.x - ruler.left()).clamp(0.0, duration.max(0.0)))
}

/// Wheel over the ruler and the canvas: Ctrl zooms, Shift or a horizontal
/// wheel scrolls.
pub fn wheel_nav(ui: &Ui, view: &mut TimeView, ruler: Rect, canvas_bottom: f32) {
    let over = ui.rect_contains_pointer(Rect::from_min_max(
        pos2(ruler.left(), ruler.top()),
        pos2(ruler.right(), canvas_bottom.max(ruler.bottom())),
    ));
    if !over {
        return;
    }
    let (scroll, zoom, mods) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers));
    let x = ui.input(|i| i.pointer.hover_pos()).map_or(0.0, |p| p.x - ruler.left());
    if zoom != 1.0 {
        view.zoom(zoom, x);
    } else if mods.ctrl || mods.command {
        if scroll.y != 0.0 {
            view.zoom((scroll.y * 0.005).exp(), x);
        }
    } else if mods.shift || scroll.x != 0.0 {
        view.scroll(-(scroll.x + if mods.shift { scroll.y } else { 0.0 }));
    }
}

/// Row background (selected / zebra) and the shading past the end.
pub fn draw_row_bg(ui: &Ui, r: Rect, canvas: Rect, view: &TimeView, duration: f32, row: usize, selected: bool) {
    let bg = if selected {
        ui.visuals().selection.bg_fill.gamma_multiply(0.35)
    } else if row % 2 == 0 {
        ui.visuals().faint_bg_color
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(r, 0.0, bg);
    let end_x = canvas.left() + view.x_of(duration);
    if end_x < canvas.right() {
        ui.painter().rect_filled(
            Rect::from_min_max(pos2(end_x.max(canvas.left()), r.top()), pos2(canvas.right(), r.bottom())),
            0.0,
            Color32::from_black_alpha(40),
        );
    }
}

/// The segments (`animated[w]`: the value changes between key `w` and
/// `w + 1`) and the key diamonds of one row.
pub fn draw_lane(
    ui: &Ui,
    r: Rect,
    canvas: Rect,
    view: &TimeView,
    times: &[f32],
    animated: &[bool],
    selected: impl Fn(usize) -> bool,
) {
    let accent = accent(ui);
    let lane = Rect::from_min_max(pos2(canvas.left(), r.top()), pos2(canvas.right(), r.bottom()));
    for (w, on) in animated.iter().enumerate() {
        if !*on || w + 1 >= times.len() {
            continue;
        }
        let x0 = canvas.left() + view.x_of(times[w]);
        let x1 = canvas.left() + view.x_of(times[w + 1]);
        let bar = Rect::from_min_max(pos2(x0, r.center().y - 2.0), pos2(x1, r.center().y + 2.0)).intersect(lane);
        if bar.width() > 0.0 {
            ui.painter().rect_filled(bar, CornerRadius::same(2), accent.gamma_multiply(0.45));
        }
    }
    for (ki, &t) in times.iter().enumerate() {
        let c = pos2(canvas.left() + view.x_of(t), r.center().y);
        if c.x < lane.left() - KEY_R || c.x > lane.right() + KEY_R {
            continue;
        }
        let fill = if selected(ki) { accent } else { ui.visuals().widgets.inactive.fg_stroke.color };
        ui.painter().add(Shape::convex_polygon(diamond(c, KEY_R), fill, Stroke::new(1.0, Color32::from_black_alpha(160))));
    }
}

/// The playhead line over the rows, and the frame of the canvas.
pub fn draw_playhead(ui: &Ui, area: Rect, canvas: Rect, view: &TimeView, t: f32) {
    let visible = ui.clip_rect();
    let x = canvas.left() + view.x_of(t);
    if x >= canvas.left() && x <= canvas.right() {
        let top = area.top().max(visible.top());
        let bottom = area.bottom().min(visible.bottom());
        ui.painter().line_segment([pos2(x, top), pos2(x, bottom)], Stroke::new(1.5, danger(ui)));
    }
    ui.painter().rect_stroke(
        Rect::from_min_max(pos2(canvas.left(), area.top()), area.right_bottom()),
        0.0,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        StrokeKind::Inside,
    );
}

/// The key under `p` (nearest within the key radius), over rows of key times.
pub fn hit_key(canvas: Rect, area: Rect, view: &TimeView, rows: &[Vec<f32>], p: Pos2) -> Option<KeyRef> {
    let mut best: Option<(f32, KeyRef)> = None;
    for (row, times) in rows.iter().enumerate() {
        for (ki, &t) in times.iter().enumerate() {
            let d = (key_pos(canvas, view, row, area, t) - p).length();
            if d <= KEY_R + 3.0 && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, (row, ki)));
            }
        }
    }
    best.map(|(_, k)| k)
}

/// The row under `p`.
pub fn row_at(area: Rect, rows: usize, p: Pos2) -> Option<usize> {
    let row = ((p.y - area.top()) / ROW_H).floor();
    (row >= 0.0 && (row as usize) < rows).then_some(row as usize)
}

/// Keys of `rows` whose diamond lies inside `rect` (marquee selection).
pub fn keys_in_rect(canvas: Rect, area: Rect, view: &TimeView, rows: &[Vec<f32>], rect: Rect) -> Vec<KeyRef> {
    let mut out = Vec::new();
    for (row, times) in rows.iter().enumerate() {
        for (ki, &t) in times.iter().enumerate() {
            if rect.contains(key_pos(canvas, view, row, area, t)) {
                out.push((row, ki));
            }
        }
    }
    out
}

/// The marquee rectangle.
pub fn draw_marquee(ui: &Ui, rect: Rect) {
    ui.painter().rect(rect, 0.0, accent(ui).gamma_multiply(0.15), Stroke::new(1.0, accent(ui)), StrokeKind::Inside);
}

/// Playhead advance for a playing preview: new time and whether it is still
/// playing. Looping wraps past `duration`, otherwise it stops there.
pub fn advance(t: f32, dt: f32, duration: f32, looping: bool) -> (f32, bool) {
    let next = t + dt.max(0.0);
    if duration <= 0.0 {
        return (0.0, false);
    }
    if next < duration {
        (next, true)
    } else if looping {
        (next % duration, true)
    } else {
        (duration, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_formatted_without_trailing_zeros() {
        assert_eq!(fmt_time(1.0), "1s");
        assert_eq!(fmt_time(0.25), "0.25s");
    }

    #[test]
    fn playing_wraps_or_stops_at_the_end() {
        assert_eq!(advance(0.5, 0.25, 1.0, true), (0.75, true));
        let (t, on) = advance(0.9, 0.3, 1.0, true);
        assert!(on && (t - 0.2).abs() < 1e-5);
        assert_eq!(advance(0.9, 0.3, 1.0, false), (1.0, false));
        assert_eq!(advance(0.0, 0.3, 0.0, true), (0.0, false));
    }

    #[test]
    fn picking_finds_the_nearest_key_and_the_row() {
        let view = TimeView { left: 0.0, pps: 100.0 };
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, ROW_H * 2.0));
        let canvas = Rect::from_min_max(pos2(100.0, 0.0), area.right_bottom());
        let rows = vec![vec![0.0, 1.0], vec![0.5]];
        // Second row, key at 0.5 s: x = 100 + 50, y in row 1.
        let p = pos2(150.0, ROW_H * 1.5);
        assert_eq!(hit_key(canvas, area, &view, &rows, p), Some((1, 0)));
        assert_eq!(hit_key(canvas, area, &view, &rows, pos2(300.0, 10.0)), None);
        assert_eq!(row_at(area, 2, p), Some(1));
        assert_eq!(row_at(area, 2, pos2(0.0, ROW_H * 5.0)), None);
        let picked = keys_in_rect(canvas, area, &view, &rows, Rect::from_min_max(pos2(90.0, 0.0), pos2(210.0, ROW_H * 2.0)));
        assert_eq!(picked, vec![(0, 0), (0, 1), (1, 0)]);
    }
}
