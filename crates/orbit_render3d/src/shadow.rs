//! Directional shadow map fitting (M6-04). One cascade fitted to the
//! bounds of the visible geometry: VN decors are small, fixed-camera sets,
//! so a scene-bounds fit gives stable shadows that do not shimmer when the
//! camera moves.

use crate::math::{self, Mat4, Vec3};

/// Axis-aligned bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn of_points(points: &[[f32; 3]]) -> Option<Self> {
        let mut it = points.iter().filter(|p| p.iter().all(|v| v.is_finite()));
        let first = *it.next()?;
        let mut b = Self {
            min: first,
            max: first,
        };
        for p in it {
            b.include(*p);
        }
        Some(b)
    }

    pub fn include(&mut self, p: Vec3) {
        self.min = std::array::from_fn(|k| self.min[k].min(p[k]));
        self.max = std::array::from_fn(|k| self.max[k].max(p[k]));
    }

    pub fn union(&mut self, o: &Aabb) {
        self.include(o.min);
        self.include(o.max);
    }

    pub fn corners(&self) -> [Vec3; 8] {
        let (a, b) = (self.min, self.max);
        std::array::from_fn(|i| {
            [
                if i & 1 == 0 { a[0] } else { b[0] },
                if i & 2 == 0 { a[1] } else { b[1] },
                if i & 4 == 0 { a[2] } else { b[2] },
            ]
        })
    }

    /// World bounds of this (local) box under `m`.
    pub fn transformed(&self, m: &Mat4) -> Aabb {
        let c = self.corners().map(|p| math::transform_point(m, p));
        Aabb::of_points(&c).unwrap_or(*self)
    }
}

/// Orthographic light view-projection for a directional light travelling
/// along `direction`, enclosing `bounds` (depth mapped to `0..1`).
pub fn fit_directional(direction: Vec3, bounds: &Aabb) -> Option<Mat4> {
    let d = math::normalize(direction);
    if !d.iter().all(|v| v.is_finite()) || math::dot(d, d) < 0.5 {
        return None;
    }
    let center = [
        (bounds.min[0] + bounds.max[0]) * 0.5,
        (bounds.min[1] + bounds.max[1]) * 0.5,
        (bounds.min[2] + bounds.max[2]) * 0.5,
    ];
    let radius = math::dot(math::sub(bounds.max, center), math::sub(bounds.max, center))
        .sqrt()
        .max(1e-3);
    let eye = [
        center[0] - d[0] * radius * 2.0,
        center[1] - d[1] * radius * 2.0,
        center[2] - d[2] * radius * 2.0,
    ];
    let up = if d[1].abs() > 0.99 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let view = math::look_at(eye, center, up);
    let ls = bounds.corners().map(|p| math::transform_point(&view, p));
    let b = Aabb::of_points(&ls)?;
    // Small margin so PCF taps at the border stay inside.
    let pad = radius * 0.02;
    let (l, r) = (b.min[0] - pad, b.max[0] + pad);
    let (bo, t) = (b.min[1] - pad, b.max[1] + pad);
    // View space looks down -Z.
    let near = (-b.max[2] - pad).max(1e-3);
    let far = -b.min[2] + pad;
    let range = near - far;
    let proj = [
        [2.0 / (r - l), 0.0, 0.0, 0.0],
        [0.0, 2.0 / (t - bo), 0.0, 0.0],
        [0.0, 0.0, 1.0 / range, 0.0],
        [-(r + l) / (r - l), -(t + bo) / (t - bo), near / range, 1.0],
    ];
    let m = math::mul(&proj, &view);
    m.iter().flatten().all(|v| v.is_finite()).then_some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_matrix_encloses_bounds() {
        let b = Aabb {
            min: [-2.0, -1.0, -3.0],
            max: [4.0, 2.0, 1.0],
        };
        for dir in [[0.0, -1.0, 0.0], [1.0, -1.0, -1.0], [0.0, 0.0, -1.0]] {
            let m = fit_directional(dir, &b).unwrap();
            for c in b.corners() {
                let p = math::transform4(&m, [c[0], c[1], c[2], 1.0]);
                for k in 0..2 {
                    assert!(p[k].abs() <= 1.0 + 1e-4, "{dir:?} {p:?}");
                }
                assert!(p[2] >= -1e-4 && p[2] <= 1.0 + 1e-4, "{dir:?} {p:?}");
            }
        }
    }

    #[test]
    fn nearer_to_light_has_smaller_depth() {
        let b = Aabb {
            min: [-1.0; 3],
            max: [1.0; 3],
        };
        let m = fit_directional([0.0, -1.0, 0.0], &b).unwrap();
        let top = math::transform4(&m, [0.0, 1.0, 0.0, 1.0])[2];
        let bottom = math::transform4(&m, [0.0, -1.0, 0.0, 1.0])[2];
        assert!(top < bottom);
        assert!(fit_directional([0.0; 3], &b).is_none());
    }
}
