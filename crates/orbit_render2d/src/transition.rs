//! Transitions between two render targets, driven by a 0..1 progress value.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use orbit_gfx::wgpu;

use crate::TextureId;

/// Kind of transition. The `progress` (0 = fully "from", 1 = fully "to") is
/// passed separately when drawing.
#[derive(Debug, Clone, PartialEq)]
pub enum Transition {
    /// Fade out to a solid color then in to the new image (linear RGBA; only
    /// RGB is used).
    Fade { color: [f32; 4] },
    /// Crossfade between the two images.
    Dissolve,
    /// Dissolve driven by a mask image: dark mask pixels switch first. The mask
    /// should be created with [`crate::TextureOptions::DATA`]. `softness` is
    /// the width of the soft edge in mask-luminance units (0.05..0.3 is typical).
    MaskDissolve { mask: TextureId, softness: f32 },
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub(crate) struct TransitionUniform {
    pub color: [f32; 4],
    pub progress: f32,
    pub softness: f32,
    pub mode: f32,
    pub _pad: f32,
}

impl TransitionUniform {
    pub(crate) fn new(transition: &Transition, progress: f32) -> Self {
        let progress = if progress.is_nan() {
            0.0
        } else {
            progress.clamp(0.0, 1.0)
        };
        let (mode, color, softness) = match transition {
            Transition::Dissolve => (0.0, [0.0; 4], 0.0),
            Transition::Fade { color } => (1.0, *color, 0.0),
            Transition::MaskDissolve { softness, .. } => (2.0, [0.0; 4], softness.max(0.0)),
        };
        Self {
            color,
            progress,
            softness,
            mode,
            _pad: 0.0,
        }
    }
}

pub(crate) struct TransitionPipeline {
    shader: wgpu::ShaderModule,
    pub layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    pub uniform: wgpu::Buffer,
}

impl TransitionPipeline {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("orbit2d transition shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("transition.wgsl").into()),
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
            label: Some("orbit2d transition layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex(1),
                tex(2),
                tex(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("orbit2d transition pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("orbit2d transition params"),
            size: std::mem::size_of::<TransitionUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            shader,
            layout,
            pipeline_layout,
            pipelines: HashMap::new(),
            uniform,
        }
    }

    pub(crate) fn pipeline_ref(
        &self,
        format: wgpu::TextureFormat,
    ) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.get(&format)
    }

    pub(crate) fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        self.pipelines.entry(format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("orbit2d transition pipeline"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_main"),
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_encoding() {
        let u = TransitionUniform::new(&Transition::Dissolve, 2.0);
        assert_eq!((u.mode, u.progress), (0.0, 1.0));
        let u = TransitionUniform::new(
            &Transition::Fade {
                color: [1.0, 0.0, 0.0, 1.0],
            },
            0.25,
        );
        assert_eq!((u.mode, u.progress, u.color[0]), (1.0, 0.25, 1.0));
        let u = TransitionUniform::new(
            &Transition::MaskDissolve {
                mask: TextureId(3),
                softness: -1.0,
            },
            f32::NAN,
        );
        assert_eq!((u.mode, u.progress, u.softness), (2.0, 0.0, 0.0));
        assert_eq!(std::mem::size_of::<TransitionUniform>(), 32);
    }
}
