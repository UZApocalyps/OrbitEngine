//! Mesh weights (M5B-05): normalization (≤ 4 bones per vertex, sum 1),
//! automatic weights from the distance to the bones, the weight brush
//! (add / subtract / smooth) and the heat map colors.
//!
//! Pure functions, no egui: unit tested headlessly.

use orbit_anim::rig::{Affine, BoneId, Influence, MAX_INFLUENCES, MeshAttachment, Rig};

use super::meshgen::{P, segment_distance};
use super::{bone_index, rest_locals, world_matrices};

/// Influences below this are dropped.
pub const EPSILON: f32 = 1e-4;

/// Merges duplicate bones, drops tiny / invalid weights, keeps the
/// [`MAX_INFLUENCES`] heaviest and scales them to sum 1 (heaviest first).
/// An empty or all-zero list stays empty.
pub fn normalize(list: &mut Vec<Influence>) {
    let mut merged: Vec<Influence> = Vec::with_capacity(list.len());
    for (b, w) in list.drain(..) {
        if !w.is_finite() || w <= 0.0 {
            continue;
        }
        match merged.iter_mut().find(|(x, _)| *x == b) {
            Some(e) => e.1 += w,
            None => merged.push((b, w)),
        }
    }
    merged.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    merged.truncate(MAX_INFLUENCES);
    let sum: f32 = merged.iter().map(|x| x.1).sum();
    if sum <= 0.0 {
        return;
    }
    merged.iter_mut().for_each(|x| x.1 /= sum);
    merged.retain(|x| x.1 >= EPSILON);
    let sum: f32 = merged.iter().map(|x| x.1).sum();
    merged.iter_mut().for_each(|x| x.1 /= sum);
    *list = merged;
}

/// Weight of `bone` in `list`.
pub fn weight_of(list: &[Influence], bone: &BoneId) -> f32 {
    list.iter().find(|(b, _)| b == bone).map_or(0.0, |x| x.1)
}

/// Makes the weights explicit: an unweighted mesh (rigid) gets 100 % on the
/// slot's bone; vertices without weight too. Everything is normalized.
pub fn ensure_weights(m: &mut MeshAttachment, slot_bone: &BoneId) {
    m.weights.resize(m.vertices.len(), Vec::new());
    for w in &mut m.weights {
        normalize(w);
        if w.is_empty() {
            w.push((slot_bone.clone(), 1.0));
        }
    }
}

/// A bone as a segment in rig space (bind pose).
#[derive(Clone, Debug, PartialEq)]
pub struct BoneSeg {
    pub id: BoneId,
    pub head: P,
    pub tip: P,
}

/// The bones of the rest pose as segments.
pub fn bone_segments(rig: &Rig) -> Vec<BoneSeg> {
    let world = world_matrices(rig, &rest_locals(rig));
    rig.bones
        .iter()
        .zip(&world)
        .map(|(b, m)| BoneSeg {
            id: b.id.clone(),
            head: m.origin(),
            tip: m.apply([b.length, 0.0]),
        })
        .collect()
}

/// Automatic weights: inverse distance to each bone segment raised to
/// `falloff` (bigger: sharper joints), the 4 nearest bones, normalized.
/// `scale` (rig units, e.g. the mesh size / 20) keeps vertices lying on a
/// bone from taking it all.
pub fn auto_weights(
    vertices: &[P],
    bones: &[BoneSeg],
    falloff: f32,
    scale: f32,
) -> Vec<Vec<Influence>> {
    if bones.is_empty() {
        return vec![Vec::new(); vertices.len()];
    }
    let scale = scale.max(1e-3);
    vertices
        .iter()
        .map(|v| {
            let mut list: Vec<Influence> = bones
                .iter()
                .map(|b| {
                    let d = segment_distance(*v, b.head, b.tip) / scale;
                    (b.id.clone(), 1.0 / (1.0 + d).powf(falloff.max(0.1)))
                })
                .collect();
            normalize(&mut list);
            list
        })
        .collect()
}

/// Blends every vertex's weights toward the average of its mesh neighbours
/// (`amount` `0..1`), `iterations` times. Softens the seams the distance
/// rule leaves between bones; lists stay normalized.
pub fn smooth_weights(weights: &mut [Vec<Influence>], adj: &[Vec<usize>], iterations: usize, amount: f32) {
    let f = amount.clamp(0.0, 1.0);
    for _ in 0..iterations {
        let before = weights.to_vec();
        for (i, list) in weights.iter_mut().enumerate() {
            let near = adj.get(i).map_or(&[][..], Vec::as_slice);
            if near.is_empty() || before[i].is_empty() {
                continue;
            }
            let share = f / near.len() as f32;
            let mut out: Vec<Influence> = before[i].iter().map(|(b, w)| (b.clone(), w * (1.0 - f))).collect();
            for &n in near {
                for (b, w) in before.get(n).map_or(&[][..], Vec::as_slice) {
                    match out.iter_mut().find(|(x, _)| x == b) {
                        Some(e) => e.1 += w * share,
                        None => out.push((b.clone(), w * share)),
                    }
                }
            }
            normalize(&mut out);
            if !out.is_empty() {
                *list = out;
            }
        }
    }
}

/// "Bind to bones": automatic weights of a mesh from the bones of `rig`
/// (distance to each bone *segment*, so a vertex along a forearm follows
/// the forearm, not the closest joint), then two light smoothing passes
/// along the mesh edges. `falloff`: sharpness of the split between bones.
pub fn bind_mesh(rig: &Rig, m: &MeshAttachment, falloff: f32) -> Vec<Vec<Influence>> {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for v in &m.vertices {
        lo = [lo[0].min(v[0]), lo[1].min(v[1])];
        hi = [hi[0].max(v[0]), hi[1].max(v[1])];
    }
    let diag = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    let mut w = auto_weights(&m.vertices, &bone_segments(rig), falloff, (diag / 20.0).max(1.0));
    let adj = adjacency(m.vertices.len(), &m.triangles);
    smooth_weights(&mut w, &adj, 2, 0.35);
    w
}

/// A brush gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BrushMode {
    Add,
    Subtract,
    Smooth,
}

/// Brush strength at `d` from the center: smooth falloff, 1 at the center,
/// 0 at `radius`.
pub fn falloff(d: f32, radius: f32) -> f32 {
    if radius <= 0.0 || d >= radius {
        return 0.0;
    }
    let x = d / radius;
    (1.0 - x * x).powi(2)
}

/// Vertex neighbours along triangle edges.
pub fn adjacency(n: usize, tris: &[[u16; 3]]) -> Vec<Vec<usize>> {
    let mut adj = vec![Vec::new(); n];
    for t in tris {
        for k in 0..3 {
            let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
            if a < n && b < n {
                if !adj[a].contains(&b) {
                    adj[a].push(b);
                }
                if !adj[b].contains(&a) {
                    adj[b].push(a);
                }
            }
        }
    }
    adj
}

/// Sets the weight of `bone` to `target` and scales the others to fill the
/// rest. A vertex fully on `bone` with no other bone cannot go down.
fn set_weight(list: &mut Vec<Influence>, bone: &BoneId, target: f32) {
    let target = target.clamp(0.0, 1.0);
    let cur = weight_of(list, bone);
    let others: f32 = list.iter().filter(|(b, _)| b != bone).map(|x| x.1).sum();
    if others <= EPSILON && target < cur {
        return;
    }
    let k = if others > EPSILON {
        (1.0 - target) / others
    } else {
        0.0
    };
    for e in list.iter_mut() {
        if &e.0 != bone {
            e.1 *= k;
        }
    }
    match list.iter_mut().find(|(b, _)| b == bone) {
        Some(e) => e.1 = target,
        None => list.push((bone.clone(), target)),
    }
    normalize(list);
}

/// One brush dab at `center` (same space as `positions`, e.g. screen
/// pixels). `strength` `0..1`. Returns whether a weight changed. The
/// weights must be explicit (see [`ensure_weights`]).
#[allow(clippy::too_many_arguments)]
pub fn apply_brush(
    weights: &mut [Vec<Influence>],
    positions: &[P],
    adj: &[Vec<usize>],
    center: P,
    radius: f32,
    strength: f32,
    mode: BrushMode,
    bone: &BoneId,
) -> bool {
    let before: Vec<Vec<Influence>> = if mode == BrushMode::Smooth {
        weights.to_vec()
    } else {
        Vec::new()
    };
    let mut changed = false;
    for (i, p) in positions.iter().enumerate() {
        let Some(list) = weights.get_mut(i) else {
            break;
        };
        let f =
            falloff((p[0] - center[0]).hypot(p[1] - center[1]), radius) * strength.clamp(0.0, 1.0);
        if f <= 0.0 {
            continue;
        }
        let old = list.clone();
        match mode {
            BrushMode::Add => {
                let w = weight_of(list, bone);
                set_weight(list, bone, w + f * (1.0 - w));
            }
            BrushMode::Subtract => {
                let w = weight_of(list, bone);
                set_weight(list, bone, w * (1.0 - f));
            }
            BrushMode::Smooth => {
                let near = adj.get(i).map_or(&[][..], Vec::as_slice);
                if near.is_empty() {
                    continue;
                }
                // Average of the neighbours, blended in by f.
                let mut avg: Vec<Influence> = Vec::new();
                let share = 1.0 / near.len() as f32;
                for &n in near {
                    for (b, w) in &before[n] {
                        match avg.iter_mut().find(|(x, _)| x == b) {
                            Some(e) => e.1 += w * share,
                            None => avg.push((b.clone(), w * share)),
                        }
                    }
                }
                let mut out: Vec<Influence> = before[i]
                    .iter()
                    .map(|(b, w)| (b.clone(), w * (1.0 - f)))
                    .collect();
                for (b, w) in avg {
                    match out.iter_mut().find(|(x, _)| *x == b) {
                        Some(e) => e.1 += w * f,
                        None => out.push((b, w * f)),
                    }
                }
                normalize(&mut out);
                if !out.is_empty() {
                    *list = out;
                }
            }
        }
        changed |= *list != old;
    }
    changed
}

/// Positions of the mesh vertices skinned by the displayed bones: `world`
/// (displayed pose) × inverse of `bind` (rest pose), linear blend. An
/// unweighted mesh follows `slot_bone` rigidly.
pub fn skin_positions(
    rig: &Rig,
    m: &MeshAttachment,
    slot_bone: usize,
    world: &[Affine],
    bind: &[Affine],
) -> Vec<P> {
    let skinning: Vec<Affine> = world
        .iter()
        .zip(bind)
        .map(|(w, b)| b.inverse().map_or(*w, |inv| w.mul(&inv)))
        .collect();
    let rigid = skinning.get(slot_bone).copied().unwrap_or(Affine::IDENTITY);
    m.vertices
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let list = m.weights.get(i).filter(|l| !l.is_empty());
            let Some(list) = list else {
                return rigid.apply(*v);
            };
            let mut acc = Affine::ZERO;
            let mut sum = 0.0;
            for (b, w) in list {
                if let Some(k) = bone_index(rig, b)
                    && let Some(s) = skinning.get(k)
                {
                    Affine::add_scaled(&mut acc, s, *w);
                    sum += w;
                }
            }
            if sum <= 0.0 {
                return rigid.apply(*v);
            }
            let mut m = Affine::ZERO;
            Affine::add_scaled(&mut m, &acc, 1.0 / sum);
            m.apply(*v)
        })
        .collect()
}

/// Heat map color of a weight: blue (0) → cyan → green → yellow → red (1).
pub fn heat_color(w: f32) -> [u8; 3] {
    const STOPS: [[f32; 3]; 5] = [
        [40.0, 60.0, 200.0],
        [40.0, 200.0, 220.0],
        [60.0, 210.0, 80.0],
        [240.0, 220.0, 50.0],
        [230.0, 50.0, 40.0],
    ];
    let x = w.clamp(0.0, 1.0) * 4.0;
    let i = (x.floor() as usize).min(3);
    let t = x - i as f32;
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    [0, 1, 2].map(|k| (a[k] + (b[k] - a[k]) * t).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> BoneId {
        BoneId::new(s)
    }

    fn sum(list: &[Influence]) -> f32 {
        list.iter().map(|x| x.1).sum()
    }

    #[test]
    fn normalize_keeps_four_heaviest_and_sums_to_one() {
        let mut l = vec![
            (id("a"), 1.0),
            (id("b"), 5.0),
            (id("c"), 2.0),
            (id("d"), 0.5),
            (id("e"), 3.0),
            (id("b"), 1.0),
        ];
        normalize(&mut l);
        assert_eq!(l.len(), 4);
        assert!((sum(&l) - 1.0).abs() < 1e-5);
        assert_eq!(l[0].0, id("b"), "merged duplicates, heaviest first");
        assert!(!l.iter().any(|x| x.0 == id("d")), "lightest dropped");
        let mut bad = vec![(id("a"), f32::NAN), (id("b"), -1.0), (id("c"), 0.0)];
        normalize(&mut bad);
        assert!(bad.is_empty());
    }

    #[test]
    fn ensure_weights_makes_rigid_meshes_explicit() {
        let mut m = MeshAttachment {
            image: String::new(),
            vertices: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            uvs: vec![[0.0; 2]; 3],
            triangles: vec![[0, 1, 2]],
            weights: Vec::new(),
        };
        ensure_weights(&mut m, &id("root"));
        assert_eq!(m.weights, vec![vec![(id("root"), 1.0)]; 3]);
    }

    #[test]
    fn auto_weights_follow_bone_proximity() {
        let bones = vec![
            BoneSeg {
                id: id("upper"),
                head: [0.0, 0.0],
                tip: [100.0, 0.0],
            },
            BoneSeg {
                id: id("lower"),
                head: [100.0, 0.0],
                tip: [200.0, 0.0],
            },
            BoneSeg {
                id: id("far"),
                head: [0.0, 500.0],
                tip: [10.0, 500.0],
            },
        ];
        let w = auto_weights(
            &[[20.0, 5.0], [100.0, 5.0], [180.0, 5.0]],
            &bones,
            3.0,
            10.0,
        );
        for l in &w {
            assert!((sum(l) - 1.0).abs() < 1e-5 && l.len() <= 4);
        }
        assert!(weight_of(&w[0], &id("upper")) > 0.9);
        assert!(weight_of(&w[2], &id("lower")) > 0.9);
        // On the joint: shared evenly.
        assert!((weight_of(&w[1], &id("upper")) - weight_of(&w[1], &id("lower"))).abs() < 1e-4);
        assert!(weight_of(&w[0], &id("far")) < 0.01);
        // A sharper falloff gives a crisper split.
        let soft = auto_weights(&[[80.0, 5.0]], &bones, 1.0, 10.0);
        let sharp = auto_weights(&[[80.0, 5.0]], &bones, 6.0, 10.0);
        assert!(weight_of(&sharp[0], &id("upper")) > weight_of(&soft[0], &id("upper")));
    }

    fn line() -> (Vec<Vec<Influence>>, Vec<P>, Vec<Vec<usize>>) {
        let positions: Vec<P> = (0..5).map(|i| [i as f32 * 10.0, 0.0]).collect();
        let weights = vec![vec![(id("a"), 1.0)]; 5];
        let adj = (0..5usize)
            .map(|i| {
                [i.checked_sub(1), (i < 4).then_some(i + 1)]
                    .into_iter()
                    .flatten()
                    .collect()
            })
            .collect();
        (weights, positions, adj)
    }

    #[test]
    fn brush_adds_with_falloff_and_keeps_sums() {
        let (mut w, p, adj) = line();
        assert!(apply_brush(
            &mut w,
            &p,
            &adj,
            [0.0, 0.0],
            25.0,
            0.5,
            BrushMode::Add,
            &id("b")
        ));
        let wb: Vec<f32> = w.iter().map(|l| weight_of(l, &id("b"))).collect();
        assert!((wb[0] - 0.5).abs() < 1e-5, "center: strength");
        assert!(wb[1] > 0.0 && wb[1] < wb[0] && wb[2] < wb[1]);
        assert_eq!(wb[3], 0.0, "outside the radius");
        for l in &w {
            assert!((sum(l) - 1.0).abs() < 1e-5);
        }
        // Repeated dabs converge to 1.
        for _ in 0..40 {
            apply_brush(
                &mut w,
                &p,
                &adj,
                [0.0, 0.0],
                25.0,
                0.5,
                BrushMode::Add,
                &id("b"),
            );
        }
        assert!(weight_of(&w[0], &id("b")) > 0.999);
    }

    #[test]
    fn brush_subtracts_into_the_other_bones_but_never_empties_a_vertex() {
        let (mut w, p, adj) = line();
        // Only bone a: subtracting it is impossible.
        assert!(!apply_brush(
            &mut w,
            &p,
            &adj,
            [0.0, 0.0],
            25.0,
            1.0,
            BrushMode::Subtract,
            &id("a")
        ));
        w[0] = vec![(id("a"), 0.5), (id("b"), 0.5)];
        apply_brush(
            &mut w,
            &p,
            &adj,
            [0.0, 0.0],
            5.0,
            0.5,
            BrushMode::Subtract,
            &id("a"),
        );
        assert!((weight_of(&w[0], &id("a")) - 0.25).abs() < 1e-5);
        assert!((weight_of(&w[0], &id("b")) - 0.75).abs() < 1e-5);
    }

    #[test]
    fn brush_smooths_towards_neighbours() {
        let (mut w, p, adj) = line();
        w[2] = vec![(id("b"), 1.0)];
        apply_brush(
            &mut w,
            &p,
            &adj,
            [20.0, 0.0],
            5.0,
            1.0,
            BrushMode::Smooth,
            &id("a"),
        );
        // Neighbours are all "a": the spike is flattened.
        assert!(weight_of(&w[2], &id("a")) > 0.99);
        assert!((sum(&w[2]) - 1.0).abs() < 1e-5);
        // A half-strength smooth blends.
        let (mut w, p, adj) = line();
        w[2] = vec![(id("b"), 1.0)];
        apply_brush(
            &mut w,
            &p,
            &adj,
            [20.0, 0.0],
            5.0,
            0.5,
            BrushMode::Smooth,
            &id("a"),
        );
        assert!((weight_of(&w[2], &id("a")) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn brush_never_exceeds_four_bones() {
        let (mut w, p, adj) = line();
        for b in ["b", "c", "d", "e", "f"] {
            apply_brush(
                &mut w,
                &p,
                &adj,
                [0.0, 0.0],
                5.0,
                0.3,
                BrushMode::Add,
                &id(b),
            );
        }
        assert!(w[0].len() <= MAX_INFLUENCES);
        assert!((sum(&w[0]) - 1.0).abs() < 1e-5);
        assert!(weight_of(&w[0], &id("f")) > 0.0, "the painted bone is kept");
    }

    #[test]
    fn skinning_follows_weights() {
        let a = BoneId::new("a");
        let rig = Rig {
            bones: vec![orbit_anim::rig::Bone {
                id: a.clone(),
                name: "a".into(),
                parent: None,
                rest: orbit_anim::rig::Xform::IDENTITY,
                length: 10.0,
            }],
            ..Rig::default()
        };
        let bind = world_matrices(&rig, &rest_locals(&rig));
        let moved: Vec<Affine> = bind
            .iter()
            .map(|m| Affine {
                tx: m.tx + 5.0,
                ..*m
            })
            .collect();
        let m = MeshAttachment {
            image: String::new(),
            vertices: vec![[1.0, 1.0]],
            uvs: vec![[0.0; 2]],
            triangles: Vec::new(),
            weights: vec![vec![(a, 0.5)]],
        };
        let p = skin_positions(&rig, &m, 0, &moved, &bind);
        // Partial weights are renormalized: the vertex follows the bone.
        assert!((p[0][0] - 6.0).abs() < 1e-4);
        assert_eq!(heat_color(0.0), [40, 60, 200]);
        assert_eq!(heat_color(1.0), [230, 50, 40]);
    }
}
