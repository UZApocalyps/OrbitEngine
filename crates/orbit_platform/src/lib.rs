//! OrbitEngine platform layer: a window and an application loop.
//!
//! Built on winit 0.30's `ApplicationHandler`. Works on desktop and in the
//! browser (rendering into a `<canvas>`). Time is measured with `web-time`.
//!
//! Usage: implement [`App`], then call [`run`] with an async factory that
//! builds the app once the window exists (GPU initialization is async, and on
//! the web it must not block the browser event loop).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use web_time::{Instant, SystemTime, UNIX_EPOCH};
use winit::application::ApplicationHandler;
#[cfg(not(target_arch = "wasm32"))]
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

pub use winit::dpi::PhysicalSize;
pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode;
pub use winit::window::Window as PlatformWindow;

/// Window creation options.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub title: String,
    /// Initial inner size in logical pixels (ignored on the web, where the
    /// canvas is sized by the page).
    pub initial_size: (f64, f64),
    /// Web only: id of an existing `<canvas>` element to render into. When
    /// absent or not found, a canvas is created and appended to `<body>`.
    pub canvas_id: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            title: "OrbitEngine".to_owned(),
            initial_size: (1280.0, 720.0),
            canvas_id: Some("orbit-canvas".to_owned()),
        }
    }
}

/// Basic input events, already decoupled from winit's event types.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    Key {
        code: KeyCode,
        pressed: bool,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    /// Cursor position in physical pixels, origin top-left.
    CursorMoved {
        x: f64,
        y: f64,
    },
    /// Scroll in lines (pixel deltas are converted approximately).
    Scroll {
        dx: f32,
        dy: f32,
    },
}

/// Something the application asks of the host window / process. Polled with
/// [`App::host_requests`] after every update and input, so the app never
/// needs a handle on the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRequest {
    /// Borderless fullscreen on or off. Ignored on the web (the page decides).
    SetFullscreen(bool),
    /// Close the application (native only; a browser tab cannot quit).
    Quit,
}

/// Whether [`HostRequest::SetFullscreen`] does anything on this platform.
pub const FULLSCREEN_SUPPORTED: bool = cfg!(not(target_arch = "wasm32"));

/// Milliseconds since the Unix epoch (works on the web too).
pub fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Offset of the local time zone from UTC, in minutes (east is positive).
/// `0` when it cannot be determined.
pub fn local_utc_offset_minutes() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        // `getTimezoneOffset` is UTC minus local, hence the sign.
        -(js_sys::Date::new_0().get_timezone_offset() as i32)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        time::UtcOffset::current_local_offset()
            .map_or(0, |offset| i32::from(offset.whole_minutes()))
    }
}

/// The user's preferred language as a BCP 47 tag (`fr-FR`), when the system or
/// browser tells.
pub fn system_language() -> Option<String> {
    sys_locale::get_locale().filter(|l| !l.is_empty())
}

/// The application driven by [`run`].
pub trait App {
    /// Advances the simulation. Called once per frame before [`App::render`].
    fn update(&mut self, _dt: Duration) {}
    /// Draws one frame.
    fn render(&mut self);
    /// The drawable area changed. Size is in physical pixels (may be zero
    /// when minimized) and `scale_factor` is the current DPI scale.
    fn resize(&mut self, size: PhysicalSize<u32>, scale_factor: f64);
    /// Keyboard / mouse input.
    fn input(&mut self, _event: &InputEvent) {}
    /// The user asked to close the window. Return `false` to veto (native
    /// only; the browser has no notion of closing the canvas).
    fn close_requested(&mut self) -> bool {
        true
    }
    /// Requests for the host, drained after every update and input event.
    fn host_requests(&mut self) -> Vec<HostRequest> {
        Vec::new()
    }
}

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
}

type AppFuture = Pin<Box<dyn Future<Output = Box<dyn App>>>>;
type AppFactory = Box<dyn FnOnce(Arc<Window>) -> AppFuture>;

/// Event used to hand the asynchronously built app back to the loop (web).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
enum UserEvent {
    AppReady(Box<dyn App>),
}

/// Creates the window and runs the application until it is closed.
///
/// `factory` receives the window (shared, so the GPU surface can keep it
/// alive) and resolves to the app. On native it is awaited synchronously; on
/// the web it runs as a browser task and the first frames are skipped until it
/// completes. On the web this function returns immediately after scheduling
/// the loop (the loop keeps running through browser callbacks).
pub fn run<F, Fut, A>(config: AppConfig, factory: F) -> Result<(), PlatformError>
where
    F: FnOnce(Arc<Window>) -> Fut + 'static,
    Fut: Future<Output = A> + 'static,
    A: App + 'static,
{
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let runner = Runner {
        config,
        factory: Some(Box::new(move |window| {
            Box::pin(async move { Box::new(factory(window).await) as Box<dyn App> })
        })),
        proxy: event_loop.create_proxy(),
        window: None,
        app: None,
        last_frame: None,
    };

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut runner = runner;
        event_loop.run_app(&mut runner)?;
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(runner);
    }
    Ok(())
}

struct Runner {
    config: AppConfig,
    factory: Option<AppFactory>,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    app: Option<Box<dyn App>>,
    last_frame: Option<Instant>,
}

/// Web: bind the window to the page's canvas (or append a new one).
#[cfg(target_arch = "wasm32")]
fn platform_attributes(
    attrs: winit::window::WindowAttributes,
    config: &AppConfig,
) -> winit::window::WindowAttributes {
    use wasm_bindgen::JsCast;
    use winit::platform::web::WindowAttributesExtWebSys;

    let existing = config.canvas_id.as_deref().and_then(|id| {
        web_sys::window()?
            .document()?
            .get_element_by_id(id)?
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .ok()
    });
    match existing {
        Some(canvas) => attrs.with_canvas(Some(canvas)),
        None => attrs.with_append(true),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_attributes(
    attrs: winit::window::WindowAttributes,
    _config: &AppConfig,
) -> winit::window::WindowAttributes {
    attrs
}

impl Runner {
    /// Executes what the app asked of the window.
    fn serve_requests(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(app), Some(window)) = (self.app.as_mut(), self.window.as_ref()) else {
            return;
        };
        for request in app.host_requests() {
            match request {
                HostRequest::SetFullscreen(on) => {
                    #[cfg(not(target_arch = "wasm32"))]
                    window
                        .set_fullscreen(on.then_some(winit::window::Fullscreen::Borderless(None)));
                    #[cfg(target_arch = "wasm32")]
                    let _ = (on, window);
                }
                HostRequest::Quit => event_loop.exit(),
            }
        }
    }

    fn create_window(&self, event_loop: &ActiveEventLoop) -> Arc<Window> {
        let attrs = Window::default_attributes().with_title(self.config.title.clone());
        // On the web the page (CSS) sizes the canvas: forcing an inner size
        // would pin inline styles on it and fight the layout.
        #[cfg(not(target_arch = "wasm32"))]
        let attrs = attrs.with_inner_size(LogicalSize::new(
            self.config.initial_size.0,
            self.config.initial_size.1,
        ));
        let attrs = platform_attributes(attrs, &self.config);
        Arc::new(
            event_loop
                .create_window(attrs)
                .expect("failed to create the window"),
        )
    }

    fn install_app(&mut self, mut app: Box<dyn App>) {
        if let Some(window) = &self.window {
            let size = window.inner_size();
            app.resize(size, window.scale_factor());
            window.request_redraw();
        }
        self.last_frame = Some(Instant::now());
        self.app = Some(app);
    }
}

impl ApplicationHandler<UserEvent> for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // `resumed` can fire several times (Android); only build once.
        if self.window.is_some() {
            return;
        }
        let window = self.create_window(event_loop);
        self.window = Some(window.clone());
        let Some(factory) = self.factory.take() else {
            return;
        };
        let future = factory(window);

        #[cfg(not(target_arch = "wasm32"))]
        {
            let app = pollster::block_on(future);
            self.install_app(app);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let proxy = self.proxy.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let app = future.await;
                if proxy.send_event(UserEvent::AppReady(app)).is_err() {
                    log::error!("event loop closed before the app was ready");
                }
            });
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::AppReady(app) => self.install_app(app),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.handle_window_event(event_loop, id, event);
        self.serve_requests(event_loop);
    }
}

impl Runner {
    fn handle_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        let Some(app) = self.app.as_mut() else {
            return; // still initializing
        };
        let Some(window) = self.window.as_ref() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                if app.close_requested() {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                app.resize(size, window.scale_factor());
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                // The following `Resized` carries the new size; refresh the scale.
                app.resize(window.inner_size(), window.scale_factor());
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = self
                    .last_frame
                    .map(|t| now.saturating_duration_since(t))
                    .unwrap_or_default();
                self.last_frame = Some(now);
                app.update(dt);
                app.render();
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    app.input(&InputEvent::Key {
                        code,
                        pressed: event.state == ElementState::Pressed,
                    });
                }
            }
            WindowEvent::MouseInput { state, button, .. } => app.input(&InputEvent::MouseButton {
                button,
                pressed: state == ElementState::Pressed,
            }),
            WindowEvent::CursorMoved { position, .. } => app.input(&InputEvent::CursorMoved {
                x: position.x,
                y: position.y,
            }),
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x, y),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32 / 20.0, p.y as f32 / 20.0),
                };
                app.input(&InputEvent::Scroll { dx, dy });
            }
            _ => {}
        }
    }
}
