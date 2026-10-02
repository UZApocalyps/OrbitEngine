//! Logging: `env_logger` on stderr, plus a copy of the useful records for
//! the editor's console panel.

use std::collections::VecDeque;
use std::sync::Mutex;

use orbit_editor_api::Level;

/// Records waiting for the console (bounded).
static PENDING: Mutex<VecDeque<(Level, String)>> = Mutex::new(VecDeque::new());
const MAX_PENDING: usize = 500;

struct Tee {
    inner: env_logger::Logger,
}

impl log::Log for Tee {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.inner.enabled(record.metadata()) {
            return;
        }
        self.inner.log(record);
        // Our own crates from `info` up, everything else only from `warn`.
        // `orbit_editor::perf` (frame counter) stays out of the console:
        // drawing a log line must not itself cause another frame.
        let ours = record.target().starts_with("orbit") && !record.target().ends_with("::perf");
        let level = match record.level() {
            log::Level::Error => Level::Error,
            log::Level::Warn => Level::Warn,
            log::Level::Info if ours => Level::Info,
            _ => return,
        };
        if let Ok(mut pending) = PENDING.lock() {
            if pending.len() == MAX_PENDING {
                pending.pop_front();
            }
            pending.push_back((level, record.args().to_string()));
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// Installs the logger (`RUST_LOG` as usual, `info` by default). Safe to
/// call twice: the second call is ignored.
pub fn init() {
    let inner = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"),
    )
    .build();
    let max = inner.filter();
    if log::set_boxed_logger(Box::new(Tee { inner })).is_ok() {
        log::set_max_level(max);
    }
}

/// Records logged since the last call.
pub fn drain() -> Vec<(Level, String)> {
    PENDING
        .lock()
        .map(|mut pending| pending.drain(..).collect())
        .unwrap_or_default()
}
