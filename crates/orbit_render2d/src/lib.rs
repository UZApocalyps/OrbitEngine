//! OrbitEngine 2D renderer on top of `orbit_gfx`.
//!
//! * batched sprites (position, size, rotation, scale, anchor, tint, UV
//!   sub-region) sorted by layer then `z`, one draw call per run of equal
//!   textures;
//! * ordered [`Layer`]s and a [`Camera2d`] with a virtual reference
//!   resolution, letterboxed/pillarboxed to the real surface;
//! * render targets (a texture a [`Scene`] can be drawn into) and
//!   [`Transition`]s (fade, dissolve, mask dissolve) between two targets;
//! * a texture cache with stable [`TextureId`]s that respects the device
//!   limits (WebGL2: 2048x2048 max).
//!
//! Typical frame:
//!
//! ```ignore
//! let Some(mut frame) = renderer.begin_frame()? else { return };
//! renderer.draw_scene(&mut frame, &scene)?;
//! renderer.end_frame(frame);
//! ```
//!
//! During a transition draw the old and new scenes into two targets with
//! [`Renderer2d::render_to_target`], then call
//! [`Renderer2d::draw_transition`] instead of `draw_scene`.

mod batcher;
mod camera;
mod mesh;
mod readback;
mod rig;
mod sprite;
mod texture;
mod transition;

pub use camera::{Camera2d, Viewport, letterbox};
pub use orbit_gfx;
pub use orbit_gfx::SurfaceFrame;
pub use readback::{Readback, ReadbackStatus};
pub use rig::{RigPlacement, push_rig};
pub use sprite::{Layer, Mesh2d, MeshVertex, Scene, Sprite2d, intersect_rects, nine_slice_quads};
pub use texture::{TARGET_FORMAT, TextureId, TextureOptions};
pub use transition::Transition;

/// CPU side of the sprite batcher without a GPU, for benchmarks
/// (`orbit-editor --bench`, TL-04).
#[doc(hidden)]
pub mod bench {
    use crate::sprite::{Batch, Sprite2d, Vertex, emit_batches, sort_sprites};
    pub use crate::texture::TextureId;

    /// A texture id for sprites that are only batched, never drawn.
    pub fn texture(n: u32) -> TextureId {
        TextureId(n)
    }

    /// Reusable buffers, like the renderer's.
    #[derive(Default)]
    pub struct CpuBatcher {
        order: Vec<usize>,
        vertices: Vec<Vertex>,
        batches: Vec<Batch>,
    }

    impl CpuBatcher {
        /// Sorts and batches `sprites` like a frame does; returns the
        /// number of draw calls.
        pub fn build(&mut self, sprites: &[Sprite2d]) -> usize {
            sort_sprites(sprites, &mut self.order);
            emit_batches(sprites, &self.order, &mut self.vertices, &mut self.batches, &[]);
            self.batches.len()
        }
    }
}

use orbit_gfx::{Gfx, GfxError, wgpu};
use thiserror::Error;

use batcher::SpriteGpu;
use mesh::MeshGpu;
use texture::TextureStore;
use transition::{TransitionPipeline, TransitionUniform};

#[derive(Debug, Error)]
pub enum RenderError {
    #[error(transparent)]
    Gfx(#[from] GfxError),
    #[error("texture {width}x{height} is empty or exceeds the device limit of {max}")]
    TextureSize { width: u32, height: u32, max: u32 },
    #[error("invalid texture data: expected {expected} bytes, got {actual}")]
    InvalidData { expected: usize, actual: usize },
    #[error("unknown or released texture {0:?}")]
    UnknownTexture(TextureId),
    #[error("texture {0:?} is not a render target")]
    NotATarget(TextureId),
    #[error("texture update out of bounds")]
    OutOfBounds,
}

/// A frame being composed. Obtained from [`Renderer2d::begin_frame`].
pub struct Frame {
    surface: SurfaceFrame,
    cleared: bool,
}

/// What the load op of the next pass on a surface should be.
fn load_op(cleared: &mut bool, bars: [f64; 4]) -> wgpu::LoadOp<wgpu::Color> {
    if std::mem::replace(cleared, true) {
        wgpu::LoadOp::Load
    } else {
        wgpu::LoadOp::Clear(wgpu::Color {
            r: bars[0],
            g: bars[1],
            b: bars[2],
            a: bars[3],
        })
    }
}

pub struct Renderer2d {
    gfx: Gfx,
    textures: TextureStore,
    sprites: SpriteGpu,
    meshes: MeshGpu,
    transitions: TransitionPipeline,
    camera: Camera2d,
    bar_color: [f64; 4],
    white: TextureId,
    scratch: Vec<Sprite2d>,
    /// Physical pixels per virtual pixel to report instead of the surface's
    /// (while drawing into a render target of another size).
    pixel_ratio_override: Option<f32>,
}

impl Renderer2d {
    /// Takes ownership of the GPU context. The default camera is 1920x1080.
    pub fn new(gfx: Gfx) -> Result<Self, RenderError> {
        let mut textures = TextureStore::new(gfx.device());
        let sprites = SpriteGpu::new(gfx.device(), gfx.queue(), &textures.layout);
        let meshes = MeshGpu::new(gfx.device(), &textures.layout);
        let transitions = TransitionPipeline::new(gfx.device());
        let white = textures.create(
            gfx.device(),
            gfx.queue(),
            gfx.max_texture_dimension(),
            "white",
            1,
            1,
            Some(&[255; 4]),
            TextureOptions::COLOR,
            false,
        )?;
        Ok(Self {
            gfx,
            textures,
            sprites,
            meshes,
            transitions,
            camera: Camera2d::default(),
            bar_color: [0.0, 0.0, 0.0, 1.0],
            white,
            scratch: Vec::new(),
            pixel_ratio_override: None,
        })
    }

    pub fn gfx(&self) -> &Gfx {
        &self.gfx
    }

    pub fn gfx_mut(&mut self) -> &mut Gfx {
        &mut self.gfx
    }

    /// Surface size in physical pixels.
    pub fn surface_size(&self) -> (u32, u32) {
        self.gfx.size()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.gfx.resize(width, height);
    }

    pub fn camera(&self) -> &Camera2d {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut Camera2d {
        &mut self.camera
    }

    /// Sets the virtual reference resolution (and recenters the camera).
    pub fn set_virtual_size(&mut self, width: f32, height: f32) {
        self.camera = Camera2d::new(width, height);
    }

    /// Linear RGBA color of the letterbox bars.
    pub fn set_bar_color(&mut self, color: [f64; 4]) {
        self.bar_color = color;
    }

    /// Where the virtual canvas lands on the surface, in physical pixels.
    pub fn viewport(&self) -> Viewport {
        let (w, h) = self.gfx.size();
        letterbox(self.camera.virtual_size, [w, h])
    }

    /// Physical pixels per virtual pixel (use it to rasterize text crisply).
    pub fn pixel_ratio(&self) -> f32 {
        self.pixel_ratio_override
            .unwrap_or_else(|| self.viewport().height / self.camera.virtual_size[1].max(1.0))
    }

    /// Makes [`Renderer2d::pixel_ratio`] report `ratio` (`None`: the surface
    /// again). Set it to `target height / virtual height` while drawing into
    /// a render target, so text is rasterized for that target.
    pub fn set_pixel_ratio_override(&mut self, ratio: Option<f32>) {
        self.pixel_ratio_override = ratio.filter(|r| r.is_finite() && *r > 0.0);
    }

    /// Converts a surface-pixel position (mouse) to world/virtual coordinates.
    pub fn screen_to_world(&self, point: [f32; 2]) -> [f32; 2] {
        self.camera.screen_to_world(&self.viewport(), point)
    }

    /// A 1x1 white texture, handy for solid-color rectangles via `tint`.
    pub fn white_texture(&self) -> TextureId {
        self.white
    }

    // ---- textures ------------------------------------------------------

    fn create_texture_inner(
        &mut self,
        label: &str,
        width: u32,
        height: u32,
        rgba: Option<&[u8]>,
        options: TextureOptions,
        is_target: bool,
    ) -> Result<TextureId, RenderError> {
        self.textures.create(
            self.gfx.device(),
            self.gfx.queue(),
            self.gfx.max_texture_dimension(),
            label,
            width,
            height,
            rgba,
            options,
            is_target,
        )
    }

    /// Uploads RGBA8 pixels under `name`. If a texture with that name is
    /// already cached its id is returned and `rgba` is ignored.
    pub fn create_texture(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        rgba: &[u8],
        options: TextureOptions,
    ) -> Result<TextureId, RenderError> {
        if let Some(id) = self.textures.by_name(name) {
            return Ok(id);
        }
        let id = self.create_texture_inner(name, width, height, Some(rgba), options, false)?;
        self.textures.name(name, id);
        Ok(id)
    }

    /// Creates an updatable, uninitialized (transparent) texture, e.g. a glyph
    /// atlas. Not name-cached.
    pub fn create_empty_texture(
        &mut self,
        label: &str,
        width: u32,
        height: u32,
        options: TextureOptions,
    ) -> Result<TextureId, RenderError> {
        let zeros = vec![0u8; width as usize * height as usize * 4];
        self.create_texture_inner(label, width, height, Some(&zeros), options, false)
    }

    /// Looks a texture up by the name it was created with.
    pub fn texture_by_name(&self, name: &str) -> Option<TextureId> {
        self.textures.by_name(name)
    }

    /// Writes a sub-rectangle of RGBA8 pixels into an existing texture.
    pub fn update_texture(
        &self,
        id: TextureId,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), RenderError> {
        let entry = self
            .textures
            .get(id)
            .ok_or(RenderError::UnknownTexture(id))?;
        if x.checked_add(width).is_none_or(|e| e > entry.width)
            || y.checked_add(height).is_none_or(|e| e > entry.height)
        {
            return Err(RenderError::OutOfBounds);
        }
        let expected = width as usize * height as usize * 4;
        if rgba.len() != expected {
            return Err(RenderError::InvalidData {
                expected,
                actual: rgba.len(),
            });
        }
        if width == 0 || height == 0 {
            return Ok(());
        }
        self.gfx.queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &entry.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// Creates a render target (sRGB RGBA8). Draw into it with
    /// [`Renderer2d::render_to_target`]; sample it like any texture (as a
    /// sprite, or as a transition input). Typically sized like the camera's
    /// virtual resolution. Keep the scene opaque (`Scene::clear`) so that
    /// transitions blend correctly.
    pub fn create_render_target(
        &mut self,
        label: &str,
        width: u32,
        height: u32,
    ) -> Result<TextureId, RenderError> {
        self.create_texture_inner(label, width, height, None, TextureOptions::COLOR, true)
    }

    /// Opt-in, native only: render targets created from now on can also be
    /// viewed as plain `Rgba8Unorm` ([`Renderer2d::target_gamma_view`]), the
    /// way a gamma-space UI toolkit (egui) wants to sample them. A no-op on
    /// the web (WebGL2 has no view formats).
    pub fn enable_target_gamma_views(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.textures.target_view_formats = vec![TARGET_FORMAT.remove_srgb_suffix()];
        }
    }

    /// A view of the render target `id` that returns its stored (sRGB
    /// encoded) values untouched, for gamma-space consumers. `None` if the
    /// target is unknown or was created without
    /// [`Renderer2d::enable_target_gamma_views`].
    pub fn target_gamma_view(&self, id: TextureId) -> Option<wgpu::TextureView> {
        let entry = self.textures.get(id).filter(|e| e.is_target)?;
        let gamma = TARGET_FORMAT.remove_srgb_suffix();
        if !self.textures.target_view_formats.contains(&gamma) {
            return None;
        }
        Some(entry.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(gamma),
            ..Default::default()
        }))
    }

    /// Creates a render target the size of the virtual canvas, clamped to the
    /// device texture limit (aspect ratio preserved).
    pub fn create_canvas_target(&mut self, label: &str) -> Result<TextureId, RenderError> {
        let [vw, vh] = self.camera.virtual_size;
        let max = self.gfx.max_texture_dimension() as f32;
        let k = (max / vw.max(vh)).min(1.0);
        self.create_render_target(label, (vw * k).round() as u32, (vh * k).round() as u32)
    }

    /// Starts copying a render target to the CPU without blocking: submits the
    /// copy and requests the mapping. Poll the result with
    /// [`Renderer2d::poll_readback`] on the following frames (works on
    /// WebGL2, where nothing may block).
    pub fn start_readback(&self, target: TextureId) -> Result<Readback, RenderError> {
        let entry = self
            .textures
            .get(target)
            .ok_or(RenderError::UnknownTexture(target))?;
        if !entry.is_target {
            return Err(RenderError::NotATarget(target));
        }
        let (readback, state) = Readback::new(self.gfx.device(), entry.width, entry.height);
        let mut encoder =
            self.gfx
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("orbit2d readback encoder"),
                });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &entry.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(readback.padded_row),
                    rows_per_image: Some(entry.height),
                },
            },
            wgpu::Extent3d {
                width: entry.width,
                height: entry.height,
                depth_or_array_layers: 1,
            },
        );
        self.gfx.queue().submit(Some(encoder.finish()));
        readback
            .buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                Readback::complete(&state, result.is_ok());
            });
        Ok(readback)
    }

    /// Checks a read-back without blocking. Natively this also lets wgpu make
    /// progress (`Device::poll` in non-blocking mode); on the web the browser
    /// delivers the result by itself.
    pub fn poll_readback(&self, readback: &mut Readback) -> ReadbackStatus {
        let _ = self.gfx.device().poll(wgpu::PollType::Poll);
        readback.take()
    }

    /// The GPU texture behind `id` (always `COPY_DST`; render targets are
    /// sRGB RGBA8), for content produced outside this renderer, e.g. a 3D
    /// render copied in to be composited as a sprite.
    pub fn raw_texture(&self, id: TextureId) -> Option<&wgpu::Texture> {
        self.textures.get(id).map(|e| &e.texture)
    }

    /// Size of a texture in pixels.
    pub fn texture_size(&self, id: TextureId) -> Option<[u32; 2]> {
        self.textures.get(id).map(|e| [e.width, e.height])
    }

    /// Frees a texture. Its id becomes invalid (sprites using it are skipped).
    pub fn release_texture(&mut self, id: TextureId) {
        if id != self.white {
            self.textures.release(id);
        }
    }

    // ---- drawing -------------------------------------------------------

    /// Renders `scene` through the camera into a pass on `view`.
    #[allow(clippy::too_many_arguments)]
    fn draw_pass(
        &mut self,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        target_size: [u32; 2],
        viewport: Viewport,
        load: wgpu::LoadOp<wgpu::Color>,
        scene: &Scene,
    ) {
        orbit_profile::scope!("2d draw pass");
        self.scratch.clear();
        if let Some(color) = scene.clear {
            let r = self.camera.visible_rect();
            self.scratch.push(
                Sprite2d::new(self.white, [r[0], r[1]], [r[2] - r[0], r[3] - r[1]])
                    .anchor([0.0, 0.0])
                    .layer(Layer(0))
                    .z(-1.0e9)
                    .tint(color),
            );
        }
        let offset = self.scratch.len();
        self.scratch.extend_from_slice(scene.sprites());

        self.sprites.ensure_pipeline(self.gfx.device(), format);
        let view_proj = self.camera.view_proj();
        // Where each mesh falls in the sorted sprite sequence.
        let mut slots: Vec<(usize, usize)> = Vec::new();
        let scratch = &self.scratch;
        self.sprites.prepare(
            self.gfx.device(),
            self.gfx.queue(),
            &view_proj,
            scratch,
            |order| {
                if scene.meshes().is_empty() {
                    return Vec::new();
                }
                slots = mesh::plan(scratch, order, offset, scene.meshes(), scene.mesh_after());
                slots.iter().map(|s| s.0).collect()
            },
        );
        if !slots.is_empty() {
            self.meshes.ensure_pipeline(self.gfx.device(), format);
        }
        self.meshes.prepare(self.gfx.device(), self.gfx.queue(), &view_proj, scene.meshes());

        let mut encoder =
            self.gfx
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("orbit2d scene encoder"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("orbit2d scene pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.gfx.timestamp_writes("2d scene"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            set_viewport(&mut pass, viewport, target_size);
            let clip = batcher::ClipSpace {
                visible: self.camera.visible_rect(),
                viewport,
                size: target_size,
            };
            let batches = self.sprites.batch_count();
            if slots.is_empty() {
                self.sprites.draw(&mut pass, format, &self.textures, &clip, 0..batches);
            } else {
                // Interleave: sprite batches never span a mesh slot (breaks).
                let mut next = 0;
                let mut start = 0;
                for b in 0..=batches {
                    let pos = if b < batches { self.sprites.batch_start(b) } else { usize::MAX };
                    if next < slots.len() && slots[next].0 <= pos {
                        self.sprites.draw(&mut pass, format, &self.textures, &clip, start..b);
                        start = b;
                        while next < slots.len() && slots[next].0 <= pos {
                            self.meshes.draw(&mut pass, format, &self.textures, &clip, slots[next].1);
                            next += 1;
                        }
                    }
                }
                self.sprites.draw(&mut pass, format, &self.textures, &clip, start..batches);
            }
        }
        self.gfx.queue().submit(Some(encoder.finish()));
    }

    /// Number of meshes drawn by the last scene (diagnostics).
    pub fn last_mesh_count(&self) -> usize {
        self.meshes.draw_count()
    }

    /// Number of draw calls used by the last scene (diagnostics).
    pub fn last_batch_count(&self) -> usize {
        self.sprites.batch_count()
    }

    /// Draws `scene` into a render target created with
    /// [`Renderer2d::create_render_target`], using the camera at the target's
    /// full size. The target is cleared to transparent black first.
    pub fn render_to_target(
        &mut self,
        target: TextureId,
        scene: &Scene,
    ) -> Result<(), RenderError> {
        self.render_to_target_with(target, scene, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT))
    }

    /// Like [`Renderer2d::render_to_target`], but draws over what the target
    /// already holds (a second pass: the UI over the stage).
    pub fn render_to_target_over(
        &mut self,
        target: TextureId,
        scene: &Scene,
    ) -> Result<(), RenderError> {
        self.render_to_target_with(target, scene, wgpu::LoadOp::Load)
    }

    fn render_to_target_with(
        &mut self,
        target: TextureId,
        scene: &Scene,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> Result<(), RenderError> {
        let entry = self
            .textures
            .get(target)
            .ok_or(RenderError::UnknownTexture(target))?;
        if !entry.is_target {
            return Err(RenderError::NotATarget(target));
        }
        let (view, size) = (entry.view.clone(), [entry.width, entry.height]);
        self.draw_pass(
            &view,
            TARGET_FORMAT,
            size,
            Viewport::full(size),
            load,
            scene,
        );
        Ok(())
    }

    /// Acquires the next surface frame. `Ok(None)` means skip this frame.
    pub fn begin_frame(&mut self) -> Result<Option<Frame>, RenderError> {
        Ok(self.gfx.begin_frame()?.map(|surface| Frame {
            surface,
            cleared: false,
        }))
    }

    /// Presents the frame. A frame on which nothing was drawn shows the bar
    /// color.
    pub fn end_frame(&mut self, mut frame: Frame) {
        if !frame.cleared {
            let load = load_op(&mut frame.cleared, self.bar_color);
            self.clear_surface(&frame, load);
        }
        self.gfx.end_frame(frame.surface);
    }

    fn clear_surface(&self, frame: &Frame, load: wgpu::LoadOp<wgpu::Color>) {
        let mut encoder =
            self.gfx
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("orbit2d clear encoder"),
                });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("orbit2d clear pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &frame.surface.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        self.gfx.queue().submit(Some(encoder.finish()));
    }

    /// Draws `scene` onto the surface, letterboxed.
    pub fn draw_scene(&mut self, frame: &mut Frame, scene: &Scene) -> Result<(), RenderError> {
        let load = load_op(&mut frame.cleared, self.bar_color);
        let (w, h) = self.gfx.size();
        let viewport = self.viewport();
        let format = self.gfx.surface_format();
        self.draw_pass(&frame.surface.view, format, [w, h], viewport, load, scene);
        Ok(())
    }

    /// Draws a transition between two render targets onto the surface,
    /// letterboxed. `progress` 0 shows `from`, 1 shows `to`.
    pub fn draw_transition(
        &mut self,
        frame: &mut Frame,
        from: TextureId,
        to: TextureId,
        transition: &Transition,
        progress: f32,
    ) -> Result<(), RenderError> {
        let load = load_op(&mut frame.cleared, self.bar_color);
        let (w, h) = self.gfx.size();
        let viewport = self.viewport();
        let format = self.gfx.surface_format();
        let view = frame.surface.view.clone();
        self.transition_pass(
            &view,
            format,
            [w, h],
            viewport,
            load,
            (from, to),
            transition,
            progress,
        )
    }

    /// Like [`Renderer2d::draw_transition`], into a render target (covering
    /// all of it, over what it already holds).
    pub fn draw_transition_to_target(
        &mut self,
        target: TextureId,
        from: TextureId,
        to: TextureId,
        transition: &Transition,
        progress: f32,
    ) -> Result<(), RenderError> {
        let entry = self
            .textures
            .get(target)
            .ok_or(RenderError::UnknownTexture(target))?;
        if !entry.is_target {
            return Err(RenderError::NotATarget(target));
        }
        let (view, size) = (entry.view.clone(), [entry.width, entry.height]);
        self.transition_pass(
            &view,
            TARGET_FORMAT,
            size,
            Viewport::full(size),
            wgpu::LoadOp::Load,
            (from, to),
            transition,
            progress,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn transition_pass(
        &mut self,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        viewport: Viewport,
        load: wgpu::LoadOp<wgpu::Color>,
        (from, to): (TextureId, TextureId),
        transition: &Transition,
        progress: f32,
    ) -> Result<(), RenderError> {
        let mask_id = match transition {
            Transition::MaskDissolve { mask, .. } => *mask,
            _ => self.white,
        };
        let get = |id: TextureId| self.textures.get(id).ok_or(RenderError::UnknownTexture(id));
        let (from_e, to_e, mask_e) = (get(from)?, get(to)?, get(mask_id)?);

        let uniform = TransitionUniform::new(transition, progress);
        self.gfx
            .queue()
            .write_buffer(&self.transitions.uniform, 0, bytemuck::bytes_of(&uniform));

        self.transitions.pipeline(self.gfx.device(), format);
        let bind_group = self
            .gfx
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("orbit2d transition bind group"),
                layout: &self.transitions.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.transitions.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&from_e.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&to_e.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&mask_e.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(self.textures.sampler(true)),
                    },
                ],
            });

        let mut encoder =
            self.gfx
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("orbit2d transition encoder"),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("orbit2d transition pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.gfx.timestamp_writes("2d transition"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            set_viewport(&mut pass, viewport, size);
            if let Some(pipeline) = self.transitions.pipeline_ref(format) {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, Some(&bind_group), &[]);
                pass.draw(0..3, 0..1);
            }
        }
        self.gfx.queue().submit(Some(encoder.finish()));
        Ok(())
    }
}

/// Sets viewport + scissor to `vp`, clamped inside a target of `size`.
fn set_viewport(pass: &mut wgpu::RenderPass<'_>, vp: Viewport, size: [u32; 2]) {
    let x = (vp.x.max(0.0) as u32).min(size[0].saturating_sub(1));
    let y = (vp.y.max(0.0) as u32).min(size[1].saturating_sub(1));
    let w = (vp.width as u32).clamp(1, size[0].saturating_sub(x).max(1));
    let h = (vp.height as u32).clamp(1, size[1].saturating_sub(y).max(1));
    pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
    pass.set_scissor_rect(x, y, w, h);
}
