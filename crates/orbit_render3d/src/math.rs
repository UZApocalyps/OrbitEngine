//! Minimal column-major matrix math (no extra dependency, wasm friendly).
//!
//! Conventions: right-handed, camera looks down -Z, clip depth in `0..1`
//! (wgpu / WebGPU), `Mat4[c][r]` is column `c`, row `r` (WGSL `mat4x4`).

pub type Vec3 = [f32; 3];
pub type Quat = [f32; 4];
pub type Mat4 = [[f32; 4]; 4];

pub const IDENTITY: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (c, col) in out.iter_mut().enumerate() {
        for (r, v) in col.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

pub fn transform4(m: &Mat4, v: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (r, o) in out.iter_mut().enumerate() {
        *o = (0..4).map(|c| m[c][r] * v[c]).sum();
    }
    out
}

pub fn transform_point(m: &Mat4, p: Vec3) -> Vec3 {
    let v = transform4(m, [p[0], p[1], p[2], 1.0]);
    [v[0], v[1], v[2]]
}

pub fn transform_vector(m: &Mat4, d: Vec3) -> Vec3 {
    let v = transform4(m, [d[0], d[1], d[2], 0.0]);
    [v[0], v[1], v[2]]
}

/// Translation * rotation * scale. The quaternion `[x, y, z, w]` is
/// normalized here (a zero or non-finite quaternion yields no rotation).
pub fn from_trs(t: Vec3, q: Quat, s: Vec3) -> Mat4 {
    let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    let [x, y, z, w] = if len > 1e-8 && len.is_finite() {
        [q[0] / len, q[1] / len, q[2] / len, q[3] / len]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    };
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    [
        [
            (1.0 - 2.0 * (yy + zz)) * s[0],
            2.0 * (xy + wz) * s[0],
            2.0 * (xz - wy) * s[0],
            0.0,
        ],
        [
            2.0 * (xy - wz) * s[1],
            (1.0 - 2.0 * (xx + zz)) * s[1],
            2.0 * (yz + wx) * s[1],
            0.0,
        ],
        [
            2.0 * (xz + wy) * s[2],
            2.0 * (yz - wx) * s[2],
            (1.0 - 2.0 * (xx + yy)) * s[2],
            0.0,
        ],
        [t[0], t[1], t[2], 1.0],
    ]
}

pub fn transpose(m: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (c, col) in m.iter().enumerate() {
        for (r, v) in col.iter().enumerate() {
            out[r][c] = *v;
        }
    }
    out
}

/// General 4x4 inverse (cofactor expansion); `None` when singular.
#[rustfmt::skip]
pub fn inverse(m: &Mat4) -> Option<Mat4> {
    // Row-major flat copy: a[r * 4 + c].
    let mut a = [0.0f32; 16];
    for c in 0..4 {
        for r in 0..4 {
            a[r * 4 + c] = m[c][r];
        }
    }
    let mut inv = [0.0f32; 16];
    inv[0] = a[5] * a[10] * a[15] - a[5] * a[11] * a[14] - a[9] * a[6] * a[15] + a[9] * a[7] * a[14] + a[13] * a[6] * a[11] - a[13] * a[7] * a[10];
    inv[4] = -a[4] * a[10] * a[15] + a[4] * a[11] * a[14] + a[8] * a[6] * a[15] - a[8] * a[7] * a[14] - a[12] * a[6] * a[11] + a[12] * a[7] * a[10];
    inv[8] = a[4] * a[9] * a[15] - a[4] * a[11] * a[13] - a[8] * a[5] * a[15] + a[8] * a[7] * a[13] + a[12] * a[5] * a[11] - a[12] * a[7] * a[9];
    inv[12] = -a[4] * a[9] * a[14] + a[4] * a[10] * a[13] + a[8] * a[5] * a[14] - a[8] * a[6] * a[13] - a[12] * a[5] * a[10] + a[12] * a[6] * a[9];
    inv[1] = -a[1] * a[10] * a[15] + a[1] * a[11] * a[14] + a[9] * a[2] * a[15] - a[9] * a[3] * a[14] - a[13] * a[2] * a[11] + a[13] * a[3] * a[10];
    inv[5] = a[0] * a[10] * a[15] - a[0] * a[11] * a[14] - a[8] * a[2] * a[15] + a[8] * a[3] * a[14] + a[12] * a[2] * a[11] - a[12] * a[3] * a[10];
    inv[9] = -a[0] * a[9] * a[15] + a[0] * a[11] * a[13] + a[8] * a[1] * a[15] - a[8] * a[3] * a[13] - a[12] * a[1] * a[11] + a[12] * a[3] * a[9];
    inv[13] = a[0] * a[9] * a[14] - a[0] * a[10] * a[13] - a[8] * a[1] * a[14] + a[8] * a[2] * a[13] + a[12] * a[1] * a[10] - a[12] * a[2] * a[9];
    inv[2] = a[1] * a[6] * a[15] - a[1] * a[7] * a[14] - a[5] * a[2] * a[15] + a[5] * a[3] * a[14] + a[13] * a[2] * a[7] - a[13] * a[3] * a[6];
    inv[6] = -a[0] * a[6] * a[15] + a[0] * a[7] * a[14] + a[4] * a[2] * a[15] - a[4] * a[3] * a[14] - a[12] * a[2] * a[7] + a[12] * a[3] * a[6];
    inv[10] = a[0] * a[5] * a[15] - a[0] * a[7] * a[13] - a[4] * a[1] * a[15] + a[4] * a[3] * a[13] + a[12] * a[1] * a[7] - a[12] * a[3] * a[5];
    inv[14] = -a[0] * a[5] * a[14] + a[0] * a[6] * a[13] + a[4] * a[1] * a[14] - a[4] * a[2] * a[13] - a[12] * a[1] * a[6] + a[12] * a[2] * a[5];
    inv[3] = -a[1] * a[6] * a[11] + a[1] * a[7] * a[10] + a[5] * a[2] * a[11] - a[5] * a[3] * a[10] - a[9] * a[2] * a[7] + a[9] * a[3] * a[6];
    inv[7] = a[0] * a[6] * a[11] - a[0] * a[7] * a[10] - a[4] * a[2] * a[11] + a[4] * a[3] * a[10] + a[8] * a[2] * a[7] - a[8] * a[3] * a[6];
    inv[11] = -a[0] * a[5] * a[11] + a[0] * a[7] * a[9] + a[4] * a[1] * a[11] - a[4] * a[3] * a[9] - a[8] * a[1] * a[7] + a[8] * a[3] * a[5];
    inv[15] = a[0] * a[5] * a[10] - a[0] * a[6] * a[9] - a[4] * a[1] * a[10] + a[4] * a[2] * a[9] + a[8] * a[1] * a[6] - a[8] * a[2] * a[5];
    let det = a[0] * inv[0] + a[1] * inv[4] + a[2] * inv[8] + a[3] * inv[12];
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let mut out = [[0.0; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            out[c][r] = inv[r * 4 + c] / det;
        }
    }
    Some(out)
}

/// Inverse-transpose of the model's upper 3x3, for normals.
pub fn normal_matrix(model: &Mat4) -> Mat4 {
    let mut m = inverse(model).map(|i| transpose(&i)).unwrap_or(IDENTITY);
    m[3] = [0.0, 0.0, 0.0, 1.0];
    m[0][3] = 0.0;
    m[1][3] = 0.0;
    m[2][3] = 0.0;
    m
}

/// Right-handed perspective projection with depth mapped to `0..1`.
pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    let range = near - far;
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / range, -1.0],
        [0.0, 0.0, near * far / range, 0.0],
    ]
}

/// Right-handed orthographic projection (`height` world units tall) with
/// depth mapped to `0..1`.
pub fn orthographic(height: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let h = height * 0.5;
    let w = h * aspect;
    let range = near - far;
    [
        [1.0 / w, 0.0, 0.0, 0.0],
        [0.0, 1.0 / h, 0.0, 0.0],
        [0.0, 0.0, 1.0 / range, 0.0],
        [0.0, 0.0, near / range, 1.0],
    ]
}

/// View matrix of a camera at `eye` looking at `target`.
pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    let f = normalize(sub(target, eye));
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn normalize(v: Vec3) -> Vec3 {
    let len = dot(v, v).sqrt();
    if len > 1e-12 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &Mat4, b: &Mat4) -> bool {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .all(|(x, y)| (x - y).abs() < 1e-4)
    }

    fn close3(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
    }

    #[test]
    fn trs_applies_scale_then_rotation_then_translation() {
        // 90 degrees around +Y: +X goes to -Z.
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let m = from_trs([1.0, 2.0, 3.0], [0.0, h, 0.0, h], [2.0, 2.0, 2.0]);
        assert!(close3(
            transform_point(&m, [1.0, 0.0, 0.0]),
            [1.0, 2.0, 1.0]
        ));
        assert!(close3(
            transform_vector(&m, [1.0, 0.0, 0.0]),
            [0.0, 0.0, -2.0]
        ));
    }

    #[test]
    fn inverse_roundtrips_and_detects_singular() {
        let m = from_trs([3.0, -1.0, 0.5], [0.2, 0.3, 0.1, 0.9], [1.0, 2.0, 0.5]);
        let inv = inverse(&m).unwrap();
        assert!(close(&mul(&m, &inv), &IDENTITY));
        let flat = from_trs([0.0; 3], [0.0, 0.0, 0.0, 1.0], [0.0, 1.0, 1.0]);
        assert!(inverse(&flat).is_none());
    }

    #[test]
    fn mul_composes_in_order() {
        let t = from_trs([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0; 3]);
        let s = from_trs([0.0; 3], [0.0, 0.0, 0.0, 1.0], [2.0; 3]);
        // t * s: scale first.
        let p = [1.0, 0.0, 0.0];
        assert!(close3(transform_point(&mul(&t, &s), p), [3.0, 0.0, 0.0]));
        assert!(close3(transform_point(&mul(&s, &t), p), [4.0, 0.0, 0.0]));
    }

    #[test]
    fn perspective_maps_near_to_0_and_far_to_1() {
        let p = perspective(1.0, 1.5, 0.1, 100.0);
        let n = transform4(&p, [0.0, 0.0, -0.1, 1.0]);
        let f = transform4(&p, [0.0, 0.0, -100.0, 1.0]);
        assert!((n[2] / n[3]).abs() < 1e-5);
        assert!((f[2] / f[3] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn orthographic_maps_depth_range() {
        let p = orthographic(10.0, 2.0, 1.0, 11.0);
        assert!((transform4(&p, [0.0, 0.0, -1.0, 1.0])[2]).abs() < 1e-5);
        assert!((transform4(&p, [0.0, 0.0, -11.0, 1.0])[2] - 1.0).abs() < 1e-5);
        assert!((transform4(&p, [10.0, 5.0, -2.0, 1.0])[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn look_at_equals_inverse_camera_transform() {
        let eye = [0.0, 0.0, 5.0];
        let v = look_at(eye, [0.0; 3], [0.0, 1.0, 0.0]);
        let world = from_trs(eye, [0.0, 0.0, 0.0, 1.0], [1.0; 3]);
        assert!(close(&v, &inverse(&world).unwrap()));
    }

    #[test]
    fn normal_matrix_handles_non_uniform_scale() {
        let m = from_trs([5.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [2.0, 1.0, 1.0]);
        let n = normal_matrix(&m);
        // A 45 degree surface normal must stay perpendicular after scaling.
        let normal = normalize(transform_vector(&n, [1.0, 1.0, 0.0]));
        let tangent = transform_vector(&m, [1.0, -1.0, 0.0]);
        assert!(dot(normal, tangent).abs() < 1e-5);
        assert_eq!(n[3], [0.0, 0.0, 0.0, 1.0]);
    }
}
