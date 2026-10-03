//! Textured, indexed triangle meshes (skinned rigs, ADR 0013 §5).
//!
//! A pipeline of its own (`mesh.wgsl`) so a GPU skinner can later add bone
//! attributes without touching sprites. WebGL2: no storage buffers, no
//! `base_vertex` (indices are rebased on the CPU), `u32` indices, one 64-byte
//! uniform. Same vertex layout and alpha blending as sprites.

use std::collections::HashMap;

use orbit_gfx::wgpu;

use crate::batcher::ClipSpace;
use crate::sprite::{Mesh2d, MeshVertex, Sprite2d, clip_to_scissor};
use crate::texture::TextureStore;

const VERTEX_SIZE: u64 = std::mem::size_of::<MeshVertex>() as u64;

/// Where meshes go in the sorted sprite sequence: `(position, mesh index)`
/// pairs in drawing order; mesh `m` is drawn just before the sorted sprite at
/// `position` (or after every sprite when `position == order.len()`).
///
/// Sorting key: (layer, z, submission). A sprite at scene index `j` sorts as
/// `2j + 1`, a mesh pushed after `a` sprites as `2a` (then by mesh index), so
/// equal layer/z keep their submission order. Sprites before `offset` in
/// `sprites` are internal (the clear quad) and sort first.
pub(crate) fn plan(
    sprites: &[Sprite2d],
    order: &[usize],
    offset: usize,
    meshes: &[Mesh2d],
    after: &[usize],
) -> Vec<(usize, usize)> {
    let sprite_key = |i: usize| (sprites[i].layer, sprites[i].z, 2 * i as i64 - 2 * offset as i64 + 1);
    let mut idx: Vec<usize> = (0..meshes.len()).collect();
    let mesh_key = |m: usize| (meshes[m].layer, meshes[m].z, 2 * after.get(m).copied().unwrap_or(0) as i64);
    let cmp = |a: &(crate::Layer, f32, i64), b: &(crate::Layer, f32, i64)| {
        a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2))
    };
    idx.sort_by(|&a, &b| cmp(&mesh_key(a), &mesh_key(b)).then(a.cmp(&b)));
    idx.into_iter()
        .map(|m| {
            let k = mesh_key(m);
            let pos = order.partition_point(|&i| cmp(&sprite_key(i), &k).is_lt());
            (pos, m)
        })
        .collect()
}

pub(crate) struct MeshGpu {
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    index_capacity: usize,
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
    /// Per mesh index: (texture, index range, clip).
    draws: Vec<Option<MeshDraw>>,
}

#[derive(Clone, Copy)]
struct MeshDraw {
    texture: crate::TextureId,
    first_index: u32,
    index_count: u32,
    clip: Option<[f32; 4]>,
}

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(4),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl MeshGpu {
    pub(crate) fn new(device: &wgpu::Device, texture_layout: &wgpu::BindGroupLayout) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("orbit2d mesh shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mesh.wgsl").into()),
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("orbit2d mesh globals layout"),
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
        let globals_buffer = buffer(
            device,
            "orbit2d mesh globals",
            std::mem::size_of::<[[f32; 4]; 4]>() as u64,
            wgpu::BufferUsages::UNIFORM,
        );
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("orbit2d mesh globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("orbit2d mesh pipeline layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(texture_layout)],
            immediate_size: 0,
        });
        let (vertex_capacity, index_capacity) = (1024, 3072);
        Self {
            vertex_buffer: buffer(device, "orbit2d mesh vertices", vertex_capacity as u64 * VERTEX_SIZE, wgpu::BufferUsages::VERTEX),
            index_buffer: buffer(device, "orbit2d mesh indices", index_capacity as u64 * 4, wgpu::BufferUsages::INDEX),
            shader,
            pipeline_layout,
            pipelines: HashMap::new(),
            globals_buffer,
            globals_bind_group,
            vertex_capacity,
            index_capacity,
            vertices: Vec::new(),
            indices: Vec::new(),
            draws: Vec::new(),
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
            label: Some("orbit2d mesh pipeline"),
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

    /// Concatenates every mesh (indices rebased) and uploads them.
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view_proj: &[[f32; 4]; 4],
        meshes: &[Mesh2d],
    ) {
        self.vertices.clear();
        self.indices.clear();
        self.draws.clear();
        for m in meshes {
            let base = self.vertices.len() as u32;
            let n = m.vertices.len() as u32;
            let first = self.indices.len() as u32;
            // Whole triangles only, and only valid ones.
            for tri in m.indices.chunks_exact(3) {
                if tri.iter().all(|&i| i < n) {
                    self.indices.extend(tri.iter().map(|&i| i + base));
                }
            }
            self.vertices.extend_from_slice(&m.vertices);
            let count = self.indices.len() as u32 - first;
            self.draws.push((count > 0).then_some(MeshDraw {
                texture: m.texture,
                first_index: first,
                index_count: count,
                clip: m.clip,
            }));
        }
        if self.draws.iter().all(Option::is_none) {
            return;
        }
        queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(view_proj));
        if self.vertices.len() > self.vertex_capacity {
            self.vertex_capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = buffer(
                device,
                "orbit2d mesh vertices",
                self.vertex_capacity as u64 * VERTEX_SIZE,
                wgpu::BufferUsages::VERTEX,
            );
        }
        if self.indices.len() > self.index_capacity {
            self.index_capacity = self.indices.len().next_power_of_two();
            self.index_buffer =
                buffer(device, "orbit2d mesh indices", self.index_capacity as u64 * 4, wgpu::BufferUsages::INDEX);
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        // write_buffer needs a multiple of 4 bytes: u32 indices always are.
        queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&self.indices));
    }

    /// Draws mesh `index` of the last [`MeshGpu::prepare`].
    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        format: wgpu::TextureFormat,
        textures: &TextureStore,
        clip: &ClipSpace,
        index: usize,
    ) {
        let (Some(pipeline), Some(Some(d))) = (self.pipelines.get(&format), self.draws.get(index)) else {
            return;
        };
        let scissor = match d.clip {
            Some(rect) => match clip_to_scissor(rect, clip.visible, clip.viewport, clip.size) {
                Some(s) => s,
                None => return,
            },
            None => clip.viewport_scissor(),
        };
        let Some(entry) = textures.get(d.texture) else {
            log::debug!("mesh uses a released texture {:?}", d.texture);
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, Some(&self.globals_bind_group), &[]);
        pass.set_bind_group(1, Some(&entry.bind_group), &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
        pass.draw_indexed(d.first_index..d.first_index + d.index_count, 0, 0..1);
    }

    /// Meshes with something to draw in the last prepare (diagnostics).
    pub(crate) fn draw_count(&self) -> usize {
        self.draws.iter().flatten().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Layer, TextureId};

    fn sprite(layer: Layer, z: f32) -> Sprite2d {
        Sprite2d::new(TextureId(0), [0.0, 0.0], [1.0, 1.0]).layer(layer).z(z)
    }

    fn mesh(layer: Layer, z: f32) -> Mesh2d {
        Mesh2d::new(TextureId(1), Vec::new(), Vec::new()).layer(layer).z(z)
    }

    #[test]
    fn meshes_interleave_with_sprites_by_layer_z_then_submission() {
        // Scene: s0, m0, s1 all on CHARACTERS z 0; m1 on BACKGROUND; m2 on UI.
        let sprites = vec![sprite(Layer::CHARACTERS, 0.0), sprite(Layer::CHARACTERS, 0.0)];
        let meshes = vec![mesh(Layer::CHARACTERS, 0.0), mesh(Layer::BACKGROUND, 0.0), mesh(Layer::UI, 0.0)];
        let after = vec![1, 2, 2];
        let mut order = Vec::new();
        crate::sprite::sort_sprites(&sprites, &mut order);
        let p = plan(&sprites, &order, 0, &meshes, &after);
        assert_eq!(p, vec![(0, 1), (1, 0), (2, 2)]);
        // With an internal clear quad at index 0 (offset 1), same answer
        // shifted by one sorted position.
        let mut with_clear = vec![sprite(Layer(0), -1.0e9)];
        with_clear.extend(sprites.iter().cloned());
        crate::sprite::sort_sprites(&with_clear, &mut order);
        let p = plan(&with_clear, &order, 1, &meshes, &after);
        assert_eq!(p, vec![(1, 1), (2, 0), (3, 2)]);
    }

    #[test]
    fn breaks_split_sprite_batches_around_meshes() {
        let sprites = vec![sprite(Layer::CHARACTERS, 0.0), sprite(Layer::CHARACTERS, 0.0)];
        let mut order = Vec::new();
        crate::sprite::sort_sprites(&sprites, &mut order);
        let (mut v, mut b) = (Vec::new(), Vec::new());
        crate::sprite::emit_batches(&sprites, &order, &mut v, &mut b, &[]);
        assert_eq!(b.len(), 1);
        crate::sprite::emit_batches(&sprites, &order, &mut v, &mut b, &[1]);
        assert_eq!(b.len(), 2);
    }
}
