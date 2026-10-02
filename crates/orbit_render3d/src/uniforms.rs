//! CPU mirrors of the WGSL uniform blocks (`pbr.wgsl`) and their packing.
//!
//! Everything is a uniform buffer (no storage buffers: WebGL2). Layouts use
//! only `vec4` / `mat4x4` members so the std140 rules of WebGL2 and WGSL
//! agree byte for byte; sizes are checked by unit tests.

use bytemuck::{Pod, Zeroable};
use orbit_scene::{AlphaMode, Light, MaterialDef};

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
    /// xyz: camera world position.
    pub camera_pos: [f32; 4],
    /// rgb: ambient color * intensity (linear).
    pub ambient: [f32; 4],
    /// x: light count.
    pub counts: [u32; 4],
    pub lights: [LightGpu; MAX_LIGHTS],
}

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
/// `max_lights` (clamped to [`MAX_LIGHTS`]) are kept.
pub fn pack_frame(
    view_proj: Mat4,
    camera_pos: Vec3,
    ambient: Vec3,
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
        camera_pos: [camera_pos[0], camera_pos[1], camera_pos[2], 1.0],
        ambient: [ambient[0], ambient[1], ambient[2], 0.0],
        counts: [count as u32, 0, 0, 0],
        lights: packed,
    }
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
        assert_eq!(size_of::<FrameUniform>(), 64 + 16 * 3 + 64 * MAX_LIGHTS);
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
        let f = pack_frame(math::IDENTITY, [0.0, 1.0, 2.0], [0.1; 3], &lights, 4);
        assert_eq!(f.counts[0], 4);
        assert_eq!(f.lights[0].position, [1.0, 2.0, 3.0, LIGHT_POINT]);
        assert_eq!(f.lights[0].color, [2.0, 1.0, 0.5, 0.0]);
        assert_eq!(f.lights[0].direction[3], 10.0);
        assert_eq!(f.lights[4], LightGpu::zeroed());
        let f = pack_frame(math::IDENTITY, [0.0; 3], [0.0; 3], &lights, 99);
        assert_eq!(f.counts[0], 6);
        // Byte layout: counts start right after view_proj + 2 vec4.
        let bytes = bytemuck::bytes_of(&f);
        assert_eq!(&bytes[96..100], &6u32.to_le_bytes());
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
