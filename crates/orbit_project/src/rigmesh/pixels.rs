//! Pixel selections and the "cut to new part" image work of the rig editor
//! (and the MCP `rig_cut` tool): polygon / lasso rasterization, magic wand,
//! extraction of the selected pixels and a hole fill (push-pull inpaint).
//!
//! Editor-side tooling only: plain loops on RGBA8 buffers (`w * h * 4`
//! bytes, straight alpha, rows top to bottom), no codec, no dependency. The
//! game never runs any of this.

/// A pixel selection: one byte per pixel (`0` out, anything else in).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    pub w: u32,
    pub h: u32,
    pub bits: Vec<u8>,
}

impl Mask {
    pub fn new(w: u32, h: u32) -> Self {
        Self { w, h, bits: vec![0; w as usize * h as usize] }
    }

    pub fn get(&self, x: u32, y: u32) -> bool {
        x < self.w && y < self.h && self.bits[(y * self.w + x) as usize] != 0
    }

    pub fn set(&mut self, x: u32, y: u32, on: bool) {
        if x < self.w && y < self.h {
            self.bits[(y * self.w + x) as usize] = u8::from(on);
        }
    }

    /// Selected pixels.
    pub fn count(&self) -> usize {
        self.bits.iter().filter(|&&b| b != 0).count()
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|&b| b == 0)
    }

    /// `[x0, y0, x1, y1]` (end exclusive) of the selected pixels.
    pub fn bbox(&self) -> Option<[u32; 4]> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for y in 0..self.h {
            let row = &self.bits[(y * self.w) as usize..((y + 1) * self.w) as usize];
            if let Some(first) = row.iter().position(|&b| b != 0) {
                let last = row.iter().rposition(|&b| b != 0).unwrap_or(first);
                x0 = x0.min(first as u32);
                x1 = x1.max(last as u32 + 1);
                y0 = y0.min(y);
                y1 = y + 1;
            }
        }
        (x0 != u32::MAX).then_some([x0, y0, x1, y1])
    }

    /// Adds `other` (same size) to the selection.
    pub fn union(&mut self, other: &Mask) {
        if other.w == self.w && other.h == self.h {
            for (a, &b) in self.bits.iter_mut().zip(&other.bits) {
                *a = u8::from(*a != 0 || b != 0);
            }
        }
    }

    /// Removes `other` (same size) from the selection.
    pub fn subtract(&mut self, other: &Mask) {
        if other.w == self.w && other.h == self.h {
            for (a, &b) in self.bits.iter_mut().zip(&other.bits) {
                if b != 0 {
                    *a = 0;
                }
            }
        }
    }
}

/// The pixels whose center is inside the closed polygon `poly` (image pixel
/// coordinates, even-odd rule): what a lasso (many points) or a polygon
/// (clicked corners) selects. Fewer than 3 points select nothing.
pub fn polygon_mask(w: u32, h: u32, poly: &[[f32; 2]]) -> Mask {
    let mut mask = Mask::new(w, h);
    if poly.len() < 3 || poly.iter().flatten().any(|v| !v.is_finite()) {
        return mask;
    }
    let (lo, hi) = poly.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
    let y_start = (lo - 0.5).ceil().max(0.0) as u32;
    let y_end = ((hi - 0.5).floor() + 1.0).clamp(0.0, h as f32) as u32;
    let mut xs: Vec<f32> = Vec::new();
    for y in y_start..y_end {
        let yc = y as f32 + 0.5;
        xs.clear();
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            if (a[1] <= yc) != (b[1] <= yc) {
                xs.push(a[0] + (yc - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks_exact(2) {
            // Pixel x is in when x + 0.5 is in [pair[0], pair[1]).
            let from = (pair[0] - 0.5).ceil().max(0.0);
            let to = (pair[1] - 0.5).ceil().min(w as f32);
            let row = (y * w) as usize;
            let mut x = from as u32;
            while (x as f32) < to {
                mask.bits[row + x as usize] = 1;
                x += 1;
            }
        }
    }
    mask
}

fn px(rgba: &[u8], i: usize) -> [u8; 4] {
    [rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2], rgba[i * 4 + 3]]
}

/// Whether two pixels are within `tolerance` (largest channel difference;
/// two fully transparent pixels always match, whatever their color).
pub fn similar(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    if a[3] == 0 && b[3] == 0 {
        return true;
    }
    a.iter().zip(&b).all(|(&x, &y)| x.abs_diff(y) <= tolerance)
}

/// Magic wand: the pixels like the one at `seed` (within `tolerance`, see
/// [`similar`]), connected to it (4-neighbours) when `contiguous`, else
/// anywhere in the picture.
pub fn magic_wand(w: u32, h: u32, rgba: &[u8], seed: [u32; 2], tolerance: u8, contiguous: bool) -> Mask {
    let mut mask = Mask::new(w, h);
    if seed[0] >= w || seed[1] >= h || rgba.len() < w as usize * h as usize * 4 {
        return mask;
    }
    let reference = px(rgba, (seed[1] * w + seed[0]) as usize);
    if !contiguous {
        for i in 0..(w * h) as usize {
            mask.bits[i] = u8::from(similar(px(rgba, i), reference, tolerance));
        }
        return mask;
    }
    let mut stack = vec![seed];
    mask.set(seed[0], seed[1], true);
    while let Some([x, y]) = stack.pop() {
        let visit = |nx: u32, ny: u32, mask: &mut Mask, stack: &mut Vec<[u32; 2]>| {
            let i = (ny * w + nx) as usize;
            if mask.bits[i] == 0 && similar(px(rgba, i), reference, tolerance) {
                mask.bits[i] = 1;
                stack.push([nx, ny]);
            }
        };
        if x > 0 {
            visit(x - 1, y, &mut mask, &mut stack);
        }
        if x + 1 < w {
            visit(x + 1, y, &mut mask, &mut stack);
        }
        if y > 0 {
            visit(x, y - 1, &mut mask, &mut stack);
        }
        if y + 1 < h {
            visit(x, y + 1, &mut mask, &mut stack);
        }
    }
    mask
}

/// Selected pixels copied out of a picture, cropped to their bounding box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crop {
    /// Top-left corner in the source picture.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// The selected pixels of `rgba` (`w`×`h`) exactly, everything else of the
/// bounding box transparent. `None`: nothing visible is selected.
pub fn extract(w: u32, h: u32, rgba: &[u8], mask: &Mask) -> Option<Crop> {
    if mask.w != w || mask.h != h || rgba.len() < w as usize * h as usize * 4 {
        return None;
    }
    let [x0, y0, x1, y1] = mask.bbox()?;
    let (cw, ch) = (x1 - x0, y1 - y0);
    let mut out = vec![0u8; cw as usize * ch as usize * 4];
    let mut visible = false;
    for y in y0..y1 {
        for x in x0..x1 {
            if mask.get(x, y) {
                let s = ((y * w + x) * 4) as usize;
                let d = (((y - y0) * cw + (x - x0)) * 4) as usize;
                out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
                visible |= rgba[s + 3] > 0;
            }
        }
    }
    visible.then_some(Crop { x: x0, y: y0, w: cw, h: ch, rgba: out })
}

/// One pyramid level of [`inpaint`]: size, premultiplied colors, weights.
type Level = (usize, usize, Vec<[f32; 4]>, Vec<f32>);

/// Fills the `hole` pixels of `rgba` from the pixels around it (push-pull:
/// a pyramid of averages of the known pixels, then blended back up, so the
/// hole gets a smooth continuation of its border — good enough under hair
/// strands, on skin or flat colors). Pixels outside the hole are never
/// touched. Alpha is filled like a color (a hole on the silhouette stays
/// partly transparent). Nothing known (the whole picture selected): no-op.
pub fn inpaint(w: u32, h: u32, rgba: &mut [u8], hole: &Mask) {
    let (wu, hu) = (w as usize, h as usize);
    if hole.w != w || hole.h != h || rgba.len() < wu * hu * 4 || hole.is_empty() {
        return;
    }
    // Level 0: premultiplied color + alpha, and the weight (1 = known).
    let mut levels: Vec<Level> = Vec::new();
    let mut c0 = vec![[0.0f32; 4]; wu * hu];
    let mut w0 = vec![0.0f32; wu * hu];
    for i in 0..wu * hu {
        if hole.bits[i] == 0 {
            let a = rgba[i * 4 + 3] as f32 / 255.0;
            c0[i] = [
                rgba[i * 4] as f32 / 255.0 * a,
                rgba[i * 4 + 1] as f32 / 255.0 * a,
                rgba[i * 4 + 2] as f32 / 255.0 * a,
                a,
            ];
            w0[i] = 1.0;
        }
    }
    if w0.iter().all(|&k| k == 0.0) {
        return;
    }
    levels.push((wu, hu, c0, w0));
    // Pull: halve until 1×1.
    while levels.last().is_some_and(|l| l.0 > 1 || l.1 > 1) {
        let (lw, lh, c, k) = levels.last().expect("level");
        let (nw, nh) = (lw.div_ceil(2), lh.div_ceil(2));
        let mut nc = vec![[0.0f32; 4]; nw * nh];
        let mut nk = vec![0.0f32; nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                let mut sum = [0.0f32; 4];
                let mut wsum = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (x * 2 + dx, y * 2 + dy);
                    if sx < *lw && sy < *lh {
                        let i = sy * lw + sx;
                        for ch in 0..4 {
                            sum[ch] += c[i][ch] * k[i];
                        }
                        wsum += k[i];
                    }
                }
                let j = y * nw + x;
                if wsum > 0.0 {
                    nc[j] = sum.map(|s| s / wsum);
                }
                nk[j] = wsum.min(1.0);
            }
        }
        levels.push((nw, nh, nc, nk));
    }
    // Push: fill each level's unknown part from the coarser one (bilinear).
    for li in (0..levels.len() - 1).rev() {
        let (fine, coarse) = levels.split_at_mut(li + 1);
        let (cw, ch, coarse_c) = (coarse[0].0, coarse[0].1, &coarse[0].2);
        let (lw, lh, c, k) = &mut fine[li];
        for y in 0..*lh {
            for x in 0..*lw {
                let i = y * *lw + x;
                if k[i] >= 1.0 {
                    continue;
                }
                let fx = ((x as f32 + 0.5) * 0.5 - 0.5).clamp(0.0, (cw - 1) as f32);
                let fy = ((y as f32 + 0.5) * 0.5 - 0.5).clamp(0.0, (ch - 1) as f32);
                let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
                let (x1, y1) = ((x0 + 1).min(cw - 1), (y0 + 1).min(ch - 1));
                let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                let at = |xx: usize, yy: usize| coarse_c[yy * cw + xx];
                let (a, b, cc, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
                let mut up = [0.0f32; 4];
                for n in 0..4 {
                    let top = a[n] + (b[n] - a[n]) * tx;
                    let bottom = cc[n] + (d[n] - cc[n]) * tx;
                    up[n] = top + (bottom - top) * ty;
                }
                let kk = k[i];
                for n in 0..4 {
                    c[i][n] = c[i][n] * kk + up[n] * (1.0 - kk);
                }
                k[i] = 1.0;
            }
        }
    }
    let (_, _, c, _) = &levels[0];
    for i in 0..wu * hu {
        if hole.bits[i] == 0 {
            continue;
        }
        let [r, g, b, a] = c[i];
        let a = a.clamp(0.0, 1.0);
        let un = |v: f32| if a > 1e-6 { (v / a).clamp(0.0, 1.0) } else { 0.0 };
        rgba[i * 4] = (un(r) * 255.0).round() as u8;
        rgba[i * 4 + 1] = (un(g) * 255.0).round() as u8;
        rgba[i * 4 + 2] = (un(b) * 255.0).round() as u8;
        rgba[i * 4 + 3] = (a * 255.0).round() as u8;
    }
}

/// Clears the `mask` pixels (fully transparent).
pub fn clear(w: u32, h: u32, rgba: &mut [u8], mask: &Mask) {
    if mask.w != w || mask.h != h {
        return;
    }
    for (i, &b) in mask.bits.iter().enumerate() {
        if b != 0 && let Some(p) = rgba.get_mut(i * 4..i * 4 + 4) {
            p.copy_from_slice(&[0, 0, 0, 0]);
        }
    }
}

/// Whether the alpha of two same-size pictures differs on the in/out side of
/// `threshold` anywhere (what the traced mesh follows).
pub fn alpha_shape_changed(a: &[u8], b: &[u8], threshold: u8) -> bool {
    a.len() != b.len() || a.chunks_exact(4).zip(b.chunks_exact(4)).any(|(p, q)| (p[3] > threshold) != (q[3] > threshold))
}

/// The smallest rectangle `[x0, y0, x1, y1]` (end exclusive) where two
/// same-size pictures differ.
pub fn diff_rect(w: u32, h: u32, a: &[u8], b: &[u8]) -> Option<[u32; 4]> {
    if a.len() != b.len() || a.len() < w as usize * h as usize * 4 {
        return None;
    }
    let mut r: Option<[u32; 4]> = None;
    for y in 0..h {
        let row = (y * w * 4) as usize..((y + 1) * w * 4) as usize;
        let (ra, rb) = (&a[row.clone()], &b[row]);
        if ra == rb {
            continue;
        }
        let first = ra.chunks_exact(4).zip(rb.chunks_exact(4)).position(|(p, q)| p != q).unwrap_or(0) as u32;
        let last = ra.chunks_exact(4).zip(rb.chunks_exact(4)).rposition(|(p, q)| p != q).unwrap_or(0) as u32;
        r = Some(match r {
            None => [first, y, last + 1, y + 1],
            Some([x0, y0, x1, _]) => [x0.min(first), y0, x1.max(last + 1), y + 1],
        });
    }
    r
}

/// Copies the rectangle `[x0, y0, x1, y1]` out of a `w`-wide picture.
pub fn copy_rect(w: u32, rgba: &[u8], r: [u32; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(((r[2] - r[0]) * (r[3] - r[1]) * 4) as usize);
    for y in r[1]..r[3] {
        out.extend_from_slice(&rgba[((y * w + r[0]) * 4) as usize..((y * w + r[2]) * 4) as usize]);
    }
    out
}

/// Writes `data` (from [`copy_rect`]) back into the rectangle `r`.
pub fn paste_rect(w: u32, rgba: &mut [u8], r: [u32; 4], data: &[u8]) {
    let rw = ((r[2] - r[0]) * 4) as usize;
    for (row, y) in (r[1]..r[3]).enumerate() {
        let d = ((y * w + r[0]) * 4) as usize;
        if let (Some(dst), Some(src)) = (rgba.get_mut(d..d + rw), data.get(row * rw..(row + 1) * rw)) {
            dst.copy_from_slice(src);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(m: &Mask) -> Vec<String> {
        (0..m.h).map(|y| (0..m.w).map(|x| if m.get(x, y) { '#' } else { '.' }).collect()).collect()
    }

    #[test]
    fn polygon_rasterizes_pixel_centers_even_odd() {
        // A 4×3 rectangle from (1,1): exactly those 12 pixels.
        let m = polygon_mask(6, 5, &[[1.0, 1.0], [5.0, 1.0], [5.0, 4.0], [1.0, 4.0]]);
        assert_eq!(rows(&m), ["......", ".####.", ".####.", ".####.", "......"]);
        assert_eq!(m.count(), 12);
        assert_eq!(m.bbox(), Some([1, 1, 5, 4]));
        // A triangle: half of a square, centers on the diagonal excluded.
        let t = polygon_mask(4, 4, &[[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]]);
        assert_eq!(rows(&t), ["###.", "##..", "#...", "...."]);
        // Self-intersecting bow tie: even-odd, never panics; off-image is clipped.
        let b = polygon_mask(4, 4, &[[-5.0, -5.0], [9.0, 9.0], [9.0, -5.0], [-5.0, 9.0]]);
        assert!(b.count() > 0 && b.count() < 16);
        assert!(polygon_mask(4, 4, &[[0.0, 0.0], [4.0, 4.0]]).is_empty());
        assert!(polygon_mask(4, 4, &[[0.0, 0.0], [f32::NAN, 4.0], [4.0, 0.0]]).is_empty());
    }

    #[test]
    fn lasso_is_a_dense_polygon() {
        let circle: Vec<[f32; 2]> = (0..64)
            .map(|i| {
                let a = i as f32 / 64.0 * std::f32::consts::TAU;
                [10.0 + 6.0 * a.cos(), 10.0 + 6.0 * a.sin()]
            })
            .collect();
        let m = polygon_mask(20, 20, &circle);
        let area = m.count() as f32;
        assert!((area - std::f32::consts::PI * 36.0).abs() < 10.0, "{area}");
        assert!(m.get(10, 10) && !m.get(3, 3) && !m.get(17, 17));
    }

    fn picture() -> (u32, u32, Vec<u8>) {
        // 6×4: red left half, blue right half, a transparent bottom row.
        let (w, h) = (6u32, 4u32);
        let mut p = Vec::new();
        for y in 0..h {
            for x in 0..w {
                p.extend_from_slice(match (y, x) {
                    (3, _) => &[9, 9, 9, 0],
                    (_, 0..=2) => &[200, 0, 0, 255],
                    _ => &[0, 0, 200, 255],
                });
            }
        }
        (w, h, p)
    }

    #[test]
    fn magic_wand_follows_color_and_connectivity() {
        let (w, h, mut p) = picture();
        let m = magic_wand(w, h, &p, [0, 0], 10, true);
        assert_eq!(rows(&m), ["###...", "###...", "###...", "......"]);
        let t = magic_wand(w, h, &p, [5, 3], 0, true);
        assert_eq!(rows(&t), ["......", "......", "......", "######"]);
        // A red pixel cut off from the others: only the global mode takes it.
        p[(3 * 4) as usize..(3 * 4 + 4) as usize].copy_from_slice(&[200, 0, 0, 255]);
        p[(4 * 4) as usize..(4 * 4 + 4) as usize].copy_from_slice(&[0, 0, 200, 255]);
        let _ = magic_wand(w, h, &p, [0, 0], 10, true);
        let global = magic_wand(w, h, &p, [0, 0], 10, false);
        assert!(global.get(3, 0));
        assert!(magic_wand(w, h, &p, [99, 0], 10, true).is_empty());
    }

    #[test]
    fn extraction_is_pixel_exact_and_cropped() {
        let (w, h, p) = picture();
        let mut m = Mask::new(w, h);
        m.set(2, 1, true);
        m.set(3, 2, true);
        let c = extract(w, h, &p, &m).unwrap();
        assert_eq!((c.x, c.y, c.w, c.h), (2, 1, 2, 2));
        assert_eq!(&c.rgba[0..4], &[200, 0, 0, 255]);
        assert_eq!(&c.rgba[4..8], &[0, 0, 0, 0]);
        assert_eq!(&c.rgba[8..12], &[0, 0, 0, 0]);
        assert_eq!(&c.rgba[12..16], &[0, 0, 200, 255]);
        // Only transparent pixels selected: nothing to cut.
        let mut t = Mask::new(w, h);
        t.set(0, 3, true);
        assert!(extract(w, h, &p, &t).is_none());
        assert!(extract(w, h, &p, &Mask::new(w, h)).is_none());
    }

    #[test]
    fn inpaint_fills_the_hole_and_nothing_else() {
        // A uniform skin color with a dark "strand" across it.
        let (w, h) = (16u32, 12u32);
        let skin = [230u8, 190, 170, 255];
        let mut p: Vec<u8> = (0..w * h).flat_map(|_| skin).collect();
        let strand = polygon_mask(w, h, &[[3.0, 0.0], [6.0, 0.0], [10.0, 12.0], [7.0, 12.0]]);
        for (i, &b) in strand.bits.iter().enumerate() {
            if b != 0 {
                p[i * 4..i * 4 + 4].copy_from_slice(&[40, 20, 10, 255]);
            }
        }
        let before = p.clone();
        inpaint(w, h, &mut p, &strand);
        for i in 0..(w * h) as usize {
            if strand.bits[i] == 0 {
                assert_eq!(p[i * 4..i * 4 + 4], before[i * 4..i * 4 + 4], "outside pixel {i} touched");
            } else {
                for c in 0..4 {
                    assert!(p[i * 4 + c].abs_diff(skin[c]) <= 2, "hole pixel {i} = {:?}", &p[i * 4..i * 4 + 4]);
                }
            }
        }
        // A gradient around the hole: the fill stays between its borders.
        let mut g: Vec<u8> = (0..w * h).flat_map(|i| [((i % w) * 15) as u8, 0, 0, 255]).collect();
        let mut hole = Mask::new(w, h);
        for y in 4..8 {
            for x in 6..10 {
                hole.set(x, y, true);
            }
        }
        inpaint(w, h, &mut g, &hole);
        for x in 6..10 {
            let r = g[((5 * w + x) * 4) as usize];
            assert!((60..=165).contains(&r), "x={x} r={r}");
        }
        // Everything selected: nothing known, nothing changes.
        let mut all = before.clone();
        let full = Mask { w, h, bits: vec![1; (w * h) as usize] };
        inpaint(w, h, &mut all, &full);
        assert_eq!(all, before);
    }

    #[test]
    fn transparent_surroundings_do_not_bleed_color() {
        // An opaque green disk on transparent black garbage: a hole on its
        // edge gets green (premultiplied), never the garbage color.
        let (w, h) = (12u32, 12u32);
        let disk = polygon_mask(w, h, &(0..32).map(|i| {
            let a = i as f32 / 32.0 * std::f32::consts::TAU;
            [6.0 + 5.0 * a.cos(), 6.0 + 5.0 * a.sin()]
        }).collect::<Vec<_>>());
        let mut p: Vec<u8> = disk.bits.iter().flat_map(|&b| if b != 0 { [0, 255, 0, 255] } else { [255, 0, 255, 0] }).collect();
        let mut hole = Mask::new(w, h);
        for y in 4..8 {
            for x in 8..12 {
                hole.set(x, y, true);
            }
        }
        inpaint(w, h, &mut p, &hole);
        for y in 4..8 {
            for x in 8..12 {
                let i = ((y * w + x) * 4) as usize;
                if p[i + 3] > 20 {
                    assert!(p[i + 1] > 200 && p[i] < 40, "{:?}", &p[i..i + 4]);
                }
            }
        }
    }

    #[test]
    fn rect_helpers_round_trip() {
        let (w, h, p) = picture();
        let mut q = p.clone();
        q[((w + 2) * 4) as usize] = 1;
        q[((2 * w + 4) * 4 + 3) as usize] = 7;
        let r = diff_rect(w, h, &p, &q).unwrap();
        assert_eq!(r, [2, 1, 5, 3]);
        let saved = copy_rect(w, &p, r);
        paste_rect(w, &mut q, r, &saved);
        assert_eq!(q, p);
        assert!(diff_rect(w, h, &p, &p).is_none());
        let mut c = p.clone();
        clear(w, h, &mut c, &magic_wand(w, h, &p, [0, 0], 0, true));
        assert!(alpha_shape_changed(&p, &c, 8));
        assert!(!alpha_shape_changed(&p, &p, 8));
    }
}
