//! "Never compute what is not visible" (ADR 0009, addendum): which panels
//! are on screen, and caches that only recompute for a visible panel whose
//! inputs changed.
//!
//! * [`PanelVisibility`] is fed by the dock: a panel is *visible* when its
//!   tab was drawn this frame or the previous one (egui_dock only draws the
//!   active tab of each group). The dock calls the panel's `on_shown` hook
//!   on the first frame it is drawn again, and `on_hidden` once it was not
//!   drawn for a whole frame.
//! * [`Lazy`] keeps a value computed for a key (project generation,
//!   revision, asset epoch, ...: whatever the value depends on) and
//!   recomputes it only when asked for a *visible* use with another key.
//!   A hidden panel keeps its old value and costs nothing.
//! * [`LazyAsync`] is the same for work too heavy for a frame: the value is
//!   computed on the worker pool from a snapshot of its inputs; the last
//!   value stays shown while a newer one is computed, results for a key that
//!   is no longer wanted are dropped, and a burst of changes (typing) starts
//!   at most one job per `min_interval`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::context::Waker;
use crate::tasks::{Progress, TaskError, TaskHandle, Tasks};

/// What most project-derived caches depend on: which project is open, which
/// version of its story, which version of its asset folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DocKey {
    pub generation: u64,
    pub revision: u64,
    pub asset_epoch: u64,
}

/// Which panels are on screen (see the module documentation).
#[derive(Debug, Default)]
pub struct PanelVisibility {
    frame: u64,
    /// Panel id -> last frame it was drawn.
    drawn: HashMap<String, u64>,
}

impl PanelVisibility {
    /// Starts a new frame (the host calls it before the dock is drawn).
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Records that `id` is drawn this frame. Returns `true` when it was not
    /// drawn the previous frame: it was just shown.
    pub fn mark_drawn(&mut self, id: &str) -> bool {
        let frame = self.frame;
        match self.drawn.get_mut(id) {
            Some(last) => {
                let shown = *last + 1 < frame;
                *last = frame;
                shown
            }
            None => {
                self.drawn.insert(id.to_owned(), frame);
                true
            }
        }
    }

    /// Whether `id` was drawn this frame or the previous one.
    pub fn is_visible(&self, id: &str) -> bool {
        self.drawn.get(id).is_some_and(|last| last + 1 >= self.frame)
    }

    /// Panels drawn the previous frame but not this one (call at the end of
    /// a frame): they were just hidden. They are reported once.
    pub fn take_hidden(&mut self) -> Vec<String> {
        let frame = self.frame;
        let hidden: Vec<String> = self
            .drawn
            .iter()
            .filter(|(_, last)| **last + 1 == frame)
            .map(|(id, _)| id.clone())
            .collect();
        // Marked as "long gone" so they are not reported again.
        for id in &hidden {
            if let Some(last) = self.drawn.get_mut(id) {
                *last = 0;
            }
        }
        hidden
    }
}

/// A value computed for a key, recomputed only for a visible use with
/// another key (see the module documentation).
#[derive(Debug)]
pub struct Lazy<K, V> {
    key: Option<K>,
    value: Option<V>,
}

impl<K, V> Default for Lazy<K, V> {
    fn default() -> Self {
        Self { key: None, value: None }
    }
}

impl<K: PartialEq, V> Lazy<K, V> {
    /// The value for `key`: computed when missing or stale and `visible`;
    /// otherwise the last value (possibly stale, possibly none).
    pub fn get(&mut self, visible: bool, key: K, compute: impl FnOnce() -> V) -> Option<&V> {
        if visible && self.key.as_ref() != Some(&key) {
            self.value = Some(compute());
            self.key = Some(key);
        }
        self.value.as_ref()
    }

    /// Whether the value is missing or was computed for another key.
    pub fn is_stale(&self, key: &K) -> bool {
        self.key.as_ref() != Some(key)
    }

    /// The last value, whatever its key.
    pub fn value(&self) -> Option<&V> {
        self.value.as_ref()
    }

    /// Forces the next visible use to recompute.
    pub fn invalidate(&mut self) {
        self.key = None;
    }

    /// Drops the value (memory) and forces a recompute.
    pub fn clear(&mut self) {
        self.key = None;
        self.value = None;
    }
}

type BoxedJob<V> = Box<dyn FnOnce(&Progress) -> Result<V, TaskError> + Send>;

/// [`Lazy`] computed off the UI thread (see the module documentation).
pub struct LazyAsync<K, V> {
    key: Option<K>,
    value: Option<V>,
    running: Option<(K, TaskHandle<V>)>,
    last_start: Option<Instant>,
    min_interval: Duration,
}

impl<K, V> Default for LazyAsync<K, V> {
    fn default() -> Self {
        Self::new(Duration::from_millis(250))
    }
}

impl<K, V> LazyAsync<K, V> {
    /// A cache that starts at most one job per `min_interval` while its key
    /// keeps changing.
    pub fn new(min_interval: Duration) -> Self {
        Self {
            key: None,
            value: None,
            running: None,
            last_start: None,
            min_interval,
        }
    }
}

/// What [`LazyAsync::poll`] says about the value it returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Computed for the asked key.
    Fresh,
    /// An older value (or none yet); a newer one is being computed or will
    /// be once the burst of changes settles: ask again later.
    Pending,
    /// An older value; nothing is computed because the use is not visible.
    Stale,
}

impl<K: PartialEq + Clone + Send + 'static, V: Send + 'static> LazyAsync<K, V> {
    /// Collects a finished job, then, for a `visible` use whose `key`
    /// differs from the value's, starts `prepare()` (called on the UI
    /// thread to snapshot the inputs; it returns the job). Returns how fresh
    /// [`LazyAsync::value`] is (the value to show). `waker` is woken when the job ends.
    pub fn poll<P>(
        &mut self,
        visible: bool,
        key: K,
        tasks: &Tasks,
        waker: &Waker,
        prepare: P,
    ) -> Freshness
    where
        P: FnOnce() -> BoxedJob<V>,
    {
        self.collect();
        if self.key.as_ref() == Some(&key) {
            return Freshness::Fresh;
        }
        if !visible {
            return Freshness::Stale;
        }
        let running_for = self.running.as_ref().map(|(k, _)| k);
        if running_for != Some(&key) {
            let throttled = self
                .last_start
                .is_some_and(|t| t.elapsed() < self.min_interval);
            // A job for an older key keeps running until the burst settles:
            // its result is better than nothing.
            if self.running.is_none() || !throttled {
                let job = prepare();
                let handle = tasks.spawn_quiet(waker, move |p| job(p));
                self.running = Some((key, handle));
                self.last_start = Some(Instant::now());
            }
        }
        Freshness::Pending
    }

    /// When a throttled change can start its job: the host should draw a
    /// frame then (see [`LazyAsync::poll`]).
    pub fn retry_after(&self) -> Option<Duration> {
        self.last_start
            .map(|t| self.min_interval.saturating_sub(t.elapsed()))
    }

    fn collect(&mut self) {
        let Some((_, handle)) = self.running.as_mut() else {
            return;
        };
        match handle.poll() {
            None => {}
            Some(result) => {
                let (key, _) = self.running.take().expect("running job");
                match result {
                    Ok(value) => {
                        self.value = Some(value);
                        self.key = Some(key);
                    }
                    // Cancelled or failed: the last value stays.
                    Err(_) => {}
                }
            }
        }
    }

    /// The last value, whatever its key.
    pub fn value(&self) -> Option<&V> {
        self.value.as_ref()
    }

    /// Whether a job runs.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Forces the next visible use to recompute (the value stays shown).
    pub fn invalidate(&mut self) {
        self.key = None;
        self.last_start = None;
    }

    /// Drops the value and any running job.
    pub fn clear(&mut self) {
        self.key = None;
        self.value = None;
        self.running = None;
        self.last_start = None;
    }

    /// Waits for the running job, if any (tests).
    pub fn wait(&mut self) {
        if let Some((key, handle)) = self.running.take() {
            if let Ok(value) = handle.wait() {
                self.value = Some(value);
                self.key = Some(key);
            }
        }
    }
}

/// Boxes a job for [`LazyAsync::poll`].
pub fn job<V, F>(f: F) -> BoxedJob<V>
where
    F: FnOnce(&Progress) -> Result<V, TaskError> + Send + 'static,
{
    Box::new(f)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn visibility_follows_the_frames_a_panel_is_drawn() {
        let mut v = PanelVisibility::default();
        v.begin_frame();
        assert!(v.mark_drawn("a"), "first draw: shown");
        assert!(v.is_visible("a"));
        assert!(!v.is_visible("b"));
        v.begin_frame();
        assert!(!v.mark_drawn("a"), "drawn again: not newly shown");
        assert!(v.take_hidden().is_empty());
        // Not drawn this frame: still visible (drawn the previous one),
        // then reported hidden once.
        v.begin_frame();
        assert!(v.is_visible("a"));
        assert_eq!(v.take_hidden(), ["a"]);
        v.begin_frame();
        assert!(!v.is_visible("a"));
        assert!(v.take_hidden().is_empty());
        v.begin_frame();
        assert!(v.mark_drawn("a"), "drawn after a gap: shown again");
    }

    #[test]
    fn lazy_recomputes_only_when_visible_and_stale() {
        let calls = Cell::new(0);
        let mut lazy: Lazy<u64, u64> = Lazy::default();
        let get = |lazy: &mut Lazy<u64, u64>, visible, key| {
            lazy.get(visible, key, || {
                calls.set(calls.get() + 1);
                key * 10
            })
            .copied()
        };
        assert_eq!(get(&mut lazy, false, 1), None, "hidden: nothing computed");
        assert_eq!(calls.get(), 0);
        assert_eq!(get(&mut lazy, true, 1), Some(10));
        assert_eq!(get(&mut lazy, true, 1), Some(10));
        assert_eq!(calls.get(), 1, "same key: cached");
        assert_eq!(get(&mut lazy, false, 2), Some(10), "hidden: stale value kept");
        assert_eq!(calls.get(), 1);
        assert!(lazy.is_stale(&2));
        assert_eq!(get(&mut lazy, true, 2), Some(20));
        assert_eq!(calls.get(), 2);
        lazy.invalidate();
        assert_eq!(get(&mut lazy, true, 2), Some(20));
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn lazy_async_computes_off_thread_and_keeps_the_last_value() {
        let tasks = Tasks::default();
        let waker = Waker::default();
        let runs = Arc::new(AtomicUsize::new(0));
        let mut cache: LazyAsync<u64, u64> = LazyAsync::new(Duration::ZERO);
        let prepare = |key: u64, runs: Arc<AtomicUsize>| {
            move || {
                job(move |_| {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Ok(key * 2)
                })
            }
        };
        let f = cache.poll(false, 1, &tasks, &waker, prepare(1, runs.clone()));
        let v = cache.value();
        assert_eq!((v.copied(), f), (None, Freshness::Stale));
        let f = cache.poll(true, 1, &tasks, &waker, prepare(1, runs.clone()));
        let v = cache.value();
        assert_eq!((v.copied(), f), (None, Freshness::Pending));
        cache.wait();
        let f = cache.poll(true, 1, &tasks, &waker, prepare(1, runs.clone()));
        let v = cache.value();
        assert_eq!((v.copied(), f), (Some(2), Freshness::Fresh));
        // A new key: the old value stays while the new one is computed.
        let f = cache.poll(true, 5, &tasks, &waker, prepare(5, runs.clone()));
        let v = cache.value();
        assert_eq!((v.copied(), f), (Some(2), Freshness::Pending));
        cache.wait();
        cache.poll(true, 5, &tasks, &waker, prepare(5, runs.clone()));
        let v = cache.value();
        assert_eq!(v.copied(), Some(10));
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        // Quiet jobs are not listed in the status bar.
        assert!(tasks.running().is_empty());
    }

    #[test]
    fn lazy_async_throttles_a_burst_of_changes() {
        let tasks = Tasks::default();
        let waker = Waker::default();
        let runs = Arc::new(AtomicUsize::new(0));
        let mut cache: LazyAsync<u64, u64> = LazyAsync::new(Duration::from_secs(60));
        for key in 0..20u64 {
            let runs = runs.clone();
            cache.poll(true, key, &tasks, &waker, move || {
                job(move |_| {
                    runs.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(5));
                    Ok(key)
                })
            });
        }
        cache.wait();
        assert_eq!(runs.load(Ordering::SeqCst), 1, "one job for the whole burst");
        assert_eq!(cache.value(), Some(&0));
        assert!(cache.retry_after().is_some());
    }
}
