//! Lightweight profiler for the runtime and the editor (TL-04).
//!
//! * [`scope!`] opens a named CPU span that ends with the enclosing block;
//!   spans nest (their depth is kept) and may be opened on any thread.
//! * [`finish_frame`] closes the current frame: its spans and total time go
//!   into a ring of the last [`HISTORY`] frames.
//! * [`report_gpu`] attaches GPU pass durations (wgpu timestamp queries,
//!   see `orbit_gfx`) to the newest frame; they arrive a frame or two late.
//! * [`frames`], [`top_scopes`] and [`gpu_passes`] summarize the history for
//!   the editor panel and the player overlay.
//!
//! The profiler is compiled in only when [`COMPILED`] is true: native debug
//! builds, or any native build with the `enabled` feature (the editor, the
//! player built with `--features profiling`). Otherwise every function is an
//! empty `#[inline]` stub and [`Scope`] is a zero-sized type, so release
//! players and the web pay nothing. When compiled in, recording can still be
//! paused at run time with [`set_enabled`] (one atomic load per span).

/// Whether the profiler is compiled in (see the crate documentation).
pub const COMPILED: bool = cfg!(all(
    not(target_arch = "wasm32"),
    any(feature = "enabled", debug_assertions)
));

/// Frames kept in the history.
pub const HISTORY: usize = 240;

/// Most spans recorded per frame (the rest are dropped, the frame total is
/// still right).
pub const MAX_SCOPES_PER_FRAME: usize = 4096;

/// One span of a frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeRecord {
    pub name: &'static str,
    /// Nesting depth on its thread (0 = outermost).
    pub depth: u16,
    /// Start, in milliseconds since the frame started.
    pub start_ms: f64,
    pub duration_ms: f64,
    /// Recorded on the thread that drives the frames (the one calling
    /// [`begin_frame`]); `false` for worker threads.
    pub frame_thread: bool,
}

/// One finished frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameRecord {
    /// Frame number since the start (or the last [`clear`]).
    pub index: u64,
    /// Wall time of the frame: since [`begin_frame`], or since the previous
    /// [`finish_frame`] when the host does not call `begin_frame`.
    pub cpu_ms: f64,
    pub scopes: Vec<ScopeRecord>,
    /// GPU pass durations, when the device supports timestamp queries.
    pub gpu: Vec<(&'static str, f64)>,
}

impl FrameRecord {
    /// Total of the GPU passes, if any were measured.
    pub fn gpu_ms(&self) -> Option<f64> {
        (!self.gpu.is_empty()).then(|| self.gpu.iter().map(|(_, ms)| ms).sum())
    }
}

/// Aggregated statistics of one span name over several frames.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeStat {
    pub name: &'static str,
    /// Calls per frame, on average.
    pub calls: f64,
    /// Time per frame, on average (all calls added up).
    pub mean_ms: f64,
    /// Worst frame (all calls of that frame added up).
    pub max_ms: f64,
}

/// Aggregates `frames` per span name, the most expensive first (by mean).
/// Only outermost occurrences of a name count, so recursion is not counted
/// twice.
pub fn aggregate(frames: &[FrameRecord]) -> Vec<ScopeStat> {
    use std::collections::HashMap;
    if frames.is_empty() {
        return Vec::new();
    }
    let mut stats: HashMap<&'static str, (f64, f64, usize)> = HashMap::new();
    for frame in frames {
        let mut per_frame: HashMap<&'static str, (f64, usize)> = HashMap::new();
        for s in &frame.scopes {
            let e = per_frame.entry(s.name).or_default();
            e.0 += s.duration_ms;
            e.1 += 1;
        }
        for (name, (ms, calls)) in per_frame {
            let e = stats.entry(name).or_default();
            e.0 += ms;
            e.1 = e.1.max(ms);
            e.2 += calls;
        }
    }
    let n = frames.len() as f64;
    let mut out: Vec<ScopeStat> = stats
        .into_iter()
        .map(|(name, (total, max, calls))| ScopeStat {
            name,
            calls: calls as f64 / n,
            mean_ms: total / n,
            max_ms: max,
        })
        .collect();
    out.sort_by(|a, b| b.mean_ms.total_cmp(&a.mean_ms).then(a.name.cmp(b.name)));
    out
}

/// Average GPU time per pass over `frames` (only frames that have GPU
/// timings count), in first-seen order.
pub fn aggregate_gpu(frames: &[FrameRecord]) -> Vec<(&'static str, f64)> {
    let mut out: Vec<(&'static str, f64, usize)> = Vec::new();
    for frame in frames {
        for &(name, ms) in &frame.gpu {
            match out.iter_mut().find(|(n, ..)| *n == name) {
                Some(e) => {
                    e.1 += ms;
                    e.2 += 1;
                }
                None => out.push((name, ms, 1)),
            }
        }
    }
    out.into_iter()
        .map(|(name, total, count)| (name, total / count as f64))
        .collect()
}

/// Spans whose name starts with this are waits on the GPU or the driver
/// (surface acquire, queue submit, buffer mapping, present): a hitch lists
/// them separately.
pub const WAIT_PREFIX: &str = "gpu wait:";

/// Hitches kept (see [`hitches`]).
pub const MAX_HITCHES: usize = 20;

/// Default hitch threshold, in milliseconds.
pub const DEFAULT_HITCH_MS: f64 = 100.0;

/// A frame that took longer than the hitch threshold, with what it spent
/// its time on (spans of the frame thread only).
#[derive(Debug, Clone, PartialEq)]
pub struct Hitch {
    /// Frame number (as in [`FrameRecord::index`]).
    pub frame: u64,
    /// Wall time of the frame.
    pub cpu_ms: f64,
    /// Unix time of the end of the frame, milliseconds.
    pub at_unix_ms: u64,
    /// Most expensive spans: (name, smallest depth, all calls added up),
    /// the longest first.
    pub top: Vec<(&'static str, u16, f64)>,
    /// Spans named [`WAIT_PREFIX`]`...`, added up per name.
    pub waits: Vec<(&'static str, f64)>,
}

impl Hitch {
    /// Builds the hitch record of `frame` (whatever its duration).
    pub fn of(frame: &FrameRecord, at_unix_ms: u64) -> Self {
        let mut top: Vec<(&'static str, u16, f64)> = Vec::new();
        let mut waits: Vec<(&'static str, f64)> = Vec::new();
        for s in frame.scopes.iter().filter(|s| s.frame_thread) {
            if s.name.starts_with(WAIT_PREFIX) {
                match waits.iter_mut().find(|(n, _)| *n == s.name) {
                    Some(w) => w.1 += s.duration_ms,
                    None => waits.push((s.name, s.duration_ms)),
                }
            }
            match top.iter_mut().find(|(n, ..)| *n == s.name) {
                Some(t) => {
                    t.1 = t.1.min(s.depth);
                    t.2 += s.duration_ms;
                }
                None => top.push((s.name, s.depth, s.duration_ms)),
            }
        }
        top.sort_by(|a, b| b.2.total_cmp(&a.2));
        top.truncate(10);
        waits.sort_by(|a, b| b.1.total_cmp(&a.1));
        Self {
            frame: frame.index,
            cpu_ms: frame.cpu_ms,
            at_unix_ms,
            top,
            waits,
        }
    }

    /// Time of the frame not covered by any outermost span (work that has
    /// no span yet).
    pub fn unaccounted_ms(&self) -> f64 {
        let covered: f64 = self.top.iter().filter(|t| t.1 == 0).map(|t| t.2).sum();
        (self.cpu_ms - covered).max(0.0)
    }

    /// One line: `hitch: frame 12 took 312.4 ms; spans: editor ui 290.1, ...`
    /// (nested spans are prefixed with one `>` per level).
    pub fn summary(&self) -> String {
        let spans: Vec<String> = self
            .top
            .iter()
            .take(8)
            .map(|(n, d, ms)| format!("{}{n} {ms:.1}", ">".repeat((*d).min(4) as usize)))
            .collect();
        let mut line = format!(
            "hitch: frame {} took {:.1} ms; spans: {}",
            self.frame,
            self.cpu_ms,
            if spans.is_empty() { "none".to_owned() } else { spans.join(", ") }
        );
        if !self.waits.is_empty() {
            let waits: Vec<String> = self
                .waits
                .iter()
                .map(|(n, ms)| format!("{} {ms:.1}", n.trim_start_matches(WAIT_PREFIX).trim()))
                .collect();
            line.push_str(&format!("; gpu waits: {}", waits.join(", ")));
        }
        line.push_str(&format!("; unaccounted {:.1} ms", self.unaccounted_ms()));
        line
    }
}

/// Opens a span named `$name` (a `&'static str`) until the end of the
/// enclosing block. Free when the profiler is not compiled in.
#[macro_export]
macro_rules! scope {
    ($name:expr) => {
        let _orbit_profile_scope = $crate::Scope::new($name);
    };
}

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "enabled", debug_assertions)
))]
mod imp {
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, MutexGuard};
    use std::time::Instant;

    use std::sync::atomic::AtomicU64;

    use super::{FrameRecord, HISTORY, Hitch, MAX_HITCHES, MAX_SCOPES_PER_FRAME, ScopeRecord};

    static ENABLED: AtomicBool = AtomicBool::new(true);
    /// Hitch threshold, `f64` bits (100.0).
    static HITCH_MS: AtomicU64 = AtomicU64::new(0x4059_0000_0000_0000);

    struct State {
        frame_start: Option<Instant>,
        next_index: u64,
        current: Vec<ScopeRecord>,
        frames: VecDeque<FrameRecord>,
        hitches: VecDeque<Hitch>,
        /// Hitches not yet returned by `take_new_hitches`.
        new_hitches: usize,
    }

    static STATE: Mutex<State> = Mutex::new(State {
        frame_start: None,
        next_index: 0,
        current: Vec::new(),
        frames: VecDeque::new(),
        hitches: VecDeque::new(),
        new_hitches: 0,
    });

    thread_local! {
        static DEPTH: Cell<u16> = const { Cell::new(0) };
        static FRAME_THREAD: Cell<bool> = const { Cell::new(false) };
    }

    fn state() -> MutexGuard<'static, State> {
        STATE.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_enabled(on: bool) {
        ENABLED.store(on, Ordering::Relaxed);
    }

    pub fn is_enabled() -> bool {
        ENABLED.load(Ordering::Relaxed)
    }

    /// A running span; records itself when dropped.
    #[must_use = "a span ends when this guard is dropped"]
    pub struct Scope {
        name: &'static str,
        start: Option<Instant>,
        depth: u16,
    }

    impl Scope {
        #[inline]
        pub fn new(name: &'static str) -> Self {
            if !is_enabled() {
                return Self {
                    name,
                    start: None,
                    depth: 0,
                };
            }
            let depth = DEPTH.with(|d| {
                let v = d.get();
                d.set(v.saturating_add(1));
                v
            });
            Self {
                name,
                start: Some(Instant::now()),
                depth,
            }
        }
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            let Some(start) = self.start else {
                return;
            };
            let end = Instant::now();
            DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
            let mut s = state();
            if s.current.len() >= MAX_SCOPES_PER_FRAME {
                return;
            }
            let frame_start = *s.frame_start.get_or_insert(start);
            let start_ms = start
                .checked_duration_since(frame_start)
                .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
            s.current.push(ScopeRecord {
                name: self.name,
                depth: self.depth,
                start_ms,
                duration_ms: (end - start).as_secs_f64() * 1000.0,
                frame_thread: FRAME_THREAD.with(Cell::get),
            });
        }
    }

    pub fn begin_frame() {
        FRAME_THREAD.with(|f| f.set(true));
        if is_enabled() {
            state().frame_start = Some(Instant::now());
        }
    }

    pub fn set_hitch_threshold_ms(ms: f64) {
        HITCH_MS.store(ms.to_bits(), Ordering::Relaxed);
    }

    pub fn hitch_threshold_ms() -> f64 {
        f64::from_bits(HITCH_MS.load(Ordering::Relaxed))
    }

    pub fn hitches() -> Vec<Hitch> {
        state().hitches.iter().cloned().collect()
    }

    pub fn take_new_hitches() -> Vec<Hitch> {
        let mut s = state();
        let n = std::mem::take(&mut s.new_hitches).min(s.hitches.len());
        let skip = s.hitches.len() - n;
        s.hitches.iter().skip(skip).cloned().collect()
    }

    pub fn finish_frame() {
        FRAME_THREAD.with(|f| f.set(true));
        if !is_enabled() {
            return;
        }
        let now = Instant::now();
        let mut s = state();
        let cpu_ms = s
            .frame_start
            .map_or(0.0, |t| (now - t).as_secs_f64() * 1000.0);
        let index = s.next_index;
        s.next_index += 1;
        let mut scopes = std::mem::take(&mut s.current);
        scopes.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
        if s.frames.len() >= HISTORY {
            s.frames.pop_front();
        }
        let frame = FrameRecord {
            index,
            cpu_ms,
            scopes,
            gpu: Vec::new(),
        };
        if cpu_ms > hitch_threshold_ms() {
            let at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64);
            if s.hitches.len() >= MAX_HITCHES {
                s.hitches.pop_front();
            }
            s.hitches.push_back(Hitch::of(&frame, at));
            s.new_hitches = (s.new_hitches + 1).min(MAX_HITCHES);
        }
        s.frames.push_back(frame);
        s.frame_start = Some(now);
    }

    pub fn report_gpu(passes: Vec<(&'static str, f64)>) {
        if let Some(last) = state().frames.back_mut() {
            last.gpu = passes;
        }
    }

    pub fn frames(last: usize) -> Vec<FrameRecord> {
        let s = state();
        let skip = s.frames.len().saturating_sub(last);
        s.frames.iter().skip(skip).cloned().collect()
    }

    pub fn frame_times(last: usize) -> Vec<(f64, Option<f64>)> {
        let s = state();
        let skip = s.frames.len().saturating_sub(last);
        s.frames
            .iter()
            .skip(skip)
            .map(|f| (f.cpu_ms, f.gpu_ms()))
            .collect()
    }

    /// Span names built at run time (panel ids...): leaked once each, so
    /// keep the set small and bounded.
    pub fn intern(name: &str) -> &'static str {
        use std::collections::HashSet;
        static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
        let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
        let names = names.get_or_insert_with(HashSet::new);
        if let Some(found) = names.get(name) {
            return found;
        }
        let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
        names.insert(leaked);
        leaked
    }

    pub fn clear() {
        let mut s = state();
        s.frames.clear();
        s.current.clear();
        s.frame_start = None;
        s.next_index = 0;
        s.hitches.clear();
        s.new_hitches = 0;
    }
}

#[cfg(not(all(
    not(target_arch = "wasm32"),
    any(feature = "enabled", debug_assertions)
)))]
mod imp {
    use super::{FrameRecord, Hitch};

    #[inline(always)]
    pub fn set_hitch_threshold_ms(_ms: f64) {}

    #[inline(always)]
    pub fn hitch_threshold_ms() -> f64 {
        super::DEFAULT_HITCH_MS
    }

    #[inline(always)]
    pub fn hitches() -> Vec<Hitch> {
        Vec::new()
    }

    #[inline(always)]
    pub fn take_new_hitches() -> Vec<Hitch> {
        Vec::new()
    }

    #[inline(always)]
    pub fn set_enabled(_on: bool) {}

    #[inline(always)]
    pub fn is_enabled() -> bool {
        false
    }

    /// Zero-sized: the profiler is compiled out.
    pub struct Scope;

    impl Scope {
        #[inline(always)]
        pub fn new(_name: &'static str) -> Self {
            Self
        }
    }

    #[inline(always)]
    pub fn begin_frame() {}

    #[inline(always)]
    pub fn finish_frame() {}

    #[inline(always)]
    pub fn report_gpu(_passes: Vec<(&'static str, f64)>) {}

    #[inline(always)]
    pub fn frames(_last: usize) -> Vec<FrameRecord> {
        Vec::new()
    }

    #[inline(always)]
    pub fn frame_times(_last: usize) -> Vec<(f64, Option<f64>)> {
        Vec::new()
    }

    #[inline(always)]
    pub fn clear() {}

    #[inline(always)]
    pub fn intern(_name: &str) -> &'static str {
        ""
    }
}

/// A `&'static str` for a span name built at run time (a panel id...). Each
/// distinct name is allocated once and kept: only for small, bounded sets.
pub fn intern(name: &str) -> &'static str {
    imp::intern(name)
}

/// A CPU span guard (see [`scope!`]).
pub use imp::Scope;

/// Pauses or resumes recording (on by default when compiled in).
pub fn set_enabled(on: bool) {
    imp::set_enabled(on);
}

/// Whether spans are being recorded right now.
pub fn is_enabled() -> bool {
    COMPILED && imp::is_enabled()
}

/// Starts a frame's clock. Optional: hosts that sleep between frames (the
/// editor redraws on input only) call it so that a frame's time is its busy
/// time; without it a frame lasts from the previous [`finish_frame`].
/// Spans recorded since the previous frame (worker threads) are kept.
pub fn begin_frame() {
    imp::begin_frame();
}

/// Closes the current frame (call once per frame, after presenting).
pub fn finish_frame() {
    imp::finish_frame();
}

/// GPU pass durations (milliseconds) for the newest frame.
pub fn report_gpu(passes: Vec<(&'static str, f64)>) {
    imp::report_gpu(passes);
}

/// The last `last` frames, oldest first.
pub fn frames(last: usize) -> Vec<FrameRecord> {
    imp::frames(last)
}

/// CPU and GPU time of the last `last` frames, oldest first (cheap: no
/// span is copied).
pub fn frame_times(last: usize) -> Vec<(f64, Option<f64>)> {
    imp::frame_times(last)
}

/// The most expensive spans over the last `last` frames.
pub fn top_scopes(last: usize) -> Vec<ScopeStat> {
    aggregate(&frames(last))
}

/// Average GPU time per pass over the last `last` frames.
pub fn gpu_passes(last: usize) -> Vec<(&'static str, f64)> {
    aggregate_gpu(&frames(last))
}

/// Frames longer than `ms` are recorded as hitches (100 ms by default).
pub fn set_hitch_threshold_ms(ms: f64) {
    imp::set_hitch_threshold_ms(ms);
}

/// The hitch threshold, in milliseconds.
pub fn hitch_threshold_ms() -> f64 {
    imp::hitch_threshold_ms()
}

/// The last [`MAX_HITCHES`] hitches, oldest first.
pub fn hitches() -> Vec<Hitch> {
    imp::hitches()
}

/// Hitches recorded since the previous call (to log each one once).
pub fn take_new_hitches() -> Vec<Hitch> {
    imp::take_new_hitches()
}

/// Forgets the history.
pub fn clear() {
    imp::clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(scopes: &[(&'static str, f64)], gpu: &[(&'static str, f64)]) -> FrameRecord {
        FrameRecord {
            index: 0,
            cpu_ms: 16.0,
            scopes: scopes
                .iter()
                .map(|&(name, duration_ms)| ScopeRecord {
                    name,
                    depth: 0,
                    start_ms: 0.0,
                    duration_ms,
                    frame_thread: true,
                })
                .collect(),
            gpu: gpu.to_vec(),
        }
    }

    #[test]
    fn aggregate_sums_calls_per_frame_and_sorts_by_mean() {
        let frames = [
            frame(&[("a", 1.0), ("a", 1.0), ("b", 3.0)], &[]),
            frame(&[("a", 4.0)], &[]),
        ];
        let stats = aggregate(&frames);
        assert_eq!(stats[0].name, "a");
        assert_eq!(stats[0].mean_ms, 3.0);
        assert_eq!(stats[0].max_ms, 4.0);
        assert_eq!(stats[0].calls, 1.5);
        assert_eq!(stats[1].name, "b");
        assert_eq!(stats[1].mean_ms, 1.5);
        assert!(aggregate(&[]).is_empty());
    }

    #[test]
    fn gpu_passes_are_averaged_over_frames_that_have_them() {
        let frames = [
            frame(&[], &[("scene", 2.0), ("ui", 1.0)]),
            frame(&[], &[]),
            frame(&[], &[("scene", 4.0)]),
        ];
        assert_eq!(aggregate_gpu(&frames), vec![("scene", 3.0), ("ui", 1.0)]);
        assert_eq!(frames[0].gpu_ms(), Some(3.0));
        assert_eq!(frames[1].gpu_ms(), None);
    }

    // The recorder is global: one test drives it end to end.
    #[test]
    fn spans_nest_and_frames_are_recorded() {
        if !COMPILED {
            scope!("free");
            finish_frame();
            assert!(frames(10).is_empty());
            assert_eq!(std::mem::size_of::<Scope>(), 0);
            return;
        }
        clear();
        finish_frame(); // starts the clock
        {
            scope!("outer");
            {
                scope!("inner");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        finish_frame();
        report_gpu(vec![("pass", 0.5)]);
        let last = frames(1).pop().unwrap();
        assert_eq!(last.scopes.len(), 2);
        let outer = last.scopes.iter().find(|s| s.name == "outer").unwrap();
        let inner = last.scopes.iter().find(|s| s.name == "inner").unwrap();
        assert_eq!((outer.depth, inner.depth), (0, 1));
        assert!(inner.duration_ms >= 1.5 && outer.duration_ms >= inner.duration_ms);
        assert!(last.cpu_ms >= outer.duration_ms);
        assert_eq!(gpu_passes(1), vec![("pass", 0.5)]);
        assert_eq!(top_scopes(1)[0].name, "outer");

        set_enabled(false);
        {
            scope!("paused");
        }
        finish_frame();
        set_enabled(true);
        assert!(frames(1)[0].scopes.iter().all(|s| s.name != "paused"));
        // A slow frame is a hitch, with its spans and GPU waits.
        set_hitch_threshold_ms(5.0);
        let _ = take_new_hitches();
        begin_frame();
        {
            scope!("slow");
            {
                scope!("gpu wait: surface acquire");
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        }
        finish_frame();
        set_hitch_threshold_ms(DEFAULT_HITCH_MS);
        let new = take_new_hitches();
        assert_eq!(new.len(), 1);
        assert!(take_new_hitches().is_empty());
        let h = &new[0];
        assert_eq!(h.top[0].0, "slow");
        assert_eq!(h.waits[0].0, "gpu wait: surface acquire");
        assert!(h.summary().contains("surface acquire"));
        assert_eq!(hitches().last(), Some(h));
        for _ in 0..HISTORY + 5 {
            finish_frame();
        }
        assert_eq!(frames(usize::MAX).len(), HISTORY);
        assert_eq!(frame_times(3).len(), 3);
    }
}
