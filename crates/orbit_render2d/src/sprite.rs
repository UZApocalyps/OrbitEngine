//! Sprites, layers, scenes and CPU-side batching.

use bytemuck::{Pod, Zeroable};

use crate::TextureId;

/// Draw layer. Lower layers are drawn first. Use the constants for the
/// standard visual-novel stack, or any other value in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Layer(pub u8);

impl Layer {
    pub const BACKGROUND: Layer = Layer(0);
    pub const CHARACTERS: Layer = Layer(10);
    pub const UI: Layer = Layer(20);
    pub const OVERLAY: Layer = Layer(30);
}

/// A sprite to draw. Coordinates are in world (virtual) pixels, Y down.
#[derive(Debug, Clone, PartialEq)]
pub struct Sprite2d {
    pub texture: TextureId,
    pub layer: Layer,
    /// Order inside a layer (lower first). Sprites with equal layer and `z`
    /// keep their submission order.
    pub z: f32,
    /// World position of the anchor point.
    pub position: [f32; 2],
    /// Unscaled size.
    pub size: [f32; 2],
    pub scale: [f32; 2],
    /// Radians, clockwise on screen, around the anchor.
    pub rotation: f32,
    /// Anchor inside the sprite in `0..1` (`[0.5, 0.5]` = center,
    /// `[0.5, 1.0]` = bottom center).
    pub anchor: [f32; 2],
    /// Linear RGBA multiplied with the texture.
    pub tint: [f32; 4],
    /// Sub-region of the texture: `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
    /// Optional scissor rectangle `[x, y, w, h]` in world pixels: the sprite
    /// is only visible inside it. Usually set through [`Scene::push_clip`].
    pub clip: Option<[f32; 4]>,
}

impl Sprite2d {
    pub fn new(texture: TextureId, position: [f32; 2], size: [f32; 2]) -> Self {
        Self {
            texture,
            layer: Layer::CHARACTERS,
            z: 0.0,
            position,
            size,
            scale: [1.0, 1.0],
            rotation: 0.0,
            anchor: [0.5, 0.5],
            tint: [1.0; 4],
            uv: [0.0, 0.0, 1.0, 1.0],
            clip: None,
        }
    }

    /// Restricts the sprite to the world rectangle `[x, y, w, h]`.
    pub fn clip(mut self, rect: [f32; 4]) -> Self {
        self.clip = Some(rect);
        self
    }

    pub fn layer(mut self, layer: Layer) -> Self {
        self.layer = layer;
        self
    }

    pub fn z(mut self, z: f32) -> Self {
        self.z = z;
        self
    }

    pub fn anchor(mut self, anchor: [f32; 2]) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn scale(mut self, scale: [f32; 2]) -> Self {
        self.scale = scale;
        self
    }

    pub fn rotation(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }

    /// Tint color (linear RGB) and alpha.
    pub fn tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }

    pub fn alpha(mut self, alpha: f32) -> Self {
        self.tint[3] = alpha;
        self
    }

    pub fn uv(mut self, uv: [f32; 4]) -> Self {
        self.uv = uv;
        self
    }

    /// Mirrors the picture in place, on the X and/or Y axis, by swapping the
    /// texture coordinates (the quad keeps its footprint, so the anchor and
    /// the bounding box are unchanged). Calling it twice undoes it. Works with
    /// any sub-region set by [`Self::uv`], and does not affect batching.
    pub fn flipped(mut self, flip_x: bool, flip_y: bool) -> Self {
        if flip_x {
            self.uv.swap(0, 2);
        }
        if flip_y {
            self.uv.swap(1, 3);
        }
        self
    }
}

/// A list of sprites drawn together (one scene = one render pass).
#[derive(Debug, Clone, Default)]
pub struct Scene {
    /// Linear RGBA fill of the visible canvas drawn below everything, or
    /// `None` to leave the target as it is.
    pub clear: Option<[f32; 4]>,
    sprites: Vec<Sprite2d>,
    clips: Vec<[f32; 4]>,
}

/// Intersection of two `[x, y, w, h]` rectangles (zero size when disjoint).
pub fn intersect_rects(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

/// The nine quads of a nine-slice picture: `(rect [x, y, w, h], uv [u0, v0,
/// u1, v1])`, row by row. `tex` is the picture size in pixels and `insets`
/// the `[left, top, right, bottom]` border widths in pixels of the picture;
/// the borders are drawn at that size (shrunk together when `rect` is too
/// small for them), the edges and the center stretch. Empty quads are left
/// out.
pub fn nine_slice_quads(
    rect: [f32; 4],
    tex: [f32; 2],
    insets: [f32; 4],
) -> Vec<([f32; 4], [f32; 4])> {
    let [tw, th] = [tex[0].max(1.0), tex[1].max(1.0)];
    // Source borders, never wider than the picture.
    let mut l = insets[0].clamp(0.0, tw);
    let mut r = insets[2].clamp(0.0, tw);
    if l + r > tw {
        let k = tw / (l + r);
        (l, r) = (l * k, r * k);
    }
    let mut t = insets[1].clamp(0.0, th);
    let mut b = insets[3].clamp(0.0, th);
    if t + b > th {
        let k = th / (t + b);
        (t, b) = (t * k, b * k);
    }
    // Destination borders: shrunk when the rectangle is smaller than them.
    let kx = if l + r > rect[2] && l + r > 0.0 { rect[2] / (l + r) } else { 1.0 };
    let ky = if t + b > rect[3] && t + b > 0.0 { rect[3] / (t + b) } else { 1.0 };
    let xs = [rect[0], rect[0] + l * kx, rect[0] + rect[2] - r * kx, rect[0] + rect[2]];
    let ys = [rect[1], rect[1] + t * ky, rect[1] + rect[3] - b * ky, rect[1] + rect[3]];
    let us = [0.0, l / tw, 1.0 - r / tw, 1.0];
    let vs = [0.0, t / th, 1.0 - b / th, 1.0];
    let mut out = Vec::with_capacity(9);
    for j in 0..3 {
        for i in 0..3 {
            let (w, h) = (xs[i + 1] - xs[i], ys[j + 1] - ys[j]);
            if w > 0.0 && h > 0.0 {
                out.push((
                    [xs[i], ys[j], w, h],
                    [us[i], vs[j], us[i + 1], vs[j + 1]],
                ));
            }
        }
    }
    out
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws `texture` (`tex_size` pixels) as a nine-slice picture filling
    /// `rect` (see [`nine_slice_quads`]): nine sprites on `layer` / `z`.
    pub fn push_nine_slice(
        &mut self,
        texture: TextureId,
        tex_size: [f32; 2],
        rect: [f32; 4],
        insets: [f32; 4],
        layer: Layer,
        z: f32,
        tint: [f32; 4],
    ) {
        for (r, uv) in nine_slice_quads(rect, tex_size, insets) {
            self.push(
                Sprite2d::new(texture, [r[0], r[1]], [r[2], r[3]])
                    .anchor([0.0, 0.0])
                    .layer(layer)
                    .z(z)
                    .uv(uv)
                    .tint(tint),
            );
        }
    }

    /// Adds a sprite, clipped to the active clip stack (see
    /// [`Scene::push_clip`]).
    pub fn push(&mut self, mut sprite: Sprite2d) {
        if let Some(&top) = self.clips.last() {
            sprite.clip = Some(match sprite.clip {
                Some(own) => intersect_rects(top, own),
                None => top,
            });
        }
        self.sprites.push(sprite);
    }

    /// Clips every sprite pushed from now on to the world rectangle
    /// `[x, y, w, h]` (intersected with the enclosing clip). Pair with
    /// [`Scene::pop_clip`].
    pub fn push_clip(&mut self, rect: [f32; 4]) {
        let rect = match self.clips.last() {
            Some(&top) => intersect_rects(top, rect),
            None => rect,
        };
        self.clips.push(rect);
    }

    pub fn pop_clip(&mut self) {
        self.clips.pop();
    }

    pub fn clear_sprites(&mut self) {
        self.sprites.clear();
        self.clips.clear();
    }

    pub fn sprites(&self) -> &[Sprite2d] {
        &self.sprites
    }

    pub fn len(&self) -> usize {
        self.sprites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sprites.is_empty()
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub(crate) struct Vertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

pub(crate) const VERTEX_SIZE: u64 = std::mem::size_of::<Vertex>() as u64;
pub(crate) const VERTICES_PER_SPRITE: usize = 4;
pub(crate) const INDICES_PER_SPRITE: usize = 6;

/// One draw call: consecutive quads sharing a texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Batch {
    pub texture: TextureId,
    pub first_index: u32,
    pub index_count: u32,
    pub clip: Option<[f32; 4]>,
}

/// Converts a world clip rectangle to a pixel scissor `[x, y, w, h]` inside
/// `viewport` (itself inside a target of `size`). `visible` is the camera
/// visible world rect `[left, top, right, bottom]`. `None` when nothing is
/// left to draw.
pub(crate) fn clip_to_scissor(
    clip: [f32; 4],
    visible: [f32; 4],
    viewport: crate::camera::Viewport,
    size: [u32; 2],
) -> Option<[u32; 4]> {
    let ww = (visible[2] - visible[0]).max(1e-6);
    let wh = (visible[3] - visible[1]).max(1e-6);
    let (sx, sy) = (viewport.width / ww, viewport.height / wh);
    let px0 = viewport.x + (clip[0] - visible[0]) * sx;
    let py0 = viewport.y + (clip[1] - visible[1]) * sy;
    let px1 = viewport.x + (clip[0] + clip[2] - visible[0]) * sx;
    let py1 = viewport.y + (clip[1] + clip[3] - visible[1]) * sy;
    // Stay inside the letterboxed viewport and the target.
    let x0 = px0.floor().max(viewport.x.max(0.0));
    let y0 = py0.floor().max(viewport.y.max(0.0));
    let x1 = px1.ceil().min((viewport.x + viewport.width).min(size[0] as f32));
    let y1 = py1.ceil().min((viewport.y + viewport.height).min(size[1] as f32));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some([x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32])
}

/// The four corners (TL, TR, BL, BR) of a sprite in world space.
pub(crate) fn corners(s: &Sprite2d) -> [[f32; 2]; 4] {
    let (w, h) = (s.size[0] * s.scale[0], s.size[1] * s.scale[1]);
    let (l, t) = (-s.anchor[0] * w, -s.anchor[1] * h);
    let (r, b) = (l + w, t + h);
    let (sin, cos) = s.rotation.sin_cos();
    let place = |x: f32, y: f32| {
        [
            s.position[0] + x * cos - y * sin,
            s.position[1] + x * sin + y * cos,
        ]
    };
    [place(l, t), place(r, t), place(l, b), place(r, b)]
}

/// Sorts `sprites` by (layer, z) — stable, so submission order breaks ties —
/// and emits quads plus one [`Batch`] per run of equal textures.
pub(crate) fn build_batches(
    sprites: &[Sprite2d],
    order: &mut Vec<usize>,
    vertices: &mut Vec<Vertex>,
    batches: &mut Vec<Batch>,
) {
    order.clear();
    order.extend(0..sprites.len());
    order.sort_by(|&a, &b| {
        let (sa, sb) = (&sprites[a], &sprites[b]);
        sa.layer.cmp(&sb.layer).then(sa.z.total_cmp(&sb.z))
    });

    vertices.clear();
    batches.clear();
    for (n, &i) in order.iter().enumerate() {
        let s = &sprites[i];
        let c = corners(s);
        let [u0, v0, u1, v1] = s.uv;
        let uvs = [[u0, v0], [u1, v0], [u0, v1], [u1, v1]];
        for k in 0..4 {
            vertices.push(Vertex {
                position: c[k],
                uv: uvs[k],
                color: s.tint,
            });
        }
        match batches.last_mut() {
            Some(last) if last.texture == s.texture && last.clip == s.clip => {
                last.index_count += INDICES_PER_SPRITE as u32;
            }
            _ => batches.push(Batch {
                texture: s.texture,
                first_index: (n * INDICES_PER_SPRITE) as u32,
                index_count: INDICES_PER_SPRITE as u32,
                clip: s.clip,
            }),
        }
    }
}

/// Fills `out` with the index pattern for `sprites` quads
/// (`0 1 2 2 1 3` + 4 per quad).
pub(crate) fn fill_indices(out: &mut Vec<u32>, sprites: usize) {
    out.clear();
    out.reserve(sprites * INDICES_PER_SPRITE);
    for i in 0..sprites as u32 {
        let b = i * VERTICES_PER_SPRITE as u32;
        out.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite(tex: u32, layer: Layer, z: f32) -> Sprite2d {
        Sprite2d::new(TextureId(tex), [0.0, 0.0], [10.0, 10.0])
            .layer(layer)
            .z(z)
    }

    #[test]
    fn nine_slice_keeps_the_corners_and_stretches_the_rest() {
        // A 32x32 picture with 8 px borders drawn at 100x60.
        let q = nine_slice_quads([10.0, 20.0, 100.0, 60.0], [32.0, 32.0], [8.0, 8.0, 8.0, 8.0]);
        assert_eq!(q.len(), 9);
        // Top-left corner: 8x8, uv 0..0.25.
        assert_eq!(q[0], ([10.0, 20.0, 8.0, 8.0], [0.0, 0.0, 0.25, 0.25]));
        // Top edge stretches between the corners.
        assert_eq!(q[1], ([18.0, 20.0, 84.0, 8.0], [0.25, 0.0, 0.75, 0.25]));
        // Center fills the rest.
        assert_eq!(q[4], ([18.0, 28.0, 84.0, 44.0], [0.25, 0.25, 0.75, 0.75]));
        // Bottom-right corner.
        assert_eq!(q[8], ([102.0, 72.0, 8.0, 8.0], [0.75, 0.75, 1.0, 1.0]));
        // The quads tile the rectangle exactly.
        let area: f32 = q.iter().map(|(r, _)| r[2] * r[3]).sum();
        assert!((area - 6000.0).abs() < 1e-3);
    }

    #[test]
    fn nine_slice_shrinks_borders_in_a_tiny_rect_and_skips_empty_quads() {
        let q = nine_slice_quads([0.0, 0.0, 8.0, 100.0], [32.0, 32.0], [8.0, 8.0, 8.0, 8.0]);
        let w: f32 = q.iter().map(|(r, _)| r[2]).fold(0.0, f32::max);
        assert!(w <= 4.0 + 1e-4, "borders halve: {w}");
        // Zero insets: a single quad over the whole picture.
        let one = nine_slice_quads([0.0, 0.0, 50.0, 50.0], [32.0, 32.0], [0.0; 4]);
        assert_eq!(one, vec![([0.0, 0.0, 50.0, 50.0], [0.0, 0.0, 1.0, 1.0])]);
    }

    fn run(sprites: &[Sprite2d]) -> (Vec<usize>, Vec<Batch>) {
        let (mut order, mut v, mut b) = (Vec::new(), Vec::new(), Vec::new());
        build_batches(sprites, &mut order, &mut v, &mut b);
        assert_eq!(v.len(), sprites.len() * 4);
        (order, b)
    }

    #[test]
    fn sorts_by_layer_then_z_and_is_stable() {
        let s = [
            sprite(1, Layer::UI, 0.0),
            sprite(1, Layer::BACKGROUND, 0.0),
            sprite(2, Layer::CHARACTERS, 1.0),
            sprite(2, Layer::CHARACTERS, 0.0),
            sprite(3, Layer::CHARACTERS, 0.0),
        ];
        let (order, _) = run(&s);
        assert_eq!(order, vec![1, 3, 4, 2, 0]);
    }

    #[test]
    fn batches_merge_consecutive_textures_only() {
        let s = [
            sprite(1, Layer::BACKGROUND, 0.0),
            sprite(1, Layer::BACKGROUND, 0.0),
            sprite(2, Layer::BACKGROUND, 0.0),
            sprite(1, Layer::BACKGROUND, 0.0),
        ];
        let (_, b) = run(&s);
        assert_eq!(b.len(), 3);
        assert_eq!((b[0].first_index, b[0].index_count), (0, 12));
        assert_eq!((b[1].first_index, b[1].index_count), (12, 6));
        assert_eq!((b[2].first_index, b[2].index_count), (18, 6));
    }

    #[test]
    fn clip_change_breaks_batches() {
        let a = sprite(1, Layer::UI, 0.0);
        let b = sprite(1, Layer::UI, 0.0).clip([0.0, 0.0, 5.0, 5.0]);
        let (_, batches) = run(&[a.clone(), b.clone(), b, a]);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[1].index_count, 12);
        assert_eq!(batches[1].clip, Some([0.0, 0.0, 5.0, 5.0]));
    }

    #[test]
    fn scene_clip_stack_intersects_and_stamps() {
        let mut scene = Scene::new();
        scene.push_clip([0.0, 0.0, 100.0, 100.0]);
        scene.push_clip([50.0, 50.0, 100.0, 100.0]);
        scene.push(sprite(1, Layer::UI, 0.0));
        scene.pop_clip();
        scene.push(sprite(1, Layer::UI, 0.0));
        scene.pop_clip();
        scene.push(sprite(1, Layer::UI, 0.0));
        let s = scene.sprites();
        assert_eq!(s[0].clip, Some([50.0, 50.0, 50.0, 50.0]));
        assert_eq!(s[1].clip, Some([0.0, 0.0, 100.0, 100.0]));
        assert_eq!(s[2].clip, None);
        let none = intersect_rects([0.0, 0.0, 10.0, 10.0], [20.0, 20.0, 5.0, 5.0]);
        assert_eq!(none[2], 0.0);
    }

    #[test]
    fn scissor_follows_viewport_scale_and_clamps() {
        use crate::camera::{Camera2d, Viewport};
        let vis = Camera2d::new(1920.0, 1080.0).visible_rect();
        let vp = Viewport { x: 0.0, y: 0.0, width: 960.0, height: 540.0 };
        let sc = clip_to_scissor([100.0, 100.0, 200.0, 100.0], vis, vp, [960, 540]);
        assert_eq!(sc, Some([50, 50, 100, 50]));
        let sc = clip_to_scissor([-50.0, 1000.0, 200.0, 200.0], vis, vp, [960, 540]);
        assert_eq!(sc, Some([0, 500, 75, 40]));
        assert_eq!(clip_to_scissor([3000.0, 0.0, 10.0, 10.0], vis, vp, [960, 540]), None);
        let vp = Viewport { x: 10.0, y: 20.0, width: 960.0, height: 540.0 };
        let sc = clip_to_scissor([0.0, 0.0, 192.0, 108.0], vis, vp, [1000, 600]);
        assert_eq!(sc, Some([10, 20, 96, 54]));
    }

    #[test]
    fn empty_input_has_no_batches() {
        let (order, b) = run(&[]);
        assert!(order.is_empty() && b.is_empty());
    }

    #[test]
    fn anchor_and_rotation() {
        let s = Sprite2d::new(TextureId(0), [100.0, 100.0], [20.0, 10.0]).anchor([0.0, 0.0]);
        let c = corners(&s);
        assert_eq!(c[0], [100.0, 100.0]);
        assert_eq!(c[3], [120.0, 110.0]);
        let r = corners(&s.clone().rotation(std::f32::consts::FRAC_PI_2));
        assert!((r[1][0] - 100.0).abs() < 1e-4 && (r[1][1] - 120.0).abs() < 1e-4);
        let sc = corners(&s.scale([2.0, 2.0]));
        assert_eq!(sc[3], [140.0, 120.0]);
    }

    #[test]
    fn flipping_swaps_uvs_in_place_and_keeps_batching() {
        let base = Sprite2d::new(TextureId(1), [100.0, 100.0], [20.0, 10.0]).anchor([0.5, 1.0]);
        let flipped = base.clone().flipped(true, false);
        assert_eq!(flipped.uv, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(base.clone().flipped(false, true).uv, [0.0, 1.0, 1.0, 0.0]);
        assert_eq!(flipped.clone().flipped(true, false).uv, base.uv);
        // Same footprint.
        assert_eq!(corners(&flipped), corners(&base));
        // Mirrored and plain sprites of one texture still share a batch, and
        // the vertices carry the swapped coordinates.
        let (mut order, mut v, mut b) = (Vec::new(), Vec::new(), Vec::new());
        build_batches(&[base, flipped], &mut order, &mut v, &mut b);
        assert_eq!(b.len(), 1);
        assert_eq!(v[0].uv, [0.0, 0.0]);
        assert_eq!(v[4].uv, [1.0, 0.0]);
        assert_eq!(v[7].uv, [0.0, 1.0]);
    }

    #[test]
    fn negative_scale_mirrors_around_the_anchor() {
        // The renderer does not cull back faces, so a negative scale is also
        // a valid way to mirror around the anchor.
        let s = Sprite2d::new(TextureId(0), [100.0, 100.0], [20.0, 10.0])
            .anchor([0.5, 1.0])
            .scale([-1.0, 1.0]);
        let c = corners(&s);
        assert_eq!(c[0], [110.0, 90.0]);
        assert_eq!(c[1], [90.0, 90.0]);
    }

    #[test]
    fn index_pattern() {
        let mut idx = Vec::new();
        fill_indices(&mut idx, 2);
        assert_eq!(idx, vec![0, 1, 2, 2, 1, 3, 4, 5, 6, 6, 5, 7]);
    }
}
