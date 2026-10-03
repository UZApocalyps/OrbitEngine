//! OrbitEngine low-level graphics layer on top of wgpu.
//!
//! Responsibilities: instance/adapter/device/surface setup (async, native and
//! WebGL2), resize handling, surface loss recovery, RGBA8 texture upload and a
//! simple textured sprite pipeline.
//!
//! The crate does not depend on any windowing library: the caller passes
//! anything convertible to a [`wgpu::SurfaceTarget`] (e.g. an
//! `Arc<winit::window::Window>`).

mod error;
mod sprite;
mod texture;

pub use error::GfxError;
pub use sprite::Sprite;
pub use texture::{TEXTURE_FORMAT, Texture};
pub use wgpu;

use sprite::SpritePipeline;

/// Result of [`Gfx::render`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStatus {
    /// A frame was drawn and presented.
    Presented,
    /// The frame was skipped (zero-sized, occluded, timed out, or the surface
    /// had to be reconfigured/recreated); try again next frame.
    Skipped,
}

/// A surface texture being drawn to, returned by [`Gfx::begin_frame`].
pub struct SurfaceFrame {
    texture: wgpu::SurfaceTexture,
    /// View of the surface texture, to use as a render pass color attachment.
    pub view: wgpu::TextureView,
    suboptimal: bool,
}

type TargetFactory = Box<dyn Fn() -> wgpu::SurfaceTarget<'static>>;

pub struct Gfx {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    /// `None` while the app is suspended (Android): see [`Gfx::suspend`].
    surface: Option<wgpu::Surface<'static>>,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    target_factory: TargetFactory,
    pub(crate) sprites: SpritePipeline,
    /// Set by wgpu's device-lost callback: see [`Gfx::device_lost`].
    lost: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl Gfx {
    /// Initializes the GPU. `size` is the initial surface size in physical
    /// pixels (a zero dimension is clamped to 1 and the surface stays inactive
    /// until [`Gfx::resize`] provides a real size).
    pub async fn new<T>(target: T, size: (u32, u32)) -> Result<Self, GfxError>
    where
        T: Into<wgpu::SurfaceTarget<'static>> + Clone + 'static,
    {
        let target_factory: TargetFactory = Box::new(move || target.clone().into());

        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        // Web: WebGL2 only. Native: every backend wgpu was built with.
        #[cfg(target_arch = "wasm32")]
        {
            desc.backends = wgpu::Backends::GL;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            desc = desc.with_env();
        }
        let instance = wgpu::Instance::new(desc);

        let surface = instance.create_surface(target_factory())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;
        log::info!("GPU adapter: {:?}", adapter.get_info());

        // WebGL2 downlevel limits on the web; regular defaults natively.
        #[cfg(target_arch = "wasm32")]
        let required_limits = wgpu::Limits::downlevel_webgl2_defaults();
        #[cfg(not(target_arch = "wasm32"))]
        let required_limits = wgpu::Limits::default();

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("orbit device"),
                required_limits: required_limits.using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await?;
        // A lost device (driver reset / TDR, GPU removed) makes wgpu fail every
        // later call *silently* (no validation panic): record it so that the
        // caller can stop instead of tripping over `None` results downstream.
        let lost = std::sync::Arc::new(std::sync::Mutex::new(None));
        {
            let lost = lost.clone();
            device.set_device_lost_callback(move |reason, message| {
                let text = format!("{reason:?}: {message}");
                if reason != wgpu::DeviceLostReason::Destroyed {
                    log::error!("GPU device lost ({text})");
                }
                if let Ok(mut slot) = lost.lock() {
                    slot.get_or_insert(text);
                }
            });
        }

        let mut config = surface
            .get_default_config(&adapter, size.0.max(1), size.1.max(1))
            .ok_or(GfxError::UnsupportedSurface)?;
        // Prefer an sRGB surface so linear shader output is encoded correctly.
        let caps = surface.get_capabilities(&adapter);
        if let Some(srgb) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = srgb;
        }
        surface.configure(&device, &config);

        let sprites = SpritePipeline::new(&device, config.format);
        let mut gfx = Self {
            instance,
            adapter,
            surface: Some(surface),
            device,
            queue,
            config,
            target_factory,
            sprites,
            lost,
        };
        gfx.resize(size.0, size.1);
        Ok(gfx)
    }

    /// Why the GPU device was lost, if it was (driver reset, GPU removed...).
    /// Once lost, the device and everything created from it are unusable:
    /// wgpu turns every call into a silent no-op (`Queue::write_buffer_with`
    /// returns `None`, which egui-wgpu turns into a panic), so frames must
    /// stop being recorded.
    pub fn device_lost(&self) -> Option<String> {
        self.lost.lock().ok().and_then(|slot| slot.clone())
    }

    /// Current surface size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Opt-in for renderers that want the surface in gamma space (egui):
    /// allows [`Gfx::gamma_view`] to view the sRGB surface as its non-sRGB
    /// twin, so shaders can write sRGB-encoded values and blend in gamma
    /// space. Native only (WebGL2 has no view formats): a no-op on the web,
    /// where [`Gfx::gamma_view_format`] stays the surface format.
    pub fn enable_gamma_view(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let gamma = self.config.format.remove_srgb_suffix();
            if gamma != self.config.format {
                self.config.view_formats = vec![gamma];
                self.configure_surface();
            }
        }
    }

    /// Format of [`Gfx::gamma_view`]: the non-sRGB twin of the surface format
    /// once [`Gfx::enable_gamma_view`] was called, else the surface format.
    pub fn gamma_view_format(&self) -> wgpu::TextureFormat {
        if self.config.view_formats.is_empty() {
            self.config.format
        } else {
            self.config.format.remove_srgb_suffix()
        }
    }

    /// A view of `frame` in [`Gfx::gamma_view_format`].
    pub fn gamma_view(&self, frame: &SurfaceFrame) -> wgpu::TextureView {
        frame
            .texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor {
                format: Some(self.gamma_view_format()),
                ..Default::default()
            })
    }

    fn configure_surface(&self) {
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
    }

    /// Releases the surface (Android: the native window is going away when
    /// the app is sent to the background). The device, the textures and the
    /// pipelines are kept; frames are skipped until [`Gfx::resume`].
    pub fn suspend(&mut self) {
        self.surface = None;
    }

    /// Whether the surface was released by [`Gfx::suspend`].
    pub fn is_suspended(&self) -> bool {
        self.surface.is_none()
    }

    /// Recreates the surface on the (new) native window after
    /// [`Gfx::suspend`], with the current configuration. A no-op when the
    /// surface exists. `size` is the window size now, in physical pixels.
    pub fn resume(&mut self, size: (u32, u32)) -> Result<(), GfxError> {
        if self.surface.is_none() {
            self.surface = Some(self.instance.create_surface((self.target_factory)())?);
        }
        if size.0 > 0 && size.1 > 0 {
            self.config.width = size.0;
            self.config.height = size.1;
        }
        self.configure_surface();
        Ok(())
    }

    /// Reconfigures the surface. A zero dimension (minimized window) is
    /// ignored: rendering is skipped until a valid size arrives.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.configure_surface();
    }

    /// Draws `sprites` (in order) over a cleared background and presents.
    /// `clear` is a *linear* RGBA color. This is the M0 convenience path; richer
    /// renderers use [`Gfx::begin_frame`] / [`Gfx::end_frame`] instead.
    pub fn render(
        &mut self,
        clear: [f64; 4],
        sprites: &[Sprite<'_>],
    ) -> Result<FrameStatus, GfxError> {
        let Some(frame) = self.begin_frame()? else {
            return Ok(FrameStatus::Skipped);
        };
        self.draw_frame(&frame.texture, clear, sprites);
        self.end_frame(frame);
        Ok(FrameStatus::Presented)
    }

    /// Acquires the next surface texture. Returns `Ok(None)` when the frame
    /// must be skipped (zero-sized, occluded, timed out, or the surface had to
    /// be reconfigured/recreated); try again next frame.
    ///
    /// Surface problems are handled here: `Outdated` reconfigures, `Lost`
    /// recreates the surface, `Suboptimal` still yields a drawable frame and the
    /// surface is reconfigured by [`Gfx::end_frame`].
    pub fn begin_frame(&mut self) -> Result<Option<SurfaceFrame>, GfxError> {
        use wgpu::CurrentSurfaceTexture as Cst;

        let Some(surface) = &self.surface else {
            return Ok(None); // suspended
        };
        let (texture, suboptimal) = match surface.get_current_texture() {
            Cst::Success(f) => (f, false),
            Cst::Suboptimal(f) => (f, true),
            Cst::Outdated => {
                self.configure_surface();
                return Ok(None);
            }
            Cst::Lost => {
                log::warn!("surface lost, recreating");
                self.surface = Some(self.instance.create_surface((self.target_factory)())?);
                self.configure_surface();
                return Ok(None);
            }
            Cst::Timeout | Cst::Occluded => return Ok(None),
            Cst::Validation => {
                log::error!("validation error while acquiring the surface texture");
                return Ok(None);
            }
        };
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Some(SurfaceFrame {
            texture,
            view,
            suboptimal,
        }))
    }

    /// Presents a frame obtained from [`Gfx::begin_frame`].
    pub fn end_frame(&mut self, frame: SurfaceFrame) {
        let SurfaceFrame {
            texture,
            suboptimal,
            ..
        } = frame;
        self.queue.present(texture);
        if suboptimal {
            self.configure_surface();
        }
    }

    /// The wgpu device.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The wgpu queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Largest supported 2D texture dimension (2048 on WebGL2 downlevel limits).
    pub fn max_texture_dimension(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    fn draw_frame(
        &mut self,
        frame: &wgpu::SurfaceTexture,
        clear: [f64; 4],
        sprites: &[Sprite<'_>],
    ) {
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.sprites.prepare(
            &self.device,
            &self.queue,
            [self.config.width, self.config.height],
            sprites,
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0],
                            g: clear[1],
                            b: clear[2],
                            a: clear[3],
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.sprites.draw(&mut pass, sprites);
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// The adapter in use (diagnostics).
    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }
}
