//! One background thread for the editor's own disk writes (user config,
//! crash recovery snapshot, graph layout sidecar, hitch log): the UI thread
//! never waits for a disk, an antivirus or an `fsync`.
//!
//! * A job has a key (usually the file it writes): a newer job replaces a
//!   queued one with the same key (latest wins), so a burst of edits costs
//!   one write. Jobs without a key (appends) are never merged.
//! * Jobs run in the order their key was first queued; a replaced job keeps
//!   its place.
//! * The writes themselves are atomic ([`crate::storage::write_atomic`]).
//! * [`flush`] waits until the queue is empty: before reading back what was
//!   written (recovery check, config load), at shutdown, in tests.
//!
//! Errors are logged (they reach the console panel).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct Queue {
    jobs: VecDeque<(Option<PathBuf>, Job)>,
    /// A job is running (taken off the queue).
    busy: bool,
    /// Jobs submitted since the start (tests: no write per keystroke).
    submitted: u64,
}

struct Writer {
    queue: Mutex<Queue>,
    /// Signalled when a job is queued and when the queue becomes idle.
    changed: Condvar,
}

fn writer() -> &'static Writer {
    static WRITER: OnceLock<&'static Writer> = OnceLock::new();
    WRITER.get_or_init(|| {
        let w: &'static Writer = Box::leak(Box::new(Writer {
            queue: Mutex::new(Queue::default()),
            changed: Condvar::new(),
        }));
        let spawned = std::thread::Builder::new()
            .name("orbit-writer".into())
            .spawn(move || run(w));
        if let Err(e) = spawned {
            log::error!("cannot start the background writer: {e}");
        }
        w
    })
}

fn run(w: &'static Writer) {
    loop {
        let job = {
            let mut q = w.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some((_, job)) = q.jobs.pop_front() {
                    q.busy = true;
                    break job;
                }
                q.busy = false;
                w.changed.notify_all();
                q = w.changed.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
            log::error!("a background write panicked");
        }
    }
}

/// Queues `job`, replacing a queued job with the same `key` (latest wins).
pub fn submit(key: Option<PathBuf>, job: impl FnOnce() + Send + 'static) {
    let w = writer();
    let mut q = w.queue.lock().unwrap_or_else(|e| e.into_inner());
    q.submitted += 1;
    let job: Job = Box::new(job);
    match key
        .as_ref()
        .and_then(|k| q.jobs.iter_mut().find(|(other, _)| other.as_ref() == Some(k)))
    {
        Some(slot) => slot.1 = job,
        None => q.jobs.push_back((key, job)),
    }
    w.changed.notify_all();
}

/// Writes `contents` to `path` atomically in the background (latest wins
/// per path). `what` names the file in the warning logged on failure.
pub fn write(path: PathBuf, contents: Vec<u8>, what: &'static str) {
    submit(Some(path.clone()), move || {
        if let Err(e) = crate::storage::write_atomic(&path, &contents) {
            log::warn!("cannot save {what} ({}): {e}", path.display());
        }
    });
}

/// Appends `text` to `path` in the background (never merged).
pub fn append(path: PathBuf, text: String) {
    submit(None, move || {
        use std::io::Write;
        let result = (|| {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
            file.write_all(text.as_bytes())
        })();
        if let Err(e) = result {
            log::warn!("cannot append to {}: {e}", path.display());
        }
    });
}

/// Waits until every queued job has run.
pub fn flush() {
    let w = writer();
    let mut q = w.queue.lock().unwrap_or_else(|e| e.into_inner());
    while q.busy || !q.jobs.is_empty() {
        q = w.changed.wait(q).unwrap_or_else(|e| e.into_inner());
    }
}

/// Whether jobs are queued or running (does not wait).
pub fn pending() -> bool {
    writer().queue.lock().map_or(false, |q| q.busy || !q.jobs.is_empty())
}

/// Jobs submitted since the start of the process.
pub fn submitted() -> u64 {
    writer().queue.lock().map_or(0, |q| q.submitted)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::storage::testing::TempDir;

    #[test]
    fn writes_land_after_flush_and_the_latest_wins() {
        let dir = TempDir::new("writer");
        let file = dir.path().join("a.txt");
        // Hold the worker on a slow job so the next ones queue up.
        let ran = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let g = gate.clone();
        let key = dir.path().join("gate");
        submit(Some(key), move || {
            let (m, c) = &*g;
            let mut open = m.lock().unwrap();
            while !*open {
                open = c.wait(open).unwrap();
            }
        });
        for i in 0..5 {
            let ran = ran.clone();
            let file = file.clone();
            submit(Some(file.clone()), move || {
                ran.fetch_add(1, Ordering::SeqCst);
                crate::storage::write_atomic(&file, format!("{i}").as_bytes()).unwrap();
            });
        }
        {
            let (m, c) = &*gate;
            *m.lock().unwrap() = true;
            c.notify_all();
        }
        flush();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "4");
        assert_eq!(ran.load(Ordering::SeqCst), 1, "queued writes of one file are merged");

        let log = dir.path().join("log.txt");
        append(log.clone(), "a\n".into());
        append(log.clone(), "b\n".into());
        flush();
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "a\nb\n");
    }
}
