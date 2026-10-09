//! Point & click navigation in the player: a `Hotspots` node waiting for the
//! player (the VM sees a choice), its zones hovered, focused with the
//! keyboard / gamepad and activated, and drawn over the stage as a tinted
//! fill with a soft glowing outline (or the zone's hover image).
//!
//! Everything is computed from the map (a few zones): no texture, no shader,
//! two small meshes per highlighted zone, nothing at all when the story is
//! not on a map.

use orbit_core::hotspot::{pulse, triangulate};
use orbit_core::{AssetId, HotspotStyle};
use orbit_render2d::{Layer, Mesh2d, MeshVertex, Scene, TextureId};
use orbit_vm::HotspotChoice;

use crate::stage::CANVAS;

/// Segments of an ellipse outline.
const ELLIPSE_SEGMENTS: usize = 48;
/// Seconds for a highlight to fade in.
const FADE_IN: f32 = 0.15;

/// A hotspot map on screen.
#[derive(Debug)]
pub(crate) struct HotspotPlay {
    pub choice: HotspotChoice,
    /// Tooltip of each hotspot (index into the map), translated.
    pub tooltips: Vec<String>,
    /// The highlighted hotspot (pointer or keyboard), an index into the map.
    pub hover: Option<usize>,
    /// Every zone is outlined (the "show hotspots" key).
    pub show_all: bool,
    /// Seconds since the map was shown (pulse, shimmer).
    pub time: f64,
    /// Seconds since `hover` changed (fade-in).
    pub hover_time: f32,
    /// A hover sound waiting to be played.
    pub sound: Option<AssetId>,
}

impl HotspotPlay {
    pub fn new(choice: HotspotChoice, tooltips: Vec<String>, show_all: bool) -> Self {
        Self {
            choice,
            tooltips,
            hover: None,
            show_all,
            time: 0.0,
            hover_time: 0.0,
            sound: None,
        }
    }

    fn norm(p: [f32; 2]) -> (f32, f32) {
        (p[0] / CANVAS[0], p[1] / CANVAS[1])
    }

    fn set_hover(&mut self, h: Option<usize>) {
        if h != self.hover {
            self.hover = h;
            self.hover_time = 0.0;
            self.sound = h.and_then(|i| self.choice.map.hotspots[i].hover_sound);
        }
    }

    /// The `Vm::choose` index of hotspot `i`, when it is clickable.
    fn choose_index(&self, i: usize) -> Option<usize> {
        self.choice.active.iter().position(|&a| a == i)
    }

    /// The clickable hotspot under a virtual point.
    pub fn hit(&self, p: [f32; 2]) -> Option<usize> {
        let (x, y) = Self::norm(p);
        self.choice.map.hit(&self.choice.active, x, y)
    }

    /// Pointer moved: highlights the zone under it.
    pub fn pointer(&mut self, p: [f32; 2]) {
        let h = self.hit(p);
        self.set_hover(h);
    }

    /// Click / tap: the `Vm::choose` index of the zone under the point.
    pub fn click(&mut self, p: [f32; 2]) -> Option<usize> {
        let h = self.hit(p)?;
        self.set_hover(Some(h));
        self.choose_index(h)
    }

    /// Tab / Shift+Tab: next or previous zone in reading order.
    pub fn cycle(&mut self, forward: bool) {
        let next = self.choice.map.next_focus(&self.choice.active, self.hover, forward);
        self.set_hover(next);
    }

    /// Arrow keys / D-pad: the nearest zone that way.
    pub fn direction(&mut self, dx: f32, dy: f32) {
        let next = self.choice.map.directional_focus(&self.choice.active, self.hover, dx, dy);
        self.set_hover(next);
    }

    /// Enter / A: the highlighted zone, else the first one is focused.
    pub fn confirm(&mut self) -> Option<usize> {
        match self.hover.and_then(|h| self.choose_index(h)) {
            Some(i) => Some(i),
            None => {
                self.cycle(true);
                None
            }
        }
    }

    pub fn update(&mut self, dt: f32) {
        self.time += f64::from(dt);
        self.hover_time += dt;
    }

    /// The pointer should be a hand (over a clickable zone with that style).
    pub fn wants_hand(&self) -> bool {
        self.hover
            .is_some_and(|h| self.choice.map.style_of(h).hand_cursor)
    }

    /// Tooltip of the highlighted zone (empty: none).
    pub fn tooltip(&self) -> Option<&str> {
        self.hover
            .and_then(|h| self.tooltips.get(h))
            .map(String::as_str)
            .filter(|t| !t.is_empty())
    }

    /// Anything moves (pulse, shimmer, fade): keep drawing frames.
    pub fn is_animated(&self, reduced_motion: bool) -> bool {
        if self.hover.is_some() && self.hover_time < FADE_IN {
            return true;
        }
        if reduced_motion {
            return false;
        }
        let map = &self.choice.map;
        self.choice.active.iter().any(|&i| {
            let s = map.style_of(i);
            (Some(i) == self.hover && s.pulse_speed > 0.0) || s.idle_hint > 0.0
        })
    }

    /// Draws the highlights into the stage scene. `texture` resolves hover
    /// images (`None`: not loaded, the procedural glow is drawn instead).
    pub fn draw(
        &self,
        scene: &mut Scene,
        white: TextureId,
        reduced_motion: bool,
        texture: impl Fn(AssetId) -> Option<TextureId>,
    ) {
        let map = &self.choice.map;
        let time = if reduced_motion { 0.0 } else { self.time };
        let fade = (self.hover_time / FADE_IN).clamp(0.0, 1.0);
        let fade = if reduced_motion { 1.0 } else { fade };
        let mut z = 0.0;
        let mut draw_zone = |i: usize, scene: &mut Scene| {
            let h = &map.hotspots[i];
            let style = map.style_of(i);
            let outline = to_px(&h.shape.outline(ELLIPSE_SEGMENTS));
            let hovered = Some(i) == self.hover;
            let disabled = self.choice.disabled.contains(&i);
            z += 0.01;
            if hovered {
                if let Some(tex) = h.hover_image.and_then(&texture) {
                    scene.push_mesh(fill_mesh(&outline, tex, [1.0, 1.0, 1.0, fade], true).z(z));
                    return;
                }
                let color = linear(style, style.fill_alpha * fade);
                scene.push_mesh(fill_mesh(&outline, white, color, false).z(z));
                let a = style.intensity * pulse(style, time) * fade;
                scene.push_mesh(glow_mesh(&outline, white, style, a).z(z + 0.001));
                return;
            }
            let idle = if self.show_all {
                style.intensity.max(0.35) * 0.6
            } else {
                // A faint shimmer running around the zones.
                let phase = (time * 1.7 + i as f64 * 0.9).sin() as f32;
                style.idle_hint * (0.65 + 0.35 * phase)
            };
            let idle = if disabled { idle * 0.4 } else { idle };
            if idle > 0.003 {
                let mut thin = style.clone();
                thin.glow_width = style.glow_width * 0.6;
                if disabled {
                    thin.color = orbit_core::Color { r: 150, g: 150, b: 150 };
                }
                scene.push_mesh(glow_mesh(&outline, white, &thin, idle).z(z));
            }
        };
        for &i in self.choice.active.iter().chain(&self.choice.disabled) {
            if i < map.hotspots.len() {
                draw_zone(i, scene);
            }
        }
    }
}

fn to_px(points: &[(f32, f32)]) -> Vec<[f32; 2]> {
    points.iter().map(|&(x, y)| [x * CANVAS[0], y * CANVAS[1]]).collect()
}

fn linear(style: &HotspotStyle, alpha: f32) -> [f32; 4] {
    let c = |v: u8| (f32::from(v) / 255.0).powf(2.2);
    [c(style.color.r), c(style.color.g), c(style.color.b), alpha.clamp(0.0, 1.0)]
}

/// The zone filled with `color` (white texture) or with the matching part of
/// a full-screen image (`screen_uv`).
fn fill_mesh(outline: &[[f32; 2]], texture: TextureId, color: [f32; 4], screen_uv: bool) -> Mesh2d {
    let pts: Vec<(f32, f32)> = outline.iter().map(|p| (p[0], p[1])).collect();
    let vertices = outline
        .iter()
        .map(|&p| MeshVertex {
            position: p,
            uv: if screen_uv { [p[0] / CANVAS[0], p[1] / CANVAS[1]] } else { [0.5, 0.5] },
            color,
        })
        .collect();
    Mesh2d::new(texture, vertices, triangulate(&pts)).layer(Layer::OVERLAY)
}

/// A soft band along the outline: transparent inside, `alpha` on the edge,
/// fading out over `glow_width` virtual pixels outside (and half of it
/// inside).
fn glow_mesh(outline: &[[f32; 2]], white: TextureId, style: &HotspotStyle, alpha: f32) -> Mesh2d {
    let n = outline.len();
    let mut vertices = Vec::with_capacity(n * 3);
    let mut indices = Vec::with_capacity(n * 12);
    if n >= 2 {
        // Orientation: outward normals point away from the inside.
        let area: f32 = (0..n)
            .map(|i| {
                let (a, b) = (outline[i], outline[(i + 1) % n]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum();
        let sign = if area >= 0.0 { 1.0 } else { -1.0 };
        let normal = |a: [f32; 2], b: [f32; 2]| {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            // In Y-down screen space a positive area is clockwise.
            [sign * dy / len, -sign * dx / len]
        };
        let width = style.glow_width.max(0.5);
        let core = linear(style, alpha);
        let clear = [core[0], core[1], core[2], 0.0];
        for i in 0..n {
            let prev = outline[(i + n - 1) % n];
            let p = outline[i];
            let next = outline[(i + 1) % n];
            let (n1, n2) = (normal(prev, p), normal(p, next));
            let mut m = [n1[0] + n2[0], n1[1] + n2[1]];
            let len = (m[0] * m[0] + m[1] * m[1]).sqrt();
            if len < 1e-4 {
                m = n2;
            } else {
                // Miter, limited to twice the width on sharp corners.
                let cos = (len * 0.5).max(0.5);
                m = [m[0] / len / cos, m[1] / len / cos];
            }
            let at = |d: f32| [p[0] + m[0] * d, p[1] + m[1] * d];
            vertices.push(MeshVertex { position: at(-width * 0.5), uv: [0.5, 0.5], color: clear });
            vertices.push(MeshVertex { position: p, uv: [0.5, 0.5], color: core });
            vertices.push(MeshVertex { position: at(width), uv: [0.5, 0.5], color: clear });
        }
        for i in 0..n {
            let (a, b) = ((i * 3) as u32, (((i + 1) % n) * 3) as u32);
            for ring in 0..2 {
                let (a0, a1, b0, b1) = (a + ring, a + ring + 1, b + ring, b + ring + 1);
                indices.extend([a0, b0, a1, a1, b0, b1]);
            }
        }
    }
    Mesh2d::new(white, vertices, indices).layer(Layer::OVERLAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::{Hotspot, HotspotMap, HotspotShape, NodeId, StringId};

    fn play() -> HotspotPlay {
        let spot = |shape| Hotspot {
            name: String::new(),
            text_id: StringId::new(),
            shape,
            target: NodeId::new(),
            condition: None,
            enabled: None,
            style: None,
            hover_image: None,
            hover_sound: Some(AssetId::new()),
        };
        let map = HotspotMap {
            hotspots: vec![
                spot(HotspotShape::Rect { x: 0.0, y: 0.0, w: 0.25, h: 0.25 }),
                spot(HotspotShape::Ellipse { cx: 0.75, cy: 0.25, rx: 0.1, ry: 0.1 }),
                spot(HotspotShape::Polygon { points: vec![(0.4, 0.6), (0.6, 0.6), (0.5, 0.9)] }),
                spot(HotspotShape::Rect { x: 0.7, y: 0.7, w: 0.2, h: 0.2 }),
            ],
            ..Default::default()
        };
        let choice = HotspotChoice {
            node: NodeId::new(),
            map,
            active: vec![0, 1, 2],
            disabled: vec![3],
        };
        HotspotPlay::new(choice, vec!["a".into(), "b".into(), String::new(), "d".into()], false)
    }

    #[test]
    fn pointer_hover_and_click_map_to_choose_indices() {
        let mut p = play();
        p.pointer([100.0, 100.0]);
        assert_eq!(p.hover, Some(0));
        assert!(p.sound.is_some(), "entering a zone plays its sound");
        assert_eq!(p.tooltip(), Some("a"));
        assert!(p.wants_hand());
        // The disabled zone does not react.
        assert_eq!(p.click([0.8 * 1920.0, 0.8 * 1080.0]), None);
        assert_eq!(p.click([0.5 * 1920.0, 0.7 * 1080.0]), Some(2));
        assert_eq!(p.tooltip(), None, "empty tooltips are not shown");
        p.pointer([1900.0, 1000.0]);
        assert_eq!(p.hover, None);
        assert!(!p.wants_hand());
    }

    #[test]
    fn keyboard_focus_cycles_and_confirms() {
        let mut p = play();
        // Enter with nothing focused focuses the first zone.
        assert_eq!(p.confirm(), None);
        assert_eq!(p.hover, Some(0));
        p.cycle(true);
        assert_eq!(p.hover, Some(1));
        p.cycle(true);
        assert_eq!(p.hover, Some(2));
        p.cycle(true);
        assert_eq!(p.hover, Some(0), "wraps, skipping the disabled zone");
        p.direction(1.0, 0.0);
        assert_eq!(p.hover, Some(1));
        assert_eq!(p.confirm(), Some(1));
    }

    #[test]
    fn highlights_are_small_meshes_over_the_stage() {
        let mut p = play();
        let white = TextureId::placeholder();
        let mut scene = Scene::new();
        p.draw(&mut scene, white, false, |_| None);
        // Idle shimmer only: one glow per visible zone (3 active + 1 disabled).
        assert_eq!(scene.meshes().len(), 4);
        p.pointer([100.0, 100.0]);
        p.update(1.0);
        let mut scene = Scene::new();
        p.draw(&mut scene, white, false, |_| None);
        // The hovered zone: fill + glow.
        assert_eq!(scene.meshes().len(), 5);
        assert!(scene.meshes().iter().all(|m| m.indices.len() % 3 == 0 && !m.indices.is_empty()));
        // No idle hint: only the hovered zone.
        let mut quiet = play();
        quiet.choice.map.style.idle_hint = 0.0;
        let mut scene = Scene::new();
        quiet.draw(&mut scene, white, false, |_| None);
        assert!(scene.meshes().is_empty());
        assert!(!quiet.is_animated(false));
    }
}
