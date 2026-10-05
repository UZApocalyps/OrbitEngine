//! Painting a style background ([`Fill`]) on a scene: shared by the widget
//! toolkit, the dialogue box, the choices and the quick menu so that a
//! style looks the same everywhere.

use std::collections::HashMap;

use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId};

use crate::theme::{Fill, Shadow, tint};

/// Bands a vertical gradient is drawn with.
const GRADIENT_BANDS: usize = 24;

/// What painting needs from the host.
pub struct Painter<'a> {
    pub renderer: &'a Renderer2d,
    pub white: TextureId,
    /// Loaded project images, by asset path.
    pub images: &'a HashMap<String, TextureId>,
    /// Opacity multiplier (screen fades).
    pub alpha: f32,
}

impl Painter<'_> {
    /// A plain rectangle.
    pub fn quad(&self, scene: &mut Scene, rect: [f32; 4], color: [u8; 4], layer: Layer, z: f32) {
        if rect[2] <= 0.0 || rect[3] <= 0.0 || color[3] == 0 {
            return;
        }
        let mut t = tint(color);
        t[3] *= self.alpha;
        scene.push(
            Sprite2d::new(self.white, [rect[0], rect[1]], [rect[2], rect[3]])
                .anchor([0.0, 0.0])
                .layer(layer)
                .z(z)
                .tint(t),
        );
    }

    /// A frame of `width` inside `rect`.
    pub fn border(&self, scene: &mut Scene, rect: [f32; 4], width: f32, color: [u8; 4], layer: Layer, z: f32) {
        if width <= 0.0 {
            return;
        }
        let [x, y, w, h] = rect;
        self.quad(scene, [x, y, w, width], color, layer, z);
        self.quad(scene, [x, y + h - width, w, width], color, layer, z);
        self.quad(scene, [x, y + width, width, h - 2.0 * width], color, layer, z);
        self.quad(scene, [x + w - width, y + width, width, h - 2.0 * width], color, layer, z);
    }

    /// The shadow of a `rect` (drawn just under `z`).
    pub fn shadow(&self, scene: &mut Scene, shadow: &Shadow, rect: [f32; 4], layer: Layer, z: f32) {
        let [dx, dy] = shadow.offset;
        self.quad(scene, [rect[0] + dx, rect[1] + dy, rect[2], rect[3]], shadow.color, layer, z - 0.05);
    }

    /// Paints `fill` over `rect`. Returns `false` when it could not (an
    /// image that is not loaded): the caller then draws its fallback color.
    pub fn fill(&self, scene: &mut Scene, fill: &Fill, rect: [f32; 4], layer: Layer, z: f32) -> bool {
        match fill {
            Fill::Color(c) => {
                self.quad(scene, rect, *c, layer, z);
                true
            }
            Fill::Gradient { top, bottom } => {
                if rect[2] <= 0.0 || rect[3] <= 0.0 {
                    return true;
                }
                let band = rect[3] / GRADIENT_BANDS as f32;
                for i in 0..GRADIENT_BANDS {
                    let t = (i as f32 + 0.5) / GRADIENT_BANDS as f32;
                    let mut c = [0u8; 4];
                    for k in 0..4 {
                        c[k] = (top[k] as f32 + (bottom[k] as f32 - top[k] as f32) * t).round() as u8;
                    }
                    // A hair of overlap hides the seams between bands.
                    let y = rect[1] + i as f32 * band;
                    self.quad(scene, [rect[0], y, rect[2], band + 0.5], c, layer, z);
                }
                true
            }
            Fill::NineSlice { image, insets } => {
                let Some((texture, size)) = self.texture(image) else {
                    return false;
                };
                if rect[2] > 0.0 && rect[3] > 0.0 {
                    scene.push_nine_slice(texture, size, rect, *insets, layer, z, [1.0, 1.0, 1.0, self.alpha]);
                }
                true
            }
            Fill::Image { image, fit } => {
                let Some((texture, size)) = self.texture(image) else {
                    return false;
                };
                let (r, uv) = fit.place(rect, size);
                if r[2] > 0.0 && r[3] > 0.0 {
                    scene.push(
                        Sprite2d::new(texture, [r[0], r[1]], [r[2], r[3]])
                            .anchor([0.0, 0.0])
                            .layer(layer)
                            .z(z)
                            .uv(uv)
                            .tint([1.0, 1.0, 1.0, self.alpha]),
                    );
                }
                true
            }
        }
    }

    /// An image by path, with its size in pixels.
    pub fn texture(&self, path: &str) -> Option<(TextureId, [f32; 2])> {
        let texture = *self.images.get(path)?;
        let size = self.renderer.texture_size(texture)?;
        Some((texture, [size[0] as f32, size[1] as f32]))
    }
}

#[cfg(test)]
mod tests {
    use crate::theme::ImageFit;

    #[test]
    fn image_fits_keep_or_drop_the_aspect_ratio() {
        let rect = [0.0, 0.0, 200.0, 100.0];
        let (r, uv) = ImageFit::Stretch.place(rect, [50.0, 50.0]);
        assert_eq!((r, uv), (rect, [0.0, 0.0, 1.0, 1.0]));
        let (r, _) = ImageFit::Fit.place(rect, [50.0, 50.0]);
        assert_eq!(r, [50.0, 0.0, 100.0, 100.0]);
        let (r, uv) = ImageFit::Cover.place(rect, [50.0, 50.0]);
        assert_eq!(r, rect);
        assert_eq!(uv, [0.0, 0.25, 1.0, 0.75]);
    }
}
