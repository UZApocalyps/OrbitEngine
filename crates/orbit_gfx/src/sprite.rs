//! Simple textured-quad pipeline.
//!
//! WebGL2 constraints respected: no storage buffers, no compute, no
//! `base_vertex` indexed draws (each sprite is 6 plain vertices drawn with
//! `draw(first..last)`), one 64-byte uniform block, two bind groups.

use bytemuck::{Pod, Zeroable};

use crate::Texture;

/// A sprite to draw this frame. Coordinates are in physical pixels with the
/// origin at the top-left of the surface and Y pointing down.
pub struct Sprite<'a> {
    pub texture: &'a Texture,
    /// Center of the sprite.
    pub center: [f32; 2],
    /// Width and height on screen.
    pub size: [f32; 2],
    /// Linear RGBA tint multiplied with the texture (white = unchanged).
    pub tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

const VERTEX_SIZE: u64 = std::mem::size_of::<Vertex>() as u64;
const VERTICES_PER_SPRITE: usize = 6;
const INITIAL_SPRITE_CAPACITY: usize = 64;

/// Orthographic projection mapping pixel coordinates (origin top-left, Y down)
/// to clip space. Column-major.
pub(crate) fn ortho_pixels(width: f32, height: f32) -> [[f32; 4]; 4] {
    [
        [2.0 / width, 0.0, 0.0, 0.0],
        [0.0, -2.0 / height, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0, 1.0],
    ]
}

pub(crate) struct SpritePipeline {
    pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    pub(crate) texture_layout: wgpu::BindGroupLayout,
    pub(crate) sampler: wgpu::Sampler,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize, // in sprites
    scratch: Vec<Vertex>,
}

impl SpritePipeline {
    pub(crate) fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite texture layout"),
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

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite globals"),
            size: std::mem::size_of::<[[f32; 4]; 4]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite pipeline layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
        });

        let attributes = wgpu::vertex_attr_array![
            0 => Float32x2,
            1 => Float32x2,
            2 => Float32x4,
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_SIZE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            globals_buffer,
            globals_bind_group,
            texture_layout,
            sampler,
            vertex_buffer: Self::create_vertex_buffer(device, INITIAL_SPRITE_CAPACITY),
            vertex_capacity: INITIAL_SPRITE_CAPACITY,
            scratch: Vec::new(),
        }
    }

    fn create_vertex_buffer(device: &wgpu::Device, sprites: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite vertices"),
            size: sprites as u64 * VERTICES_PER_SPRITE as u64 * VERTEX_SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Uploads the projection and vertices for this frame.
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_size: [u32; 2],
        sprites: &[Sprite<'_>],
    ) {
        let proj = ortho_pixels(surface_size[0] as f32, surface_size[1] as f32);
        queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&proj));

        self.scratch.clear();
        for s in sprites {
            let [cx, cy] = s.center;
            let (hw, hh) = (s.size[0] * 0.5, s.size[1] * 0.5);
            let (l, r, t, b) = (cx - hw, cx + hw, cy - hh, cy + hh);
            let v = |x, y, u, v| Vertex {
                position: [x, y],
                uv: [u, v],
                color: s.tint,
            };
            self.scratch.extend_from_slice(&[
                v(l, t, 0.0, 0.0),
                v(r, t, 1.0, 0.0),
                v(l, b, 0.0, 1.0),
                v(l, b, 0.0, 1.0),
                v(r, t, 1.0, 0.0),
                v(r, b, 1.0, 1.0),
            ]);
        }

        if sprites.len() > self.vertex_capacity {
            self.vertex_capacity = sprites.len().next_power_of_two();
            self.vertex_buffer = Self::create_vertex_buffer(device, self.vertex_capacity);
        }
        if !self.scratch.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.scratch));
        }
    }

    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, sprites: &[Sprite<'_>]) {
        if sprites.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, Some(&self.globals_bind_group), &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        for (i, s) in sprites.iter().enumerate() {
            pass.set_bind_group(1, Some(&s.texture.bind_group), &[]);
            let first = (i * VERTICES_PER_SPRITE) as u32;
            pass.draw(first..first + VERTICES_PER_SPRITE as u32, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ortho_maps_corners_to_clip_space() {
        let m = ortho_pixels(800.0, 600.0);
        let apply = |x: f32, y: f32| (m[0][0] * x + m[3][0], m[1][1] * y + m[3][1]);
        assert_eq!(apply(0.0, 0.0), (-1.0, 1.0));
        assert_eq!(apply(800.0, 600.0), (1.0, -1.0));
        assert_eq!(apply(400.0, 300.0), (0.0, 0.0));
    }
}
