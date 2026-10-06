//! Paint tools on a part's picture (brush, eraser, clone stamp, fill bucket,
//! eyedropper): plain loops on RGBA8 (straight alpha), no egui, no GPU, unit
//! tested headlessly. The viewport drives them (see `pixeltools`); a stroke
//! is one undo step of the rig document.
//!
//! A stroke composites from the picture as it was when the stroke began and
//! keeps, per pixel, the strongest brush coverage seen so far: going over the
//! same place again in one stroke never builds up past the opacity (like the
//! "build-up off" brushes of paint programs).

use orbit_project::rigmesh::pixels;

/// Brush settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Brush {
    /// Diameter in picture pixels.
    pub size: f32,
    /// `0` soft edge … `1` hard edge.
    pub hardness: f32,
    /// `0..1`.
    pub opacity: f32,
    /// Straight RGBA.
    pub color: [u8; 4],
}

impl Default for Brush {
    fn default() -> Self {
        Self { size: 12.0, hardness: 0.7, opacity: 1.0, color: [40, 30, 30, 255] }
    }
}

/// What a stroke does to the pixels it covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Paint,
    Erase,
    /// Copies the picture (as it was when the stroke began) from `offset`
    /// pixels away.
    Clone { offset: [f32; 2] },
}

/// Coverage of a pixel at `d` pixels from the dab center.
pub fn coverage(d: f32, radius: f32, hardness: f32) -> f32 {
    if d >= radius {
        return 0.0;
    }
    let inner = radius * hardness.clamp(0.0, 0.999);
    if d <= inner {
        return 1.0;
    }
    let t = 1.0 - (d - inner) / (radius - inner);
    t * t * (3.0 - 2.0 * t)
}

/// `top` with alpha `k` (`0..1`, its own alpha included) over `under`.
pub fn over(under: [u8; 4], top: [u8; 4], k: f32) -> [u8; 4] {
    let a_s = (top[3] as f32 / 255.0 * k).clamp(0.0, 1.0);
    let a_d = under[3] as f32 / 255.0;
    let a_o = a_s + a_d * (1.0 - a_s);
    if a_o <= 1e-6 {
        return [0, 0, 0, 0];
    }
    let mut out = [0u8; 4];
    for c in 0..3 {
        let v = (top[c] as f32 * a_s + under[c] as f32 * a_d * (1.0 - a_s)) / a_o;
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (a_o * 255.0).round() as u8;
    out
}

fn lerp(a: [u8; 4], b: [u8; 4], k: f32) -> [u8; 4] {
    let mut out = [0u8; 4];
    for c in 0..4 {
        out[c] = (a[c] as f32 + (b[c] as f32 - a[c] as f32) * k).round().clamp(0.0, 255.0) as u8;
    }
    out
}

fn get(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

/// A stroke in progress on one picture.
pub struct Stroke {
    pub w: u32,
    pub h: u32,
    /// The pixels when the stroke began (also what undo records against).
    pub before: Vec<u8>,
    cover: Vec<f32>,
    last: Option<[f32; 2]>,
    pub mode: Mode,
    pub brush: Brush,
}

impl Stroke {
    pub fn new(w: u32, h: u32, before: Vec<u8>, mode: Mode, brush: Brush) -> Self {
        Self { w, h, cover: vec![0.0; before.len() / 4], before, last: None, mode, brush }
    }

    /// Paints from the last point to `p` (picture pixels). Returns whether a
    /// pixel changed.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn to(&mut self, rgba: &mut [u8], p: [f32; 2]) -> bool {
        let from = self.last.unwrap_or(p);
        self.last = Some(p);
        let spacing = (self.brush.size * 0.15).max(0.5);
        let len = ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt();
        let steps = (len / spacing).ceil().max(1.0) as usize;
        let mut changed = false;
        for s in 1..=steps {
            let t = s as f32 / steps as f32;
            changed |= self.dab(rgba, [from[0] + (p[0] - from[0]) * t, from[1] + (p[1] - from[1]) * t]);
        }
        changed
    }

    /// One dab of diameter `size` picture pixels at `c` (pose mode: the size
    /// follows the local scale of the deformation). Returns whether a pixel
    /// changed.
    pub fn dab_sized(&mut self, rgba: &mut [u8], c: [f32; 2], size: f32) -> bool {
        self.dab_r(rgba, c, (size * 0.5).max(0.5))
    }

    fn dab(&mut self, rgba: &mut [u8], c: [f32; 2]) -> bool {
        self.dab_r(rgba, c, (self.brush.size * 0.5).max(0.5))
    }

    fn dab_r(&mut self, rgba: &mut [u8], c: [f32; 2], r: f32) -> bool {
        let x0 = (c[0] - r).floor().max(0.0) as u32;
        let y0 = (c[1] - r).floor().max(0.0) as u32;
        let x1 = ((c[0] + r).ceil().max(0.0) as u32).min(self.w);
        let y1 = ((c[1] + r).ceil().max(0.0) as u32).min(self.h);
        let mut changed = false;
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f32 + 0.5 - c[0]).powi(2) + (y as f32 + 0.5 - c[1]).powi(2)).sqrt();
                let k = coverage(d, r, self.brush.hardness);
                let i = (y * self.w + x) as usize;
                if k <= self.cover[i] {
                    continue;
                }
                self.cover[i] = k;
                let k = k * self.brush.opacity.clamp(0.0, 1.0);
                let under = get(&self.before, self.w, x, y);
                let out = match self.mode {
                    Mode::Paint => over(under, self.brush.color, k),
                    Mode::Erase => {
                        let mut p = under;
                        p[3] = (p[3] as f32 * (1.0 - k)).round() as u8;
                        if p[3] == 0 {
                            p = [0, 0, 0, 0];
                        }
                        p
                    }
                    Mode::Clone { offset } => {
                        let sx = (x as f32 + offset[0]).round();
                        let sy = (y as f32 + offset[1]).round();
                        if sx < 0.0 || sy < 0.0 || sx >= self.w as f32 || sy >= self.h as f32 {
                            continue;
                        }
                        lerp(under, get(&self.before, self.w, sx as u32, sy as u32), k)
                    }
                };
                let px = &mut rgba[i * 4..i * 4 + 4];
                if px != out {
                    px.copy_from_slice(&out);
                    changed = true;
                }
            }
        }
        changed
    }
}

/// Fill bucket: the area like the pixel at `seed` (connected, within
/// `tolerance`) gets `color` at `opacity`. Returns whether a pixel changed.
pub fn fill(w: u32, h: u32, rgba: &mut [u8], seed: [u32; 2], tolerance: u8, color: [u8; 4], opacity: f32) -> bool {
    let mask = pixels::magic_wand(w, h, rgba, seed, tolerance, true);
    let mut changed = false;
    for (i, &b) in mask.bits.iter().enumerate() {
        if b == 0 {
            continue;
        }
        let under = [rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2], rgba[i * 4 + 3]];
        let out = over(under, color, opacity.clamp(0.0, 1.0));
        if out != under {
            rgba[i * 4..i * 4 + 4].copy_from_slice(&out);
            changed = true;
        }
    }
    changed
}

/// Eyedropper: the pixel at `p` (picture pixels), if inside.
pub fn pick(w: u32, h: u32, rgba: &[u8], p: [f32; 2]) -> Option<[u8; 4]> {
    (p[0] >= 0.0 && p[1] >= 0.0 && (p[0] as u32) < w && (p[1] as u32) < h).then(|| get(rgba, w, p[0] as u32, p[1] as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(w: u32, h: u32) -> Vec<u8> {
        vec![0; (w * h * 4) as usize]
    }

    #[test]
    fn a_hard_dab_paints_its_disk_and_nothing_else() {
        let (w, h) = (16, 16);
        let mut px = blank(w, h);
        let brush = Brush { size: 6.0, hardness: 1.0, opacity: 1.0, color: [255, 0, 0, 255] };
        let mut s = Stroke::new(w, h, px.clone(), Mode::Paint, brush);
        assert!(s.to(&mut px, [8.0, 8.0]));
        assert_eq!(get(&px, w, 7, 7), [255, 0, 0, 255]);
        assert_eq!(get(&px, w, 0, 0), [0, 0, 0, 0]);
        assert_eq!(get(&px, w, 12, 8), [0, 0, 0, 0]);
        // Off the picture: clipped, no panic.
        assert!(s.to(&mut px, [-50.0, -50.0]));
    }

    #[test]
    fn going_over_twice_does_not_build_up_past_the_opacity() {
        let (w, h) = (8, 8);
        let mut px: Vec<u8> = (0..w * h).flat_map(|_| [255u8, 255, 255, 255]).collect();
        let brush = Brush { size: 4.0, hardness: 1.0, opacity: 0.5, color: [0, 0, 0, 255] };
        let mut s = Stroke::new(w, h, px.clone(), Mode::Paint, brush);
        s.to(&mut px, [4.0, 4.0]);
        let once = get(&px, w, 4, 4);
        s.to(&mut px, [4.1, 4.0]);
        s.to(&mut px, [4.0, 4.0]);
        assert_eq!(get(&px, w, 4, 4), once);
        assert!((120..=135).contains(&once[0]), "{once:?}");
    }

    #[test]
    fn eraser_clone_and_soft_edges() {
        let (w, h) = (12, 4);
        // Left half red, right half blue.
        let base: Vec<u8> = (0..w * h).flat_map(|i| if i % w < 6 { [255u8, 0, 0, 255] } else { [0, 0, 255, 255] }).collect();
        let hard = Brush { size: 2.0, hardness: 1.0, opacity: 1.0, color: [0; 4] };
        let mut px = base.clone();
        Stroke::new(w, h, base.clone(), Mode::Erase, hard).to(&mut px, [2.0, 2.0]);
        assert_eq!(get(&px, w, 1, 1), [0, 0, 0, 0]);
        assert_eq!(get(&px, w, 5, 1), [255, 0, 0, 255]);
        // Clone: paint blue (6 px to the right) over the red.
        let mut px = base.clone();
        Stroke::new(w, h, base.clone(), Mode::Clone { offset: [6.0, 0.0] }, hard).to(&mut px, [2.0, 2.0]);
        assert_eq!(get(&px, w, 1, 1), [0, 0, 255, 255]);
        assert_eq!(get(&px, w, 4, 1), [255, 0, 0, 255]);
        // Soft: full in the middle, fading to nothing at the radius.
        assert_eq!(coverage(0.0, 10.0, 0.5), 1.0);
        assert!(coverage(8.0, 10.0, 0.5) > 0.0 && coverage(8.0, 10.0, 0.5) < 1.0);
        assert_eq!(coverage(10.0, 10.0, 0.5), 0.0);
    }

    #[test]
    fn fill_bucket_and_eyedropper() {
        let (w, h) = (6, 3);
        let mut px: Vec<u8> = (0..w * h).flat_map(|i| if i % w == 3 { [0u8, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
        assert!(fill(w, h, &mut px, [0, 0], 10, [0, 200, 0, 255], 1.0));
        assert_eq!(get(&px, w, 2, 2), [0, 200, 0, 255]);
        assert_eq!(get(&px, w, 3, 0), [0, 0, 0, 255]);
        assert_eq!(get(&px, w, 5, 0), [255, 255, 255, 255], "the other side of the wall");
        assert_eq!(pick(w, h, &px, [0.5, 0.5]), Some([0, 200, 0, 255]));
        assert_eq!(pick(w, h, &px, [6.0, 0.0]), None);
        // Over: half transparent red on white.
        assert_eq!(over([255, 255, 255, 255], [255, 0, 0, 255], 0.5), [255, 128, 128, 255]);
        assert_eq!(over([0, 0, 0, 0], [10, 20, 30, 255], 1.0), [10, 20, 30, 255]);
    }
}
