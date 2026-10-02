use crate::{Gfx, GfxError};

/// Texture format used for all sprite textures (sRGB-encoded RGBA8).
pub const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// A GPU texture ready to be sampled by the sprite pipeline.
pub struct Texture {
    // Kept alive for the lifetime of the view / bind group.
    _texture: wgpu::Texture,
    pub(crate) bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

impl Texture {
    /// Uploads tightly packed RGBA8 pixels (`width * height * 4` bytes).
    pub fn from_rgba8(
        gfx: &Gfx,
        width: u32,
        height: u32,
        rgba: &[u8],
        label: &str,
    ) -> Result<Self, GfxError> {
        let max = gfx.device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > max || height > max {
            return Err(GfxError::TextureTooLarge { width, height, max });
        }
        let expected = width as usize * height as usize * 4;
        if rgba.len() != expected {
            return Err(GfxError::InvalidTextureData {
                width,
                height,
                expected,
                actual: rgba.len(),
            });
        }

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = gfx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEXTURE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gfx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = gfx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &gfx.sprites.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&gfx.sprites.sampler),
                },
            ],
        });

        Ok(Self {
            _texture: texture,
            bind_group,
            width,
            height,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn size(&self) -> [u32; 2] {
        [self.width, self.height]
    }
}
