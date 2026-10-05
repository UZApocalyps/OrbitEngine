//! Rig meshes from pictures (M5B-04): alpha outline tracing (marching
//! squares), Douglas-Peucker simplification, interior points, constrained
//! Delaunay triangulation (`spade`, ADR 0013: editor only) and the manual
//! edits of the mesh tool (add / delete / move vertices, add / flip edges).
//!
//! Pure functions, no egui: unit tested headlessly.

use std::collections::HashMap;

use orbit_anim::rig::{Affine, Influence, MeshAttachment, WEB_MAX_VERTICES};
use spade::handles::FixedVertexHandle;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

pub type P = [f32; 2];

/// Hard limit: triangles index vertices with `u16`.
pub const MAX_VERTICES: usize = u16::MAX as usize;

/// Settings of the automatic mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshParams {
    /// Pixels whose alpha is above this are part of the shape.
    pub alpha_threshold: u8,
    /// The shape is grown by this many pixels before tracing, so the
    /// simplified outline does not cut anti-aliased edges.
    pub padding: u32,
    /// Douglas-Peucker tolerance (pixels): bigger, fewer outline vertices.
    pub tolerance: f32,
    /// Distance between interior vertices (pixels); 0: outline only.
    pub spacing: f32,
}

impl Default for MeshParams {
    fn default() -> Self {
        Self {
            alpha_threshold: 8,
            padding: 2,
            tolerance: 2.0,
            spacing: 0.0,
        }
    }
}

impl MeshParams {
    /// Default settings for a picture of `w`×`h` pixels: about 6 interior
    /// vertices across the largest side.
    pub fn for_image(w: u32, h: u32) -> Self {
        let side = w.max(h) as f32;
        Self {
            tolerance: (side / 100.0).clamp(1.0, 8.0),
            spacing: (side / 6.0).max(4.0),
            ..Self::default()
        }
    }

    /// Settings for a whole character picture: denser than
    /// [`MeshParams::for_image`] (about 12 interior vertices across the
    /// largest side) so elbows and knees have vertices to bend on.
    pub fn for_character(w: u32, h: u32) -> Self {
        let side = w.max(h) as f32;
        Self {
            tolerance: (side / 150.0).clamp(1.0, 6.0),
            spacing: (side / 12.0).max(4.0),
            ..Self::default()
        }
    }
}

/// Why a mesh operation was refused (`key()`: i18n message).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshError {
    /// Too many vertices for the index format.
    TooMany,
    /// A new vertex too close to an existing one.
    TooClose,
    /// A new vertex or edge outside the mesh.
    Outside,
    /// A mesh needs at least one triangle.
    TooFew,
    /// The edge crosses the outline or another fixed edge.
    Crossing,
    /// Invalid coordinates.
    Invalid,
}

impl MeshError {
    pub fn key(self) -> &'static str {
        match self {
            MeshError::TooMany => "rig.mesh.err.too_many",
            MeshError::TooClose => "rig.mesh.err.too_close",
            MeshError::Outside => "rig.mesh.err.outside",
            MeshError::TooFew => "rig.mesh.err.too_few",
            MeshError::Crossing => "rig.mesh.err.crossing",
            MeshError::Invalid => "rig.mesh.err.invalid",
        }
    }
}

// ------------------------------------------------------------------- mask

/// Opaque pixels of a picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub on: Vec<bool>,
}

impl Mask {
    /// `rgba`: `w * h * 4` bytes; a pixel is in when its alpha `> threshold`.
    pub fn from_rgba(w: usize, h: usize, rgba: &[u8], threshold: u8) -> Self {
        let on = (0..w * h)
            .map(|i| rgba.get(i * 4 + 3).is_some_and(|&a| a > threshold))
            .collect();
        Self { w, h, on }
    }

    pub fn get(&self, x: isize, y: isize) -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < self.w
            && (y as usize) < self.h
            && self.on[y as usize * self.w + x as usize]
    }

    /// Square dilation by `r` pixels (stays inside the picture).
    pub fn dilate(&self, r: u32) -> Self {
        if r == 0 {
            return self.clone();
        }
        let r = r as isize;
        let (w, h) = (self.w as isize, self.h as isize);
        let mut horiz = vec![false; self.on.len()];
        for y in 0..h {
            for x in 0..w {
                horiz[(y * w + x) as usize] = (x - r..=x + r).any(|k| self.get(k, y));
            }
        }
        let tmp = Mask {
            w: self.w,
            h: self.h,
            on: horiz,
        };
        let mut out = vec![false; self.on.len()];
        for y in 0..h {
            for x in 0..w {
                out[(y * w + x) as usize] = (y - r..=y + r).any(|k| tmp.get(x, k));
            }
        }
        Mask {
            w: self.w,
            h: self.h,
            on: out,
        }
    }
}

// ---------------------------------------------------------------- tracing

/// Closed outlines of the mask (outer outlines and holes), in picture
/// pixels (`0..w`, pixel centers at `i + 0.5`). Marching squares on the
/// pixel centers; diagonal-only contacts are kept apart.
pub fn trace(mask: &Mask) -> Vec<Vec<P>> {
    // Edge midpoints in doubled integer coordinates of the cell grid.
    type K = (i32, i32);
    let mut segs: Vec<[K; 2]> = Vec::new();
    for y in -1..mask.h as i32 {
        for x in -1..mask.w as i32 {
            let v = |dx: i32, dy: i32| mask.get((x + dx) as isize, (y + dy) as isize);
            let case =
                (v(0, 0) as u8) << 3 | (v(1, 0) as u8) << 2 | (v(1, 1) as u8) << 1 | v(0, 1) as u8;
            let t = (2 * x + 1, 2 * y);
            let r = (2 * x + 2, 2 * y + 1);
            let b = (2 * x + 1, 2 * y + 2);
            let l = (2 * x, 2 * y + 1);
            match case {
                1 | 14 => segs.push([l, b]),
                2 | 13 => segs.push([b, r]),
                3 | 12 => segs.push([l, r]),
                4 | 11 => segs.push([t, r]),
                6 | 9 => segs.push([t, b]),
                7 | 8 => segs.push([l, t]),
                5 => {
                    segs.push([t, r]);
                    segs.push([l, b]);
                }
                10 => {
                    segs.push([l, t]);
                    segs.push([b, r]);
                }
                _ => {}
            }
        }
    }
    let mut at: HashMap<K, Vec<usize>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        at.entry(s[0]).or_default().push(i);
        at.entry(s[1]).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let mut loops = Vec::new();
    let to_px = |k: K| [k.0 as f32 * 0.5 + 0.5, k.1 as f32 * 0.5 + 0.5];
    for s in 0..segs.len() {
        if used[s] {
            continue;
        }
        used[s] = true;
        let start = segs[s][0];
        let mut cur = segs[s][1];
        let mut pts = vec![to_px(start)];
        while cur != start {
            pts.push(to_px(cur));
            let Some(&n) = at[&cur].iter().find(|&&k| !used[k]) else {
                break;
            };
            used[n] = true;
            cur = if segs[n][0] == cur {
                segs[n][1]
            } else {
                segs[n][0]
            };
        }
        if pts.len() >= 3 {
            loops.push(pts);
        }
    }
    loops
}

/// Signed area of a closed polygon (shoelace).
pub fn area(poly: &[P]) -> f32 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        * 0.5
}

fn dist(a: P, b: P) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Distance from `p` to the segment `a`-`b`.
pub fn segment_distance(p: P, a: P, b: P) -> f32 {
    dist(p, closest_on_segment(p, a, b).0)
}

/// The closest point of `a`-`b` to `p`, and its parameter `0..1`.
pub fn closest_on_segment(p: P, a: P, b: P) -> (P, f32) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ([a[0] + dx * t, a[1] + dy * t], t)
}

/// Douglas-Peucker on a closed outline: keeps the points farther than `tol`
/// from the simplified shape (at least 3).
pub fn simplify_closed(pts: &[P], tol: f32) -> Vec<P> {
    let n = pts.len();
    if n <= 3 || tol <= 0.0 {
        return pts.to_vec();
    }
    let far = (1..n)
        .max_by(|&a, &b| dist(pts[0], pts[a]).total_cmp(&dist(pts[0], pts[b])))
        .unwrap_or(n / 2);
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    let first: Vec<usize> = (0..=far).collect();
    let second: Vec<usize> = (far..n).chain(std::iter::once(0)).collect();
    for chain in [first, second] {
        let mut stack = vec![(0usize, chain.len() - 1)];
        while let Some((s, e)) = stack.pop() {
            if e <= s + 1 {
                continue;
            }
            let (a, b) = (pts[chain[s]], pts[chain[e]]);
            let (mut best, mut bi) = (-1.0f32, s);
            for (k, &idx) in chain.iter().enumerate().take(e).skip(s + 1) {
                let d = segment_distance(pts[idx], a, b);
                if d > best {
                    best = d;
                    bi = k;
                }
            }
            if best > tol {
                keep[chain[bi]] = true;
                stack.push((s, bi));
                stack.push((bi, e));
            }
        }
    }
    let mut out: Vec<P> = (0..n).filter(|&i| keep[i]).map(|i| pts[i]).collect();
    if out.len() < 3 {
        // A sliver: keep the third farthest point.
        let third = (0..n).filter(|&i| i != 0 && i != far).max_by(|&a, &b| {
            segment_distance(pts[a], pts[0], pts[far])
                .total_cmp(&segment_distance(pts[b], pts[0], pts[far]))
        });
        out = [0, far]
            .into_iter()
            .chain(third)
            .collect::<Vec<_>>()
            .into_iter()
            .map(|i| pts[i])
            .collect();
        out.sort_by_key(|p| pts.iter().position(|q| q == p));
    }
    out
}

/// Even-odd test against closed outlines.
pub fn inside_loops(loops: &[Vec<P>], p: P) -> bool {
    let mut inside = false;
    for poly in loops {
        let n = poly.len();
        for i in 0..n {
            if crosses(poly[i], poly[(i + 1) % n], p) {
                inside = !inside;
            }
        }
    }
    inside
}

fn crosses(a: P, b: P, p: P) -> bool {
    (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
}

/// Even-odd test against an edge soup (closed loops of vertex indices).
pub fn inside_edges(vertices: &[P], edges: &[[usize; 2]], p: P) -> bool {
    edges
        .iter()
        .filter(|e| crosses(vertices[e[0]], vertices[e[1]], p))
        .count()
        % 2
        == 1
}

/// Interior vertices: a staggered grid of step `spacing`, inside the
/// outlines and at least `spacing / 2` away from them.
pub fn interior_points(loops: &[Vec<P>], spacing: f32) -> Vec<P> {
    if spacing <= 0.0 || loops.is_empty() {
        return Vec::new();
    }
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in loops.iter().flatten() {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let row = spacing * 0.866;
    let mut out = Vec::new();
    let mut y = lo[1] + row * 0.5;
    let mut odd = false;
    while y < hi[1] {
        let mut x = lo[0] + if odd { spacing } else { spacing * 0.5 };
        while x < hi[0] {
            let p = [x, y];
            if inside_loops(loops, p) && !near_outline(loops, p, spacing * 0.5) {
                out.push(p);
            }
            x += spacing;
        }
        y += row;
        odd = !odd;
    }
    out
}

fn near_outline(loops: &[Vec<P>], p: P, d: f32) -> bool {
    loops.iter().any(|poly| {
        (0..poly.len()).any(|i| segment_distance(p, poly[i], poly[(i + 1) % poly.len()]) < d)
    })
}

// ---------------------------------------------------------- triangulation

/// Constrained Delaunay triangulation of `points` with the `constraints`
/// edges, keeping the triangles whose centroid passes `keep`. Constraints
/// that would cross an earlier one are skipped (their count is returned).
/// Duplicate points are merged into the first one (the later index is then
/// unused).
pub fn triangulate(
    points: &[P],
    constraints: &[[usize; 2]],
    keep: impl Fn(P) -> bool,
) -> Result<(Vec<[usize; 3]>, usize), MeshError> {
    let mut cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::new();
    let mut handles: Vec<FixedVertexHandle> = Vec::with_capacity(points.len());
    let mut owner: HashMap<usize, usize> = HashMap::new();
    for (i, p) in points.iter().enumerate() {
        let h = cdt
            .insert(Point2::new(p[0] as f64, p[1] as f64))
            .map_err(|_| MeshError::Invalid)?;
        owner.entry(h.index()).or_insert(i);
        handles.push(h);
    }
    let mut skipped = 0;
    for c in constraints {
        let (Some(&a), Some(&b)) = (handles.get(c[0]), handles.get(c[1])) else {
            continue;
        };
        if a == b || cdt.exists_constraint(a, b) {
            continue;
        }
        if cdt.can_add_constraint(a, b) {
            cdt.add_constraint(a, b);
        } else {
            skipped += 1;
        }
    }
    let mut tris = Vec::new();
    for f in cdt.inner_faces() {
        let v = f.vertices();
        let idx = v.map(|h| owner.get(&h.fix().index()).copied());
        let [Some(a), Some(b), Some(c)] = idx else {
            continue;
        };
        let (pa, pb, pc) = (points[a], points[b], points[c]);
        let centroid = [(pa[0] + pb[0] + pc[0]) / 3.0, (pa[1] + pb[1] + pc[1]) / 3.0];
        let twice = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pc[0] - pa[0]) * (pb[1] - pa[1]);
        if twice.abs() > 1e-9 && keep(centroid) {
            tris.push([a, b, c]);
        }
    }
    Ok((tris, skipped))
}

/// A mesh in picture pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Generated {
    pub points: Vec<P>,
    pub triangles: Vec<[usize; 3]>,
    /// Outline edges that could not be kept (self-crossing outlines).
    pub skipped: usize,
}

/// The automatic mesh of a `w`×`h` RGBA picture: traced, simplified,
/// filled with interior points and triangulated. A fully transparent (or
/// unreadable) picture gives its rectangle.
pub fn generate(w: u32, h: u32, rgba: &[u8], params: &MeshParams) -> Result<Generated, MeshError> {
    let (wf, hf) = (w as f32, h as f32);
    let mask = Mask::from_rgba(w as usize, h as usize, rgba, params.alpha_threshold)
        .dilate(params.padding);
    let min_area = (params.tolerance * 2.0).max(2.0).powi(2);
    let mut loops: Vec<Vec<P>> = trace(&mask)
        .into_iter()
        .map(|l| {
            let mut c: Vec<P> = l
                .iter()
                .map(|p| [p[0].clamp(0.0, wf), p[1].clamp(0.0, hf)])
                .collect();
            c.dedup();
            densify_closed(&simplify_closed(&c, params.tolerance), params.spacing)
        })
        .filter(|l| l.len() >= 3 && area(l).abs() >= min_area)
        .collect();
    if loops.is_empty() {
        loops.push(vec![[0.0, 0.0], [wf, 0.0], [wf, hf], [0.0, hf]]);
    }
    let mut points: Vec<P> = Vec::new();
    let mut constraints = Vec::new();
    for l in &loops {
        let base = points.len();
        points.extend_from_slice(l);
        for i in 0..l.len() {
            constraints.push([base + i, base + (i + 1) % l.len()]);
        }
    }
    points.extend(interior_points(&loops, params.spacing));
    if points.len() > MAX_VERTICES.min(WEB_MAX_VERTICES * 4) {
        return Err(MeshError::TooMany);
    }
    let (tris, skipped) = triangulate(&points, &constraints, |c| inside_loops(&loops, c))?;
    let (points, triangles) = compact(&points, &tris);
    if triangles.is_empty() {
        return Err(MeshError::TooFew);
    }
    Ok(Generated {
        points,
        triangles,
        skipped,
    })
}

/// Splits the outline edges longer than `spacing` (0: unchanged), so a
/// long straight side (an arm, a leg) has vertices to bend on, like the
/// interior.
pub fn densify_closed(pts: &[P], spacing: f32) -> Vec<P> {
    if spacing <= 0.0 || pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity(pts.len());
    for (i, a) in pts.iter().enumerate() {
        let b = pts[(i + 1) % pts.len()];
        out.push(*a);
        let n = ((b[0] - a[0]).hypot(b[1] - a[1]) / spacing).ceil() as usize;
        for k in 1..n.min(256) {
            let t = k as f32 / n as f32;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    out
}

/// Drops the points no triangle uses.
fn compact(points: &[P], tris: &[[usize; 3]]) -> (Vec<P>, Vec<[usize; 3]>) {
    let mut map = vec![usize::MAX; points.len()];
    let mut out = Vec::new();
    let tris = tris
        .iter()
        .map(|t| {
            t.map(|i| {
                if map[i] == usize::MAX {
                    map[i] = out.len();
                    out.push(points[i]);
                }
                map[i]
            })
        })
        .collect();
    (out, tris)
}

/// Builds the attachment: picture pixels → rig space with `to_rig` (bind
/// pose), UVs from the picture size. Weights are left empty (rigid).
pub fn to_attachment(
    image: &str,
    g: &Generated,
    w: u32,
    h: u32,
    to_rig: &Affine,
) -> MeshAttachment {
    let (wf, hf) = (w.max(1) as f32, h.max(1) as f32);
    MeshAttachment {
        image: image.to_owned(),
        vertices: g
            .points
            .iter()
            .map(|p| to_rig.apply([p[0] / wf, p[1] / hf]))
            .collect(),
        uvs: g.points.iter().map(|p| [p[0] / wf, p[1] / hf]).collect(),
        triangles: g.triangles.iter().map(|t| t.map(|i| i as u16)).collect(),
        weights: Vec::new(),
    }
}

/// UV (`0..1`) → rig space of a region attachment of `size` placed by
/// `placement` (bone world × region transform): the quad is centered.
pub fn region_uv_to_rig(placement: &Affine, size: [f32; 2]) -> Affine {
    let local = Affine {
        a: size[0],
        b: 0.0,
        c: 0.0,
        d: size[1],
        tx: -size[0] * 0.5,
        ty: -size[1] * 0.5,
    };
    placement.mul(&local)
}

/// Rig space → UV map of an existing mesh, solved on its largest triangle
/// (meshes made here are an affine cut of their picture).
pub fn uv_map(m: &MeshAttachment) -> Option<Affine> {
    let t = m
        .triangles
        .iter()
        .max_by(|a, b| tri_area(&m.vertices, a).total_cmp(&tri_area(&m.vertices, b)))?;
    let [i0, i1, i2] = t.map(usize::from);
    let (p0, p1, p2) = (
        *m.vertices.get(i0)?,
        *m.vertices.get(i1)?,
        *m.vertices.get(i2)?,
    );
    let (u0, u1, u2) = (*m.uvs.get(i0)?, *m.uvs.get(i1)?, *m.uvs.get(i2)?);
    // M = [p1-p0 | p2-p0], U = [u1-u0 | u2-u0], L = U M^-1.
    let pm = Affine {
        a: p1[0] - p0[0],
        b: p1[1] - p0[1],
        c: p2[0] - p0[0],
        d: p2[1] - p0[1],
        tx: 0.0,
        ty: 0.0,
    };
    let um = Affine {
        a: u1[0] - u0[0],
        b: u1[1] - u0[1],
        c: u2[0] - u0[0],
        d: u2[1] - u0[1],
        tx: 0.0,
        ty: 0.0,
    };
    if pm.det().abs() < 1e-9 {
        return None;
    }
    let mut l = um.mul(&pm.inverse()?);
    let q = l.apply(p0);
    l.tx = u0[0] - q[0];
    l.ty = u0[1] - q[1];
    Some(l)
}

fn tri_area(v: &[P], t: &[u16; 3]) -> f32 {
    let get = |i: u16| v.get(i as usize).copied().unwrap_or([0.0, 0.0]);
    let (a, b, c) = (get(t[0]), get(t[1]), get(t[2]));
    ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() * 0.5
}

// ------------------------------------------------------------ topology

/// Edges used by exactly one triangle (the outline, holes included).
pub fn boundary_edges(tris: &[[u16; 3]]) -> Vec<[usize; 2]> {
    let mut count: HashMap<(usize, usize), (usize, [usize; 2])> = HashMap::new();
    for t in tris {
        for k in 0..3 {
            let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
            count.entry((a.min(b), a.max(b))).or_insert((0, [a, b])).0 += 1;
        }
    }
    let mut out: Vec<[usize; 2]> = count
        .into_values()
        .filter(|(n, _)| *n == 1)
        .map(|(_, e)| e)
        .collect();
    out.sort_unstable();
    out
}

/// Every edge once, `a < b`, sorted.
pub fn edges(tris: &[[u16; 3]]) -> Vec<[usize; 2]> {
    let mut out: Vec<[usize; 2]> = tris
        .iter()
        .flat_map(|t| (0..3).map(move |k| (t[k] as usize, t[(k + 1) % 3] as usize)))
        .map(|(a, b)| [a.min(b), a.max(b)])
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Triangulates again, keeping the outline (`boundary`) and the `pinned`
/// edges (user edges). Fails when nothing remains.
pub fn retriangulate(
    vertices: &[P],
    boundary: &[[usize; 2]],
    pinned: &[[usize; 2]],
) -> Result<Vec<[u16; 3]>, MeshError> {
    if vertices.len() > MAX_VERTICES {
        return Err(MeshError::TooMany);
    }
    let constraints: Vec<[usize; 2]> = boundary.iter().chain(pinned).copied().collect();
    let (tris, _) = triangulate(vertices, &constraints, |c| {
        inside_edges(vertices, boundary, c)
    })?;
    if tris.is_empty() {
        return Err(MeshError::TooFew);
    }
    Ok(tris.into_iter().map(|t| t.map(|i| i as u16)).collect())
}

/// The vertex of `vertices` closest to `p` within `radius`.
pub fn nearest_vertex(vertices: &[P], p: P, radius: f32) -> Option<usize> {
    vertices
        .iter()
        .enumerate()
        .map(|(i, v)| (i, dist(*v, p)))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The edge of `list` closest to `p` within `radius`.
pub fn nearest_edge(vertices: &[P], list: &[[usize; 2]], p: P, radius: f32) -> Option<[usize; 2]> {
    list.iter()
        .map(|e| (*e, segment_distance(p, vertices[e[0]], vertices[e[1]])))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _)| e)
}

fn lerp_weights(a: &[Influence], b: &[Influence], ws: [f32; 2]) -> Vec<Influence> {
    blend_weights(&[(a, ws[0]), (b, ws[1])])
}

fn blend_weights(parts: &[(&[Influence], f32)]) -> Vec<Influence> {
    let mut acc: Vec<Influence> = Vec::new();
    for (list, w) in parts {
        for (bone, v) in *list {
            match acc.iter_mut().find(|(b, _)| b == bone) {
                Some(e) => e.1 += v * w,
                None => acc.push((bone.clone(), v * w)),
            }
        }
    }
    super::weights::normalize(&mut acc);
    acc
}

/// Adds a vertex at `p` (rig space): on the outline when an outline edge is
/// within `snap` (the edge is split), otherwise inside the mesh. UV and
/// weights are interpolated. Returns its index.
pub fn add_vertex(
    m: &mut MeshAttachment,
    p: P,
    snap: f32,
    pinned: &mut [[usize; 2]],
) -> Result<usize, MeshError> {
    if m.vertices.len() + 1 > MAX_VERTICES {
        return Err(MeshError::TooMany);
    }
    if !p[0].is_finite() || !p[1].is_finite() {
        return Err(MeshError::Invalid);
    }
    if m.vertices.iter().any(|v| dist(*v, p) < snap * 0.5) {
        return Err(MeshError::TooClose);
    }
    let uv = uv_map(m);
    let weighted = !m.weights.is_empty();
    let mut boundary = boundary_edges(&m.triangles);
    let n = m.vertices.len();
    let (pos, weight) = if let Some(e) = nearest_edge(&m.vertices, &boundary, p, snap) {
        let (q, t) = closest_on_segment(p, m.vertices[e[0]], m.vertices[e[1]]);
        boundary.retain(|x| x != &e);
        boundary.push([e[0], n]);
        boundary.push([n, e[1]]);
        let w = weighted.then(|| lerp_weights(&m.weights[e[0]], &m.weights[e[1]], [1.0 - t, t]));
        (q, w)
    } else if inside_edges(&m.vertices, &boundary, p) {
        let w = weighted.then(|| {
            let t = m
                .triangles
                .iter()
                .find(|t| bary(&m.vertices, t, p).is_some());
            match t.and_then(|t| bary(&m.vertices, t, p).map(|b| (t, b))) {
                Some((t, b)) => blend_weights(&[
                    (&m.weights[t[0] as usize], b[0]),
                    (&m.weights[t[1] as usize], b[1]),
                    (&m.weights[t[2] as usize], b[2]),
                ]),
                None => m.weights[nearest_vertex(&m.vertices, p, f32::MAX).unwrap_or(0)].clone(),
            }
        });
        (p, w)
    } else {
        return Err(MeshError::Outside);
    };
    let mut verts = m.vertices.clone();
    verts.push(pos);
    let tris = retriangulate(&verts, &boundary, pinned)?;
    m.vertices = verts;
    m.uvs.push(uv.map_or([0.0, 0.0], |u| u.apply(pos)));
    if let Some(w) = weight {
        m.weights.push(w);
    }
    m.triangles = tris;
    Ok(n)
}

/// Barycentric coordinates of `p` in triangle `t`, when inside.
fn bary(v: &[P], t: &[u16; 3], p: P) -> Option<[f32; 3]> {
    let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
    let det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if det.abs() < 1e-12 {
        return None;
    }
    let l0 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / det;
    let l1 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / det;
    let l2 = 1.0 - l0 - l1;
    let e = -1e-5;
    (l0 >= e && l1 >= e && l2 >= e).then_some([l0, l1, l2])
}

/// Deletes vertex `i`; on the outline, its two neighbours are joined.
/// `pinned` edges are remapped (those using `i` are dropped).
pub fn delete_vertex(
    m: &mut MeshAttachment,
    i: usize,
    pinned: &mut Vec<[usize; 2]>,
) -> Result<(), MeshError> {
    if i >= m.vertices.len() {
        return Err(MeshError::Invalid);
    }
    if m.vertices.len() <= 3 {
        return Err(MeshError::TooFew);
    }
    let mut boundary = boundary_edges(&m.triangles);
    let into: Vec<usize> = boundary
        .iter()
        .filter(|e| e[1] == i)
        .map(|e| e[0])
        .collect();
    let out: Vec<usize> = boundary
        .iter()
        .filter(|e| e[0] == i)
        .map(|e| e[1])
        .collect();
    boundary.retain(|e| e[0] != i && e[1] != i);
    if let (Some(&a), Some(&b)) = (into.first(), out.first())
        && a != b
    {
        boundary.push([a, b]);
    }
    let remap = |k: usize| if k > i { k - 1 } else { k };
    let boundary: Vec<[usize; 2]> = boundary.iter().map(|e| e.map(remap)).collect();
    let new_pinned: Vec<[usize; 2]> = pinned
        .iter()
        .filter(|e| e[0] != i && e[1] != i)
        .map(|e| e.map(remap))
        .collect();
    let mut verts = m.vertices.clone();
    verts.remove(i);
    let tris = retriangulate(&verts, &boundary, &new_pinned)?;
    m.vertices = verts;
    if i < m.uvs.len() {
        m.uvs.remove(i);
    }
    if i < m.weights.len() {
        m.weights.remove(i);
    }
    m.triangles = tris;
    *pinned = new_pinned;
    Ok(())
}

/// Moves vertex `i` (rig space); its UV follows so the picture does not
/// slide when `keep_uv` is false (Alt-drag keeps the UV: the picture warps).
pub fn move_vertex(m: &mut MeshAttachment, i: usize, p: P, map: Option<&Affine>, keep_uv: bool) {
    if let Some(v) = m.vertices.get_mut(i) {
        *v = p;
        if !keep_uv && let (Some(map), Some(uv)) = (map, m.uvs.get_mut(i)) {
            *uv = map.apply(p);
        }
    }
}

/// Adds the user edge `a`-`b` (kept by later re-triangulations).
pub fn add_edge(
    m: &mut MeshAttachment,
    a: usize,
    b: usize,
    pinned: &mut Vec<[usize; 2]>,
) -> Result<(), MeshError> {
    let n = m.vertices.len();
    if a == b || a >= n || b >= n {
        return Err(MeshError::Invalid);
    }
    let key = [a.min(b), a.max(b)];
    if edges(&m.triangles).contains(&key) && pinned.contains(&key) {
        return Ok(());
    }
    let boundary = boundary_edges(&m.triangles);
    let (pa, pb) = (m.vertices[a], m.vertices[b]);
    if !inside_edges(
        &m.vertices,
        &boundary,
        [(pa[0] + pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5],
    ) {
        return Err(MeshError::Outside);
    }
    let mut new_pinned = pinned.clone();
    new_pinned.push(key);
    let tris = retriangulate(&m.vertices, &boundary, &new_pinned)?;
    if !edges(&tris).contains(&key) {
        return Err(MeshError::Crossing);
    }
    m.triangles = tris;
    *pinned = new_pinned;
    Ok(())
}

/// Flips the inner edge `a`-`b` (shared by two triangles forming a convex
/// quad). Returns false when it cannot be flipped.
pub fn flip_edge(m: &mut MeshAttachment, a: usize, b: usize) -> bool {
    let has = |t: &[u16; 3], v: usize| t.iter().any(|&x| x as usize == v);
    let shared: Vec<usize> = (0..m.triangles.len())
        .filter(|&k| has(&m.triangles[k], a) && has(&m.triangles[k], b))
        .collect();
    let [t1, t2] = shared[..] else { return false };
    let other = |t: &[u16; 3]| t.iter().map(|&x| x as usize).find(|&x| x != a && x != b);
    let (Some(c), Some(d)) = (other(&m.triangles[t1]), other(&m.triangles[t2])) else {
        return false;
    };
    let v = &m.vertices;
    let side = |p: P, q: P, r: P| (q[0] - p[0]) * (r[1] - p[1]) - (r[0] - p[0]) * (q[1] - p[1]);
    // Convex quad: c, d on both sides of a-b and a, b on both sides of c-d.
    if side(v[a], v[b], v[c]) * side(v[a], v[b], v[d]) >= 0.0
        || side(v[c], v[d], v[a]) * side(v[c], v[d], v[b]) >= 0.0
    {
        return false;
    }
    // Keep the winding of the mesh.
    let o = m.triangles[t1].map(usize::from);
    let sign = side(v[o[0]], v[o[1]], v[o[2]]).signum();
    let orient = |t: [usize; 3]| -> [u16; 3] {
        let s = side(v[t[0]], v[t[1]], v[t[2]]).signum();
        if s == sign {
            t.map(|x| x as u16)
        } else {
            [t[0] as u16, t[2] as u16, t[1] as u16]
        }
    };
    let (n1, n2) = (orient([c, d, a]), orient([d, c, b]));
    m.triangles[t1] = n1;
    m.triangles[t2] = n2;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(w: usize, h: usize, on: impl Fn(usize, usize) -> bool) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                if on(x, y) {
                    v[(y * w + x) * 4 + 3] = 255;
                }
            }
        }
        v
    }

    fn tri_area_usize(p: &[P], t: &[usize; 3]) -> f32 {
        let (a, b, c) = (p[t[0]], p[t[1]], p[t[2]]);
        ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) * 0.5
    }

    #[test]
    fn traces_a_square_and_a_hole() {
        // 10x10 square with a 4x4 hole.
        let px = rgba(16, 16, |x, y| {
            (3..13).contains(&x)
                && (3..13).contains(&y)
                && !((6..10).contains(&x) && (6..10).contains(&y))
        });
        let mask = Mask::from_rgba(16, 16, &px, 8);
        let loops = trace(&mask);
        assert_eq!(loops.len(), 2, "outer outline and hole");
        let mut areas: Vec<f32> = loops.iter().map(|l| area(l).abs()).collect();
        areas.sort_by(f32::total_cmp);
        // Through pixel-edge midpoints: 10x10 minus the 4 corner half-cells.
        assert!((areas[1] - 99.5).abs() < 1.0, "{areas:?}");
        assert!((areas[0] - 15.5).abs() < 1.0, "{areas:?}");
        // Center of the hole is out, the ring is in.
        assert!(!inside_loops(&loops, [8.0, 8.0]));
        assert!(inside_loops(&loops, [4.0, 4.0]));
        assert!(!inside_loops(&loops, [1.0, 1.0]));
    }

    #[test]
    fn diagonal_pixels_stay_separate_islands() {
        let px = rgba(4, 4, |x, y| (x, y) == (1, 1) || (x, y) == (2, 2));
        let loops = trace(&Mask::from_rgba(4, 4, &px, 0));
        assert_eq!(loops.len(), 2);
    }

    #[test]
    fn simplification_keeps_corners_and_drops_collinear_points() {
        let mut sq = Vec::new();
        for i in 0..10 {
            sq.push([i as f32, 0.0]);
        }
        for i in 0..10 {
            sq.push([10.0, i as f32]);
        }
        for i in 0..10 {
            sq.push([10.0 - i as f32, 10.0]);
        }
        for i in 0..10 {
            sq.push([0.0, 10.0 - i as f32]);
        }
        let s = simplify_closed(&sq, 0.5);
        assert_eq!(s.len(), 4, "{s:?}");
        assert!((area(&s).abs() - 100.0).abs() < 1e-3);
        // A circle keeps more points at a small tolerance.
        let circle: Vec<P> = (0..200)
            .map(|i| {
                let a = i as f32 / 200.0 * std::f32::consts::TAU;
                [50.0 * a.cos(), 50.0 * a.sin()]
            })
            .collect();
        let fine = simplify_closed(&circle, 0.2).len();
        let coarse = simplify_closed(&circle, 3.0).len();
        assert!(fine > coarse && coarse >= 6, "{fine} {coarse}");
        for p in &circle {
            let s = simplify_closed(&circle, 3.0);
            let d = (0..s.len())
                .map(|i| segment_distance(*p, s[i], s[(i + 1) % s.len()]))
                .fold(f32::MAX, f32::min);
            assert!(d <= 3.0 + 1e-3);
        }
    }

    /// Valid triangulation: indices in range, no degenerate triangle, every
    /// edge shared by at most 2 triangles, total area = shape area.
    fn assert_valid(points: &[P], tris: &[[usize; 3]], expected_area: f32) {
        assert!(!tris.is_empty());
        let mut count: HashMap<(usize, usize), usize> = HashMap::new();
        let mut total = 0.0;
        let sign = tri_area_usize(points, &tris[0]).signum();
        for t in tris {
            assert!(t.iter().all(|&i| i < points.len()));
            let a = tri_area_usize(points, t);
            assert!(a.abs() > 1e-6, "degenerate");
            assert_eq!(a.signum(), sign, "consistent winding");
            total += a.abs();
            for k in 0..3 {
                let (x, y) = (t[k], t[(k + 1) % 3]);
                *count.entry((x.min(y), x.max(y))).or_default() += 1;
            }
        }
        assert!(count.values().all(|&n| n <= 2), "non-manifold edge");
        assert!(
            (total - expected_area).abs() < expected_area * 0.02 + 0.5,
            "area {total} vs {expected_area}"
        );
    }

    #[test]
    fn generated_mesh_covers_an_l_shape_with_a_hole() {
        let (w, h) = (64usize, 64usize);
        let shape = |x: usize, y: usize| {
            let l = (x < 24 && y < 60) || ((40..60).contains(&y) && x < 60);
            let hole = (8..16).contains(&x) && (8..30).contains(&y);
            l && !hole
        };
        let px = rgba(w, h, shape);
        let params = MeshParams {
            alpha_threshold: 8,
            padding: 0,
            tolerance: 0.6,
            spacing: 8.0,
        };
        let g = generate(w as u32, h as u32, &px, &params).unwrap();
        let expected = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| shape(x, y))
            .count() as f32;
        // The traced outline runs through edge midpoints: slightly smaller.
        assert_valid(&g.points, &g.triangles, expected - 6.0);
        assert_eq!(g.skipped, 0);
        // Interior points were added and the hole stays empty.
        assert!(g.points.len() > 12);
        for t in &g.triangles {
            let c = [
                (g.points[t[0]][0] + g.points[t[1]][0] + g.points[t[2]][0]) / 3.0,
                (g.points[t[0]][1] + g.points[t[1]][1] + g.points[t[2]][1]) / 3.0,
            ];
            assert!(
                !((9.0..15.0).contains(&c[0]) && (9.0..29.0).contains(&c[1])),
                "triangle in the hole"
            );
        }
        // Denser spacing, more vertices; coarser tolerance, fewer.
        let dense = generate(
            w as u32,
            h as u32,
            &px,
            &MeshParams {
                spacing: 4.0,
                ..params
            },
        )
        .unwrap();
        assert!(dense.points.len() > g.points.len());
        let none = generate(
            w as u32,
            h as u32,
            &px,
            &MeshParams {
                spacing: 0.0,
                ..params
            },
        )
        .unwrap();
        assert!(none.points.len() < g.points.len());
    }

    #[test]
    fn transparent_picture_gives_its_rectangle_and_uvs_match() {
        let px = vec![0u8; 8 * 4 * 4];
        let g = generate(8, 4, &px, &MeshParams::default()).unwrap();
        assert_eq!(g.points.len(), 4);
        assert_eq!(g.triangles.len(), 2);
        let place = Affine {
            tx: 100.0,
            ..Affine::IDENTITY
        };
        let m = to_attachment("a.png", &g, 8, 4, &region_uv_to_rig(&place, [16.0, 8.0]));
        for (v, uv) in m.vertices.iter().zip(&m.uvs) {
            assert!((0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]));
            // Centered quad of 16x8 at x = 100.
            assert!((v[0] - (100.0 - 8.0 + uv[0] * 16.0)).abs() < 1e-4);
            assert!((v[1] - (-4.0 + uv[1] * 8.0)).abs() < 1e-4);
        }
        let map = uv_map(&m).unwrap();
        for (v, uv) in m.vertices.iter().zip(&m.uvs) {
            let u = map.apply(*v);
            assert!((u[0] - uv[0]).abs() < 1e-4 && (u[1] - uv[1]).abs() < 1e-4);
        }
    }

    fn square_mesh() -> MeshAttachment {
        MeshAttachment {
            image: "a.png".into(),
            vertices: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            weights: Vec::new(),
        }
    }

    fn total_area(m: &MeshAttachment) -> f32 {
        m.triangles.iter().map(|t| tri_area(&m.vertices, t)).sum()
    }

    #[test]
    fn add_vertex_inside_and_on_the_outline() {
        let mut m = square_mesh();
        let mut pinned = Vec::new();
        let i = add_vertex(&mut m, [5.0, 4.0], 1.0, &mut pinned).unwrap();
        assert_eq!(i, 4);
        assert_eq!(m.triangles.len(), 4);
        assert!((m.uvs[4][0] - 0.5).abs() < 1e-4 && (m.uvs[4][1] - 0.4).abs() < 1e-4);
        assert!((total_area(&m) - 100.0).abs() < 1e-3);
        // Near the top edge: snapped onto it, the outline is split.
        let j = add_vertex(&mut m, [3.0, 0.4], 1.0, &mut pinned).unwrap();
        assert!((m.vertices[j][1]).abs() < 1e-5);
        assert!(boundary_edges(&m.triangles).iter().any(|e| e.contains(&j)));
        assert!((total_area(&m) - 100.0).abs() < 1e-3);
        assert_eq!(
            add_vertex(&mut m, [30.0, 30.0], 1.0, &mut pinned),
            Err(MeshError::Outside)
        );
        assert_eq!(
            add_vertex(&mut m, [5.1, 4.0], 1.0, &mut pinned),
            Err(MeshError::TooClose)
        );
    }

    #[test]
    fn delete_vertex_inner_and_outline() {
        let mut m = square_mesh();
        let mut pinned = Vec::new();
        add_vertex(&mut m, [5.0, 5.0], 1.0, &mut pinned).unwrap();
        delete_vertex(&mut m, 4, &mut pinned).unwrap();
        assert_eq!(m.vertices.len(), 4);
        assert_eq!(m.triangles.len(), 2);
        // Deleting a corner leaves a triangle.
        delete_vertex(&mut m, 2, &mut pinned).unwrap();
        assert_eq!(m.vertices.len(), 3);
        assert_eq!(m.uvs.len(), 3);
        assert!((total_area(&m) - 50.0).abs() < 1e-3);
        assert_eq!(
            delete_vertex(&mut m, 0, &mut pinned),
            Err(MeshError::TooFew)
        );
    }

    #[test]
    fn edges_can_be_flipped_and_pinned() {
        let mut m = square_mesh();
        assert!(edges(&m.triangles).contains(&[0, 2]));
        assert!(flip_edge(&mut m, 0, 2));
        assert!(edges(&m.triangles).contains(&[1, 3]));
        assert!(!edges(&m.triangles).contains(&[0, 2]));
        assert!((total_area(&m) - 100.0).abs() < 1e-3);
        assert!(!flip_edge(&mut m, 0, 1), "outline edges cannot flip");
        // Pin the other diagonal: re-triangulations keep it.
        let mut pinned = Vec::new();
        add_edge(&mut m, 0, 2, &mut pinned).unwrap();
        assert!(edges(&m.triangles).contains(&[0, 2]));
        add_vertex(&mut m, [2.0, 8.0], 1.0, &mut pinned).unwrap();
        assert!(edges(&m.triangles).contains(&[0, 2]));
        // An edge across the pinned one is refused.
        let k = add_vertex(&mut m, [8.0, 2.0], 1.0, &mut pinned).unwrap();
        assert_eq!(
            add_edge(&mut m, 4, k, &mut pinned),
            Err(MeshError::Crossing)
        );
    }

    #[test]
    fn concave_outline_rejects_outside_edges() {
        // A "U": the edge across the notch is outside.
        let pts = vec![
            [0.0, 0.0],
            [3.0, 0.0],
            [3.0, 8.0],
            [7.0, 8.0],
            [7.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
        ];
        let cons: Vec<[usize; 2]> = (0..8).map(|i| [i, (i + 1) % 8]).collect();
        let loops = vec![pts.clone()];
        let (tris, skipped) = triangulate(&pts, &cons, |c| inside_loops(&loops, c)).unwrap();
        assert_eq!(skipped, 0);
        assert_valid(&pts, &tris, 100.0 - 32.0);
        let mut m = MeshAttachment {
            image: String::new(),
            vertices: pts.clone(),
            uvs: pts.iter().map(|p| [p[0] / 10.0, p[1] / 10.0]).collect(),
            triangles: tris.iter().map(|t| t.map(|i| i as u16)).collect(),
            weights: Vec::new(),
        };
        let mut pinned = Vec::new();
        assert_eq!(add_edge(&mut m, 1, 4, &mut pinned), Err(MeshError::Outside));
    }
}
