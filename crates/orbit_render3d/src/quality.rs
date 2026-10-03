//! Quality profiles (ADR 0014 section 3): limits and post-process settings
//! are data. A scene exceeding them never breaks; extras are dropped.

use orbit_gfx::wgpu;

/// Tonemapping operator applied before sRGB encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tonemap {
    /// Exposure then clamp to `0..1` (M6-03 behavior; painted decors).
    None,
    /// ACES filmic (Narkowicz fit).
    Aces,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSettings {
    /// Shadow map resolution (square).
    pub size: u32,
    /// PCF kernel radius in texels (0 = single tap, 1 = 3x3, 2 = 5x5).
    pub pcf_radius: u32,
    /// Constant depth bias (in shadow-map depth units, applied in shader).
    pub bias: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomSettings {
    /// Linear luminance above which pixels bloom.
    pub threshold: f32,
    pub intensity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostSettings {
    pub tonemap: Tonemap,
    /// Linear multiplier applied before tonemapping.
    pub exposure: f32,
    /// Requires `hdr`; ignored otherwise.
    pub bloom: Option<BloomSettings>,
    /// 0 = none, 1 = strong darkening of the corners.
    pub vignette: f32,
}

impl Default for PostSettings {
    fn default() -> Self {
        Self {
            tonemap: Tonemap::None,
            exposure: 1.0,
            bloom: None,
            vignette: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityProfile {
    /// Dynamic light limit (clamped to [`crate::MAX_LIGHTS`]).
    pub max_lights: usize,
    /// `None` disables shadow maps. One shadow-casting directional light.
    pub shadows: Option<ShadowSettings>,
    /// Render into an `Rgba16Float` target and post-process in a separate
    /// pass. When `false` tonemapping, exposure and vignette run in the
    /// forward shader directly into the sRGB target (no bloom).
    pub hdr: bool,
    pub post: PostSettings,
}

impl Default for QualityProfile {
    /// Reference profile used by [`crate::Renderer3d::new`]: no tonemapping
    /// (output identical to M6-03), shadows on, LDR.
    fn default() -> Self {
        Self {
            max_lights: crate::MAX_LIGHTS,
            shadows: Some(ShadowSettings {
                size: 2048,
                pcf_radius: 1,
                bias: 0.002,
            }),
            hdr: false,
            post: PostSettings::default(),
        }
    }
}

impl QualityProfile {
    /// Web (WebGL2): 4 lights, 1024 shadow map with 3x3 PCF, LDR with
    /// in-shader ACES + vignette, no bloom.
    pub fn web() -> Self {
        Self {
            max_lights: 4,
            shadows: Some(ShadowSettings {
                size: 1024,
                pcf_radius: 1,
                bias: 0.003,
            }),
            hdr: false,
            post: PostSettings {
                tonemap: Tonemap::Aces,
                exposure: 1.0,
                bloom: None,
                vignette: 0.15,
            },
        }
    }

    /// Desktop low: 8 lights, 2048 shadow map, HDR + bloom.
    pub fn desktop_low() -> Self {
        Self {
            max_lights: 8,
            shadows: Some(ShadowSettings {
                size: 2048,
                pcf_radius: 1,
                bias: 0.002,
            }),
            hdr: true,
            post: PostSettings {
                tonemap: Tonemap::Aces,
                exposure: 1.0,
                bloom: Some(BloomSettings {
                    threshold: 1.0,
                    intensity: 0.6,
                }),
                vignette: 0.15,
            },
        }
    }

    /// Desktop high: like low with a 5x5 PCF kernel. (16 lights and shadow
    /// cascades are capped by the current uniform layout, see crate docs.)
    pub fn desktop_high() -> Self {
        let mut p = Self::desktop_low();
        if let Some(s) = &mut p.shadows {
            s.pcf_radius = 2;
        }
        p
    }

    /// The renderer profile of an export quality tier (M6-12). The light
    /// limit follows the tier, capped by [`crate::MAX_LIGHTS`].
    pub fn for_tier(tier: orbit_scene::quality::QualityTier) -> Self {
        use orbit_scene::quality::QualityTier;
        let mut p = match tier {
            QualityTier::Web => Self::web(),
            QualityTier::DesktopLow => Self::desktop_low(),
            QualityTier::DesktopHigh => Self::desktop_high(),
        };
        let limits = tier.limits();
        p.max_lights = limits.max_lights.min(crate::MAX_LIGHTS);
        if let Some(s) = &mut p.shadows {
            s.size = limits.shadow_map_size;
        }
        if !limits.bloom {
            p.post.bloom = None;
        }
        p
    }

    /// Disables HDR when the adapter cannot render to `Rgba16Float`
    /// (WebGL2 without `EXT_color_buffer_float`); post then runs in the
    /// forward shader (ADR 0014: fallback detected at init).
    pub fn adapt_to(mut self, adapter: &wgpu::Adapter) -> Self {
        if self.hdr && !hdr_supported(adapter) {
            self.hdr = false;
        }
        self
    }
}

/// Whether `Rgba16Float` can be rendered to and filtered on this adapter.
pub fn hdr_supported(adapter: &wgpu::Adapter) -> bool {
    let f = adapter.get_texture_format_features(crate::HDR_FORMAT);
    f.allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING)
        && f.flags
            .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
}

/// ACES filmic approximation (Krzysztof Narkowicz), same as the shaders.
pub fn aces(x: f32) -> f32 {
    let x = x.max(0.0);
    ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aces_is_monotonic_and_bounded() {
        assert_eq!(aces(0.0), 0.0);
        let mut prev = 0.0;
        for i in 1..100 {
            let v = aces(i as f32 * 0.2);
            assert!(v >= prev && v <= 1.0);
            prev = v;
        }
        assert!(aces(100.0) > 0.99);
    }

    #[test]
    fn profiles_follow_adr_table() {
        assert_eq!(QualityProfile::web().max_lights, 4);
        assert_eq!(QualityProfile::web().shadows.unwrap().size, 1024);
        assert!(QualityProfile::web().post.bloom.is_none());
        assert!(!QualityProfile::web().hdr);
        assert!(QualityProfile::desktop_low().post.bloom.is_some());
        assert_eq!(QualityProfile::default().post.tonemap, Tonemap::None);
        use orbit_scene::quality::QualityTier;
        assert_eq!(QualityProfile::for_tier(QualityTier::Web), QualityProfile::web());
        assert_eq!(
            QualityProfile::for_tier(QualityTier::DesktopLow),
            QualityProfile::desktop_low()
        );
        let high = QualityProfile::for_tier(QualityTier::DesktopHigh);
        assert_eq!(high.max_lights, crate::MAX_LIGHTS.min(16));
        assert_eq!(high.shadows.unwrap().pcf_radius, 2);
    }
}
