//! Watching the project's files from a background thread: the story file
//! (changed by another tool or another editor) and the asset folder (files
//! added, replaced, removed).
//!
//! A deliberately simple polling watcher (no new dependency, same approach as
//! `orbit_assets::FileWatcher`, which only follows one file): every
//! `interval` the thread compares the modification time and length of the
//! story file, and the modification times of the asset **folders** (a file
//! added, removed or renamed changes its folder's). The whole asset tree is
//! walked only when a folder changed, or every few intervals for files
//! rewritten in place ([`FULL_SCAN_EVERY`]). A change is reported only once it **settled** (the same state
//! seen on two consecutive scans), so a file still being copied is not read
//! half-written. The thread wakes the UI only when it has something to say;
//! idle, the editor still draws no frame.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use orbit_editor_api::Waker;

use crate::assets::scan_assets;
use crate::thumbs::Stamp;

/// What the watcher noticed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    /// The story file changed on disk (and was not written by the editor).
    StoryChanged,
    /// Files of the asset folder changed (paths relative to the root).
    Assets {
        added: Vec<String>,
        modified: Vec<String>,
        removed: Vec<String>,
    },
}

/// `path -> (length, modification time)` of the files of a folder.
pub type Snapshot = BTreeMap<String, (u64, Option<SystemTime>)>;

/// Takes the snapshot of `root/assets`.
pub fn snapshot(root: &std::path::Path) -> Snapshot {
    scan_assets(root)
        .into_iter()
        .map(|f| (f.path, (f.len, f.modified)))
        .collect()
}

/// Modification times of the folders of the asset tree. A file added,
/// removed or renamed changes the time of its folder, so looking at the
/// folders alone (a few `stat`s) tells whether a full walk is needed.
pub type DirStamps = BTreeMap<PathBuf, Option<SystemTime>>;

/// The folders under `root/assets` (and itself) with their times.
pub fn dir_stamps(root: &std::path::Path) -> DirStamps {
    let mut out = DirStamps::new();
    let mut stack = vec![root.join("assets")];
    while let Some(dir) = stack.pop() {
        let Ok(meta) = std::fs::metadata(&dir) else { continue };
        out.insert(dir.clone(), meta.modified().ok());
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(entry.path());
            }
        }
    }
    out
}

/// Whether a folder of `dirs` changed (or disappeared) since it was stamped.
fn dirs_changed(dirs: &DirStamps) -> bool {
    dirs.iter().any(|(dir, stamp)| {
        std::fs::metadata(dir).ok().map(|m| m.modified().ok()) != Some(*stamp)
    })
}

/// A full walk of the asset folder happens at least every this many
/// intervals (files modified in place change no folder time); in between,
/// only the folder times are looked at.
const FULL_SCAN_EVERY: u32 = 4;

/// Differences between two snapshots: `(added, modified, removed)`.
pub fn diff(old: &Snapshot, new: &Snapshot) -> (Vec<String>, Vec<String>, Vec<String>) {
    let added = new
        .keys()
        .filter(|k| !old.contains_key(*k))
        .cloned()
        .collect();
    let modified = new
        .iter()
        .filter(|(k, v)| old.get(*k).is_some_and(|o| o != *v))
        .map(|(k, _)| k.clone())
        .collect();
    let removed = old
        .keys()
        .filter(|k| !new.contains_key(*k))
        .cloned()
        .collect();
    (added, modified, removed)
}

/// Settling logic for one watched thing: a change is reported when the new
/// state was seen twice in a row.
#[derive(Debug)]
pub struct Settle<T: PartialEq + Clone> {
    known: T,
    candidate: Option<T>,
}

impl<T: PartialEq + Clone> Settle<T> {
    pub fn new(known: T) -> Self {
        Self {
            known,
            candidate: None,
        }
    }

    /// The state observed now. Returns the previous state once a change
    /// settled (the observed state becomes the known one).
    pub fn observe(&mut self, now: T) -> Option<T> {
        if now == self.known {
            self.candidate = None;
            return None;
        }
        if self.candidate.as_ref() == Some(&now) {
            self.candidate = None;
            return Some(std::mem::replace(&mut self.known, now));
        }
        self.candidate = Some(now);
        None
    }

    /// A change was seen once and waits for its confirmation.
    pub fn pending(&self) -> bool {
        self.candidate.is_some()
    }

    /// Forgets a pending change and takes `state` as the known one.
    pub fn accept(&mut self, state: T) {
        self.known = state;
        self.candidate = None;
    }
}

struct Shared {
    stop: AtomicBool,
    sleep: (Mutex<()>, Condvar),
    /// Stamp of the story file the editor itself wrote or loaded.
    story_accepted: Mutex<Option<Option<Stamp>>>,
}

pub struct ProjectWatcher {
    shared: Arc<Shared>,
    events: Receiver<WatchEvent>,
    thread: Option<std::thread::JoinHandle<()>>,
    story_path: PathBuf,
}

impl ProjectWatcher {
    /// Starts watching the story file and `<assets_root>/assets`.
    pub fn start(
        assets_root: PathBuf,
        story_path: PathBuf,
        interval: Duration,
        waker: Waker,
    ) -> Self {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            sleep: (Mutex::new(()), Condvar::new()),
            story_accepted: Mutex::new(None),
        });
        let (tx, rx) = channel();
        // The baseline is taken now, not when the thread gets going: what
        // changes after `start` returns must be reported.
        let story = Settle::new(Stamp::of(&story_path));
        let assets = Settle::new(snapshot(&assets_root));
        let thread = {
            let shared = shared.clone();
            let story_path = story_path.clone();
            std::thread::Builder::new()
                .name("orbit-watch".to_owned())
                .spawn(move || {
                    watch_loop(
                        &shared,
                        &assets_root,
                        &story_path,
                        interval,
                        (story, assets),
                        &tx,
                        &waker,
                    );
                })
                .ok()
        };
        Self {
            shared,
            events: rx,
            thread,
            story_path,
        }
    }

    /// Events since the last call.
    pub fn poll(&self) -> Vec<WatchEvent> {
        self.events.try_iter().collect()
    }

    /// The editor just wrote or loaded the story file: that state is not an
    /// external change.
    pub fn accept_story_now(&self) {
        if let Ok(mut slot) = self.shared.story_accepted.lock() {
            *slot = Some(Stamp::of(&self.story_path));
        }
    }
}

impl Drop for ProjectWatcher {
    /// Asks the thread to stop without waiting for it: it may be in the
    /// middle of a walk of a big folder (or of a slow disk), and closing a
    /// project must not wait for that on the UI thread. It ends on its own
    /// and sends nothing more (its channel is gone).
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.sleep.1.notify_all();
        drop(self.thread.take());
    }
}

fn watch_loop(
    shared: &Shared,
    assets_root: &std::path::Path,
    story_path: &std::path::Path,
    interval: Duration,
    baselines: (Settle<Option<Stamp>>, Settle<Snapshot>),
    tx: &Sender<WatchEvent>,
    waker: &Waker,
) {
    let (mut story, mut assets) = baselines;
    let mut dirs = dir_stamps(assets_root);
    let mut tick: u32 = 0;
    loop {
        {
            let guard = shared.sleep.0.lock();
            let Ok(guard) = guard else { return };
            let _ = shared.sleep.1.wait_timeout(guard, interval);
        }
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let mut sent = false;
        if let Ok(mut slot) = shared.story_accepted.lock()
            && let Some(accepted) = slot.take()
        {
            story.accept(accepted);
        }
        let now = Stamp::of(story_path);
        // A momentarily missing file (save by rename) is not a change.
        if now.is_some() && story.observe(now).is_some() {
            sent |= tx.send(WatchEvent::StoryChanged).is_ok();
        }
        // The whole tree is walked only when a folder changed (file added,
        // removed, renamed), while a change waits for its confirmation, or
        // every `FULL_SCAN_EVERY` intervals (a file rewritten in place).
        tick = tick.wrapping_add(1);
        let full = tick % FULL_SCAN_EVERY == 0 || assets.pending() || dirs_changed(&dirs);
        if !full {
            if sent {
                waker.wake();
            }
            continue;
        }
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        dirs = dir_stamps(assets_root);
        let current = snapshot(assets_root);
        if let Some(previous) = assets.observe(current.clone()) {
            let (added, modified, removed) = diff(&previous, &current);
            if !(added.is_empty() && modified.is_empty() && removed.is_empty()) {
                sent |= tx
                    .send(WatchEvent::Assets {
                        added,
                        modified,
                        removed,
                    })
                    .is_ok();
            }
        }
        if sent {
            waker.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Instant;

    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orbit_watch_{tag}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        fs::create_dir_all(dir.join("assets/images")).unwrap();
        dir
    }

    fn wait_for(watcher: &ProjectWatcher, mut want: impl FnMut(&WatchEvent) -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            if watcher.poll().iter().any(&mut want) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn diff_lists_added_modified_and_removed() {
        let t = |n: u64| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(n));
        let old: Snapshot = [("a".into(), (1, t(1))), ("b".into(), (1, t(1)))].into();
        let new: Snapshot = [("b".into(), (2, t(2))), ("c".into(), (1, t(1)))].into();
        let (added, modified, removed) = diff(&old, &new);
        assert_eq!(added, ["c"]);
        assert_eq!(modified, ["b"]);
        assert_eq!(removed, ["a"]);
    }

    #[test]
    fn a_change_is_reported_once_it_settled() {
        let mut s = Settle::new(1);
        assert_eq!(s.observe(1), None);
        assert_eq!(s.observe(2), None, "first sighting: maybe still changing");
        assert_eq!(s.observe(3), None, "changed again: start over");
        assert_eq!(s.observe(3), Some(1));
        assert_eq!(s.observe(3), None);
        s.accept(9);
        assert_eq!(s.observe(9), None);
    }

    #[test]
    fn modified_added_and_removed_files_raise_events() {
        let root = temp_dir("events");
        let story = root.join("story.ron");
        fs::write(&story, "(v: 1)").unwrap();
        fs::write(root.join("assets/images/a.png"), b"one").unwrap();
        let watcher = ProjectWatcher::start(
            root.clone(),
            story.clone(),
            Duration::from_millis(15),
            Waker::default(),
        );
        // Added.
        fs::write(root.join("assets/images/b.png"), b"two").unwrap();
        assert!(wait_for(&watcher, |e| matches!(
            e, WatchEvent::Assets { added, .. } if added == &["assets/images/b.png"]
        )));
        // Modified (the length differs, so the stamp does even on a coarse clock).
        fs::write(root.join("assets/images/a.png"), b"one, longer").unwrap();
        assert!(wait_for(&watcher, |e| matches!(
            e, WatchEvent::Assets { modified, .. } if modified == &["assets/images/a.png"]
        )));
        // Removed.
        fs::remove_file(root.join("assets/images/b.png")).unwrap();
        assert!(wait_for(&watcher, |e| matches!(
            e, WatchEvent::Assets { removed, .. } if removed == &["assets/images/b.png"]
        )));
        // The story file changed by someone else.
        fs::write(&story, "(v: 2, longer: true)").unwrap();
        assert!(wait_for(&watcher, |e| *e == WatchEvent::StoryChanged));
        // What the editor wrote itself is not an external change.
        fs::write(&story, "(v: 3, written: by_the_editor)").unwrap();
        watcher.accept_story_now();
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            !watcher.poll().contains(&WatchEvent::StoryChanged),
            "own write reported as external"
        );
        drop(watcher);
        fs::remove_dir_all(&root).unwrap();
    }
}
