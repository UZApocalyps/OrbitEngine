//! Comic speech bubbles (`TextBoxTheme::mode == Bubbles`): where a bubble
//! goes next to its speaker, and its outline (speech, thought, shout) as
//! plain colored triangles.
//!
//! Placement and geometry are pure functions of canvas rectangles so the
//! tests, the runtime and the editor preview agree; only [`push_shapes`]
//! touches a [`Scene`].

use orbit_render2d::{Layer, Mesh2d, MeshVertex, Scene, TextureId};

use crate::theme::{BubbleKind, BubbleTheme, tint};

/// Virtual canvas (same as the dialogue box).
const CANVAS: [f32; 2] = orbit_ui_data::CANVAS;
/// A bubble without a speaker on screen sits this far above the bottom
/// margin.
const OFFSCREEN_LIFT: f32 = 40.0;
/// Radius of the bumps of a thought cloud.
const BUMP: f32 = 16.0;
/// Segments of a full circle.
const CIRCLE_SEGMENTS: usize = 20;

/// The edge of the bubble its tail leaves from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailSide {
    Top,
    Bottom,
    Left,
    Right,
}

/// The tail: from `base` (on the outline) to `tip` (near the speaker).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tail {
    pub side: TailSide,
    pub base: [f32; 2],
    pub tip: [f32; 2],
}

/// Where a bubble goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BubbleLayout {
    /// `[x, y, w, h]` (outline included) on the canvas.
    pub rect: [f32; 4],
    pub tail: Option<Tail>,
}

/// `[left, top, right, bottom]` of the speaker's face: the upper fifth of
/// its sprite, centered, moved by the character's `head_offset`. Its top
/// is kept on screen.
pub fn face_rect(sprite: [f32; 4], head_offset: [f32; 2]) -> [f32; 4] {
    let [l, t, r, b] = sprite;
    let (w, h) = ((r - l).max(1.0), (b - t).max(1.0));
    let fw = (w * 0.4).min(h * 0.22).max(1.0);
    let fh = (h * 0.2).max(1.0);
    let cx = (l + r) * 0.5 + head_offset[0];
    let top = (t + head_offset[1]).max(0.0);
    [cx - fw * 0.5, top, cx + fw * 0.5, top + fh]
}

/// Whether some of `sprite` (`[l, t, r, b]`) is on the canvas.
pub fn on_screen(sprite: [f32; 4]) -> bool {
    let [l, t, r, b] = sprite;
    r > 0.0 && l < CANVAS[0] && b > 0.0 && t < CANVAS[1] && r > l && b > t
}

/// Outer size of a bubble around a `content` block (name + text).
pub fn bubble_size(content: [f32; 2], kind: BubbleKind, theme: &BubbleTheme) -> [f32; 2] {
    let [px, py] = theme.padding;
    let [cw, ch] = content;
    let [w, h] = match kind {
        BubbleKind::Speech => [cw + 2.0 * px, ch + 2.0 * py],
        BubbleKind::Thought => [cw + 2.0 * px + 0.3 * (ch + 2.0 * py) + 2.0 * BUMP, ch + 2.0 * py + 2.0 * BUMP],
        // The text has to fit inside the inner ellipse of the spikes.
        BubbleKind::Shout => [(cw + 2.0 * px) * 1.5, (ch + 2.0 * py) * 1.7],
    };
    let max_w = CANVAS[0] - 2.0 * theme.margin;
    let max_h = CANVAS[1] - 2.0 * theme.margin;
    [w.max(theme.min_width).min(max_w), h.min(max_h)]
}

fn overlap(a: [f32; 4], b: [f32; 4]) -> f32 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    w * h
}

fn ltrb(rect: [f32; 4]) -> [f32; 4] {
    [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]]
}

/// Places a `size` bubble for a speaker whose sprite covers `speaker`
/// (`[l, t, r, b]`, `None` or off screen: the fallback spot at the bottom,
/// without a tail). Tries above the head, then beside the face (towards the
/// middle of the screen first), clamps each inside the screen margins and
/// keeps the one that covers the face least.
pub fn place(size: [f32; 2], speaker: Option<[f32; 4]>, head_offset: [f32; 2], theme: &BubbleTheme) -> BubbleLayout {
    let m = theme.margin;
    let [w, h] = size;
    let clamp = |x: f32, y: f32| -> [f32; 2] {
        [
            x.clamp(m, (CANVAS[0] - m - w).max(m)),
            y.clamp(m, (CANVAS[1] - m - h).max(m)),
        ]
    };
    let Some(sprite) = speaker.filter(|s| on_screen(*s)) else {
        let [x, y] = clamp((CANVAS[0] - w) * 0.5, CANVAS[1] - m - h - OFFSCREEN_LIFT);
        return BubbleLayout { rect: [x, y, w, h], tail: None };
    };
    let face = face_rect(sprite, head_offset);
    let hx = (face[0] + face[2]) * 0.5;
    let hy = face[1];
    let reach = theme.gap + theme.tail_length;
    let side_y = hy - h * 0.75;
    let right = [face[2] + reach, side_y];
    let left = [face[0] - reach - w, side_y];
    let (first, second) = if hx < CANVAS[0] * 0.5 { (right, left) } else { (left, right) };
    let candidates = [[hx - w * 0.5, hy - reach - h], first, second];
    let mut best: Option<([f32; 4], f32)> = None;
    for (i, want) in candidates.into_iter().enumerate() {
        let [x, y] = clamp(want[0], want[1]);
        let rect = [x, y, w, h];
        let moved = ((x - want[0]).powi(2) + (y - want[1]).powi(2)).sqrt();
        let score = overlap(ltrb(rect), face) * 10.0 + overlap(ltrb(rect), sprite) * 0.2 + moved * 2.0 + i as f32 * 50.0;
        if best.is_none_or(|(_, s)| score < s) {
            best = Some((rect, score));
        }
    }
    let rect = best.map_or([m, m, w, h], |(r, _)| r);
    // The tail aims at the point of the face (grown by the gap) closest to
    // the middle of the bubble.
    let g = theme.gap;
    let c = [rect[0] + w * 0.5, rect[1] + h * 0.5];
    let target = [c[0].clamp(face[0] - g, face[2] + g), c[1].clamp(face[1] - g, face[3] + g)];
    BubbleLayout {
        rect,
        tail: tail_towards(rect, target, theme),
    }
}

/// The tail of a bubble at `rect` pointing at `target` (`None` when the
/// target is inside the bubble).
pub fn tail_towards(rect: [f32; 4], target: [f32; 2], theme: &BubbleTheme) -> Option<Tail> {
    let [l, t, r, b] = ltrb(rect);
    let [tx, ty] = target;
    let dx = if tx < l { l - tx } else if tx > r { tx - r } else { 0.0 };
    let dy = if ty < t { t - ty } else if ty > b { ty - b } else { 0.0 };
    if dx <= 0.0 && dy <= 0.0 {
        return None;
    }
    let side = if dy >= dx {
        if ty > b { TailSide::Bottom } else { TailSide::Top }
    } else if tx > r {
        TailSide::Right
    } else {
        TailSide::Left
    };
    let inset = theme.corner_radius.min(rect[2].min(rect[3]) * 0.5) + theme.tail_width * 0.5;
    let along = |v: f32, lo: f32, hi: f32| if lo <= hi { v.clamp(lo, hi) } else { (lo + hi) * 0.5 };
    let base = match side {
        TailSide::Bottom => [along(tx, l + inset, r - inset), b],
        TailSide::Top => [along(tx, l + inset, r - inset), t],
        TailSide::Left => [l, along(ty, t + inset, b - inset)],
        TailSide::Right => [r, along(ty, t + inset, b - inset)],
    };
    let d = [tx - base[0], ty - base[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if len < 1e-3 {
        return None;
    }
    let reach = len.clamp(theme.tail_length * 0.6, theme.tail_length * 2.2);
    Some(Tail {
        side,
        base,
        tip: [base[0] + d[0] / len * reach, base[1] + d[1] / len * reach],
    })
}

/// Where the caption of the narrator goes (top left, no tail).
pub fn caption_rect(size: [f32; 2], theme: &BubbleTheme) -> [f32; 4] {
    [theme.margin, theme.margin, size[0], size[1]]
}

// ------------------------------------------------------------- geometry

/// A filled polygon, star-shaped around its centroid (drawn as a fan from
/// it).
pub type Polygon = Vec<[f32; 2]>;

pub(crate) fn rounded_rect(rect: [f32; 4], radius: f32) -> Polygon {
    let [x, y, w, h] = rect;
    let r = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    if r < 0.5 {
        return vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    }
    let steps = 6;
    let corners = [
        ([x + w - r, y + r], -90.0f32),
        ([x + w - r, y + h - r], 0.0),
        ([x + r, y + h - r], 90.0),
        ([x + r, y + r], 180.0),
    ];
    let mut out = Vec::with_capacity(4 * (steps + 1));
    for (c, start) in corners {
        for i in 0..=steps {
            let a = (start + 90.0 * i as f32 / steps as f32).to_radians();
            out.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
        }
    }
    out
}

fn ellipse(center: [f32; 2], rx: f32, ry: f32) -> Polygon {
    (0..CIRCLE_SEGMENTS)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / CIRCLE_SEGMENTS as f32;
            [center[0] + rx * a.cos(), center[1] + ry * a.sin()]
        })
        .collect()
}

fn grow(rect: [f32; 4], by: f32) -> [f32; 4] {
    [rect[0] - by, rect[1] - by, rect[2] + 2.0 * by, rect[3] + 2.0 * by]
}

/// A spiky burst filling `rect` (`grow`: outline width added outwards).
fn burst(rect: [f32; 4], grow_by: f32) -> Polygon {
    let c = [rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5];
    let (ax, ay) = (rect[2] * 0.5, rect[3] * 0.5);
    let spikes = ((rect[2] + rect[3]) / 70.0).round().clamp(10.0, 28.0) as usize;
    (0..spikes * 2)
        .map(|i| {
            let a = std::f32::consts::PI * i as f32 / spikes as f32 - std::f32::consts::FRAC_PI_2;
            // Uneven spikes look hand drawn: every third one is shorter.
            let k = if i % 2 == 0 {
                if i % 6 == 0 { 0.93 } else { 1.0 }
            } else {
                0.8
            };
            [c[0] + (ax * k + grow_by) * a.cos(), c[1] + (ay * k + grow_by) * a.sin()]
        })
        .collect()
}

/// The tail triangle of `tail` with a base of `width`, grown by `by`
/// (outline) and pushed `inside` into the body so it hides the outline at
/// its root.
fn tail_triangle(tail: &Tail, width: f32, by: f32, inside: f32) -> Polygon {
    let d = [tail.tip[0] - tail.base[0], tail.tip[1] - tail.base[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-3);
    let u = [d[0] / len, d[1] / len];
    let n = [-u[1], u[0]];
    let half = width * 0.5 + by;
    let root = [tail.base[0] - u[0] * inside, tail.base[1] - u[1] * inside];
    // The tip bends a little sideways, like a drawn tail.
    let tip = [
        tail.tip[0] + u[0] * by * 1.8 + n[0] * width * 0.12,
        tail.tip[1] + u[1] * by * 1.8 + n[1] * width * 0.12,
    ];
    vec![
        [root[0] + n[0] * half, root[1] + n[1] * half],
        tip,
        [root[0] - n[0] * half, root[1] - n[1] * half],
    ]
}

/// The two layers of a bubble: outline polygons (drawn first, in the
/// border color) and fill polygons (on top).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shapes {
    pub outline: Vec<Polygon>,
    pub fill: Vec<Polygon>,
}

/// Outline and fill of a `kind` bubble at `layout`.
pub fn shapes(layout: &BubbleLayout, kind: BubbleKind, theme: &BubbleTheme) -> Shapes {
    let bw = theme.border_width.max(0.0);
    let rect = layout.rect;
    let mut s = Shapes::default();
    match kind {
        BubbleKind::Speech => {
            s.outline.push(rounded_rect(grow(rect, bw), theme.corner_radius + bw));
            s.fill.push(rounded_rect(rect, theme.corner_radius));
            if let Some(tail) = &layout.tail {
                s.outline.push(tail_triangle(tail, theme.tail_width, bw, 0.0));
                s.fill.push(tail_triangle(tail, theme.tail_width, 0.0, bw + 1.0));
            }
        }
        BubbleKind::Shout => {
            s.outline.push(burst(rect, bw));
            s.fill.push(burst(rect, 0.0));
            if let Some(tail) = &layout.tail {
                // From inside the burst: the spikes would leave a gap.
                let inner = Tail {
                    base: lerp(tail.base, [rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5], 0.15),
                    ..*tail
                };
                s.outline.push(tail_triangle(&inner, theme.tail_width * 0.7, bw, 0.0));
                s.fill.push(tail_triangle(&inner, theme.tail_width * 0.7, 0.0, bw + 1.0));
            }
        }
        BubbleKind::Thought => {
            let body = grow(rect, -BUMP);
            let r = body[3].min(body[2]) * 0.5;
            let outline = rounded_rect(body, r);
            // Bumps along the outline make the cloud.
            let bumps = cloud_bumps(&outline);
            s.outline.push(rounded_rect(grow(body, bw), r + bw));
            for b in &bumps {
                s.outline.push(ellipse(*b, BUMP + bw, BUMP + bw));
            }
            s.fill.push(outline);
            for b in &bumps {
                s.fill.push(ellipse(*b, BUMP, BUMP));
            }
            if let Some(tail) = &layout.tail {
                let tw = theme.tail_width;
                for (t, radius) in [(0.3, tw * 0.3), (0.65, tw * 0.21), (1.0, tw * 0.13)] {
                    let p = lerp(tail.base, tail.tip, t);
                    s.outline.push(ellipse(p, radius + bw, radius + bw));
                    s.fill.push(ellipse(p, radius, radius));
                }
            }
        }
    }
    s
}

fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Evenly spaced points along a closed outline, about one bump apart.
fn cloud_bumps(outline: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let n = outline.len();
    let seg = |i: usize| {
        let (a, b) = (outline[i], outline[(i + 1) % n]);
        ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
    };
    let total: f32 = (0..n).map(seg).sum();
    let count = (total / (BUMP * 1.6)).round().max(6.0) as usize;
    let step = total / count as f32;
    let mut out = Vec::with_capacity(count);
    let (mut i, mut walked, mut next) = (0usize, 0.0f32, 0.0f32);
    while out.len() < count && i < n {
        let l = seg(i);
        while next <= walked + l && out.len() < count {
            let t = if l > 0.0 { (next - walked) / l } else { 0.0 };
            out.push(lerp(outline[i], outline[(i + 1) % n], t));
            next += step;
        }
        walked += l;
        i += 1;
    }
    out
}

/// Pushes `shapes` on `scene` (UI layer, at `z` and just above), scaled
/// around `pivot` by `scale` and faded by `alpha` (the pop-in).
#[allow(clippy::too_many_arguments)]
pub fn push_shapes(
    scene: &mut Scene,
    white: TextureId,
    shapes: &Shapes,
    fill: [u8; 4],
    border: [u8; 4],
    z: f32,
    pivot: [f32; 2],
    scale: f32,
    alpha: f32,
) {
    let mesh = |polys: &[Polygon], color: [u8; 4], z: f32| -> Option<Mesh2d> {
        if color[3] == 0 || polys.is_empty() {
            return None;
        }
        let mut c = tint(color);
        c[3] *= alpha;
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for poly in polys {
            if poly.len() < 3 {
                continue;
            }
            let center = poly.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
            let center = [center[0] / poly.len() as f32, center[1] / poly.len() as f32];
            let at = |p: [f32; 2]| MeshVertex {
                position: [pivot[0] + (p[0] - pivot[0]) * scale, pivot[1] + (p[1] - pivot[1]) * scale],
                uv: [0.5, 0.5],
                color: c,
            };
            let first = vertices.len() as u32;
            vertices.push(at(center));
            vertices.extend(poly.iter().map(|p| at(*p)));
            let k = poly.len() as u32;
            for i in 0..k {
                indices.extend([first, first + 1 + i, first + 1 + (i + 1) % k]);
            }
        }
        Some(Mesh2d::new(white, vertices, indices).layer(Layer::UI).z(z))
    };
    if let Some(m) = mesh(&shapes.outline, border, z) {
        scene.push_mesh(m);
    }
    if let Some(m) = mesh(&shapes.fill, fill, z + 0.1) {
        scene.push_mesh(m);
    }
}

/// Scale of the pop-in `t` seconds after the bubble appeared (an ease-out
/// with a small overshoot), and its opacity.
pub fn pop(t: f32, duration: f32) -> (f32, f32) {
    if duration <= 0.0 || t >= duration {
        return (1.0, 1.0);
    }
    let p = (t / duration).clamp(0.0, 1.0);
    let c = 1.70158f32;
    let q = p - 1.0;
    let back = 1.0 + (c + 1.0) * q * q * q + c * q * q;
    (0.6 + 0.4 * back, p.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> BubbleTheme {
        BubbleTheme::default()
    }

    /// A sprite standing at `x` (center), `top` to the bottom of the screen.
    fn sprite(x: f32, top: f32) -> [f32; 4] {
        [x - 250.0, top, x + 250.0, 1080.0]
    }

    fn inside_screen(rect: [f32; 4], m: f32) -> bool {
        rect[0] >= m - 0.01
            && rect[1] >= m - 0.01
            && rect[0] + rect[2] <= CANVAS[0] - m + 0.01
            && rect[1] + rect[3] <= CANVAS[1] - m + 0.01
    }

    #[test]
    fn a_short_character_gets_the_bubble_above_with_the_tail_down() {
        let t = theme();
        let s = sprite(960.0, 600.0);
        let l = place([400.0, 120.0], Some(s), [0.0, 0.0], &t);
        let face = face_rect(s, [0.0, 0.0]);
        assert!(l.rect[1] + l.rect[3] <= face[1], "above the head: {l:?}");
        let tail = l.tail.expect("a tail");
        assert_eq!(tail.side, TailSide::Bottom);
        assert!(tail.tip[1] > tail.base[1], "the tail points down");
        assert!((tail.base[0] - 960.0).abs() < 1.0, "centered on the head");
    }

    #[test]
    fn a_tall_character_gets_the_bubble_beside_the_face_towards_the_middle() {
        let t = theme();
        // Head near the top: no room above.
        let s = sprite(400.0, 120.0);
        let l = place([500.0, 160.0], Some(s), [0.0, 0.0], &t);
        let face = face_rect(s, [0.0, 0.0]);
        assert!(l.rect[0] >= face[2], "right of the face (towards the middle): {l:?}");
        assert_eq!(overlap(ltrb(l.rect), face), 0.0, "the face stays visible");
        let tail = l.tail.expect("a tail");
        assert_eq!(tail.side, TailSide::Left);
        assert!(tail.tip[0] < tail.base[0], "the tail points back at the speaker");
        // Mirrored on the right half of the screen.
        let s = sprite(1500.0, 120.0);
        let l = place([500.0, 160.0], Some(s), [0.0, 0.0], &t);
        assert!(l.rect[0] + l.rect[2] <= face_rect(s, [0.0, 0.0])[0]);
        assert_eq!(l.tail.unwrap().side, TailSide::Right);
    }

    #[test]
    fn bubbles_are_clamped_inside_the_screen() {
        let t = theme();
        for x in [-100.0, 0.0, 120.0, 960.0, 1800.0, 2000.0] {
            for top in [-200.0, 0.0, 150.0, 700.0] {
                let l = place([700.0, 260.0], Some([x - 300.0, top, x + 300.0, 1080.0]), [0.0, 0.0], &t);
                assert!(inside_screen(l.rect, t.margin), "x {x} top {top}: {l:?}");
            }
        }
        // Even a bubble wider than the screen.
        let size = bubble_size([5000.0, 100.0], BubbleKind::Speech, &t);
        let l = place(size, Some(sprite(960.0, 300.0)), [0.0, 0.0], &t);
        assert!(inside_screen(l.rect, t.margin));
    }

    #[test]
    fn the_head_offset_moves_the_target() {
        let t = theme();
        let s = sprite(960.0, 600.0);
        let a = place([400.0, 120.0], Some(s), [0.0, 0.0], &t);
        let b = place([400.0, 120.0], Some(s), [80.0, 40.0], &t);
        assert!(b.tail.unwrap().base[0] > a.tail.unwrap().base[0]);
        assert!(b.rect[1] > a.rect[1]);
    }

    #[test]
    fn without_a_speaker_on_screen_the_bubble_has_no_tail_and_sits_low() {
        let t = theme();
        for speaker in [None, Some([-900.0, 100.0, -400.0, 1080.0]), Some([2100.0, 0.0, 2500.0, 1080.0])] {
            let l = place([600.0, 140.0], speaker, [0.0, 0.0], &t);
            assert!(l.tail.is_none());
            assert!(inside_screen(l.rect, t.margin));
            assert!(l.rect[1] > 540.0, "bottom half");
            assert!((l.rect[0] + 300.0 - 960.0).abs() < 1.0, "centered");
        }
    }

    #[test]
    fn size_wraps_and_grows_with_the_kind() {
        let t = theme();
        let speech = bubble_size([300.0, 80.0], BubbleKind::Speech, &t);
        assert_eq!(speech, [300.0 + 2.0 * t.padding[0], 80.0 + 2.0 * t.padding[1]]);
        assert!(bubble_size([10.0, 40.0], BubbleKind::Speech, &t)[0] >= t.min_width);
        let thought = bubble_size([300.0, 80.0], BubbleKind::Thought, &t);
        let shout = bubble_size([300.0, 80.0], BubbleKind::Shout, &t);
        assert!(thought[0] > speech[0] && shout[0] > speech[0]);
    }

    #[test]
    fn a_target_inside_the_bubble_has_no_tail() {
        let t = theme();
        assert!(tail_towards([100.0, 100.0, 200.0, 100.0], [150.0, 150.0], &t).is_none());
        let tail = tail_towards([100.0, 100.0, 200.0, 100.0], [600.0, 160.0], &t).unwrap();
        assert_eq!(tail.side, TailSide::Right);
        let len = ((tail.tip[0] - tail.base[0]).powi(2) + (tail.tip[1] - tail.base[1]).powi(2)).sqrt();
        assert!(len <= t.tail_length * 2.2 + 0.01, "a far target gets a capped tail");
    }

    #[test]
    fn every_kind_has_an_outline_and_a_fill() {
        let t = theme();
        let layout = place([400.0, 150.0], Some(sprite(960.0, 600.0)), [0.0, 0.0], &t);
        for kind in [BubbleKind::Speech, BubbleKind::Thought, BubbleKind::Shout] {
            let s = shapes(&layout, kind, &t);
            assert!(!s.outline.is_empty() && !s.fill.is_empty(), "{kind:?}");
            assert!(s.fill.iter().all(|p| p.len() >= 3));
        }
        // Thought: three circles for the tail.
        let speech = shapes(&layout, BubbleKind::Speech, &t);
        let thought = shapes(&layout, BubbleKind::Thought, &t);
        assert_eq!(speech.fill.len(), 2);
        assert!(thought.fill.len() > 6);
    }

    #[test]
    fn the_pop_ends_at_full_size() {
        assert_eq!(pop(0.5, 0.2), (1.0, 1.0));
        assert_eq!(pop(0.0, 0.0), (1.0, 1.0));
        let (s, a) = pop(0.0, 0.2);
        assert!(s < 1.0 && a == 0.0);
    }

    #[test]
    fn the_kind_comes_from_the_line_tag() {
        assert_eq!(BubbleKind::of_line("Hello"), BubbleKind::Speech);
        assert_eq!(BubbleKind::of_line("{bubble=thought}Hmm"), BubbleKind::Thought);
        assert_eq!(BubbleKind::of_line("{b}NO{/b}{bubble=shout}!"), BubbleKind::Shout);
        assert_eq!(BubbleKind::of_line("{bubble=wat}{bubble=shout}x"), BubbleKind::Shout);
    }
}
