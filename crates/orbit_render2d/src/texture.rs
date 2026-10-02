//! Texture cache: `TextureId`s, named cache, dynamic updates, render targets.

use std::collections::HashMap;

use orbit_gfx::wgpu;

use crate::RenderError;

/// Handle to a texture owned by the renderer. Never reused after release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub(crate) u32);

/// How a texture is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureOptions {
    /// `true`: the bytes are sRGB-encoded color (decoded to linear when
    /// sampled). `false`: raw data (masks, glyph coverage).
    pub srgb: bool,
    /// Bilinear filtering (`true`) or nearest (`false`).
    pub linear_filter: bool,
}

impl TextureOptions {
    /// sRGB color with linear filtering: the right default for artwork.
    pub const COLOR: Self = Self {
        srgb: true,
        linear_filter: true,
    };
    /// Raw data with linear filtering: masks and glyph atlases.
    pub const DATA: Self = Self {
        srgb: false,
        linear_filter: true,
    };
}

/// Format of render targets (always sRGB RGBA8: renderable on WebGL2).
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub(crate) struct Entry {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub bind_group: wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
    pub is_target: bool,
}

pub(crate) struct TextureStore {
    entries: Vec<Option<Entry>>,
    by_name: HashMap<String, TextureId>,
    pub layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
}

impl TextureStore {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("orbit2d texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = |filter: wgpu::FilterMode, label| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Self {
            entries: Vec::new(),
            by_name: HashMap::new(),
            layout,
            linear: sampler(wgpu::FilterMode::Linear, "orbit2d linear"),
            nearest: sampler(wgpu::FilterMode::Nearest, "orbit2d nearest"),
        }
    }

    pub(crate) fn sampler(&self, linear: bool) -> &wgpu::Sampler {
        if linear { &self.linear } else { &self.nearest }
    }

    pub(crate) fn get(&self, id: TextureId) -> Option<&Entry> {
        self.entries.get(id.0 as usize).and_then(Option::as_ref)
    }

    pub(crate) fn by_name(&self, name: &str) -> Option<TextureId> {
        self.by_name.get(name).copied()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        max_dim: u32,
        label: &str,
        width: u32,
        height: u32,
        rgba: Option<&[u8]>,
        options: TextureOptions,
        is_target: bool,
    ) -> Result<TextureId, RenderError> {
        if width == 0 || height == 0 || width > max_dim || height > max_dim {
            return Err(RenderError::TextureSize {
                width,
                height,
                max: max_dim,
            });
        }
        if let Some(data) = rgba {
            let expected = width as usize * height as usize * 4;
            if data.len() != expected {
                return Err(RenderError::InvalidData {
                    expected,
                    actual: data.len(),
                });
            }
        }
        let format = if is_target {
            TARGET_FORMAT
        } else if options.srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let mut usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        if is_target {
            usage |= wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        }
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        if let Some(data) = rgba {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                size,
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(self.sampler(options.linear_filter)),
                },
            ],
        });
        let id = TextureId(self.entries.len() as u32);
        self.entries.push(Some(Entry {
            texture,
            view,
            bind_group,
            width,
            height,
            is_target,
        }));
        Ok(id)
    }

    pub(crate) fn name(&mut self, name: &str, id: TextureId) {
        self.by_name.insert(name.to_owned(), id);
    }

    pub(crate) fn release(&mut self, id: TextureId) {
        if let Some(slot) = self.entries.get_mut(id.0 as usize) {
            *slot = None;
        }
        self.by_name.retain(|_, v| *v != id);
    }
}
