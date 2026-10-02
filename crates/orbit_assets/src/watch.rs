//! Development-time file watching (hot reload).
//!
//! A deliberately simple polling watcher: it compares the modification time
//! and length of one file. No threads, no dependencies. On the web it is an
//! inert stub (there is no filesystem to watch), so callers need no `cfg`.

use std::time::Duration;

/// Watches one file for modifications. Call [`FileWatcher::poll`] every frame
/// with the frame time; the file system is only queried once per `interval`.
#[derive(Debug)]
pub struct FileWatcher {
    #[cfg(not(target_arch = "wasm32"))]
    inner: native::Inner,
}

impl FileWatcher {
    /// Starts watching `path` (a native file path). The current state of the
    /// file is the baseline: only later changes are reported.
    pub fn new(path: impl Into<String>, interval: Duration) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                inner: native::Inner::new(path.into(), interval),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (path, interval);
            Self {}
        }
    }

    /// `true` once each time the file changed since the previous report.
    /// A file that disappears (editors that save by rename) is not reported
    /// until it comes back with a different timestamp or size.
    pub fn poll(&mut self, dt: Duration) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner.poll(dt)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = dt;
            false
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::time::{Duration, SystemTime};

    type Stamp = Option<(SystemTime, u64)>;

    #[derive(Debug)]
    pub(super) struct Inner {
        path: String,
        interval: Duration,
        since_check: Duration,
        last: Stamp,
    }

    fn stamp(path: &str) -> Stamp {
        let meta = std::fs::metadata(path).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    }

    impl Inner {
        pub(super) fn new(path: String, interval: Duration) -> Self {
            let last = stamp(&path);
            Self {
                path,
                interval,
                since_check: Duration::ZERO,
                last,
            }
        }

        pub(super) fn poll(&mut self, dt: Duration) -> bool {
            self.since_check += dt;
            if self.since_check < self.interval {
                return false;
            }
            self.since_check = Duration::ZERO;
            let now = stamp(&self.path);
            if now.is_none() || now == self.last {
                return false; // unchanged, or momentarily missing
            }
            self.last = now;
            true
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn reports_a_change_once() {
        let dir = std::env::temp_dir().join("orbit_watch_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("w.txt");
        std::fs::write(&file, "a").unwrap();
        let mut w = FileWatcher::new(file.to_string_lossy(), Duration::ZERO);
        assert!(!w.poll(Duration::from_millis(1)));
        std::fs::write(&file, "longer content").unwrap(); // size differs
        assert!(w.poll(Duration::from_millis(1)));
        assert!(!w.poll(Duration::from_millis(1)));
        std::fs::remove_file(&file).unwrap();
        assert!(!w.poll(Duration::from_millis(1))); // missing: no report
    }
}
