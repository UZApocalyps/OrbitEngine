//! Dynamic glyph atlas and drawing of laid-out text through `orbit_render2d`.

use std::collections::HashMap;

use cosmic_text::CacheKey;
use orbit_render2d::{Layer, RenderError, Renderer2d, Scene, Sprite2d, TextureId, TextureOptions};

use crate::engine::{TextEngine, TextLayout};
use crate::markup::srgb_to_linear;

const ATLAS_SIZE: u32 = 1024;
const PADDING: u32 = 1;

/// Simple shelf packer.
#[derive(Debug, Clone)]
pub(crate) struct ShelfPacker {
    size: u32,
    x: u32,
    y: u32,
    row_height: u32,
}

impl ShelfPacker {
    pub(crate) fn new(size: u32) -> Self {
        Self {
            size,
            x: 0,
            y: 0,
            row_height: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::new(self.size);
    }

    /// Reserves a `w x h` rectangle (plus padding). `None` when full.
    pub(crate) fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (pw, ph) = (w + PADDING, h + PADDING);
        if pw > self.size || ph > self.size {
            return None;
        }
        if self.x + pw > self.size {
            self.y += self.row_height;
            self.x = 0;
            self.row_height = 0;
        }
        if self.y + ph > self.size {
            return None;
        }
        let pos = (self.x, self.y);
        self.x += pw;
        self.row_height = self.row_height.max(ph);
        Some(pos)
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    left: i32,
    top: i32,
}

/// How to place a layout on the scene.
#[derive(Debug, Clone, Copy)]
pub struct DrawParams {
    /// Top-left of the layout box, in world (virtual) pixels.
    pub origin: [f32; 2],
    pub layer: Layer,
    pub z: f32,
    /// Number of graphemes to show (typewriter); `usize::MAX` shows all.
    pub revealed: usize,
    /// Extra opacity multiplier.
    pub alpha: f32,
}

impl Default for DrawParams {
    fn default() -> Self {
        Self {
            origin: [0.0, 0.0],
            layer: Layer::UI,
            z: 1.0,
            revealed: usize::MAX,
            alpha: 1.0,
        }
    }
}

/// GPU glyph cache + drawing. One per [`Renderer2d`].
pub struct TextRenderer {
    texture: TextureId,
    packer: ShelfPacker,
    entries: HashMap<CacheKey, Option<Entry>>,
    generation: u32,
}

impl TextRenderer {
    pub fn new(renderer: &mut Renderer2d) -> Result<Self, RenderError> {
        let size = ATLAS_SIZE.min(renderer.gfx().max_texture_dimension());
        // sRGB so color glyphs (emoji) decode correctly; white masks are
        // unaffected and the alpha channel is always linear.
        let texture =
            renderer.create_empty_texture("glyph atlas", size, size, TextureOptions::COLOR)?;
        Ok(Self {
            texture,
            packer: ShelfPacker::new(size),
            entries: HashMap::new(),
            generation: 0,
        })
    }

    /// Incremented every time the atlas had to be flushed because it was full.
    /// Sprites emitted earlier in the same frame become stale when it changes.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn glyphs_cached(&self) -> usize {
        self.entries.len()
    }

    fn reset(&mut self) {
        log::warn!("glyph atlas full: flushing {} glyphs", self.entries.len());
        self.entries.clear();
        self.packer.reset();
        self.generation += 1;
    }

    /// `Err(())` = atlas full.
    fn lookup(
        &mut self,
        engine: &mut TextEngine,
        renderer: &Renderer2d,
        key: CacheKey,
    ) -> Result<Option<Entry>, ()> {
        if let Some(e) = self.entries.get(&key) {
            return Ok(*e);
        }
        let Some(bmp) = engine.rasterize(key) else {
            self.entries.insert(key, None);
            return Ok(None);
        };
        let Some((x, y)) = self.packer.alloc(bmp.width, bmp.height) else {
            return Err(());
        };
        if let Err(err) =
            renderer.update_texture(self.texture, x, y, bmp.width, bmp.height, &bmp.rgba)
        {
            log::error!("glyph upload failed: {err}");
            self.entries.insert(key, None);
            return Ok(None);
        }
        let entry = Entry {
            x,
            y,
            w: bmp.width,
            h: bmp.height,
            left: bmp.left,
            top: bmp.top,
        };
        self.entries.insert(key, Some(entry));
        Ok(Some(entry))
    }

    /// Appends the visible glyphs of `layout` to `scene` as sprites.
    pub fn draw(
        &mut self,
        engine: &mut TextEngine,
        renderer: &Renderer2d,
        scene: &mut Scene,
        layout: &TextLayout,
        params: &DrawParams,
    ) {
        let size = renderer
            .texture_size(self.texture)
            .unwrap_or([ATLAS_SIZE; 2]);
        let (aw, ah) = (size[0] as f32, size[1] as f32);
        let inv = 1.0 / layout.scale.max(1e-4);
        let mut quads: Vec<Sprite2d> = Vec::with_capacity(layout.glyphs.len());
        // At most one flush-and-retry: a single block always fits an atlas.
        for attempt in 0..2 {
            quads.clear();
            let mut full = false;
            for g in layout
                .glyphs
                .iter()
                .filter(|g| g.grapheme < params.revealed)
            {
                match self.lookup(engine, renderer, g.key) {
                    Err(()) => {
                        full = true;
                        break;
                    }
                    Ok(None) => {}
                    Ok(Some(e)) => {
                        let c = g.color;
                        let tint = [
                            srgb_to_linear(c[0]),
                            srgb_to_linear(c[1]),
                            srgb_to_linear(c[2]),
                            c[3] as f32 / 255.0 * params.alpha,
                        ];
                        let px = (g.x + e.left) as f32 * inv + params.origin[0];
                        let py = (g.y - e.top) as f32 * inv + params.origin[1];
                        quads.push(
                            Sprite2d::new(
                                self.texture,
                                [px, py],
                                [e.w as f32 * inv, e.h as f32 * inv],
                            )
                            .anchor([0.0, 0.0])
                            .layer(params.layer)
                            .z(params.z)
                            .tint(tint)
                            .uv([
                                e.x as f32 / aw,
                                e.y as f32 / ah,
                                (e.x + e.w) as f32 / aw,
                                (e.y + e.h) as f32 / ah,
                            ]),
                        );
                    }
                }
            }
            if !full {
                break;
            }
            if attempt == 0 {
                self.reset();
            } else {
                log::error!("text block does not fit in the glyph atlas; drawing it partially");
            }
        }
        for q in quads {
            scene.push(q);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packer_fills_rows_then_fails() {
        let mut p = ShelfPacker::new(32);
        assert_eq!(p.alloc(15, 10), Some((0, 0)));
        assert_eq!(p.alloc(15, 10), Some((16, 0)));
        assert_eq!(p.alloc(15, 20), Some((0, 11)));
        assert_eq!(p.alloc(15, 10), Some((16, 11)));
        assert_eq!(p.alloc(15, 10), None); // 11 + 21 > 32
        p.reset();
        assert_eq!(p.alloc(31, 31), Some((0, 0)));
        assert_eq!(p.alloc(40, 1), None);
    }
}
