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

use orbit_editor_api::{PreviewCommand, Selection};
use orbit_gfx::{Gfx, wgpu};
use orbit_platform::{
    App, AppConfig, EventResponse, HostRequest, PhysicalSize, PlatformError, PlatformWindow,
    RedrawMode, WindowEvent,
};

use crate::args::Args;
use crate::core::{CoreOptions, EditorCore};
use crate::logging;
use crate::preview_host::{PreviewBridge, PreviewHost};

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
        ..AppConfig::default() // includes the brand window icon
    };
    orbit_platform::run(config, move |window| async move {
        EditorApp::new(window, args, t0).await
    })
}

pub struct EditorApp {
    window: Arc<PlatformWindow>,
    /// The preview owns the runtime, which owns the `Gfx` shared with egui.
    preview: PreviewHost,
    scene3d: crate::scene3d_host::Scene3dHost,
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
    /// The GPU device was lost and the quit was requested.
    device_lost_handled: bool,
    deadline: Option<Instant>,
    deadline_hit: bool,
    preview_capture_done: bool,
    /// Frame wanted by the preview (animation, hot reload debounce).
    preview_wake: Option<Instant>,
    /// Debug: edit the text of a line this long after the start.
    edit_at: Option<Instant>,
    host: Vec<HostRequest>,
    title: String,
    capture_done: bool,
    /// CPU time of each frame, with `--autoscroll` (ms).
    frame_ms: Vec<f32>,
    /// Time between two presented frames, with `--autoscroll` (ms).
    interval_ms: Vec<f32>,
}

impl EditorApp {
    pub async fn new(window: Arc<PlatformWindow>, args: Args, t0: Instant) -> Self {
        let size = window.inner_size();
        let mut gfx = Gfx::new(window.clone(), (size.width, size.height))
            .await
            .expect("GPU initialization failed");
        gfx.enable_gamma_view();
        let gpu_ready = t0.elapsed();
        let max_texture = gfx.max_texture_dimension();
        let device = gfx.device().clone();
        let gamma_format = gfx.gamma_view_format();
        let preview = PreviewHost::new(gfx).expect("preview runtime initialization failed");

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
            language: args.lang,
            open_palette: args.open_palette,
            palette_query: args.palette_query.clone(),
            ..CoreOptions::default()
        };
        if let Some(secs) = args.autosave_secs {
            options.autosave_interval = Duration::from_secs(secs);
        }
        let mut core = EditorCore::new(options, &egui_ctx);
        core.show_frame_counter = args.frame_counter;
        core.state.debug_autoscroll = args.autoscroll;
        if let Some(path) = &args.project {
            core.open_project(path);
            if let Some(n) = args.select_node {
                core.select_nth_node(n);
            }
            if args.play_preview {
                core.state.preview.command(PreviewCommand::Play);
                if args.preview_skip {
                    core.state.preview.command(PreviewCommand::SetSkip(true));
                }
            }
            if args.play_from_selection
                && let Some(Selection::Node(id)) = core.state.selection
            {
                core.state.preview.command(PreviewCommand::PlayFrom(id));
            }
            if !args.imports.is_empty() {
                core.import_dropped(args.imports.clone());
            }
            if let Some(n) = args.select_asset {
                core.select_nth_asset(n);
            }
            if let Some(query) = &args.search {
                core.open_search(query);
            }
        }
        for action in &args.run_actions {
            core.run_action_now(action, &egui_ctx);
        }
        // Debug option: the live MCP server starts at once (it is off by default).
        if let Some(port) = args.mcp_port {
            core.state.ai.port = port;
            if let Some(token) = &args.mcp_token {
                core.state.ai.token.clone_from(token);
            }
            if let Err(e) = crate::live::start(&mut core) {
                log::error!("--mcp-port: {e}");
            }
        }

        let renderer =
            egui_wgpu::Renderer::new(&device, gamma_format, egui_wgpu::RendererOptions::default());
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            None,
            Some(max_texture as usize),
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
        let edit_at = args
            .edit_text
            .is_some()
            .then(|| Instant::now() + Duration::from_secs_f32(args.edit_delay.max(0.0)));
        Self {
            window,
            preview,
            scene3d: Default::default(),
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
            device_lost_handled: false,
            deadline,
            deadline_hit: false,
            preview_capture_done: false,
            preview_wake: None,
            edit_at,
            host: Vec::new(),
            title,
            capture_done: false,
            frame_ms: Vec::new(),
            interval_ms: Vec::new(),
        }
    }

    /// Debug option `--edit-text`: edits a line once, as typing would.
    fn apply_debug_edit(&mut self, now: Instant) {
        let Some(at) = self.edit_at else { return };
        if now < at {
            return;
        }
        self.edit_at = None;
        if let Some((n, text)) = self.args.edit_text.clone() {
            let started = Instant::now();
            match self.core.edit_nth_say(n, &text) {
                Ok(()) => log::info!(
                    target: "orbit_editor::perf",
                    "debug edit applied to line {n} ({} ms)",
                    started.elapsed().as_millis()
                ),
                Err(e) => log::error!("--edit-text: {e}"),
            }
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
        let (width, height) = self.preview.gfx().size();
        let format = self.preview.gfx().gamma_view_format();
        let device = self.preview.gfx().device();
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
        self.preview.gfx().queue().submit(Some(encoder.finish()));

        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.preview
            .gfx()
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

impl EditorApp {
    /// Once the GPU device is lost (driver reset / TDR, GPU removed), every
    /// wgpu call silently fails and egui-wgpu panics in `update_buffers`:
    /// stop painting, save what can be saved and quit with a clear message.
    /// Returns whether the frame must be abandoned.
    fn stop_if_device_lost(&mut self) -> bool {
        let Some(reason) = self.preview.gfx().device_lost() else {
            return false;
        };
        if !self.device_lost_handled {
            self.device_lost_handled = true;
            log::error!("GPU device lost ({reason}); the editor closes");
            self.core.shutdown();
            self.host.push(HostRequest::Quit);
        }
        true
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
        let (width, height) = self.preview.gfx().size();
        if width == 0 || height == 0 || self.window.inner_size().width == 0 {
            return;
        }
        if self.stop_if_device_lost() {
            return;
        }
        for (level, message) in logging::drain() {
            self.core.state.console.push(level, message);
        }
        self.core.state.debug_frames = self.args.frame_counter.then_some(self.frames + 1);

        // ---- the live MCP: calls of the AI run here, on the editor thread,
        // before the UI so that the frame shows their effect.
        {
            let mut bridge = PreviewBridge {
                host: &mut self.preview,
                renderer: &mut self.renderer,
                now,
            };
            self.core.pump_live(&mut bridge);
        }

        // ---- UI
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let output = {
            let core = &mut self.core;
            self.egui_ctx.run_ui(raw_input, |ui| core.ui(ui))
        };
        // The preview: story sync, commands, input, the game's frame into its
        // target (before egui paints it), state for the next UI pass.
        self.apply_debug_edit(now);
        let wake = self.preview.update(&mut self.core, &mut self.renderer, now);
        if self.scene3d.update(&self.core.shared.scene3d, self.preview.gfx(), &mut self.renderer) {
            self.host.push(HostRequest::RequestRedraw);
        }
        self.preview_wake = wake.at;
        if wake.redraw {
            self.host.push(HostRequest::RequestRedraw);
        }
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
        // The preview may have lost the device just above: egui-wgpu would
        // panic in `update_buffers` (`write_buffer_with` returns `None`).
        if self.stop_if_device_lost() {
            return;
        }
        let jobs = self.egui_ctx.tessellate(shapes, pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point,
        };
        let device = self.preview.gfx().device().clone();
        let queue = self.preview.gfx().queue().clone();
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
        match self.preview.gfx_mut().begin_frame() {
            Ok(Some(frame)) => {
                let view = self.preview.gfx().gamma_view(&frame);
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
                self.preview.gfx_mut().end_frame(frame);
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

        if self.args.autoscroll.is_some() {
            self.frame_ms.push(now.elapsed().as_secs_f32() * 1000.0);
            if let Some(t) = self.last_frame.filter(|_| presented) {
                self.interval_ms
                    .push(now.duration_since(t).as_secs_f32() * 1000.0);
            }
        }
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
        if let Some(path) = self.args.preview_capture.clone()
            && !self.preview_capture_done
            && (self.deadline_hit
                || (self.args.exit_after.is_none() && self.frames >= SETTLE_FRAMES))
        {
            self.preview_capture_done = true;
            match self.preview.start_capture() {
                Some(mut readback) => match self.preview.write_capture(&mut readback, &path) {
                    Ok(()) => log::info!("preview capture written: {}", path.display()),
                    Err(e) => log::error!("preview capture failed: {e}"),
                },
                None => log::error!("preview capture: nothing drawn yet"),
            }
        }
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
        if self.deadline_hit && !self.frame_ms.is_empty() {
            let stats = |v: &[f32]| {
                let mut sorted = v.to_vec();
                sorted.sort_by(f32::total_cmp);
                let avg = sorted.iter().sum::<f32>() / sorted.len() as f32;
                let p95 = sorted[((sorted.len() as f32 * 0.95) as usize).min(sorted.len() - 1)];
                (avg, p95, sorted[sorted.len() - 1])
            };
            let (avg, p95, max) = stats(&self.frame_ms);
            log::info!(
                target: "orbit_editor::perf",
                "autoscroll: {} frames, CPU per frame avg {avg:.2} ms, p95 {p95:.2} ms, max {max:.2} ms",
                self.frame_ms.len()
            );
            if !self.interval_ms.is_empty() {
                let (avg, p95, max) = stats(&self.interval_ms);
                log::info!(
                    target: "orbit_editor::perf",
                    "autoscroll: presented every avg {avg:.2} ms ({:.0} fps), p95 {p95:.2} ms, max {max:.2} ms",
                    1000.0 / avg
                );
            }
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
        self.preview.gfx_mut().resize(size.width, size.height);
    }

    fn window_event(&mut self, event: &WindowEvent) -> EventResponse {
        let response = self.egui_state.on_window_event(&self.window, event);
        if let WindowEvent::Focused(focused) = event {
            // The preview mutes and stops its clock while the editor is in
            // the background; one frame applies it.
            if !self.args.always_active {
                self.preview.set_window_focused(*focused);
                self.host.push(HostRequest::RequestRedraw);
            }
        }
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
            self.preview_wake,
            self.edit_at,
            self.core.next_wake(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn host_requests(&mut self) -> Vec<HostRequest> {
        std::mem::take(&mut self.host)
    }

    fn attach_waker(&mut self, waker: orbit_platform::Waker) {
        // Background threads (thumbnails, file watching, validation) ask for
        // a frame through the event loop.
        self.core.set_waker(move || waker.wake());
    }
}
