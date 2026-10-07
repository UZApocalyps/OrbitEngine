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

/// stderr, written by a thread of its own: a console that blocks its
/// writer (a selection in a Windows console in QuickEdit mode, a slow
/// terminal, a full pipe) must not freeze the editor's UI thread.
struct AsyncStderr {
    tx: std::sync::mpsc::Sender<Vec<u8>>,
    buf: Vec<u8>,
}

impl AsyncStderr {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let _ = std::thread::Builder::new()
            .name("orbit-log".into())
            .spawn(move || {
                use std::io::Write;
                let mut err = std::io::stderr();
                while let Ok(line) = rx.recv() {
                    let _ = err.write_all(&line);
                }
            });
        Self { tx, buf: Vec::new() }
    }
}

impl std::io::Write for AsyncStderr {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            // A gone log thread only loses the line.
            let _ = self.tx.send(std::mem::take(&mut self.buf));
        }
        Ok(())
    }
}

/// Installs the logger (`RUST_LOG` as usual, `info` by default). Safe to
/// call twice: the second call is ignored.
pub fn init() {
    let inner = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"),
    )
    .target(env_logger::Target::Pipe(Box::new(AsyncStderr::new())))
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
