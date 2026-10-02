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
        }
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
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, sprite: Sprite2d) {
        self.sprites.push(sprite);
    }

    pub fn clear_sprites(&mut self) {
        self.sprites.clear();
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Batch {
    pub texture: TextureId,
    pub first_index: u32,
    pub index_count: u32,
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
            Some(last) if last.texture == s.texture => {
                last.index_count += INDICES_PER_SPRITE as u32;
            }
            _ => batches.push(Batch {
                texture: s.texture,
                first_index: (n * INDICES_PER_SPRITE) as u32,
                index_count: INDICES_PER_SPRITE as u32,
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
