//! 2D camera with a virtual reference resolution, and letterboxing.

/// A rectangle in physical surface pixels (origin top-left).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    pub fn full(size: [u32; 2]) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: size[0] as f32,
            height: size[1] as f32,
        }
    }
}

/// Largest rectangle with the aspect ratio of `virtual_size` that fits in
/// `surface`, centered (pillarbox / letterbox bars fill the remainder).
/// Coordinates are snapped to whole pixels. Degenerate sizes give an empty
/// viewport.
pub fn letterbox(virtual_size: [f32; 2], surface: [u32; 2]) -> Viewport {
    let (vw, vh) = (virtual_size[0], virtual_size[1]);
    let (sw, sh) = (surface[0] as f32, surface[1] as f32);
    if vw <= 0.0 || vh <= 0.0 || sw <= 0.0 || sh <= 0.0 {
        return Viewport {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
    }
    let scale = (sw / vw).min(sh / vh);
    let width = (vw * scale).round().clamp(1.0, sw);
    let height = (vh * scale).round().clamp(1.0, sh);
    Viewport {
        x: ((sw - width) * 0.5).floor(),
        y: ((sh - height) * 0.5).floor(),
        width,
        height,
    }
}

/// Camera over a virtual canvas. World units are virtual pixels, origin at the
/// top-left of the canvas, Y pointing down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera2d {
    /// Reference resolution the content is authored for (e.g. 1920x1080).
    pub virtual_size: [f32; 2],
    /// World point at the center of the view.
    pub center: [f32; 2],
    /// 1.0 shows exactly `virtual_size`; 2.0 magnifies twice.
    pub zoom: f32,
}

impl Camera2d {
    pub fn new(virtual_width: f32, virtual_height: f32) -> Self {
        Self {
            virtual_size: [virtual_width, virtual_height],
            center: [virtual_width * 0.5, virtual_height * 0.5],
            zoom: 1.0,
        }
    }

    /// Column-major matrix mapping world coordinates to clip space.
    pub fn view_proj(&self) -> [[f32; 4]; 4] {
        let zoom = self.zoom.max(1e-4);
        let sx = 2.0 * zoom / self.virtual_size[0].max(1.0);
        let sy = -2.0 * zoom / self.virtual_size[1].max(1.0);
        [
            [sx, 0.0, 0.0, 0.0],
            [0.0, sy, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-self.center[0] * sx, -self.center[1] * sy, 0.0, 1.0],
        ]
    }

    /// World rectangle `(left, top, right, bottom)` currently visible.
    pub fn visible_rect(&self) -> [f32; 4] {
        let zoom = self.zoom.max(1e-4);
        let (hw, hh) = (
            self.virtual_size[0] * 0.5 / zoom,
            self.virtual_size[1] * 0.5 / zoom,
        );
        [
            self.center[0] - hw,
            self.center[1] - hh,
            self.center[0] + hw,
            self.center[1] + hh,
        ]
    }

    /// Converts a point in surface pixels (e.g. the mouse position) to world
    /// coordinates, given the viewport the camera is displayed in.
    pub fn screen_to_world(&self, viewport: &Viewport, point: [f32; 2]) -> [f32; 2] {
        let r = self.visible_rect();
        let u = (point[0] - viewport.x) / viewport.width.max(1.0);
        let v = (point[1] - viewport.y) / viewport.height.max(1.0);
        [r[0] + u * (r[2] - r[0]), r[1] + v * (r[3] - r[1])]
    }
}

impl Default for Camera2d {
    fn default() -> Self {
        Self::new(1920.0, 1080.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pillarbox_on_wide_window() {
        let vp = letterbox([1920.0, 1080.0], [2000, 1000]);
        assert_eq!(vp.height, 1000.0);
        assert_eq!(vp.width, 1778.0);
        assert_eq!(vp.y, 0.0);
        assert_eq!(vp.x, 111.0);
    }

    #[test]
    fn letterbox_on_tall_window() {
        let vp = letterbox([1920.0, 1080.0], [960, 960]);
        assert_eq!(vp.width, 960.0);
        assert_eq!(vp.height, 540.0);
        assert_eq!(vp.y, 210.0);
    }

    #[test]
    fn degenerate_surface_is_empty() {
        assert_eq!(letterbox([1920.0, 1080.0], [0, 100]).width, 0.0);
        assert_eq!(letterbox([0.0, 0.0], [100, 100]).height, 0.0);
    }

    #[test]
    fn view_proj_maps_canvas_corners() {
        let m = Camera2d::new(1920.0, 1080.0).view_proj();
        let apply = |x: f32, y: f32| (m[0][0] * x + m[3][0], m[1][1] * y + m[3][1]);
        let (x, y) = apply(0.0, 0.0);
        assert!((x + 1.0).abs() < 1e-5 && (y - 1.0).abs() < 1e-5);
        let (x, y) = apply(1920.0, 1080.0);
        assert!((x - 1.0).abs() < 1e-5 && (y + 1.0).abs() < 1e-5);
    }

    #[test]
    fn screen_to_world_roundtrip_through_letterbox() {
        let cam = Camera2d::default();
        let vp = letterbox(cam.virtual_size, [2000, 1000]);
        let p = cam.screen_to_world(&vp, [vp.x + vp.width * 0.5, vp.y + vp.height * 0.5]);
        assert!((p[0] - 960.0).abs() < 1e-2 && (p[1] - 540.0).abs() < 1e-2);
    }
}
