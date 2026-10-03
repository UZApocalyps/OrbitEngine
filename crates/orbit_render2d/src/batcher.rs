//! GPU side of the sprite batcher: pipelines and dynamic vertex/index buffers.
//!
//! WebGL2 constraints respected: no storage buffers, no compute, no
//! `base_vertex` (indices are absolute), `u32` indices, one 64-byte uniform.

use std::collections::HashMap;

use orbit_gfx::wgpu;

use crate::sprite::{
    Batch, INDICES_PER_SPRITE, Sprite2d, VERTEX_SIZE, VERTICES_PER_SPRITE, Vertex, clip_to_scissor,
    emit_batches, fill_indices, sort_sprites,
};
use crate::texture::TextureStore;

/// Where scissor rectangles land: camera visible world rect, viewport and
/// target size of the pass being drawn.
pub(crate) struct ClipSpace {
    pub visible: [f32; 4],
    pub viewport: crate::camera::Viewport,
    pub size: [u32; 2],
}

impl ClipSpace {
    /// The scissor installed with the viewport (restored after clipped
    /// batches).
    pub(crate) fn viewport_scissor(&self) -> [u32; 4] {
        let (size, vp) = (self.size, self.viewport);
        let x = (vp.x.max(0.0) as u32).min(size[0].saturating_sub(1));
        let y = (vp.y.max(0.0) as u32).min(size[1].saturating_sub(1));
        let w = (vp.width as u32).clamp(1, size[0].saturating_sub(x).max(1));
        let h = (vp.height as u32).clamp(1, size[1].saturating_sub(y).max(1));
        [x, y, w, h]
    }
}

const INITIAL_CAPACITY: usize = 256; // sprites

pub(crate) struct SpriteGpu {
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    capacity: usize,
    order: Vec<usize>,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    batches: Vec<Batch>,
}

fn create_buffers(device: &wgpu::Device, capacity: usize) -> (wgpu::Buffer, wgpu::Buffer) {
    let vertex = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("orbit2d vertices"),
        size: (capacity * VERTICES_PER_SPRITE) as u64 * VERTEX_SIZE,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let index = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("orbit2d indices"),
        size: (capacity * INDICES_PER_SPRITE * 4) as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    (vertex, index)
}

impl SpriteGpu {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("orbit2d sprite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("orbit2d globals layout"),
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
        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("orbit2d globals"),
            size: std::mem::size_of::<[[f32; 4]; 4]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("orbit2d globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("orbit2d sprite pipeline layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let (vertex_buffer, index_buffer) = create_buffers(device, INITIAL_CAPACITY);
        let mut indices = Vec::new();
        fill_indices(&mut indices, INITIAL_CAPACITY);
        queue.write_buffer(&index_buffer, 0, bytemuck::cast_slice(&indices));
        Self {
            shader,
            pipeline_layout,
            pipelines: HashMap::new(),
            globals_buffer,
            globals_bind_group,
            vertex_buffer,
            index_buffer,
            capacity: INITIAL_CAPACITY,
            order: Vec::new(),
            vertices: Vec::new(),
            indices,
            batches: Vec::new(),
        }
    }

    pub(crate) fn ensure_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.pipelines.contains_key(&format) {
            return;
        }
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x2,
            1 => Float32x2,
            2 => Float32x4,
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("orbit2d sprite pipeline"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_SIZE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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
        self.pipelines.insert(format, pipeline);
    }

    /// Sorts/batches `sprites` and uploads the projection and geometry.
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view_proj: &[[f32; 4]; 4],
        sprites: &[Sprite2d],
        breaks: impl FnOnce(&[usize]) -> Vec<usize>,
    ) {
        orbit_profile::scope!("2d batch");
        queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(view_proj));
        sort_sprites(sprites, &mut self.order);
        let breaks = breaks(&self.order);
        emit_batches(
            sprites,
            &self.order,
            &mut self.vertices,
            &mut self.batches,
            &breaks,
        );
        if sprites.len() > self.capacity {
            self.capacity = sprites.len().next_power_of_two();
            let (v, i) = create_buffers(device, self.capacity);
            self.vertex_buffer = v;
            self.index_buffer = i;
            fill_indices(&mut self.indices, self.capacity);
            queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&self.indices));
        }
        if !self.vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
    }

    /// Number of prepared batches and the sorted sprite position each one
    /// starts at.
    pub(crate) fn batch_start(&self, batch: usize) -> usize {
        self.batches[batch].first_index as usize / INDICES_PER_SPRITE
    }

    /// Draws the batches `range` prepared by [`SpriteGpu::prepare`].
    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        format: wgpu::TextureFormat,
        textures: &TextureStore,
        clip: &ClipSpace,
        range: std::ops::Range<usize>,
    ) {
        let Some(pipeline) = self.pipelines.get(&format) else {
            return;
        };
        if self.batches.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, Some(&self.globals_bind_group), &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        let full = clip.viewport_scissor();
        for batch in &self.batches[range] {
            let scissor = match batch.clip {
                Some(rect) => {
                    match clip_to_scissor(rect, clip.visible, clip.viewport, clip.size) {
                        Some(s) => s,
                        None => continue,
                    }
                }
                None => full,
            };
            pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
            let Some(entry) = textures.get(batch.texture) else {
                log::debug!("sprite uses a released texture {:?}", batch.texture);
                continue;
            };
            pass.set_bind_group(1, Some(&entry.bind_group), &[]);
            pass.draw_indexed(
                batch.first_index..batch.first_index + batch.index_count,
                0,
                0..1,
            );
        }
    }

    /// Number of draw calls prepared for the last scene (diagnostics/tests).
    pub(crate) fn batch_count(&self) -> usize {
        self.batches.len()
    }
}
