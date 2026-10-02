//! OrbitEngine player.
//!
//! * default: plays a story RON file with [`orbit_runtime`]. Natively
//!   `orbit_player [story.ron | project-dir] [--root <dir>] [--no-watch]`
//!   (default `examples/stories/demo.ron`); on the web `?story=<path>` (default
//!   `examples/stories/demo.ron`, relative to the page);
//! * demo: `--demo` natively, `?demo` on the web: the hard-coded visual demo
//!   of the M1 renderer, see [`demo`].

mod demo;
mod launch;

use orbit_gfx::Gfx;
use orbit_platform::{AppConfig, PlatformError};
use orbit_render2d::Renderer2d;
use orbit_runtime::Runtime;
use thiserror::Error;

pub use launch::{Launch, parse_args, parse_query, query_param};

#[derive(Debug, Error)]
pub enum PlayerError {
    #[error(transparent)]
    Platform(#[from] PlatformError),
    #[error("invalid command line: {0}")]
    Args(String),
}

/// Installs the panic hook and the logger (browser console on the web,
/// `env_logger` natively; use `RUST_LOG` to tune the level).
pub fn init_logging() {
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        let _ = console_log::init_with_level(log::Level::Info);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }
}

/// Opens the window and runs the player until it is closed.
pub fn run() -> Result<(), PlayerError> {
    let launch = Launch::from_environment().map_err(PlayerError::Args)?;
    if launch.demo {
        return demo::run();
    }
    let config = launch.runtime_config();
    orbit_platform::run(
        AppConfig {
            title: "OrbitEngine Player".to_owned(),
            ..Default::default()
        },
        |window| async move {
            let size = window.inner_size();
            let gfx = Gfx::new(window, (size.width, size.height))
                .await
                .expect("GPU initialization failed");
            let renderer = Renderer2d::new(gfx).expect("renderer initialization failed");
            Runtime::new(renderer, config).expect("runtime initialization failed")
        },
    )?;
    Ok(())
}
