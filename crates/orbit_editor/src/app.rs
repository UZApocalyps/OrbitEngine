//! The window application: egui-winit + egui-wgpu on the shared `Gfx`
//! device, driven by `orbit_platform` in on-demand mode.
//!
//! A frame is drawn only when something asks for it: a window event egui
//! cares about, a deadline requested by egui (`repaint_delay`), a toast
//! expiring, an autosave falling due. With nothing to do the loop sleeps and
//! zero frames are drawn.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use orbit_gfx::{Gfx, wgpu};
use orbit_platform::{
    App, AppConfig, EventResponse, HostRequest, PhysicalSize, PlatformError, PlatformWindow,
    RedrawMode, WindowEvent,
};

use crate::args::Args;
use crate::core::{CoreOptions, EditorCore};
use crate::logging;

/// Frames needed for egui to settle (layout passes, font atlas) before a
/// screenshot taken without `--exit-after`.
const SETTLE_FRAMES: u64 = 6;
/// Retries (50 ms apart) when the surface could not give a frame.
const MAX_SKIPPED_RETRIES: u32 = 20;

/// Runs the editor until the window closes.
pub fn run(args: Args, t0: Instant) -> Result<(), PlatformError> {
    let (width, height) = args.size.unwrap_or((1360.0, 820.0));
    let config = AppConfig {
        title: "OrbitEngine".to_owned(),
        initial_size: (width, height),
        redraw: RedrawMode::OnDemand,
        ..AppConfig::default()
    };
    orbit_platform::run(config, move |window| async move {
        EditorApp::new(window, args, t0).await
    })
}

pub struct EditorApp {
    window: Arc<PlatformWindow>,
    gfx: Gfx,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    core: EditorCore,
    args: Args,
    t0: Instant,
    /// Frames presented.
    frames: u64,
    last_frame: Option<Instant>,
    egui_wake: Option<Instant>,
    retry_wake: Option<Instant>,
    skipped: u32,
    deadline: Option<Instant>,
    deadline_hit: bool,
    host: Vec<HostRequest>,
    title: String,
    capture_done: bool,
}

impl EditorApp {
    pub async fn new(window: Arc<PlatformWindow>, args: Args, t0: Instant) -> Self {
        let size = window.inner_size();
        let mut gfx = Gfx::new(window.clone(), (size.width, size.height))
            .await
            .expect("GPU initialization failed");
        gfx.enable_gamma_view();
        let gpu_ready = t0.elapsed();

        let egui_ctx = egui::Context::default();
        let config_dir = if args.no_config {
            None
        } else {
            args.config_dir
                .clone()
                .or_else(crate::config::default_config_dir)
        };
        let mut options = CoreOptions {
            config_dir,
            theme: args.theme,
            open_palette: args.open_palette,
            palette_query: args.palette_query.clone(),
            ..CoreOptions::default()
        };
        if let Some(secs) = args.autosave_secs {
            options.autosave_interval = Duration::from_secs(secs);
        }
        let mut core = EditorCore::new(options, &egui_ctx);
        core.show_frame_counter = args.frame_counter;
        if let Some(path) = &args.project {
            core.open_project(path);
            if let Some(n) = args.select_node {
                core.select_nth_node(n);
            }
        }
        for action in &args.run_actions {
            core.run_action_now(action, &egui_ctx);
        }

        let renderer = egui_wgpu::Renderer::new(
            gfx.device(),
            gfx.gamma_view_format(),
            egui_wgpu::RendererOptions::default(),
        );
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            None,
            Some(gfx.max_texture_dimension() as usize),
        );
        log::info!(
            "editor ready: GPU {} ms, UI + project {} ms after process start",
            gpu_ready.as_millis(),
            t0.elapsed().as_millis()
        );
        let deadline = args
            .exit_after
            .map(|s| Instant::now() + Duration::from_secs_f32(s.max(0.0)));
        let title = core.window_title();
        window.set_title(&title);
        Self {
            window,
            gfx,
            egui_ctx,
            egui_state,
            renderer,
            core,
            args,
            t0,
            frames: 0,
            last_frame: None,
            egui_wake: None,
            retry_wake: None,
            skipped: 0,
            deadline,
            deadline_hit: false,
            host: Vec::new(),
            title,
            capture_done: false,
        }
    }

    fn clear_color(&self) -> wgpu::Color {
        let [r, g, b] = self.core.theme.bg;
        wgpu::Color {
            r: f64::from(r) / 255.0,
            g: f64::from(g) / 255.0,
            b: f64::from(b) / 255.0,
            a: 1.0,
        }
    }

    /// Renders the UI once more into an offscreen texture and writes it as a
    /// PNG (what the user would see, without the window frame).
    fn capture(
        &mut self,
        path: &Path,
        jobs: &[egui::ClippedPrimitive],
        screen: &egui_wgpu::ScreenDescriptor,
    ) -> Result<(), String> {
        let (width, height) = self.gfx.size();
        let format = self.gfx.gamma_view_format();
        let device = self.gfx.device();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("editor screenshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("editor screenshot readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("editor screenshot encoder"),
        });
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("editor screenshot pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(self.clear_color()),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, jobs, screen);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.gfx.queue().submit(Some(encoder.finish()));

        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.gfx
            .device()
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("GPU poll failed: {e}"))?;
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("readback failed: {e}"))?;

        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        {
            let data = buffer
                .slice(..)
                .get_mapped_range()
                .map_err(|e| format!("readback failed: {e}"))?;
            for y in 0..height as usize {
                let row = &data[y * padded_row as usize..][..width as usize * 4];
                rgba.extend_from_slice(row);
            }
        }
        buffer.unmap();
        if matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            for px in rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        image::save_buffer(path, &rgba, width, height, image::ColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        log::info!("screenshot written: {} ({width}x{height})", path.display());
        Ok(())
    }
}

impl App for EditorApp {
    fn update(&mut self, _dt: Duration) {
        if let Some(deadline) = self.deadline
            && Instant::now() >= deadline
        {
            self.deadline_hit = true;
        }
    }

    fn render(&mut self) {
        let now = Instant::now();
        self.egui_wake = None;
        self.retry_wake = None;
        let (width, height) = self.gfx.size();
        if width == 0 || height == 0 || self.window.inner_size().width == 0 {
            return;
        }
        for (level, message) in logging::drain() {
            self.core.state.console.push(level, message);
        }
        self.core.state.debug_frames = self.args.frame_counter.then_some(self.frames + 1);

        // ---- UI
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let output = {
            let core = &mut self.core;
            self.egui_ctx.run_ui(raw_input, |ui| core.ui(ui))
        };
        let egui::FullOutput {
            platform_output,
            mut textures_delta,
            shapes,
            pixels_per_point,
            viewport_output,
        } = output;
        self.egui_state
            .handle_platform_output(&self.window, platform_output);
        let repaint_after = viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |v| v.repaint_delay);
        self.egui_wake = if repaint_after.is_zero() {
            Some(now)
        } else if repaint_after < Duration::from_secs(3600) {
            Some(now + repaint_after)
        } else {
            None
        };
        let title = self.core.window_title();
        if title != self.title {
            self.window.set_title(&title);
            self.title = title;
        }

        // ---- paint
        let jobs = self.egui_ctx.tessellate(shapes, pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point,
        };
        let device = self.gfx.device().clone();
        let queue = self.gfx.queue().clone();
        // Texture uploads must happen even when the frame ends up skipped.
        for (id, deltas) in &textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(&device, &queue, *id, delta);
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("editor encoder"),
        });
        let extra = self
            .renderer
            .update_buffers(&device, &queue, &mut encoder, &jobs, &screen);
        let mut presented = false;
        match self.gfx.begin_frame() {
            Ok(Some(frame)) => {
                let view = self.gfx.gamma_view(&frame);
                {
                    let mut pass = encoder
                        .begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("editor pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(self.clear_color()),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        })
                        .forget_lifetime();
                    self.renderer.render(&mut pass, &jobs, &screen);
                }
                queue.submit(extra.into_iter().chain(Some(encoder.finish())));
                self.gfx.end_frame(frame);
                presented = true;
                self.skipped = 0;
            }
            Ok(None) => {
                queue.submit(extra.into_iter().chain(Some(encoder.finish())));
                self.skipped += 1;
                if self.skipped <= MAX_SKIPPED_RETRIES {
                    self.retry_wake = Some(now + Duration::from_millis(50));
                }
            }
            Err(e) => {
                log::error!("cannot acquire a frame: {e}");
                queue.submit(extra.into_iter().chain(Some(encoder.finish())));
            }
        }
        for id in &textures_delta.free {
            self.renderer.free_texture(id);
        }
        textures_delta.clear();

        if presented {
            self.frames += 1;
            if self.frames == 1 {
                log::info!(
                    "first frame presented {} ms after process start",
                    self.t0.elapsed().as_millis()
                );
            }
            if self.args.frame_counter {
                let since = self.last_frame.map_or_else(
                    || "first".to_owned(),
                    |t| format!("+{} ms", now.duration_since(t).as_millis()),
                );
                log::info!(
                    target: "orbit_editor::perf",
                    "frame #{} at {:.2} s ({since}), egui repaint_delay {:?}",
                    self.frames,
                    self.t0.elapsed().as_secs_f32(),
                    repaint_after
                );
                if repaint_after.is_zero() {
                    log::debug!(
                        target: "orbit_editor::perf",
                        "  repaint causes: {:?}",
                        self.egui_ctx.repaint_causes()
                    );
                }
            }
            self.last_frame = Some(now);
        }

        // ---- screenshot, exit
        if let Some(path) = self.args.screenshot.clone()
            && !self.capture_done
            && (self.deadline_hit
                || (self.args.exit_after.is_none() && self.frames >= SETTLE_FRAMES))
        {
            self.capture_done = true;
            if let Err(e) = self.capture(&path, &jobs, &screen) {
                log::error!("screenshot failed: {e}");
            }
            self.host.push(HostRequest::Quit);
        }
        if self.deadline_hit {
            let idle = self.last_frame.map_or(0.0, |t| {
                self.t0.elapsed().as_secs_f32() - (t - self.t0).as_secs_f32()
            });
            log::info!(
                target: "orbit_editor::perf",
                "exit requested after {} frame(s); last frame {idle:.2} s before this one",
                self.frames
            );
            self.host.push(HostRequest::Quit);
        }
        if self.core.take_quit() {
            self.core.shutdown();
            self.host.push(HostRequest::Quit);
        }
    }

    fn resize(&mut self, size: PhysicalSize<u32>, _scale_factor: f64) {
        self.gfx.resize(size.width, size.height);
    }

    fn window_event(&mut self, event: &WindowEvent) -> EventResponse {
        let response = self.egui_state.on_window_event(&self.window, event);
        if response.repaint && log::log_enabled!(target: "orbit_editor::perf", log::Level::Trace) {
            let text = format!("{event:?}");
            log::trace!(
                target: "orbit_editor::perf",
                "window event wants a frame: {}",
                text.chars().take(90).collect::<String>()
            );
        }
        EventResponse {
            consumed: response.consumed,
            repaint: response.repaint,
        }
    }

    fn close_requested(&mut self) -> bool {
        if self.core.request_close() {
            self.core.shutdown();
            true
        } else {
            // The "unsaved changes" dialog is shown: draw it.
            self.host.push(HostRequest::RequestRedraw);
            false
        }
    }

    fn next_wake(&mut self) -> Option<Instant> {
        let deadline = self.deadline.filter(|_| !self.deadline_hit);
        [
            self.egui_wake,
            self.retry_wake,
            deadline,
            self.core.next_wake(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn host_requests(&mut self) -> Vec<HostRequest> {
        std::mem::take(&mut self.host)
    }
}
