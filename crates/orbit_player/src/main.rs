//! Player entry point. The same `main` serves native builds and the wasm
//! build (wasm-bindgen runs it as the module start function).

fn main() {
    orbit_player::init_logging();
    if let Err(err) = orbit_player::run() {
        log::error!("player failed: {err}");
        #[cfg(not(target_arch = "wasm32"))]
        std::process::exit(1);
    }
}
