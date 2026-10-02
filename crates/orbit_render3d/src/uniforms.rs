//! CPU mirrors of the WGSL uniform blocks (`pbr.wgsl`) and their packing.
//!
//! Everything is a uniform buffer (no storage buffers: WebGL2). Layouts use
//! only `vec4` / `mat4x4` members so the std140 rules of WebGL2 and WGSL
//! agree byte for byte; sizes are checked by unit tests.

use bytemuck::{Pod, Zeroable};
use orbit_scene::{AlphaMode, Fog, Light, MaterialDef};

use crate::environment::Sh9;
use crate::math::{self, Mat4, Vec3};

/// Hard cap of lights in the frame uniform (desktop low profile, ADR 0014
/// section 3). The active limit is [`crate::Renderer3d::set_max_lights`].
pub const MAX_LIGHTS: usize = 8;

pub const LIGHT_DIRECTIONAL: f32 = 0.0;
pub const LIGHT_POINT: f32 = 1.0;
pub const LIGHT_SPOT: f32 = 2.0;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct LightGpu {
    /// xyz: world position, w: type (`LIGHT_*`).
    pub position: [f32; 4],
    /// xyz: direction the light travels (unit), w: range (0 = infinite).
    pub direction: [f32; 4],
    /// rgb: color * intensity (linear), w: unused.
    pub color: [f32; 4],
    /// x: cos(inner), y: cos(outer) for spots.
    pub spot: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct FrameUniform {
    pub view_proj: Mat4,
    /// Inverse of `view_proj` (sky view directions).
    pub inv_view_proj: Mat4,
    /// World to shadow-map clip space of the shadow light.
    pub light_view_proj: Mat4,
    /// xyz: camera world position.
    pub camera_pos: [f32; 4],
    /// rgb: background color (linear), w: 1 when the sky cubemap is used.
    pub background: [f32; 4],
    /// x: light count, y: shadow light index, z: PCF radius.
    pub counts: [u32; 4],
    /// x: depth bias, y: shadow texel size, z: 1 when shadows are enabled.
    pub shadow: [f32; 4],
    /// rgb: fog color (linear), w: mode (0 none, 1 linear, 2 exponential).
    pub fog_color: [f32; 4],
    /// x: start, y: end, z: density.
    pub fog: [f32; 4],
    /// x: exposure, y: mode (0 raw HDR, 1 clamp, 2 ACES), z: vignette.
    pub post: [f32; 4],
    /// xy: 1 / target size in pixels.
    pub viewport: [f32; 4],
    /// Diffuse ambient SH9 (rgb per coefficient), see [`crate::environment`].
    pub sh: [[f32; 4]; 9],
    pub lights: [LightGpu; MAX_LIGHTS],
}

pub const POST_RAW: f32 = 0.0;
pub const POST_CLAMP: f32 = 1.0;
pub const POST_ACES: f32 = 2.0;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct ObjectUniform {
    pub model: Mat4,
    pub normal: Mat4,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct MaterialUniform {
    /// Linear RGBA factor.
    pub base_color: [f32; 4],
    /// rgb: emissive * strength (linear).
    pub emissive: [f32; 4],
    /// metallic, roughness, alpha cutoff, occlusion strength.
    pub params: [f32; 4],
    /// x: alpha mode (0 opaque, 1 mask, 2 blend), y: unlit, z: double sided.
    pub flags: [u32; 4],
}

/// A light with its world transform, as gathered from the scene.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldLight {
    pub light: Light,
    pub position: Vec3,
    /// Direction the light points to (node -Z), unit length.
    pub direction: Vec3,
}

fn intensity(light: &Light) -> f32 {
    match light {
        Light::Directional { intensity, .. }
        | Light::Point { intensity, .. }
        | Light::Spot { intensity, .. } => *intensity,
    }
}

/// Orders lights by importance (directional first, then intensity) so the
/// extra ones dropped by the quality profile are the least important.
pub fn sort_by_importance(lights: &mut [WorldLight]) {
    lights.sort_by(|a, b| {
        let da = matches!(a.light, Light::Directional { .. });
        let db = matches!(b.light, Light::Directional { .. });
        db.cmp(&da)
            .then(intensity(&b.light).total_cmp(&intensity(&a.light)))
    });
}

pub fn pack_light(l: &WorldLight) -> LightGpu {
    let p = l.position;
    let d = math::normalize(l.direction);
    let scaled = |c: [f32; 3], i: f32| [c[0] * i, c[1] * i, c[2] * i, 0.0];
    match &l.light {
        Light::Directional {
            color, intensity, ..
        } => LightGpu {
            position: [p[0], p[1], p[2], LIGHT_DIRECTIONAL],
            direction: [d[0], d[1], d[2], 0.0],
            color: scaled(*color, *intensity),
            spot: [0.0; 4],
        },
        Light::Point {
            color,
            intensity,
            range,
        } => LightGpu {
            position: [p[0], p[1], p[2], LIGHT_POINT],
            direction: [d[0], d[1], d[2], range.unwrap_or(0.0).max(0.0)],
            color: scaled(*color, *intensity),
            spot: [0.0; 4],
        },
        Light::Spot {
            color,
            intensity,
            range,
            inner_angle,
            outer_angle,
        } => {
            let outer = outer_angle.max(0.0);
            let inner = inner_angle.clamp(0.0, outer);
            LightGpu {
                position: [p[0], p[1], p[2], LIGHT_SPOT],
                direction: [d[0], d[1], d[2], range.unwrap_or(0.0).max(0.0)],
                color: scaled(*color, *intensity),
                spot: [inner.cos(), outer.cos(), 0.0, 0.0],
            }
        }
    }
}

/// Packs the frame uniform. `lights` must already be sorted; at most
/// `max_lights` (clamped to [`MAX_LIGHTS`]) are kept. Shadows, fog and
/// post fields are left neutral (no shadow, no fog, exposure 1, clamp).
pub fn pack_frame(
    view_proj: Mat4,
    camera_pos: Vec3,
    ambient: &Sh9,
    lights: &[WorldLight],
    max_lights: usize,
) -> FrameUniform {
    let count = lights.len().min(max_lights.min(MAX_LIGHTS));
    let mut packed = [LightGpu::zeroed(); MAX_LIGHTS];
    for (slot, light) in packed.iter_mut().zip(&lights[..count]) {
        *slot = pack_light(light);
    }
    FrameUniform {
        view_proj,
        inv_view_proj: math::inverse(&view_proj).unwrap_or(math::IDENTITY),
        light_view_proj: math::IDENTITY,
        camera_pos: [camera_pos[0], camera_pos[1], camera_pos[2], 1.0],
        background: [0.0; 4],
        counts: [count as u32, 0, 0, 0],
        shadow: [0.0; 4],
        fog_color: [0.0; 4],
        fog: [0.0; 4],
        post: [1.0, POST_CLAMP, 0.0, 0.0],
        viewport: [1.0, 1.0, 0.0, 0.0],
        sh: ambient.map(|c| [c[0], c[1], c[2], 0.0]),
        lights: packed,
    }
}

/// Fog fields: `(fog_color, fog)`.
pub fn pack_fog(fog: Option<&Fog>) -> ([f32; 4], [f32; 4]) {
    match fog {
        None => ([0.0; 4], [0.0; 4]),
        Some(Fog::Linear { color, start, end }) => (
            [color[0], color[1], color[2], 1.0],
            [*start, end.max(*start), 0.0, 0.0],
        ),
        Some(Fog::Exponential { color, density }) => (
            [color[0], color[1], color[2], 2.0],
            [0.0, 0.0, density.max(0.0), 0.0],
        ),
    }
}

/// Index (in the packed, sorted list) of the light casting shadows: the
/// first directional light with `shadows: true` among the kept ones.
pub fn shadow_light_index(lights: &[WorldLight], max_lights: usize) -> Option<usize> {
    lights
        .iter()
        .take(max_lights.min(MAX_LIGHTS))
        .position(|l| matches!(l.light, Light::Directional { shadows: true, .. }))
}

pub fn pack_object(model: Mat4) -> ObjectUniform {
    ObjectUniform {
        model,
        normal: math::normal_matrix(&model),
    }
}

/// Material factors. Colors in [`MaterialDef`] are linear (glTF), so they
/// are uploaded as is; sRGB decoding of textures is done by the sampler.
pub fn pack_material(m: &MaterialDef) -> MaterialUniform {
    let e = m.emissive_strength;
    MaterialUniform {
        base_color: m.base_color,
        emissive: [m.emissive[0] * e, m.emissive[1] * e, m.emissive[2] * e, 0.0],
        params: [
            m.metallic.clamp(0.0, 1.0),
            m.roughness.clamp(0.0, 1.0),
            m.alpha_cutoff,
            m.occlusion_strength.clamp(0.0, 1.0),
        ],
        flags: [
            match m.alpha_mode {
                AlphaMode::Opaque => 0,
                AlphaMode::Mask => 1,
                AlphaMode::Blend => 2,
            },
            u32::from(m.unlit),
            u32::from(m.double_sided),
            0,
        ],
    }
}

/// Rounds `size` up to the device's dynamic uniform offset alignment.
pub fn aligned_stride(size: u64, alignment: u64) -> u64 {
    size.div_ceil(alignment.max(1)) * alignment.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_sizes_match_wgsl() {
        assert_eq!(size_of::<LightGpu>(), 64);
        assert_eq!(
            size_of::<FrameUniform>(),
            64 * 3 + 16 * 8 + 16 * 9 + 64 * MAX_LIGHTS
        );
        assert_eq!(size_of::<ObjectUniform>(), 128);
        assert_eq!(size_of::<MaterialUniform>(), 64);
        // WebGL2 guarantees 16 KiB uniform blocks.
        assert!(size_of::<FrameUniform>() <= 16 * 1024);
    }

    #[test]
    fn frame_packing_truncates_and_counts_lights() {
        let light = WorldLight {
            light: Light::Point {
                color: [1.0, 0.5, 0.25],
                intensity: 2.0,
                range: Some(10.0),
            },
            position: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, -1.0],
        };
        let lights = vec![light; 6];
        let sh = crate::environment::constant_sh([0.1; 3]);
        let f = pack_frame(math::IDENTITY, [0.0, 1.0, 2.0], &sh, &lights, 4);
        assert_eq!(f.counts[0], 4);
        assert_eq!(f.lights[0].position, [1.0, 2.0, 3.0, LIGHT_POINT]);
        assert_eq!(f.lights[0].color, [2.0, 1.0, 0.5, 0.0]);
        assert_eq!(f.lights[0].direction[3], 10.0);
        assert_eq!(f.lights[4], LightGpu::zeroed());
        let f = pack_frame(math::IDENTITY, [0.0; 3], &sh, &lights, 99);
        assert_eq!(f.counts[0], 6);
        // Byte layout: counts start right after 3 mat4 + 2 vec4.
        let bytes = bytemuck::bytes_of(&f);
        assert_eq!(&bytes[224..228], &6u32.to_le_bytes());
    }

    #[test]
    fn shadow_light_and_fog_packing() {
        let dir = |shadows| WorldLight {
            light: Light::Directional {
                color: [1.0; 3],
                intensity: 1.0,
                shadows,
            },
            position: [0.0; 3],
            direction: [0.0, -1.0, 0.0],
        };
        assert_eq!(shadow_light_index(&[dir(false), dir(true)], 8), Some(1));
        assert_eq!(shadow_light_index(&[dir(false), dir(true)], 1), None);
        let (c, p) = pack_fog(Some(&Fog::Linear {
            color: [0.5; 3],
            start: 4.0,
            end: 2.0,
        }));
        assert_eq!(c[3], 1.0);
        assert_eq!(p[1], 4.0);
        assert_eq!(pack_fog(None).0[3], 0.0);
    }

    #[test]
    fn spot_packs_cosines_and_directional_sorts_first() {
        let spot = WorldLight {
            light: Light::Spot {
                color: [1.0; 3],
                intensity: 100.0,
                range: None,
                inner_angle: 0.0,
                outer_angle: std::f32::consts::FRAC_PI_2,
            },
            position: [0.0; 3],
            direction: [0.0, -2.0, 0.0],
        };
        let p = pack_light(&spot);
        assert!((p.spot[0] - 1.0).abs() < 1e-6 && p.spot[1].abs() < 1e-6);
        assert_eq!(p.direction, [0.0, -1.0, 0.0, 0.0]);
        let sun = WorldLight {
            light: Light::Directional {
                color: [1.0; 3],
                intensity: 1.0,
                shadows: false,
            },
            ..spot.clone()
        };
        let mut lights = vec![spot, sun];
        sort_by_importance(&mut lights);
        assert!(matches!(lights[0].light, Light::Directional { .. }));
    }

    #[test]
    fn material_packing() {
        let mut m = MaterialDef::new("m");
        m.emissive = [1.0, 0.5, 0.0];
        m.emissive_strength = 4.0;
        m.alpha_mode = AlphaMode::Mask;
        m.unlit = true;
        m.roughness = 3.0;
        let u = pack_material(&m);
        assert_eq!(u.emissive, [4.0, 2.0, 0.0, 0.0]);
        assert_eq!(u.flags, [1, 1, 0, 0]);
        assert_eq!(u.params[1], 1.0);
    }

    #[test]
    fn stride_alignment() {
        assert_eq!(aligned_stride(128, 256), 256);
        assert_eq!(aligned_stride(300, 256), 512);
        assert_eq!(aligned_stride(64, 0), 64);
    }
}
