//! OrbitEngine player.
//!
//! * default: plays a story RON file with [`orbit_runtime`]. Natively
//!   `orbit_player [story.ron | project-dir] [--root <dir>] [--no-watch]`
//!   (default `examples/stories/demo.ron`); on the web `?story=<path>` (default
//!   `examples/stories/demo.ron`, relative to the page);
//! * demo: `--demo` natively, `?demo` on the web: the hard-coded visual demo
//!   of the M1 renderer, see [`demo`];
//! * UI gallery: `--ui-gallery` natively, `?ui-gallery` on the web: every M2
//!   menu screen with fake data, see [`gallery`].
//!
//! The demo and the gallery are the `showcase` feature (on by default); the
//! player templates used to export games are built without it.
//!
//! An exported game is this same program with a **pak** (see `orbit_pak`):
//! glued to the executable, `game.pak` beside it, `--pak <file>`, or the
//! `ORBIT_PAK` global of the web page.

#[cfg(feature = "showcase")]
mod demo;
#[cfg(feature = "showcase")]
mod gallery;
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

/// The window of the player: the OrbitEngine one, or, for an exported game
/// read natively, its name and icon (stored in the pak). On the web the page
/// has the title and the favicon.
fn window_config(launch: &Launch) -> AppConfig {
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut config = AppConfig {
        title: "OrbitEngine Player".to_owned(),
        ..Default::default()
    };
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(location) = &launch.pak {
        use orbit_assets::PakSource;
        use orbit_platform::WindowIcon;
        match PakSource::open_blocking(location) {
            Ok(pak) => {
                if let Some(name) = pak.meta("name") {
                    config.title = name.to_owned();
                }
                if let Some(icon) = pak
                    .meta("icon")
                    .and_then(|path| pak.read_blocking(path).ok())
                    .and_then(|png| WindowIcon::from_png(&png))
                {
                    config.icon = Some(icon);
                }
            }
            // The runtime reports the same problem on its error screen.
            Err(err) => log::error!("cannot read the game pak: {err}"),
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = launch;
    config
}

/// Opens the window and runs the player until it is closed.
pub fn run() -> Result<(), PlayerError> {
    let launch = Launch::from_environment().map_err(PlayerError::Args)?;
    if launch.demo || launch.ui_gallery {
        #[cfg(feature = "showcase")]
        return if launch.demo {
            demo::run()
        } else {
            gallery::run()
        };
        #[cfg(not(feature = "showcase"))]
        return Err(PlayerError::Args(
            "this player was built without the demo and the UI gallery".to_owned(),
        ));
    }
    let config = launch.runtime_config();
    orbit_platform::run(window_config(&launch), |window| async move {
        let size = window.inner_size();
        let gfx = Gfx::new(window, (size.width, size.height))
            .await
            .expect("GPU initialization failed");
        let renderer = Renderer2d::new(gfx).expect("renderer initialization failed");
        Runtime::new(renderer, config).expect("runtime initialization failed")
    })?;
    Ok(())
}
