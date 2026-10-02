//! HDR post-process chain (M6-06): forward pass into `Rgba16Float`, then
//! optional bloom (bright pass at half resolution + separable Gaussian),
//! then composite (exposure, tonemap, vignette) into the sRGB target.
//! Fullscreen triangles, uniform buffers and 2D textures only (WebGL2 with
//! `EXT_color_buffer_float`; see [`crate::quality::hdr_supported`]).

use bytemuck::{Pod, Zeroable};
use orbit_gfx::wgpu;

use crate::quality::{PostSettings, Tonemap};
use crate::{COLOR_FORMAT, HDR_FORMAT};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct PostUniform {
    texel: [f32; 4],
    params: [f32; 4],
    extra: [f32; 4],
}

const BRIGHT: usize = 0;
const BLUR_H: usize = 1;
const BLUR_V: usize = 2;
const COMPOSITE: usize = 3;

pub(crate) struct PostPipelines {
    layout: wgpu::BindGroupLayout,
    bright: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniforms: [wgpu::Buffer; 4],
    black: wgpu::TextureView,
}

impl PostPipelines {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("orbit3d post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("orbit3d post layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<PostUniform>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex(2),
                tex(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("orbit3d post pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("orbit3d post sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("orbit3d post uniform"),
                size: size_of::<PostUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        Self {
            bright: pipeline("fs_bright", HDR_FORMAT),
            blur: pipeline("fs_blur", HDR_FORMAT),
            composite: pipeline("fs_composite", COLOR_FORMAT),
            layout,
            sampler,
            uniforms,
            black: crate::upload_rgba(device, queue, "black", 1, 1, &[0, 0, 0, 255], false),
        }
    }

    fn group(
        &self,
        device: &wgpu::Device,
        uniform: usize,
        a: &wgpu::TextureView,
        b: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("orbit3d post group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms[uniform].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(a),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(b),
                },
            ],
        })
    }
}

/// Size-dependent HDR textures and their bind groups.
pub(crate) struct HdrTargets {
    size: (u32, u32),
    half: (u32, u32),
    pub(crate) hdr_view: wgpu::TextureView,
    bloom_a: wgpu::TextureView,
    bloom_b: wgpu::TextureView,
    bright: wgpu::BindGroup,
    blur_h: wgpu::BindGroup,
    blur_v: wgpu::BindGroup,
    composite_bloom: wgpu::BindGroup,
    composite_plain: wgpu::BindGroup,
}

fn hdr_texture(device: &wgpu::Device, label: &str, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl HdrTargets {
    pub(crate) fn new(device: &wgpu::Device, pp: &PostPipelines, w: u32, h: u32) -> Self {
        let half = ((w / 2).max(1), (h / 2).max(1));
        let hdr_view = hdr_texture(device, "orbit3d hdr", w, h);
        let bloom_a = hdr_texture(device, "orbit3d bloom a", half.0, half.1);
        let bloom_b = hdr_texture(device, "orbit3d bloom b", half.0, half.1);
        Self {
            bright: pp.group(device, BRIGHT, &hdr_view, &pp.black),
            blur_h: pp.group(device, BLUR_H, &bloom_a, &pp.black),
            blur_v: pp.group(device, BLUR_V, &bloom_b, &pp.black),
            composite_bloom: pp.group(device, COMPOSITE, &hdr_view, &bloom_a),
            composite_plain: pp.group(device, COMPOSITE, &hdr_view, &pp.black),
            size: (w, h),
            half,
            hdr_view,
            bloom_a,
            bloom_b,
        }
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Writes the pass uniforms and records the post passes into `encoder`,
    /// writing the final image to `output` (the sRGB target).
    pub(crate) fn record(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pp: &PostPipelines,
        settings: &PostSettings,
        output: &wgpu::TextureView,
    ) {
        let inv = |(w, h): (u32, u32)| [1.0 / w as f32, 1.0 / h as f32, 0.0, 0.0];
        let exposure = if settings.exposure.is_finite() {
            settings.exposure.max(0.0)
        } else {
            1.0
        };
        let mode = match settings.tonemap {
            Tonemap::None => crate::uniforms::POST_CLAMP,
            Tonemap::Aces => crate::uniforms::POST_ACES,
        };
        let bloom = settings.bloom.filter(|b| b.intensity > 0.0);
        let (threshold, intensity) =
            bloom.map_or((0.0, 0.0), |b| (b.threshold.max(0.0), b.intensity));
        let write = |i: usize, u: PostUniform| {
            queue.write_buffer(&pp.uniforms[i], 0, bytemuck::bytes_of(&u));
        };
        write(
            COMPOSITE,
            PostUniform {
                texel: inv(self.size),
                params: [threshold, intensity, exposure, mode],
                extra: [settings.vignette.clamp(0.0, 1.0), 0.0, 0.0, 0.0],
            },
        );
        if bloom.is_some() {
            write(
                BRIGHT,
                PostUniform {
                    texel: inv(self.size),
                    params: [threshold, intensity, exposure, mode],
                    extra: [0.0; 4],
                },
            );
            write(
                BLUR_H,
                PostUniform {
                    texel: inv(self.half),
                    params: [0.0; 4],
                    extra: [0.0, 1.0, 0.0, 0.0],
                },
            );
            write(
                BLUR_V,
                PostUniform {
                    texel: inv(self.half),
                    params: [0.0; 4],
                    extra: [0.0, 0.0, 1.0, 0.0],
                },
            );
            let passes = [
                (&pp.bright, &self.bright, &self.bloom_a),
                (&pp.blur, &self.blur_h, &self.bloom_b),
                (&pp.blur, &self.blur_v, &self.bloom_a),
            ];
            for (pipeline, group, target) in passes {
                fullscreen(encoder, pipeline, group, target);
            }
        }
        let group = if bloom.is_some() {
            &self.composite_bloom
        } else {
            &self.composite_plain
        };
        fullscreen(encoder, &pp.composite, group, output);
    }
}

fn fullscreen(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
    target: &wgpu::TextureView,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("orbit3d post pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}
