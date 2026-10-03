//! OrbitEngine 3D renderer (ADR 0014, M6-03).
//!
//! Forward, single pass, PBR metallic-roughness + unlit, WebGL2 compatible
//! (uniform buffers only, `Depth24Plus`, u32 indices). A [`Scene3d`] is
//! rendered into a [`RenderTarget3d`] (`Rgba8UnormSrgb`) whose texture the 2D
//! stage composites under its sprites (M6-07). Shaders output linear color
//! and the sRGB target encodes it; textures used as color (base color,
//! emissive) are uploaded as `*Srgb`, data textures (ORM) as linear.
//!
//! The renderer only borrows a `wgpu::Device` / `Queue` (from the single
//! [`orbit_gfx::Gfx`] in the app, or a headless device in tests).
//!
//! Lighting and environment (M6-04 / M6-05): one directional light casts
//! shadows through a single-cascade shadow map fitted to the scene bounds
//! (`Depth32Float`, comparison sampler, PCF); `Background::Sky` draws a
//! cubemap (built on the CPU from an equirect image, a 6:1 / 1:6 strip or a
//! 4:3 cross, or registered as 6 faces) and gives the diffuse ambient as
//! SH9 irradiance; linear / exponential fog.
//!
//! Post-process (M6-06) per [`QualityProfile`]: either direct (exposure,
//! ACES and vignette in the forward shader, web) or HDR (`Rgba16Float`
//! target, bloom, composite pass, desktop). The default profile keeps the
//! M6-03 output (no tonemapping).
//!
//! Not yet: shadow cascades, spot shadows and more than one shadow light
//! (desktop high), prefiltered specular IBL, FXAA, normal maps, texture
//! transforms, MSAA.

pub mod environment;
mod flatten;
pub mod math;
mod mesh;
mod post;
pub mod quality;
pub mod shadow;
pub mod uniforms;

use std::collections::HashMap;

use orbit_gfx::wgpu;
use orbit_scene::{
    AlphaMode, Background, MaterialDef, MeshAsset, Prefab, Projection, Scene3d, TextureAsset,
};

pub use flatten::{DrawItem, FlatScene, WorldCamera, default_material, flatten};
pub use mesh::Vertex;
pub use quality::{BloomSettings, PostSettings, QualityProfile, ShadowSettings, Tonemap};
pub use uniforms::MAX_LIGHTS;

use environment::{CubeFaces, Sh9};
use math::Mat4;
use shadow::Aabb;
use uniforms::{FrameUniform, MaterialUniform, ObjectUniform};

/// Color format of [`RenderTarget3d`].
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Depth format (available on WebGL2).
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
/// Intermediate HDR color format (quality profiles with `hdr`).
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Shadow map format (sampled with comparison on WebGL2 too).
pub const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[derive(Debug, thiserror::Error)]
pub enum Render3dError {
    #[error("camera '{0}' not found in the scene")]
    CameraNotFound(String),
    #[error("the scene has no camera")]
    NoCamera,
}

/// Which camera to render from.
#[derive(Debug, Clone, PartialEq)]
pub enum CameraSelect<'a> {
    /// A named scene camera (the story unit, ADR 0014 section 6).
    Named(&'a str),
    /// The first camera of the scene.
    First,
    /// An explicit camera (editor viewport, scripted interpolation).
    View(Camera3d),
}

/// A resolved camera: world transform + projection.
#[derive(Debug, Clone, PartialEq)]
pub struct Camera3d {
    pub world: Mat4,
    pub projection: Projection,
    pub near: f32,
    pub far: f32,
}

impl Camera3d {
    pub fn view(&self) -> Mat4 {
        math::inverse(&self.world).unwrap_or(math::IDENTITY)
    }

    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            1.0
        };
        let near = self.near.max(1e-4);
        let far = self.far.max(near + 1e-3);
        match self.projection {
            Projection::Perspective { fov_y } => {
                math::perspective(fov_y.clamp(1e-3, 3.1), aspect, near, far)
            }
            Projection::Orthographic { height } => {
                math::orthographic(height.max(1e-4), aspect, near, far)
            }
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        math::mul(&self.projection_matrix(aspect), &self.view())
    }

    pub fn position(&self) -> math::Vec3 {
        [self.world[3][0], self.world[3][1], self.world[3][2]]
    }
}

impl From<&WorldCamera> for Camera3d {
    fn from(c: &WorldCamera) -> Self {
        Self {
            world: c.world,
            projection: c.def.projection.clone(),
            near: c.def.near,
            far: c.def.far,
        }
    }
}

/// Offscreen color + depth target. [`RenderTarget3d::view`] is what 2D
/// compositing samples.
pub struct RenderTarget3d {
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl RenderTarget3d {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let max = device.limits().max_texture_dimension_2d;
        let (width, height) = (width.clamp(1, max), height.clamp(1, max));
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("orbit3d color target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("orbit3d depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            color_view: color.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            color,
            width,
            height,
        }
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.color
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.color_view
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Copies the color target into a mappable buffer; returns the buffer
    /// and its padded row size. Map it with `map_async` (non-blocking, works
    /// on WebGL2) or [`RenderTarget3d::read_rgba_blocking`] natively.
    pub fn copy_to_buffer(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> (wgpu::Buffer, u32) {
        let padded = (self.width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("orbit3d readback"),
            size: u64::from(padded) * u64::from(self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("orbit3d readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        (buffer, padded)
    }

    /// Blocking read-back (native tools and tests only): tightly packed
    /// sRGB-encoded RGBA8, top row first.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_rgba_blocking(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<Vec<u8>> {
        let (buffer, padded) = self.copy_to_buffer(device, queue);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r.is_ok());
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .ok()?;
        if !rx.recv().ok()? {
            return None;
        }
        let data = slice.get_mapped_range().ok()?;
        let row = self.width as usize * 4;
        let mut out = Vec::with_capacity(row * self.height as usize);
        for y in 0..self.height as usize {
            let start = y * padded as usize;
            out.extend_from_slice(&data[start..start + row]);
        }
        drop(data);
        buffer.unmap();
        Some(out)
    }
}

struct GpuPrimitive {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Local bounds (shadow fitting).
    bounds: Option<Aabb>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    unlit: bool,
    blend: bool,
    double_sided: bool,
    hdr: bool,
}

impl PipelineKey {
    fn of(m: &MaterialDef, hdr: bool) -> Self {
        Self {
            unlit: m.unlit,
            blend: m.alpha_mode == AlphaMode::Blend,
            double_sided: m.double_sided,
            hdr,
        }
    }
}

fn color_format(hdr: bool) -> wgpu::TextureFormat {
    if hdr { HDR_FORMAT } else { COLOR_FORMAT }
}

struct EnvCube {
    view: wgpu::TextureView,
    sh: Sh9,
}

struct ShadowMap {
    size: u32,
    view: wgpu::TextureView,
}

struct GpuMaterial {
    _buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Material bind groups are cached across frames by this many entries max.
const MATERIAL_CACHE_LIMIT: usize = 1024;

/// The 3D renderer. Owns GPU meshes/textures and pipelines; borrows the
/// device and queue on each call so it can share the app's single `Gfx`.
pub struct Renderer3d {
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    frame_layout: wgpu::BindGroupLayout,
    object_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    white_linear: wgpu::TextureView,
    frame_buffer: wgpu::Buffer,
    object_buffer: wgpu::Buffer,
    object_group: wgpu::BindGroup,
    object_capacity: u64,
    object_stride: u64,
    meshes: HashMap<String, Vec<GpuPrimitive>>,
    cpu_textures: HashMap<String, TextureAsset>,
    textures: HashMap<(String, bool), wgpu::TextureView>,
    materials: HashMap<String, GpuMaterial>,
    prefabs: HashMap<String, Prefab>,
    quality: QualityProfile,
    last_draw_count: usize,
    // M6-04 shadows.
    shadow_shader: wgpu::ShaderModule,
    shadow_pipeline_layout: wgpu::PipelineLayout,
    shadow_pipeline: Option<wgpu::RenderPipeline>,
    shadow_buffer: wgpu::Buffer,
    shadow_group: wgpu::BindGroup,
    shadow_map: Option<ShadowMap>,
    dummy_shadow: wgpu::TextureView,
    compare_sampler: wgpu::Sampler,
    last_shadow_active: bool,
    // M6-05 environment.
    sky_pipeline_layout: wgpu::PipelineLayout,
    sky_pipelines: HashMap<bool, wgpu::RenderPipeline>,
    env_sampler: wgpu::Sampler,
    black_cube: wgpu::TextureView,
    cubes: HashMap<String, Option<EnvCube>>,
    // M6-06 post-process.
    post: Option<post::PostPipelines>,
    hdr_targets: Option<post::HdrTargets>,
}

impl Renderer3d {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("orbit3d pbr"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pbr.wgsl").into()),
        });
        let uniform_entry =
            |binding, visibility, dynamic: bool, size: usize| wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: dynamic,
                    min_binding_size: wgpu::BufferSize::new(size as u64),
                },
                count: None,
            };
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("orbit3d frame layout"),
            entries: &[
                uniform_entry(0, vf, false, size_of::<FrameUniform>()),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let object_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("orbit3d object layout"),
            entries: &[uniform_entry(0, vf, true, size_of::<ObjectUniform>())],
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("orbit3d material layout"),
            entries: &[
                uniform_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    false,
                    size_of::<MaterialUniform>(),
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(2),
                texture_entry(3),
                texture_entry(4),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("orbit3d pipeline layout"),
            bind_group_layouts: &[
                Some(&frame_layout),
                Some(&object_layout),
                Some(&material_layout),
            ],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("orbit3d sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white = upload_rgba(device, queue, "white", 1, 1, &[255; 4], true);
        let white_linear = upload_rgba(device, queue, "white linear", 1, 1, &[255; 4], false);

        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("orbit3d frame uniform"),
            size: size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let object_stride = uniforms::aligned_stride(
            size_of::<ObjectUniform>() as u64,
            u64::from(device.limits().min_uniform_buffer_offset_alignment),
        );
        let (object_buffer, object_group) =
            create_object_buffer(device, &object_layout, object_stride, 64);

        // Shadows.
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("orbit3d shadow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shadow.wgsl").into()),
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("orbit3d shadow layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX, false, 64)],
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("orbit3d shadow pipeline layout"),
                bind_group_layouts: &[Some(&shadow_layout), Some(&object_layout)],
                immediate_size: 0,
            });
        let shadow_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("orbit3d shadow uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("orbit3d shadow group"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_buffer.as_entire_binding(),
            }],
        });
        let compare_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("orbit3d shadow sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let dummy_shadow = shadow_texture(device, 1);

        // Environment.
        let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("orbit3d sky pipeline layout"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("orbit3d env sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let black_cube = upload_cube(
            device,
            queue,
            &CubeFaces {
                size: 1,
                faces: std::array::from_fn(|_| vec![0, 0, 0, 255]),
            },
        );

        Self {
            shader,
            pipeline_layout,
            frame_layout,
            object_layout,
            material_layout,
            pipelines: HashMap::new(),
            sampler,
            white,
            white_linear,
            frame_buffer,
            object_buffer,
            object_group,
            object_capacity: 64,
            object_stride,
            meshes: HashMap::new(),
            cpu_textures: HashMap::new(),
            textures: HashMap::new(),
            materials: HashMap::new(),
            prefabs: HashMap::new(),
            quality: QualityProfile::default(),
            last_draw_count: 0,
            shadow_shader,
            shadow_pipeline_layout,
            shadow_pipeline: None,
            shadow_buffer,
            shadow_group,
            shadow_map: None,
            dummy_shadow,
            compare_sampler,
            last_shadow_active: false,
            sky_pipeline_layout,
            sky_pipelines: HashMap::new(),
            env_sampler,
            black_cube,
            cubes: HashMap::new(),
            post: None,
            hdr_targets: None,
        }
    }

    /// Light limit of the quality profile (web: 4), clamped to [`MAX_LIGHTS`].
    pub fn set_max_lights(&mut self, n: usize) {
        self.quality.max_lights = n.min(MAX_LIGHTS);
    }

    /// Applies a quality profile (lights, shadows, HDR, post-process). For
    /// HDR, pass a profile checked with [`QualityProfile::adapt_to`].
    pub fn set_quality(&mut self, profile: QualityProfile) {
        self.quality = profile;
        self.quality.max_lights = profile.max_lights.min(MAX_LIGHTS);
        if !profile.hdr {
            self.hdr_targets = None;
        }
    }

    pub fn quality(&self) -> &QualityProfile {
        &self.quality
    }

    /// Whether the last [`Renderer3d::render`] rendered a shadow map.
    pub fn last_shadow_active(&self) -> bool {
        self.last_shadow_active
    }

    /// Registers a cubemap environment from 6 faces (`+X -X +Y -Y +Z -Z`,
    /// square, same size, sRGB RGBA8), usable as `Background::Sky(name)`.
    /// Returns `false` (and registers nothing) when the faces are invalid.
    pub fn add_cubemap(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        name: &str,
        faces: [TextureAsset; 6],
    ) -> bool {
        let size = faces[0].width;
        let max = device.limits().max_texture_dimension_2d;
        let valid = size > 0
            && size <= max
            && faces.iter().all(|f| {
                f.width == size && f.height == size && f.rgba8.len() == (size * size * 4) as usize
            });
        if !valid {
            log::warn!("3d: cubemap '{name}' faces are not square and of equal size");
            return false;
        }
        let cube = CubeFaces {
            size,
            faces: faces.map(|f| f.rgba8),
        };
        let env = EnvCube {
            view: upload_cube(device, queue, &cube),
            sh: environment::irradiance_sh(&cube),
        };
        self.cubes.insert(name.to_string(), Some(env));
        true
    }

    /// Resolves a sky texture: a registered cubemap, else a registered 2D
    /// texture converted once (equirect, strip or cross layout).
    fn ensure_cube(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, name: &str) {
        if self.cubes.contains_key(name) {
            return;
        }
        let max = device.limits().max_texture_dimension_2d;
        let env = self
            .cpu_textures
            .get(name)
            .and_then(environment::cube_from_texture)
            .filter(|c| c.size <= max)
            .map(|cube| EnvCube {
                view: upload_cube(device, queue, &cube),
                sh: environment::irradiance_sh(&cube),
            });
        if env.is_none() {
            log::warn!("3d: sky texture '{name}' missing or invalid");
        }
        self.cubes.insert(name.to_string(), env);
    }

    /// Uploads (or replaces) a mesh, keyed by its name (`MeshRef`).
    pub fn upload_mesh(&mut self, device: &wgpu::Device, mesh: &MeshAsset) {
        use wgpu::util::DeviceExt;
        let primitives = mesh
            .primitives
            .iter()
            .map(|p| {
                let (vertices, indices) = mesh::build(p);
                GpuPrimitive {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("orbit3d vertices"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("orbit3d indices"),
                        // Never empty: wgpu rejects zero-sized init buffers on some backends.
                        contents: bytemuck::cast_slice(if indices.is_empty() {
                            &[0u32; 3][..]
                        } else {
                            &indices[..]
                        }),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    index_count: indices.len() as u32,
                    bounds: Aabb::of_points(&p.positions),
                }
            })
            .collect();
        self.meshes.insert(mesh.name.clone(), primitives);
    }

    /// Registers a texture by name. GPU upload is lazy: the same image is
    /// uploaded as sRGB when used as color and linear when used as data.
    pub fn add_texture(&mut self, texture: TextureAsset) {
        self.textures.retain(|(name, _), _| name != &texture.name);
        self.materials.clear();
        self.cubes.remove(&texture.name);
        self.cpu_textures.insert(texture.name.clone(), texture);
    }

    /// Registers a prefab for `Node3dKind::Prefab` instances.
    pub fn add_prefab(&mut self, prefab: Prefab) {
        self.prefabs.insert(prefab.name.clone(), prefab);
    }

    pub fn has_mesh(&self, name: &str) -> bool {
        self.meshes.contains_key(name)
    }

    /// Draw calls issued by the last [`Renderer3d::render`].
    pub fn last_draw_count(&self) -> usize {
        self.last_draw_count
    }

    /// Flattens `scene` with the registered prefabs and uploaded meshes.
    pub fn flatten(&self, scene: &Scene3d) -> FlatScene {
        let meshes = |name: &str| self.meshes.get(name).map(Vec::len);
        flatten::flatten(scene, &self.prefabs, &meshes)
    }

    /// Renders `scene` into `target` (clear, depth test, opaque then
    /// back-to-front blended draws). Missing meshes/materials/textures are
    /// skipped or defaulted with a log warning, never an error.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene3d,
        camera: CameraSelect<'_>,
        target: &RenderTarget3d,
    ) -> Result<(), Render3dError> {
        let mut flat = self.flatten(scene);
        let camera = match camera {
            CameraSelect::View(c) => c,
            CameraSelect::First => {
                Camera3d::from(flat.cameras.first().ok_or(Render3dError::NoCamera)?)
            }
            CameraSelect::Named(name) => Camera3d::from(
                flat.cameras
                    .iter()
                    .find(|c| c.name == name)
                    .ok_or_else(|| Render3dError::CameraNotFound(name.to_string()))?,
            ),
        };
        let (w, h) = target.size();
        let quality = self.quality;
        let hdr = quality.hdr;
        let view_proj = camera.view_proj(w as f32 / h as f32);
        let env = &scene.environment;

        // Background and diffuse ambient (M6-05).
        let sky_name = match &env.background {
            Background::Sky(t) => Some(t.0.clone()),
            Background::Color(_) => None,
        };
        if let Some(name) = &sky_name {
            self.ensure_cube(device, queue, name);
        }
        let sky = sky_name
            .as_ref()
            .and_then(|n| self.cubes.get(n))
            .and_then(Option::as_ref);
        let tint = env.ambient_color.map(|c| c * env.ambient_intensity);
        let ambient = match sky {
            Some(cube) => cube
                .sh
                .map(|c| [c[0] * tint[0], c[1] * tint[1], c[2] * tint[2]]),
            None => environment::constant_sh(tint),
        };
        let background = match (&env.background, sky) {
            (Background::Color(c), _) => [c[0], c[1], c[2], 0.0],
            (Background::Sky(_), Some(_)) => [0.0, 0.0, 0.0, 1.0],
            (Background::Sky(_), None) => [0.0; 4],
        };
        let env_view = sky.map_or(&self.black_cube, |c| &c.view).clone();

        uniforms::sort_by_importance(&mut flat.lights);
        let mut frame = uniforms::pack_frame(
            view_proj,
            camera.position(),
            &ambient,
            &flat.lights,
            quality.max_lights,
        );
        frame.background = background;
        (frame.fog_color, frame.fog) = uniforms::pack_fog(env.fog.as_ref());
        let exposure = if quality.post.exposure.is_finite() {
            quality.post.exposure.max(0.0)
        } else {
            1.0
        };
        frame.post = if hdr {
            // Exposure and tonemapping happen in the composite pass.
            [1.0, uniforms::POST_RAW, 0.0, 0.0]
        } else {
            let mode = match quality.post.tonemap {
                Tonemap::None => uniforms::POST_CLAMP,
                Tonemap::Aces => uniforms::POST_ACES,
            };
            [exposure, mode, quality.post.vignette.clamp(0.0, 1.0), 0.0]
        };
        frame.viewport = [1.0 / w as f32, 1.0 / h as f32, 0.0, 0.0];

        // Order: opaque/mask first, then blended back to front.
        let view = camera.view();
        let depth_of = |d: &DrawItem| {
            math::transform_point(&view, [d.world[3][0], d.world[3][1], d.world[3][2]])[2]
        };
        let mut draws: Vec<&DrawItem> = flat
            .draws
            .iter()
            .filter(|d| {
                self.meshes
                    .get(&d.mesh)
                    .and_then(|p| p.get(d.primitive))
                    .is_some_and(|p| p.index_count > 0)
            })
            .collect();
        draws.sort_by(|a, b| {
            let ba = a.material.alpha_mode == AlphaMode::Blend;
            let bb = b.material.alpha_mode == AlphaMode::Blend;
            ba.cmp(&bb).then_with(|| {
                if ba {
                    // Farther (more negative view z) first.
                    depth_of(a).total_cmp(&depth_of(b))
                } else {
                    std::cmp::Ordering::Equal
                }
            })
        });

        // Per-object uniforms in one dynamic-offset buffer.
        if draws.len() as u64 > self.object_capacity {
            self.object_capacity = (draws.len() as u64).next_power_of_two();
            (self.object_buffer, self.object_group) = create_object_buffer(
                device,
                &self.object_layout,
                self.object_stride,
                self.object_capacity,
            );
        }
        if !draws.is_empty() {
            let mut bytes = vec![0u8; (self.object_stride * draws.len() as u64) as usize];
            for (i, d) in draws.iter().enumerate() {
                let start = i * self.object_stride as usize;
                let obj = uniforms::pack_object(d.world);
                bytes[start..start + size_of::<ObjectUniform>()]
                    .copy_from_slice(bytemuck::bytes_of(&obj));
            }
            queue.write_buffer(&self.object_buffer, 0, &bytes);
        }

        // Directional shadow map (M6-04): casters are the non-blended draws,
        // the light frustum is fitted to their bounds.
        let casters: Vec<usize> = draws
            .iter()
            .enumerate()
            .filter(|(_, d)| d.material.alpha_mode != AlphaMode::Blend)
            .map(|(i, _)| i)
            .collect();
        let mut shadow_active = false;
        if let (Some(settings), Some(index)) = (
            quality.shadows,
            uniforms::shadow_light_index(&flat.lights, quality.max_lights),
        ) {
            let mut bounds: Option<Aabb> = None;
            for &i in &casters {
                let d = draws[i];
                if let Some(b) = self.meshes[&d.mesh][d.primitive].bounds {
                    let wb = b.transformed(&d.world);
                    match &mut bounds {
                        Some(acc) => acc.union(&wb),
                        None => bounds = Some(wb),
                    }
                }
            }
            let fitted =
                bounds.and_then(|b| shadow::fit_directional(flat.lights[index].direction, &b));
            if let Some(light_view_proj) = fitted {
                let size = settings
                    .size
                    .clamp(16, device.limits().max_texture_dimension_2d);
                if self.shadow_map.as_ref().is_none_or(|s| s.size != size) {
                    self.shadow_map = Some(ShadowMap {
                        size,
                        view: shadow_texture(device, size),
                    });
                }
                self.ensure_shadow_pipeline(device);
                queue.write_buffer(&self.shadow_buffer, 0, bytemuck::bytes_of(&light_view_proj));
                frame.light_view_proj = light_view_proj;
                frame.counts[1] = index as u32;
                frame.counts[2] = settings.pcf_radius.min(3);
                frame.shadow = [settings.bias.max(0.0), 1.0 / size as f32, 1.0, 0.0];
                shadow_active = true;
            }
        }
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));

        // HDR targets (M6-06).
        if hdr {
            if self.post.is_none() {
                self.post = Some(post::PostPipelines::new(device, queue));
            }
            if self.hdr_targets.as_ref().is_none_or(|t| t.size() != (w, h)) {
                let pp = self.post.as_ref().expect("post pipelines");
                self.hdr_targets = Some(post::HdrTargets::new(device, pp, w, h));
            }
        }
        self.ensure_sky_pipeline(device, hdr);

        // Pipelines and materials first (needs &mut self), then the pass.
        let mut keys = Vec::with_capacity(draws.len());
        for d in &draws {
            let key = PipelineKey::of(&d.material, hdr);
            self.ensure_pipeline(device, key);
            let mat_key = format!("{:?}", d.material);
            self.ensure_material(device, queue, &mat_key, &d.material);
            keys.push((key, mat_key));
        }

        let shadow_view = match (&self.shadow_map, shadow_active) {
            (Some(s), true) => s.view.clone(),
            _ => self.dummy_shadow.clone(),
        };
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("orbit3d frame group"),
            layout: &self.frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.compare_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&env_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.env_sampler),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("orbit3d encoder"),
        });
        if shadow_active {
            let map = self.shadow_map.as_ref().expect("shadow map");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("orbit3d shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &map.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(self.shadow_pipeline.as_ref().expect("shadow pipeline"));
            pass.set_bind_group(0, &self.shadow_group, &[]);
            for &i in &casters {
                let d = draws[i];
                let prim = &self.meshes[&d.mesh][d.primitive];
                pass.set_bind_group(
                    1,
                    &self.object_group,
                    &[(i as u64 * self.object_stride) as u32],
                );
                pass.set_vertex_buffer(0, prim.vertices.slice(..));
                pass.set_index_buffer(prim.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..prim.index_count, 0, 0..1);
            }
        }
        let forward_view = match (&self.hdr_targets, hdr) {
            (Some(t), true) => &t.hdr_view,
            _ => &target.color_view,
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("orbit3d forward pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: forward_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &frame_group, &[]);
            // Background first: depth test off, no depth write.
            pass.set_pipeline(&self.sky_pipelines[&hdr]);
            pass.draw(0..3, 0..1);
            for (i, (d, (key, mat_key))) in draws.iter().zip(&keys).enumerate() {
                let prim = &self.meshes[&d.mesh][d.primitive];
                pass.set_pipeline(&self.pipelines[key]);
                pass.set_bind_group(
                    1,
                    &self.object_group,
                    &[(i as u64 * self.object_stride) as u32],
                );
                pass.set_bind_group(2, &self.materials[mat_key].bind_group, &[]);
                pass.set_vertex_buffer(0, prim.vertices.slice(..));
                pass.set_index_buffer(prim.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..prim.index_count, 0, 0..1);
            }
        }
        if hdr && let (Some(pp), Some(targets)) = (&self.post, &self.hdr_targets) {
            targets.record(queue, &mut encoder, pp, &quality.post, &target.color_view);
        }
        queue.submit(Some(encoder.finish()));
        self.last_draw_count = draws.len();
        self.last_shadow_active = shadow_active;
        Ok(())
    }

    fn ensure_shadow_pipeline(&mut self, device: &wgpu::Device) {
        if self.shadow_pipeline.is_some() {
            return;
        }
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("orbit3d shadow pipeline"),
            layout: Some(&self.shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.shadow_shader,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &mesh::VERTEX_ATTRIBUTES[..1],
                })],
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Thin decor geometry (planes, cards) must cast both ways.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.shadow_pipeline = Some(pipeline);
    }

    fn ensure_sky_pipeline(&mut self, device: &wgpu::Device, hdr: bool) {
        if self.sky_pipelines.contains_key(&hdr) {
            return;
        }
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("orbit3d sky pipeline"),
            layout: Some(&self.sky_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some("fs_sky"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format(hdr),
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.sky_pipelines.insert(hdr, pipeline);
    }

    fn ensure_pipeline(&mut self, device: &wgpu::Device, key: PipelineKey) {
        if self.pipelines.contains_key(&key) {
            return;
        }
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("orbit3d pipeline"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &mesh::VERTEX_ATTRIBUTES,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some(if key.unlit { "fs_unlit" } else { "fs_pbr" }),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format(key.hdr),
                    blend: key.blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: (!key.double_sided).then_some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(!key.blend),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, pipeline);
    }

    fn texture_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slot: Option<&orbit_scene::TextureSlot>,
        srgb: bool,
    ) -> wgpu::TextureView {
        let fallback = if srgb {
            &self.white
        } else {
            &self.white_linear
        };
        let Some(slot) = slot else {
            return fallback.clone();
        };
        let name = &slot.texture.0;
        if let Some(v) = self.textures.get(&(name.clone(), srgb)) {
            return v.clone();
        }
        let Some(cpu) = self.cpu_textures.get(name) else {
            log::warn!("3d: texture '{name}' not registered");
            return fallback.clone();
        };
        let expected = cpu.width as usize * cpu.height as usize * 4;
        let max = device.limits().max_texture_dimension_2d;
        if cpu.width == 0
            || cpu.height == 0
            || cpu.rgba8.len() != expected
            || cpu.width > max
            || cpu.height > max
        {
            log::warn!("3d: texture '{name}' has an invalid size");
            return fallback.clone();
        }
        let view = upload_rgba(device, queue, name, cpu.width, cpu.height, &cpu.rgba8, srgb);
        self.textures.insert((name.clone(), srgb), view.clone());
        view
    }

    fn ensure_material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: &str,
        m: &MaterialDef,
    ) {
        if self.materials.contains_key(key) {
            return;
        }
        if self.materials.len() >= MATERIAL_CACHE_LIMIT {
            self.materials.clear();
        }
        use wgpu::util::DeviceExt;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("orbit3d material"),
            contents: bytemuck::bytes_of(&uniforms::pack_material(m)),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let base = self.texture_view(device, queue, m.base_color_texture.as_ref(), true);
        let orm = self.texture_view(device, queue, m.orm_texture.as_ref(), false);
        let emissive = self.texture_view(device, queue, m.emissive_texture.as_ref(), true);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("orbit3d material group"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&base),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&orm),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&emissive),
                },
            ],
        });
        self.materials.insert(
            key.to_string(),
            GpuMaterial {
                _buffer: buffer,
                bind_group,
            },
        );
    }

    /// Bind group layout of the frame uniform (shared with later passes,
    /// e.g. shadows in M6-04).
    pub fn frame_layout(&self) -> &wgpu::BindGroupLayout {
        &self.frame_layout
    }
}

fn create_object_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    stride: u64,
    capacity: u64,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("orbit3d object uniforms"),
        size: stride * capacity,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("orbit3d object group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: wgpu::BufferSize::new(size_of::<ObjectUniform>() as u64),
            }),
        }],
    });
    (buffer, group)
}

fn shadow_texture(device: &wgpu::Device, size: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("orbit3d shadow map"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn upload_cube(device: &wgpu::Device, queue: &wgpu::Queue, cube: &CubeFaces) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: cube.size,
        height: cube.size,
        depth_or_array_layers: 6,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("orbit3d env cube"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (layer, face) in cube.faces.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer as u32,
                },
                aspect: wgpu::TextureAspect::All,
            },
            face,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * cube.size),
                rows_per_image: Some(cube.size),
            },
            wgpu::Extent3d {
                width: cube.size,
                height: cube.size,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("orbit3d env cube view"),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    })
}

fn upload_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
    srgb: bool,
) -> wgpu::TextureView {
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
        format: if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
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
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_view_proj_projects_target_to_center() {
        let world =
            math::inverse(&math::look_at([0.0, 2.0, 5.0], [0.0; 3], [0.0, 1.0, 0.0])).unwrap();
        let cam = Camera3d {
            world,
            projection: Projection::Perspective { fov_y: 1.0 },
            near: 0.1,
            far: 50.0,
        };
        let clip = math::transform4(&cam.view_proj(16.0 / 9.0), [0.0, 0.0, 0.0, 1.0]);
        assert!((clip[0] / clip[3]).abs() < 1e-5);
        assert!((clip[1] / clip[3]).abs() < 1e-5);
        let z = clip[2] / clip[3];
        assert!(z > 0.0 && z < 1.0);
        assert!((cam.position()[1] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn degenerate_camera_parameters_are_sanitized() {
        let cam = Camera3d {
            world: math::IDENTITY,
            projection: Projection::Perspective { fov_y: 0.0 },
            near: 0.0,
            far: 0.0,
        };
        let m = cam.projection_matrix(f32::NAN);
        assert!(m.iter().flatten().all(|v| v.is_finite()));
    }
}
