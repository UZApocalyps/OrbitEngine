//! Drawing of the splash screens (the GPU side of [`crate::splash`]).

use std::collections::HashMap;

use orbit_assets::{AssetHandle, AssetLoader, AssetPoll, ImageData, decode_image};
use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId, TextureOptions};
use orbit_text::{TextEngine, TextRenderer};
use orbit_ui::{Banner, escape_markup, tint};

use crate::splash::{ENGINE_NAME, SplashKind, SplashState, SplashStep};
use crate::stage::CANVAS;

/// The engine logo, embedded in the binary.
const LOGO: &[u8] = include_bytes!("../../../assets/brand/logo-horizontal-light.png");
/// Width of the logo on the canvas.
const LOGO_WIDTH: f32 = 900.0;

enum Slot {
    Loading(AssetHandle<ImageData>),
    Ready(TextureId),
    Failed,
}

/// Textures and texts of the splash screens, created on first use.
#[derive(Default)]
pub(crate) struct SplashAssets {
    /// `None`: not tried yet; `Some(None)`: the logo could not be decoded.
    logo: Option<Option<TextureId>>,
    images: HashMap<String, Slot>,
    texts: HashMap<usize, Banner>,
    fallback: Option<Banner>,
}

/// `size` scaled down (never up) to fit in `max`, keeping the ratio.
pub(crate) fn fit_size(size: [f32; 2], max: [f32; 2]) -> [f32; 2] {
    if size[0] <= 0.0 || size[1] <= 0.0 {
        return [0.0, 0.0];
    }
    let k = (max[0] / size[0]).min(max[1] / size[1]).min(1.0);
    [size[0] * k, size[1] * k]
}

/// Text color readable on `background`.
pub(crate) fn text_color_on(background: [u8; 4]) -> [u8; 4] {
    let lum = 0.299 * f32::from(background[0])
        + 0.587 * f32::from(background[1])
        + 0.114 * f32::from(background[2]);
    if lum > 140.0 {
        [20, 24, 40, 255]
    } else {
        [245, 247, 255, 255]
    }
}

impl SplashAssets {
    /// Starts loading the images of the project splashes.
    pub fn request(&mut self, loader: &AssetLoader, steps: &[SplashStep]) {
        for step in steps {
            if let SplashKind::Image(path) = &step.kind {
                self.images
                    .entry(path.clone())
                    .or_insert_with(|| Slot::Loading(loader.load_image(path)));
            }
        }
    }

    /// Starts loading splash images by path (as soon as the UI bundle is
    /// known, so they are ready when their splash comes).
    pub fn request_images(&mut self, loader: &AssetLoader, paths: &[String]) {
        for path in paths {
            self.images
                .entry(path.clone())
                .or_insert_with(|| Slot::Loading(loader.load_image(path)));
        }
    }

    /// Whether the splash image path was requested.
    #[cfg(test)]
    pub fn has_image(&self, path: &str) -> bool {
        self.images.contains_key(path)
    }

    fn logo(&mut self, renderer: &mut Renderer2d) -> Option<TextureId> {
        *self.logo.get_or_insert_with(|| {
            let img = decode_image(LOGO)
                .map_err(|e| log::warn!("engine logo: {e}"))
                .ok()?;
            renderer
                .create_texture(
                    "orbit/engine-logo",
                    img.width,
                    img.height,
                    &img.rgba,
                    TextureOptions::COLOR,
                )
                .ok()
        })
    }

    fn image(&mut self, renderer: &mut Renderer2d, path: &str) -> Option<TextureId> {
        let slot = self.images.get_mut(path)?;
        if let Slot::Loading(handle) = slot {
            match handle.poll() {
                AssetPoll::Loading => return None,
                AssetPoll::Ready(img) => {
                    *slot = renderer
                        .create_texture(
                            &format!("splash/{path}"),
                            img.width,
                            img.height,
                            &img.rgba,
                            TextureOptions::COLOR,
                        )
                        .map_or(Slot::Failed, Slot::Ready);
                }
                _ => {
                    log::warn!("splash image `{path}` could not be loaded");
                    *slot = Slot::Failed;
                }
            }
        }
        match slot {
            Slot::Ready(id) => Some(*id),
            _ => None,
        }
    }

    /// The scene of the current step: background, then its content with the
    /// fade applied. `fonts_ready` gates the texts.
    pub fn scene(
        &mut self,
        state: &SplashState,
        renderer: &mut Renderer2d,
        text_engine: &mut TextEngine,
        text_renderer: &mut TextRenderer,
        fonts_ready: bool,
    ) -> Scene {
        let mut scene = Scene::new();
        let Some(step) = state.current() else {
            scene.clear = Some([0.0, 0.0, 0.0, 1.0]);
            return scene;
        };
        scene.clear = Some(tint(step.background));
        let alpha = state.alpha();
        let white = [255, 255, 255, (alpha * 255.0).round() as u8];
        let picture = |tex: TextureId, size: [f32; 2], max: [f32; 2], scene: &mut Scene| {
            let size = fit_size(size, max);
            scene.push(
                Sprite2d::new(tex, [CANVAS[0] * 0.5, CANVAS[1] * 0.5], size)
                    .anchor([0.5, 0.5])
                    .layer(Layer::UI)
                    .tint(tint(white)),
            );
        };
        match &step.kind {
            SplashKind::Engine => {
                let logo = self.logo(renderer);
                match logo.and_then(|t| renderer.texture_size(t).map(|s| (t, s))) {
                    Some((tex, [w, h])) => {
                        // The logo is wide: scale it to LOGO_WIDTH.
                        let k = LOGO_WIDTH / w as f32;
                        picture(
                            tex,
                            [LOGO_WIDTH, h as f32 * k],
                            [LOGO_WIDTH, CANVAS[1]],
                            &mut scene,
                        );
                    }
                    None if fonts_ready => {
                        let banner = self.fallback.get_or_insert_with(|| {
                            Banner::centered(ENGINE_NAME, 96.0, text_color_on(step.background))
                        });
                        banner.draw(
                            text_engine,
                            text_renderer,
                            renderer,
                            &mut scene,
                            CANVAS[1] * 0.5 - 70.0,
                            alpha,
                        );
                    }
                    None => {}
                }
            }
            SplashKind::Image(path) => {
                if let Some(tex) = self.image(renderer, path)
                    && let Some([w, h]) = renderer.texture_size(tex)
                {
                    picture(tex, [w as f32, h as f32], CANVAS, &mut scene);
                }
            }
            SplashKind::Text(text) if fonts_ready => {
                let color = text_color_on(step.background);
                let banner = self.texts.entry(state.index()).or_insert_with(|| {
                    Banner::centered(&escape_markup(text), 72.0, color)
                });
                banner.draw(
                    text_engine,
                    text_renderer,
                    renderer,
                    &mut scene,
                    CANVAS[1] * 0.5 - 50.0,
                    alpha,
                );
            }
            SplashKind::Text(_) => {}
        }
        scene
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_the_ratio_and_never_upscales() {
        assert_eq!(fit_size([4000.0, 1000.0], [1920.0, 1080.0]), [1920.0, 480.0]);
        assert_eq!(fit_size([100.0, 50.0], [1920.0, 1080.0]), [100.0, 50.0]);
        assert_eq!(fit_size([0.0, 50.0], [1920.0, 1080.0]), [0.0, 0.0]);
    }

    #[test]
    fn text_is_readable_on_both_light_and_dark_pages() {
        assert_eq!(text_color_on([0, 0, 0, 255])[0], 245);
        assert_eq!(text_color_on([250, 250, 250, 255])[0], 20);
    }

    #[test]
    fn the_engine_logo_is_embedded_and_decodes() {
        let img = decode_image(LOGO).expect("the embedded logo is a valid PNG");
        assert!(img.width > img.height, "a horizontal logo");
    }
}
