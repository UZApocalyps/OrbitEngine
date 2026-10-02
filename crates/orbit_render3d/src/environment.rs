//! Environment maps on the CPU (ADR 0014 section 2): cubemap building from
//! an equirectangular image or 6 faces, and 9-coefficient spherical
//! harmonics irradiance, computed once (no GPU pass at runtime, WebGL2).
//!
//! Face order and orientation follow the wgpu / Vulkan / GL cubemap
//! convention: `+X, -X, +Y, -Y, +Z, -Z`, row 0 at the top of each face
//! (faces exported by usual cubemap tools follow it; seen from inside the
//! cube they are horizontally mirrored, as in every API).
//! Equirectangular images map their horizontal center to `-Z` (the default
//! camera forward), `+X` at three quarters, `+Y` at the top row.

use orbit_scene::TextureAsset;

use crate::math::{self, Vec3};

/// Faces are capped to this size when converted from an equirect image.
pub const MAX_FACE_SIZE: u32 = 1024;

/// Six square sRGB RGBA8 faces, `+X, -X, +Y, -Y, +Z, -Z`.
#[derive(Debug, Clone, PartialEq)]
pub struct CubeFaces {
    pub size: u32,
    pub faces: [Vec<u8>; 6],
}

/// Irradiance SH9, premultiplied by the cosine-lobe convolution and 1/pi:
/// the shader evaluates `sum(c[i] * Y_i(n))`, which for a uniform sky of
/// linear radiance `L` gives `L` (so diffuse ambient = albedo * L).
pub type Sh9 = [[f32; 3]; 9];

/// Texel `(s, t)` in `0..1` of `face` to a unit direction.
pub fn face_direction(face: usize, s: f32, t: f32) -> Vec3 {
    let (u, v) = (2.0 * s - 1.0, 2.0 * t - 1.0);
    let d = match face {
        0 => [1.0, -v, -u],
        1 => [-1.0, -v, u],
        2 => [u, 1.0, v],
        3 => [u, -1.0, -v],
        4 => [u, -v, 1.0],
        _ => [-u, -v, -1.0],
    };
    math::normalize(d)
}

/// Unit direction to equirect `(u, v)` in `0..1`.
pub fn equirect_uv(d: Vec3) -> (f32, f32) {
    let u = 0.5 + d[0].atan2(-d[2]) / std::f32::consts::TAU;
    let v = d[1].clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    (u, v)
}

fn bilinear(img: &TextureAsset, u: f32, v: f32) -> [u8; 4] {
    let (w, h) = (img.width as usize, img.height as usize);
    let x = (u * w as f32 - 0.5).rem_euclid(w as f32);
    let y = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize % w, y.floor() as usize);
    let (x1, y1) = ((x0 + 1) % w, (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let px = |x: usize, y: usize, c: usize| f32::from(img.rgba8[(y * w + x) * 4 + c]);
    let mut out = [0u8; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let top = px(x0, y0, c) * (1.0 - fx) + px(x1, y0, c) * fx;
        let bot = px(x0, y1, c) * (1.0 - fx) + px(x1, y1, c) * fx;
        *o = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Converts an equirectangular (2:1) image to cube faces.
pub fn from_equirect(img: &TextureAsset, face_size: u32) -> CubeFaces {
    let size = face_size.clamp(1, MAX_FACE_SIZE);
    let faces = std::array::from_fn(|f| {
        let mut out = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let s = (x as f32 + 0.5) / size as f32;
                let t = (y as f32 + 0.5) / size as f32;
                let (u, v) = equirect_uv(face_direction(f, s, t));
                out.extend_from_slice(&bilinear(img, u, v));
            }
        }
        out
    });
    CubeFaces { size, faces }
}

fn copy_face(img: &TextureAsset, size: u32, ox: u32, oy: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let start = (((oy + y) * img.width + ox) * 4) as usize;
        out.extend_from_slice(&img.rgba8[start..start + (size * 4) as usize]);
    }
    out
}

/// Builds cube faces from a single image, by layout detected from its
/// size: `6:1` horizontal strip or `1:6` vertical strip (`+X -X +Y -Y +Z
/// -Z`), `4:3` horizontal cross (`+Y` above `+Z`; row `-X +Z +X -Z`; `-Y`
/// below), anything else as equirectangular. `None` for invalid images.
pub fn cube_from_texture(img: &TextureAsset) -> Option<CubeFaces> {
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 || img.rgba8.len() != (w as usize) * (h as usize) * 4 {
        return None;
    }
    if w == 6 * h {
        let faces = std::array::from_fn(|f| copy_face(img, h, f as u32 * h, 0));
        return Some(CubeFaces { size: h, faces });
    }
    if h == 6 * w {
        let faces = std::array::from_fn(|f| copy_face(img, w, 0, f as u32 * w));
        return Some(CubeFaces { size: w, faces });
    }
    if w % 4 == 0 && w / 4 * 3 == h {
        let s = w / 4;
        // (column, row) of +X, -X, +Y, -Y, +Z, -Z.
        let at = [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)];
        let faces = std::array::from_fn(|f| copy_face(img, s, at[f].0 * s, at[f].1 * s));
        return Some(CubeFaces { size: s, faces });
    }
    Some(from_equirect(img, (w / 4).max(h / 2).max(1)))
}

pub fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn sh_basis(d: Vec3) -> [f32; 9] {
    let [x, y, z] = d;
    [
        0.282_095,
        0.488_603 * y,
        0.488_603 * z,
        0.488_603 * x,
        1.092_548 * x * y,
        1.092_548 * y * z,
        0.315_392 * (3.0 * z * z - 1.0),
        1.092_548 * x * z,
        0.546_274 * (x * x - y * y),
    ]
}

/// Cosine-lobe convolution / pi per band (Ramamoorthi & Hanrahan).
const BAND_FACTOR: [f32; 9] = [
    1.0,
    2.0 / 3.0,
    2.0 / 3.0,
    2.0 / 3.0,
    0.25,
    0.25,
    0.25,
    0.25,
    0.25,
];

/// Projects linear radiance of the cube onto SH9 irradiance coefficients.
pub fn irradiance_sh(cube: &CubeFaces) -> Sh9 {
    let n = cube.size.max(1);
    // Subsample large faces: SH9 is very low frequency.
    let step = (n / 64).max(1);
    let mut acc = [[0.0f64; 3]; 9];
    let mut total = 0.0f64;
    for (f, face) in cube.faces.iter().enumerate() {
        let mut y = 0;
        while y < n {
            let mut x = 0;
            while x < n {
                let s = (x as f32 + 0.5) / n as f32;
                let t = (y as f32 + 0.5) / n as f32;
                let (u, v) = (2.0 * s - 1.0, 2.0 * t - 1.0);
                let weight = 1.0 / (1.0 + u * u + v * v).powf(1.5);
                let i = ((y * n + x) * 4) as usize;
                let c = [
                    srgb_to_linear(face[i]),
                    srgb_to_linear(face[i + 1]),
                    srgb_to_linear(face[i + 2]),
                ];
                let basis = sh_basis(face_direction(f, s, t));
                for (k, b) in basis.iter().enumerate() {
                    for ch in 0..3 {
                        acc[k][ch] += f64::from(c[ch] * b * weight);
                    }
                }
                total += f64::from(weight);
                x += step;
            }
            y += step;
        }
    }
    let norm = 4.0 * std::f64::consts::PI / total.max(1e-12);
    std::array::from_fn(|k| std::array::from_fn(|ch| (acc[k][ch] * norm) as f32 * BAND_FACTOR[k]))
}

/// SH9 of a constant ambient color (evaluates to `color` everywhere).
pub fn constant_sh(color: Vec3) -> Sh9 {
    let mut sh = [[0.0; 3]; 9];
    sh[0] = color.map(|c| c / 0.282_095);
    sh
}

/// CPU evaluation, mirrors the shader.
pub fn eval_sh(sh: &Sh9, n: Vec3) -> Vec3 {
    let b = sh_basis(math::normalize(n));
    let mut out = [0.0; 3];
    for (k, c) in sh.iter().enumerate() {
        for ch in 0..3 {
            out[ch] += c[ch] * b[k];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> TextureAsset {
        let mut rgba8 = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba8.extend_from_slice(&f(x, y));
            }
        }
        TextureAsset {
            name: "t".into(),
            width: w,
            height: h,
            rgba8,
        }
    }

    #[test]
    fn face_centers_are_axes() {
        let axes = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        for (f, a) in axes.iter().enumerate() {
            let d = face_direction(f, 0.5, 0.5);
            assert!(math::dot(d, *a) > 0.999, "face {f}: {d:?}");
        }
        // Top row of side faces looks up.
        assert!(face_direction(4, 0.5, 0.0)[1] > 0.5);
        assert!(face_direction(5, 0.5, 0.0)[1] > 0.5);
    }

    #[test]
    fn equirect_center_is_forward_and_top_is_up() {
        let (u, v) = equirect_uv([0.0, 0.0, -1.0]);
        assert!((u - 0.5).abs() < 1e-5 && (v - 0.5).abs() < 1e-5);
        assert!((equirect_uv([1.0, 0.0, 0.0]).0 - 0.75).abs() < 1e-5);
        assert!(equirect_uv([0.0, 1.0, 0.0]).1 < 1e-5);
    }

    #[test]
    fn equirect_conversion_keeps_sky_up_and_ground_down() {
        // Top half red, bottom half green.
        let img = solid(16, 8, |_, y| {
            if y < 4 {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            }
        });
        let cube = cube_from_texture(&img).unwrap();
        assert_eq!(cube.size, 4);
        assert_eq!(&cube.faces[2][..4], &[255, 0, 0, 255]);
        assert_eq!(&cube.faces[3][..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn strip_and_cross_layouts() {
        let colors = [10u8, 20, 30, 40, 50, 60];
        let strip = solid(12, 2, |x, _| [colors[(x / 2) as usize], 0, 0, 255]);
        let cube = cube_from_texture(&strip).unwrap();
        let firsts: Vec<u8> = cube.faces.iter().map(|f| f[0]).collect();
        assert_eq!(firsts, colors);
        let vstrip = solid(2, 12, |_, y| [colors[(y / 2) as usize], 0, 0, 255]);
        assert_eq!(cube_from_texture(&vstrip).unwrap().faces[5][0], 60);
        // Cross: tag each cell with column*10 + row.
        let cross = solid(8, 6, |x, y| [((x / 2) * 10 + y / 2) as u8, 0, 0, 255]);
        let cube = cube_from_texture(&cross).unwrap();
        let expect = [21, 1, 10, 12, 11, 31];
        let firsts: Vec<u8> = cube.faces.iter().map(|f| f[0]).collect();
        assert_eq!(firsts, expect);
    }

    #[test]
    fn sh_of_uniform_sky_is_constant() {
        let cube = CubeFaces {
            size: 8,
            faces: std::array::from_fn(|_| [188u8, 188, 188, 255].repeat(64)),
        };
        let sh = irradiance_sh(&cube);
        for n in [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.3, -0.5, 0.8]] {
            let e = eval_sh(&sh, n);
            assert!((e[0] - srgb_to_linear(188)).abs() < 0.01, "{e:?}");
        }
        let c = eval_sh(&constant_sh([0.2, 0.4, 0.6]), [0.0, -1.0, 0.0]);
        assert!((c[1] - 0.4).abs() < 1e-5);
    }

    #[test]
    fn sh_is_directional() {
        // Bright top face only: up-facing normals get more light.
        let mut faces: [Vec<u8>; 6] = std::array::from_fn(|_| [0u8, 0, 0, 255].repeat(64));
        faces[2] = [255u8, 255, 255, 255].repeat(64);
        let sh = irradiance_sh(&CubeFaces { size: 8, faces });
        let up = eval_sh(&sh, [0.0, 1.0, 0.0])[0];
        let side = eval_sh(&sh, [1.0, 0.0, 0.0])[0];
        let down = eval_sh(&sh, [0.0, -1.0, 0.0])[0];
        assert!(up > side && side > down, "{up} {side} {down}");
        // Analytic: a lit +Y face spans ~1/6 of the sphere; up gets ~0.45.
        assert!(up > 0.3 && up < 0.6, "{up}");
    }
}
