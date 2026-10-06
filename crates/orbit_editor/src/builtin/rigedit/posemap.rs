//! Where a point of the viewport lands on a part's picture while the rig is
//! posed (pose mode, a clip at the playhead, a cross-fade): the picture tools
//! paint and select on the deformed part, the pixels change in the picture
//! itself (picture pixels, the same space as in the rest pose).
//!
//! * A rigid picture (region attachment) is one affine map: exact.
//! * A skinned mesh is mapped per triangle: the deformed (skinned) triangle
//!   that contains the point gives barycentric weights, applied to its UVs.
//!   A uniform grid over the deformed triangles (and one over the UVs, for
//!   the way back) keeps the lookup cheap; it is built once per pose change,
//!   only while a picture tool is in use.
//!
//! Limits: where a mesh folds over itself, the triangle drawn last (on top)
//! wins, like on screen. A selection outline is mapped vertex by vertex
//! (edges densified first), so an outline crossing a fold or a strongly
//! stretched area maps to the outline of what was visible, not of the hidden
//! layer below. Points off the mesh (an outline drawn around it) are
//! extrapolated from the nearest triangle.

use orbit_anim::rig::Affine;

/// A uniform grid of triangle indices over a set of points.
struct Grid {
    lo: [f32; 2],
    cell: [f32; 2],
    n: [usize; 2],
    cells: Vec<Vec<u32>>,
}

impl Grid {
    fn build(pts: &[[f32; 2]], tris: &[[u16; 3]]) -> Self {
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for p in pts.iter().filter(|p| p[0].is_finite() && p[1].is_finite()) {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if lo[0] > hi[0] {
            return Self { lo: [0.0; 2], cell: [1.0; 2], n: [1, 1], cells: vec![Vec::new()] };
        }
        let side = (tris.len() as f32).sqrt().ceil().clamp(1.0, 64.0) as usize;
        let n = [side, side];
        let cell = [((hi[0] - lo[0]) / side as f32).max(1e-6), ((hi[1] - lo[1]) / side as f32).max(1e-6)];
        let mut g = Self { lo, cell, n, cells: vec![Vec::new(); side * side] };
        for (ti, t) in tris.iter().enumerate() {
            let Some(v) = t.iter().map(|&i| pts.get(i as usize).copied()).collect::<Option<Vec<_>>>() else { continue };
            let (mut a, mut b) = ([f32::MAX; 2], [f32::MIN; 2]);
            for p in &v {
                for k in 0..2 {
                    a[k] = a[k].min(p[k]);
                    b[k] = b[k].max(p[k]);
                }
            }
            let (x0, y0) = g.index(a);
            let (x1, y1) = g.index(b);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    g.cells[y * n[0] + x].push(ti as u32);
                }
            }
        }
        g
    }

    fn index(&self, p: [f32; 2]) -> (usize, usize) {
        let f = |k: usize| (((p[k] - self.lo[k]) / self.cell[k]).floor().max(0.0) as usize).min(self.n[k] - 1);
        (f(0), f(1))
    }

    fn candidates(&self, p: [f32; 2]) -> &[u32] {
        let inside = (0..2).all(|k| p[k] >= self.lo[k] && p[k] <= self.lo[k] + self.cell[k] * self.n[k] as f32);
        if !inside {
            return &[];
        }
        let (x, y) = self.index(p);
        &self.cells[y * self.n[0] + x]
    }
}

fn cross(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Barycentric weights of `p` in `(a, b, c)` (`None` when degenerate).
fn bary(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> Option<[f32; 3]> {
    let d = cross(a, b, c);
    if d.abs() < 1e-12 {
        return None;
    }
    let l0 = cross(p, b, c) / d;
    let l1 = cross(a, p, c) / d;
    Some([l0, l1, 1.0 - l0 - l1])
}

fn mix(l: [f32; 3], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> [f32; 2] {
    [l[0] * a[0] + l[1] * b[0] + l[2] * c[0], l[0] * a[1] + l[1] * b[1] + l[2] * c[1]]
}

fn dist_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 <= 0.0 { 0.0 } else { (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0) };
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
}

/// Deformed triangles with their picture coordinates.
pub struct TriMap {
    /// Rig space (deformed).
    pub pos: Vec<[f32; 2]>,
    /// Picture pixels.
    pub px: Vec<[f32; 2]>,
    pub tris: Vec<[u16; 3]>,
    by_pos: Grid,
    by_px: Grid,
}

impl TriMap {
    fn corners(&self, ti: usize, of_px: bool) -> Option<[[f32; 2]; 3]> {
        let t = self.tris.get(ti)?;
        let src = if of_px { &self.px } else { &self.pos };
        Some([*src.get(t[0] as usize)?, *src.get(t[1] as usize)?, *src.get(t[2] as usize)?])
    }

    /// The topmost triangle containing `p` (rig space, or picture pixels
    /// when `of_px`) and the weights of `p` in it.
    fn find(&self, p: [f32; 2], of_px: bool) -> Option<(usize, [f32; 3])> {
        let grid = if of_px { &self.by_px } else { &self.by_pos };
        let eps = -1e-4;
        grid.candidates(p)
            .iter()
            .rev()
            .filter_map(|&ti| {
                let [a, b, c] = self.corners(ti as usize, of_px)?;
                let l = bary(p, a, b, c)?;
                (l.iter().all(|&x| x >= eps)).then_some((ti as usize, l))
            })
            .max_by_key(|(ti, _)| *ti)
    }

    /// The triangle nearest to `p` (rig space) and the (extrapolated) weights.
    fn nearest(&self, p: [f32; 2]) -> Option<(usize, [f32; 3])> {
        let mut best: Option<(f32, usize, [f32; 3])> = None;
        for ti in 0..self.tris.len() {
            let Some([a, b, c]) = self.corners(ti, false) else { continue };
            let Some(l) = bary(p, a, b, c) else { continue };
            let d = if l.iter().all(|&x| x >= 0.0) {
                0.0
            } else {
                dist_segment(p, a, b).min(dist_segment(p, b, c)).min(dist_segment(p, c, a))
            };
            if best.as_ref().is_none_or(|(bd, _, _)| d < *bd) {
                best = Some((d, ti, l));
            }
        }
        best.map(|(_, ti, l)| (ti, l))
    }

    fn uv_of(&self, ti: usize, l: [f32; 3]) -> Option<[f32; 2]> {
        let [a, b, c] = self.corners(ti, true)?;
        Some(mix(l, a, b, c))
    }

    /// Picture pixels per rig unit in triangle `ti` (sqrt of the Jacobian's
    /// determinant of rig → picture).
    fn scale_of(&self, ti: usize) -> Option<f32> {
        let [p0, p1, p2] = self.corners(ti, false)?;
        let [u0, u1, u2] = self.corners(ti, true)?;
        let dp = cross(p0, p1, p2);
        let du = cross(u0, u1, u2);
        (dp.abs() > 1e-12).then(|| (du / dp).abs().sqrt())
    }

    /// Edges used by one triangle only (the outline of the mesh), rig space.
    pub fn outline(&self) -> Vec<[[f32; 2]; 2]> {
        let mut count: std::collections::HashMap<(u16, u16), u32> = std::collections::HashMap::new();
        for t in &self.tris {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        count
            .into_iter()
            .filter(|(_, n)| *n == 1)
            .filter_map(|((a, b), _)| Some([*self.pos.get(a as usize)?, *self.pos.get(b as usize)?]))
            .collect()
    }
}

/// Rig space ↔ picture pixels of one shown part.
pub enum PicMap {
    /// Picture pixels → rig space (rigid picture, or any part at rest).
    Affine(Affine),
    Mesh(TriMap),
}

impl PicMap {
    /// A rigid picture: `uv_to_rig` maps UV (`0..1`) to rig space.
    pub fn affine(uv_to_rig: &Affine, w: u32, h: u32) -> Self {
        let scale = Affine { a: 1.0 / w.max(1) as f32, b: 0.0, c: 0.0, d: 1.0 / h.max(1) as f32, tx: 0.0, ty: 0.0 };
        Self::Affine(uv_to_rig.mul(&scale))
    }

    /// Skinned triangles: deformed positions (rig space) and UVs (`0..1`).
    pub fn mesh(pos: &[[f32; 2]], uvs: &[[f32; 2]], tris: &[[u16; 3]], w: u32, h: u32) -> Self {
        let px: Vec<[f32; 2]> = uvs.iter().map(|uv| [uv[0] * w as f32, uv[1] * h as f32]).collect();
        let tris: Vec<[u16; 3]> =
            tris.iter().copied().filter(|t| t.iter().all(|&i| (i as usize) < pos.len() && (i as usize) < px.len())).collect();
        let by_pos = Grid::build(pos, &tris);
        let by_px = Grid::build(&px, &tris);
        Self::Mesh(TriMap { pos: pos.to_vec(), px, tris, by_pos, by_px })
    }

    /// Picture pixel under a rig point (`None` off a mesh).
    pub fn to_px(&self, rig: [f32; 2]) -> Option<[f32; 2]> {
        match self {
            Self::Affine(m) => Some(m.inverse()?.apply(rig)),
            Self::Mesh(t) => {
                let (ti, l) = t.find(rig, false)?;
                t.uv_of(ti, l)
            }
        }
    }

    /// Like [`Self::to_px`], extrapolated from the nearest triangle off a
    /// mesh (selection outlines drawn around a part).
    pub fn to_px_extrapolated(&self, rig: [f32; 2]) -> Option<[f32; 2]> {
        match self {
            Self::Affine(_) => self.to_px(rig),
            Self::Mesh(t) => {
                let (ti, l) = t.find(rig, false).or_else(|| t.nearest(rig))?;
                t.uv_of(ti, l)
            }
        }
    }

    /// Rig point showing a picture pixel (`None` when no triangle shows it).
    pub fn to_rig(&self, px: [f32; 2]) -> Option<[f32; 2]> {
        match self {
            Self::Affine(m) => Some(m.apply(px)),
            Self::Mesh(t) => {
                let (ti, l) = t.find(px, true)?;
                let [a, b, c] = t.corners(ti, false)?;
                Some(mix(l, a, b, c))
            }
        }
    }

    /// Picture pixels per rig unit around a rig point (the local scale of
    /// the deformation).
    pub fn px_per_rig(&self, rig: [f32; 2]) -> Option<f32> {
        match self {
            Self::Affine(m) => {
                let d = m.det().abs();
                (d > 1e-12).then(|| 1.0 / d.sqrt())
            }
            Self::Mesh(t) => t.scale_of(t.find(rig, false)?.0),
        }
    }

    /// A closed outline drawn in rig space, in picture pixels: edges
    /// densified to `step` rig units (meshes only), points off the mesh
    /// extrapolated.
    pub fn outline_to_px(&self, rig: &[[f32; 2]], step: f32) -> Vec<[f32; 2]> {
        if let Self::Affine(_) = self {
            return rig.iter().filter_map(|p| self.to_px(*p)).collect();
        }
        let step = step.max(1e-4);
        let mut out = Vec::new();
        for (i, a) in rig.iter().enumerate() {
            let b = rig[(i + 1) % rig.len()];
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let n = ((len / step).ceil() as usize).clamp(1, 4096);
            for s in 0..n {
                let t = s as f32 / n as f32;
                if let Some(p) = self.to_px_extrapolated([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]) {
                    out.push(p);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_project::rigmesh::meshgen::region_uv_to_rig;

    fn rot(deg: f32, t: [f32; 2]) -> Affine {
        let (s, c) = deg.to_radians().sin_cos();
        Affine { a: c, b: s, c: -s, d: c, tx: t[0], ty: t[1] }
    }

    fn close(a: [f32; 2], b: [f32; 2], tol: f32) -> bool {
        (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol
    }

    /// A grid mesh over a `w`×`h` picture, `n`×`n` cells, rest positions
    /// centered on the origin (1 rig unit per pixel).
    fn grid_mesh(w: u32, h: u32, n: usize) -> (Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<[u16; 3]>) {
        let (mut pos, mut uvs, mut tris) = (Vec::new(), Vec::new(), Vec::new());
        for j in 0..=n {
            for i in 0..=n {
                let uv = [i as f32 / n as f32, j as f32 / n as f32];
                uvs.push(uv);
                pos.push([uv[0] * w as f32 - w as f32 * 0.5, uv[1] * h as f32 - h as f32 * 0.5]);
            }
        }
        for j in 0..n {
            for i in 0..n {
                let k = (j * (n + 1) + i) as u16;
                let r = (n + 1) as u16;
                tris.push([k, k + 1, k + r + 1]);
                tris.push([k, k + r + 1, k + r]);
            }
        }
        (pos, uvs, tris)
    }

    #[test]
    fn rest_and_rotated_bone_round_trip_within_a_pixel() {
        let (w, h) = (64, 32);
        let rest = PicMap::affine(&region_uv_to_rig(&Affine::IDENTITY, [w as f32, h as f32]), w, h);
        let posed = PicMap::affine(&region_uv_to_rig(&rot(37.0, [12.0, -5.0]), [w as f32, h as f32]), w, h);
        for px in [[0.5, 0.5], [10.25, 20.75], [63.5, 31.5], [32.0, 16.0]] {
            for m in [&rest, &posed] {
                let rig = m.to_rig(px).unwrap();
                assert!(close(m.to_px(rig).unwrap(), px, 1e-3));
            }
            // The same picture pixel, seen where the rotated bone shows it.
            let seen = rot(37.0, [12.0, -5.0]).apply(rest.to_rig(px).unwrap());
            assert!(close(posed.to_px(seen).unwrap(), px, 1e-2));
        }
        assert!((posed.px_per_rig([0.0, 0.0]).unwrap() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_skinned_bend_maps_back_to_the_rest_pixel() {
        let (w, h) = (80, 40);
        let (rest_pos, uvs, tris) = grid_mesh(w, h, 8);
        // Bend: the right half rotates 60° around the middle (rigid
        // per side, a fold-free hinge on the middle column).
        let hinge = rot(60.0, [0.0, 0.0]);
        let bend = |p: [f32; 2]| if p[0] > 1e-3 { hinge.apply(p) } else { p };
        let posed_pos: Vec<[f32; 2]> = rest_pos.iter().map(|p| bend(*p)).collect();
        let m = PicMap::mesh(&posed_pos, &uvs, &tris, w, h);
        for px in [[5.5, 5.5], [20.25, 30.5], [60.5, 10.5], [75.25, 35.75], [41.0, 20.0]] {
            let rest_rig = [px[0] - w as f32 * 0.5, px[1] - h as f32 * 0.5];
            let seen = bend(rest_rig);
            let back = m.to_px(seen).unwrap();
            assert!(close(back, px, 1.0), "{px:?} -> {back:?}");
            // And the way back.
            assert!(close(m.to_rig(px).unwrap(), seen, 1e-2));
        }
        // Off the mesh: nothing to paint, but an outline still maps.
        assert!(m.to_px([500.0, 500.0]).is_none());
        assert!(m.to_px_extrapolated([-41.0, 0.0]).is_some_and(|p| close(p, [-1.0, 20.0], 1e-2)));
        // Stretched triangles report their local scale.
        let stretched: Vec<[f32; 2]> = rest_pos.iter().map(|p| [p[0] * 2.0, p[1]]).collect();
        let s = PicMap::mesh(&stretched, &uvs, &tris, w, h);
        assert!((s.px_per_rig([10.0, 0.0]).unwrap() - 0.5f32.sqrt()).abs() < 1e-3);
    }
}
