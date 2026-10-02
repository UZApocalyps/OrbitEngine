//! Frame animation maths: which frame is shown at a given time, and where a
//! sprite sheet cell lies in its image. Pure functions, shared by the runtime
//! and the editor.

use crate::defs::{AnimFrames, Animation, LoopMode};
use crate::ids::AssetId;

/// Index of the frame shown `elapsed` seconds after the animation started.
/// `0` for degenerate input (no frame, non-positive / non-finite fps or time).
pub fn frame_index(elapsed: f32, fps: f32, frame_count: usize, mode: LoopMode) -> usize {
    if frame_count <= 1 || !(fps > 0.0) || !fps.is_finite() || !(elapsed > 0.0) {
        return 0;
    }
    let step = (elapsed * fps).floor();
    // Far beyond usize: clamp, the modulo below stays well defined.
    let step = if step >= 1.0e15 { 1.0e15 } else { step } as u64;
    let n = frame_count as u64;
    let index = match mode {
        LoopMode::Loop => step % n,
        LoopMode::Once => step.min(n - 1),
        LoopMode::PingPong => {
            let period = 2 * n - 2;
            let p = step % period;
            if p < n { p } else { period - p }
        }
    };
    index as usize
}

/// A frame to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameRef {
    /// A cell of a sprite sheet (`index` counts from the top-left cell).
    Sheet { asset: AssetId, index: usize },
    /// A whole image.
    Image(AssetId),
}

impl Animation {
    /// The frame shown `elapsed` seconds after the start, `None` for an
    /// animation without frames.
    pub fn frame_at(&self, elapsed: f32) -> Option<FrameRef> {
        let n = self.frames.len();
        if n == 0 {
            return None;
        }
        let index = frame_index(elapsed, self.fps, n, self.loop_mode);
        Some(match &self.frames {
            AnimFrames::SpriteSheet { asset, .. } => FrameRef::Sheet {
                asset: *asset,
                index,
            },
            AnimFrames::Sequence(v) => FrameRef::Image(v[index]),
        })
    }

    /// Whether the picture changes over time (more than one frame).
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1 && self.fps > 0.0
    }
}

/// Pixel size of one sprite sheet cell: `frame_size` when given, else the
/// image size divided by the grid.
pub fn sheet_cell_size(
    cols: u32,
    rows: u32,
    frame_size: Option<[u32; 2]>,
    sheet_px: [u32; 2],
) -> [u32; 2] {
    match frame_size {
        Some([w, h]) if w > 0 && h > 0 => [w, h],
        _ => [sheet_px[0] / cols.max(1), sheet_px[1] / rows.max(1)],
    }
}

/// Texture coordinates `[u0, v0, u1, v1]` of cell `index` of a sprite sheet.
/// `None` when the image has no pixels.
pub fn sheet_uv(
    index: usize,
    cols: u32,
    rows: u32,
    frame_size: Option<[u32; 2]>,
    sheet_px: [u32; 2],
) -> Option<[f32; 4]> {
    if sheet_px[0] == 0 || sheet_px[1] == 0 {
        return None;
    }
    let cols = cols.max(1) as usize;
    let _ = rows;
    let (col, row) = ((index % cols) as f32, (index / cols) as f32);
    let (sw, sh) = (sheet_px[0] as f32, sheet_px[1] as f32);
    let (fw, fh) = match frame_size {
        Some([w, h]) if w > 0 && h > 0 => (w as f32, h as f32),
        _ => (sw / cols as f32, sh / rows.max(1) as f32),
    };
    Some([
        col * fw / sw,
        row * fh / sh,
        (col + 1.0) * fw / sw,
        (row + 1.0) * fh / sh,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(n: usize) -> Vec<usize> {
        // frame at each half-second at 2 fps => one step per call
        (0..n)
            .map(|i| frame_index(i as f32 / 2.0 + 0.01, 2.0, 4, LoopMode::Loop))
            .collect()
    }

    #[test]
    fn loop_wraps() {
        assert_eq!(seq(9), vec![0, 1, 2, 3, 0, 1, 2, 3, 0]);
    }

    #[test]
    fn once_holds_the_last_frame() {
        let at = |t| frame_index(t, 10.0, 4, LoopMode::Once);
        assert_eq!(at(0.0), 0);
        assert_eq!(at(0.11), 1);
        assert_eq!(at(0.35), 3);
        assert_eq!(at(100.0), 3);
    }

    #[test]
    fn ping_pong_goes_back_without_repeating_the_ends() {
        let got: Vec<usize> = (0..10)
            .map(|i| frame_index(i as f32 + 0.5, 1.0, 4, LoopMode::PingPong))
            .collect();
        assert_eq!(got, vec![0, 1, 2, 3, 2, 1, 0, 1, 2, 3]);
    }

    #[test]
    fn degenerate_input_gives_frame_zero() {
        assert_eq!(frame_index(5.0, 0.0, 4, LoopMode::Loop), 0);
        assert_eq!(frame_index(5.0, -3.0, 4, LoopMode::Loop), 0);
        assert_eq!(frame_index(5.0, f32::NAN, 4, LoopMode::Loop), 0);
        assert_eq!(frame_index(-1.0, 10.0, 4, LoopMode::Loop), 0);
        assert_eq!(frame_index(f32::NAN, 10.0, 4, LoopMode::Loop), 0);
        assert_eq!(frame_index(5.0, 10.0, 0, LoopMode::PingPong), 0);
        assert_eq!(frame_index(5.0, 10.0, 1, LoopMode::PingPong), 0);
        assert!(frame_index(f32::MAX, 10.0, 3, LoopMode::Loop) < 3);
    }

    #[test]
    fn frame_at_picks_the_asset() {
        let (a, b) = (AssetId::new(), AssetId::new());
        let anim = Animation {
            frames: AnimFrames::Sequence(vec![a, b]),
            fps: 1.0,
            loop_mode: LoopMode::Loop,
        };
        assert_eq!(anim.frame_at(0.0), Some(FrameRef::Image(a)));
        assert_eq!(anim.frame_at(1.5), Some(FrameRef::Image(b)));
        let empty = Animation {
            frames: AnimFrames::Sequence(vec![]),
            fps: 1.0,
            loop_mode: LoopMode::Loop,
        };
        assert_eq!(empty.frame_at(0.0), None);
        let sheet = Animation {
            frames: AnimFrames::SpriteSheet {
                asset: a,
                cols: 2,
                rows: 2,
                frame_count: 3,
                frame_size: None,
            },
            fps: 1.0,
            loop_mode: LoopMode::Once,
        };
        assert_eq!(
            sheet.frame_at(9.0),
            Some(FrameRef::Sheet { asset: a, index: 2 })
        );
    }

    #[test]
    fn uv_of_grid_cells() {
        // 2 cols x 2 rows, 200x100 image.
        assert_eq!(
            sheet_uv(0, 2, 2, None, [200, 100]),
            Some([0.0, 0.0, 0.5, 0.5])
        );
        assert_eq!(
            sheet_uv(1, 2, 2, None, [200, 100]),
            Some([0.5, 0.0, 1.0, 0.5])
        );
        assert_eq!(
            sheet_uv(3, 2, 2, None, [200, 100]),
            Some([0.5, 0.5, 1.0, 1.0])
        );
        assert_eq!(sheet_cell_size(2, 2, None, [200, 100]), [100, 50]);
    }

    #[test]
    fn uv_with_explicit_frame_size_and_degenerate_sheets() {
        // 10px cells with padding in a 100x50 image, 3 per row.
        let uv = sheet_uv(4, 3, 1, Some([10, 20]), [100, 50]).unwrap();
        assert_eq!(uv, [0.1, 0.4, 0.2, 0.8]);
        assert_eq!(sheet_cell_size(3, 1, Some([10, 20]), [100, 50]), [10, 20]);
        assert_eq!(sheet_uv(0, 2, 2, None, [0, 10]), None);
        // Zero grid does not divide by zero.
        assert!(sheet_uv(0, 0, 0, None, [10, 10]).is_some());
    }
}
