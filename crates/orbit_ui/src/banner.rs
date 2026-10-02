//! Building blocks of the full-screen states (loading, start, end, error).

use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId};
use orbit_text::{DrawParams, TextAlign, TextBlock, TextEngine, TextRenderer, TextStyle};

use crate::theme::tint;

const CANVAS_W: f32 = 1920.0;

/// A static block of text, centered horizontally or left aligned in a box.
pub struct Banner {
    block: TextBlock,
    x: f32,
}

impl Banner {
    /// Centered on the canvas.
    pub fn centered(markup: &str, font_size: f32, color: [u8; 4]) -> Self {
        Self::build(markup, font_size, color, TextAlign::Center, 0.0, CANVAS_W)
    }

    /// Left aligned in a box starting at `x` and `width` wide.
    pub fn left(markup: &str, font_size: f32, color: [u8; 4], x: f32, width: f32) -> Self {
        Self::build(markup, font_size, color, TextAlign::Left, x, width)
    }

    fn build(
        markup: &str,
        font_size: f32,
        color: [u8; 4],
        align: TextAlign,
        x: f32,
        width: f32,
    ) -> Self {
        let mut block = TextBlock::new(
            markup,
            TextStyle {
                font_size,
                color,
                align,
                ..Default::default()
            },
            Some(width),
            f32::INFINITY,
        );
        block.skip_to_end();
        Self { block, x }
    }

    /// Height of the laid-out text, known after the first draw.
    pub fn height(&self) -> Option<f32> {
        self.block.layout().map(|l| l.height)
    }

    /// Draws with the box's top edge at `top` (virtual pixels).
    pub fn draw(
        &mut self,
        engine: &mut TextEngine,
        text_renderer: &mut TextRenderer,
        renderer: &Renderer2d,
        scene: &mut Scene,
        top: f32,
        alpha: f32,
    ) {
        let params = DrawParams {
            origin: [self.x, top],
            z: 2.0,
            alpha,
            ..Default::default()
        };
        self.block
            .draw(engine, text_renderer, renderer, scene, &params);
    }
}

/// Width of the filled part of a bar `width` wide.
fn fill_width(width: f32, progress: f32) -> f32 {
    if progress.is_finite() {
        width * progress.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// A horizontal progress bar centered at `center`, `progress` in `0..=1`.
#[allow(clippy::too_many_arguments)]
pub fn draw_progress_bar(
    scene: &mut Scene,
    white: TextureId,
    center: [f32; 2],
    size: [f32; 2],
    background: [u8; 4],
    fill: [u8; 4],
    progress: f32,
) {
    let left = center[0] - size[0] * 0.5;
    let top = center[1] - size[1] * 0.5;
    scene.push(
        Sprite2d::new(white, [left, top], size)
            .anchor([0.0, 0.0])
            .layer(Layer::UI)
            .z(0.0)
            .tint(tint(background)),
    );
    let filled = fill_width(size[0], progress);
    if filled > 0.0 {
        scene.push(
            Sprite2d::new(white, [left, top], [filled, size[1]])
                .anchor([0.0, 0.0])
                .layer(Layer::UI)
                .z(1.0)
                .tint(tint(fill)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_width_is_clamped_and_nan_safe() {
        assert_eq!(fill_width(700.0, 0.5), 350.0);
        assert_eq!(fill_width(700.0, 2.0), 700.0);
        assert_eq!(fill_width(700.0, -1.0), 0.0);
        assert_eq!(fill_width(700.0, f32::NAN), 0.0);
    }
}
