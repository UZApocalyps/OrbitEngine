//! Background jobs of the editor: heavy work (decode, trace, bind, serialize,
//! write) runs on a small worker pool so the UI thread never stalls.
//!
//! * A job is a closure on **owned** inputs (clones, `Arc` snapshots). It
//!   reports progress and polls cancellation through its [`Progress`].
//! * Its result comes back through the [`TaskHandle`] the caller keeps; the
//!   caller polls it each frame and applies it on the UI thread as one
//!   undoable command. A handle made [`TaskHandle::with_base`] a document
//!   revision refuses a result whose document changed meanwhile
//!   ([`TaskError::Stale`]): the caller reports the conflict.
//! * [`Tasks`] lists what runs (status bar: label, progress, cancel) and which
//!   document each job holds, so conflicting actions can be refused.
//!
//! Cost when idle: none. The workers are started by the first job and then
//! sleep on a channel; nothing is polled while no job runs.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use crate::context::Waker;

type Job = Box<dyn FnOnce() + Send + 'static>;

/// The shared worker pool: at most 4 threads (one core left to the UI),
/// started lazily.
struct Pool {
    queue: Mutex<Sender<Job>>,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let n = std::thread::available_parallelism()
            .map_or(2, |n| n.get().saturating_sub(1))
            .clamp(1, 4);
        for i in 0..n {
            let rx = Arc::clone(&rx);
            let _ = std::thread::Builder::new()
                .name(format!("orbit-task-{i}"))
                .spawn(move || {
                    loop {
                        let job = match rx.lock() {
                            Ok(rx) => rx.recv(),
                            Err(_) => return,
                        };
                        match job {
                            // A panicking job never takes its worker down.
                            Ok(job) => drop(catch_unwind(AssertUnwindSafe(job))),
                            Err(_) => return,
                        }
                    }
                });
        }
        Pool { queue: Mutex::new(tx) }
    })
}

/// Why a job gave no result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskError {
    /// Cancelled by the user (or its handle was dropped).
    Cancelled,
    /// The job refused or failed: a message for the user.
    Failed(String),
    /// The job panicked (a bug): nothing was applied.
    Panicked,
    /// The document the job worked on changed meanwhile: its result was
    /// computed on an older state and is not applied.
    Stale,
}

const INDETERMINATE: u32 = u32::MAX;

struct Shared {
    id: u64,
    title: String,
    doc: Option<String>,
    /// `f32` bits of the fraction done, or [`INDETERMINATE`].
    fraction: AtomicU32,
    label: Mutex<String>,
    cancel: AtomicBool,
    done: AtomicBool,
    started: Instant,
}

/// What a running job sees: report progress, check cancellation.
#[derive(Clone)]
pub struct Progress {
    shared: Arc<Shared>,
    waker: Waker,
}

impl Progress {
    /// The fraction done (0..=1) and what is being done now.
    pub fn set(&self, fraction: f32, label: impl Into<String>) {
        self.set_fraction(fraction);
        if let Ok(mut l) = self.shared.label.lock() {
            *l = label.into();
        }
    }

    pub fn set_fraction(&self, fraction: f32) {
        let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
        self.shared.fraction.store(f.to_bits(), Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.shared.cancel.load(Ordering::Relaxed)
    }

    /// `Err(TaskError::Cancelled)` once cancelled: `progress.check()?` at the
    /// job's safe points.
    pub fn check(&self) -> Result<(), TaskError> {
        if self.is_cancelled() { Err(TaskError::Cancelled) } else { Ok(()) }
    }

    /// Asks the host for a frame (after a progress step worth showing).
    pub fn wake(&self) {
        self.waker.wake();
    }
}

/// A running job as the status bar shows it.
#[derive(Clone, Debug)]
pub struct TaskInfo {
    pub id: u64,
    pub title: String,
    pub label: String,
    /// `None`: indeterminate (a spinner).
    pub fraction: Option<f32>,
    pub doc: Option<String>,
    pub cancelling: bool,
    pub elapsed: std::time::Duration,
}

/// The result of a job, kept by whoever started it.
pub struct TaskHandle<T> {
    shared: Arc<Shared>,
    rx: Receiver<Result<T, TaskError>>,
    base: Option<u64>,
    finished: bool,
}

impl<T> TaskHandle<T> {
    /// Remembers the revision of the document the job's inputs were taken
    /// from: [`TaskHandle::poll_fresh`] refuses a result once it moved.
    pub fn with_base(mut self, revision: u64) -> Self {
        self.base = Some(revision);
        self
    }

    pub fn id(&self) -> u64 {
        self.shared.id
    }

    pub fn title(&self) -> &str {
        &self.shared.title
    }

    /// The outcome, once (later calls: `None`). Never blocks.
    pub fn poll(&mut self) -> Option<Result<T, TaskError>> {
        if self.finished {
            return None;
        }
        let r = match self.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(TaskError::Panicked),
        };
        self.finished = true;
        Some(if self.shared.cancel.load(Ordering::Relaxed) && r.is_ok() { Err(TaskError::Cancelled) } else { r })
    }

    /// [`TaskHandle::poll`], refusing a result computed on another revision
    /// than `current` ([`TaskError::Stale`]).
    pub fn poll_fresh(&mut self, current: u64) -> Option<Result<T, TaskError>> {
        let r = self.poll()?;
        Some(match (r, self.base) {
            (Ok(_), Some(base)) if base != current => Err(TaskError::Stale),
            (r, _) => r,
        })
    }

    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.finished || self.shared.done.load(Ordering::Acquire)
    }

    /// Fraction done, `None` while indeterminate.
    pub fn fraction(&self) -> Option<f32> {
        fraction_of(&self.shared)
    }

    pub fn label(&self) -> String {
        self.shared.label.lock().map(|l| l.clone()).unwrap_or_default()
    }

    /// Waits for the outcome (tests, shutdown). Never call on the UI thread
    /// during normal work.
    pub fn wait(mut self) -> Result<T, TaskError> {
        if self.finished {
            return Err(TaskError::Cancelled);
        }
        self.finished = true;
        self.rx.recv().unwrap_or(Err(TaskError::Panicked))
    }
}

impl<T> Drop for TaskHandle<T> {
    /// Nobody wants the result any more: the job is asked to stop.
    fn drop(&mut self) {
        if !self.finished {
            self.cancel();
        }
    }
}

fn fraction_of(s: &Shared) -> Option<f32> {
    let bits = s.fraction.load(Ordering::Relaxed);
    (bits != INDETERMINATE).then(|| f32::from_bits(bits))
}

/// The jobs started by the editor and its plugins (in `EditorState`).
#[derive(Default)]
pub struct Tasks {
    live: Mutex<Vec<Arc<Shared>>>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl Tasks {
    /// Runs `job` on the worker pool. `doc` names the document it works on
    /// (see [`Tasks::busy`]); `waker` is woken when it ends. The job owns its
    /// inputs; its result comes back through the handle.
    pub fn spawn<T, F>(&self, waker: &Waker, title: impl Into<String>, doc: Option<&str>, job: F) -> TaskHandle<T>
    where
        T: Send + 'static,
        F: FnOnce(&Progress) -> Result<T, TaskError> + Send + 'static,
    {
        self.spawn_inner(waker, title.into(), doc, true, job)
    }

    /// Like [`Tasks::spawn`] for internal refreshes (a panel's cache, see
    /// `crate::lazy`): not listed in the status bar, holds no document.
    pub fn spawn_quiet<T, F>(&self, waker: &Waker, job: F) -> TaskHandle<T>
    where
        T: Send + 'static,
        F: FnOnce(&Progress) -> Result<T, TaskError> + Send + 'static,
    {
        self.spawn_inner(waker, String::new(), None, false, job)
    }

    fn spawn_inner<T, F>(&self, waker: &Waker, title: String, doc: Option<&str>, listed: bool, job: F) -> TaskHandle<T>
    where
        T: Send + 'static,
        F: FnOnce(&Progress) -> Result<T, TaskError> + Send + 'static,
    {
        let shared = Arc::new(Shared {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            title,
            doc: doc.map(str::to_owned),
            fraction: AtomicU32::new(INDETERMINATE),
            label: Mutex::new(String::new()),
            cancel: AtomicBool::new(false),
            done: AtomicBool::new(false),
            started: Instant::now(),
        });
        if listed && let Ok(mut live) = self.live.lock() {
            live.retain(|s| !s.done.load(Ordering::Acquire));
            live.push(Arc::clone(&shared));
        }
        let (tx, rx) = mpsc::channel();
        let progress = Progress { shared: Arc::clone(&shared), waker: waker.clone() };
        let run: Job = Box::new(move || {
            // Marks the job done even when it panics (the sender is dropped,
            // the handle reads `Panicked`).
            struct Done(Progress);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.shared.done.store(true, Ordering::Release);
                    self.0.waker.wake();
                }
            }
            let done = Done(progress);
            let r = if done.0.is_cancelled() { Err(TaskError::Cancelled) } else { job(&done.0) };
            done.0.shared.done.store(true, Ordering::Release);
            let _ = tx.send(r);
            drop(done);
        });
        let sent = pool().queue.lock().map(|q| q.send(run));
        if !matches!(sent, Ok(Ok(()))) {
            shared.done.store(true, Ordering::Release);
        }
        TaskHandle { shared, rx, base: None, finished: false }
    }

    /// The jobs still running, oldest first.
    pub fn running(&self) -> Vec<TaskInfo> {
        let Ok(mut live) = self.live.lock() else { return Vec::new() };
        live.retain(|s| !s.done.load(Ordering::Acquire));
        live.iter()
            .map(|s| TaskInfo {
                id: s.id,
                title: s.title.clone(),
                label: s.label.lock().map(|l| l.clone()).unwrap_or_default(),
                fraction: fraction_of(s),
                doc: s.doc.clone(),
                cancelling: s.cancel.load(Ordering::Relaxed),
                elapsed: s.started.elapsed(),
            })
            .collect()
    }

    /// Whether any job runs (the host repaints for the progress only then).
    pub fn any(&self) -> bool {
        self.live.lock().is_ok_and(|l| l.iter().any(|s| !s.done.load(Ordering::Acquire)))
    }

    /// Whether a job holds the document `doc`: actions that would edit it
    /// wait (they are disabled or refused).
    pub fn busy(&self, doc: &str) -> bool {
        self.live
            .lock()
            .is_ok_and(|l| l.iter().any(|s| !s.done.load(Ordering::Acquire) && s.doc.as_deref() == Some(doc)))
    }

    /// Asks the job `id` to stop (its result, if any, is then dropped).
    pub fn cancel(&self, id: u64) {
        if let Ok(live) = self.live.lock() {
            for s in live.iter().filter(|s| s.id == id) {
                s.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::time::Duration;

    use super::*;

    fn wait_for<T>(h: &mut TaskHandle<T>) -> Result<T, TaskError> {
        let start = Instant::now();
        loop {
            if let Some(r) = h.poll() {
                return r;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "job never ended");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn result_comes_back_once_and_wakes_the_host() {
        let tasks = Tasks::default();
        let waker = Waker::default();
        let woken = Arc::new(AtomicU32::new(0));
        let w = Arc::clone(&woken);
        waker.set(move || {
            w.fetch_add(1, Ordering::Relaxed);
        });
        let mut h = tasks.spawn(&waker, "sum", Some("doc"), |_| Ok((1..=10).sum::<u32>()));
        assert_eq!(wait_for(&mut h), Ok(55));
        assert!(h.poll().is_none(), "a result is delivered once");
        let t = Instant::now();
        while woken.load(Ordering::Relaxed) == 0 {
            assert!(t.elapsed() < Duration::from_secs(10), "never woken");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!tasks.busy("doc"));
        assert!(tasks.running().is_empty());
    }

    #[test]
    fn progress_and_busy_are_visible_while_running() {
        let tasks = Tasks::default();
        let gate = Arc::new(Barrier::new(2));
        let g = Arc::clone(&gate);
        let mut h = tasks.spawn(&Waker::default(), "trace", Some("rig:a"), move |p| {
            p.set(0.5, "half");
            g.wait(); // reported
            g.wait(); // checked
            Ok(())
        });
        gate.wait();
        assert!(tasks.busy("rig:a"));
        assert!(!tasks.busy("rig:b"));
        let info = tasks.running();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].fraction, Some(0.5));
        assert_eq!(info[0].label, "half");
        assert_eq!(h.fraction(), Some(0.5));
        gate.wait();
        assert_eq!(wait_for(&mut h), Ok(()));
    }

    #[test]
    fn cancel_stops_the_job_and_drops_its_result() {
        let tasks = Tasks::default();
        let mut h = tasks.spawn(&Waker::default(), "loop", None, |p| {
            for _ in 0..10_000 {
                p.check()?;
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(1)
        });
        tasks.cancel(h.id());
        assert_eq!(wait_for(&mut h), Err(TaskError::Cancelled));
    }

    #[test]
    fn stale_result_is_refused() {
        let tasks = Tasks::default();
        let mut h = tasks.spawn(&Waker::default(), "x", None, |_| Ok(3)).with_base(7);
        let start = Instant::now();
        let r = loop {
            if let Some(r) = h.poll_fresh(8) {
                break r;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(r, Err(TaskError::Stale));
        let mut fresh = tasks.spawn(&Waker::default(), "x", None, |_| Ok(3)).with_base(7);
        let r = loop {
            if let Some(r) = fresh.poll_fresh(7) {
                break r;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(r, Ok(3));
    }

    #[test]
    fn a_panicking_job_reports_and_the_pool_survives() {
        let tasks = Tasks::default();
        let mut h = tasks.spawn(&Waker::default(), "boom", None, |_| -> Result<(), TaskError> { panic!("boom") });
        assert_eq!(wait_for(&mut h), Err(TaskError::Panicked));
        let mut ok = tasks.spawn(&Waker::default(), "after", None, |_| Ok(1));
        assert_eq!(wait_for(&mut ok), Ok(1));
    }

    #[test]
    fn dropping_the_handle_cancels() {
        let tasks = Tasks::default();
        let seen = Arc::new(AtomicBool::new(false));
        let s = Arc::clone(&seen);
        let started = Arc::new(Barrier::new(2));
        let st = Arc::clone(&started);
        let h = tasks.spawn(&Waker::default(), "x", None, move |p| {
            st.wait();
            while !p.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            s.store(true, Ordering::Relaxed);
            Ok(())
        });
        started.wait();
        drop(h);
        let t = Instant::now();
        while !seen.load(Ordering::Relaxed) {
            assert!(t.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
