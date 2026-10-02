//! Advisory lock on a project folder (`.orbit/lock`).
//!
//! The editor and `orbit mcp` (headless) both keep a project in memory and
//! write it back whole: opened together on the same folder, the later save
//! silently overwrites the other's work. The lock does not prevent that by
//! itself, it lets each side **see** the other and warn clearly.
//!
//! The holder rewrites the file every few seconds (a heartbeat); a lock whose
//! heartbeat is older than [`STALE_AFTER`] is a leftover of a crashed process
//! and is ignored. No process-liveness API is needed (std only, portable).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Lock file, relative to the project folder.
pub const LOCK_FILE: &str = ".orbit/lock";
/// How often the holder proves it is alive.
pub const HEARTBEAT: Duration = Duration::from_secs(3);
/// A heartbeat older than this means the holder is gone.
pub const STALE_AFTER: Duration = Duration::from_secs(12);

/// Who holds a lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockInfo {
    /// `editor` or `headless-mcp`.
    pub owner: String,
    pub pid: u32,
    pub started_ms: u64,
    pub heartbeat_ms: u64,
}

impl LockInfo {
    fn encode(&self) -> String {
        format!(
            "owner={}\npid={}\nstarted_ms={}\nheartbeat_ms={}\n",
            self.owner, self.pid, self.started_ms, self.heartbeat_ms
        )
    }

    fn decode(text: &str) -> Option<Self> {
        let mut owner = None;
        let (mut pid, mut started, mut beat) = (None, None, None);
        for line in text.lines() {
            let (key, value) = line.split_once('=')?;
            match key.trim() {
                "owner" => owner = Some(value.trim().to_owned()),
                "pid" => pid = value.trim().parse().ok(),
                "started_ms" => started = value.trim().parse().ok(),
                "heartbeat_ms" => beat = value.trim().parse().ok(),
                _ => {}
            }
        }
        Some(Self {
            owner: owner?,
            pid: pid?,
            started_ms: started?,
            heartbeat_ms: beat?,
        })
    }

    /// Whether the holder still beats at `now_ms`.
    pub fn is_fresh(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.heartbeat_ms) < STALE_AFTER.as_millis() as u64
    }

    /// Human description for a warning: `l'éditeur (pid 123)`.
    pub fn describe(&self) -> String {
        let who = match self.owner.as_str() {
            "editor" => "l'éditeur",
            "headless-mcp" => "un serveur MCP headless (`orbit mcp`)",
            other => other,
        };
        format!("{who} (pid {})", self.pid)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn lock_path(root: &Path) -> PathBuf {
    root.join(LOCK_FILE)
}

/// The live lock held on `root` by **another process**, if any. A stale
/// lock, an unreadable one and our own are not reported.
pub fn inspect(root: &Path) -> Option<LockInfo> {
    let text = fs::read_to_string(lock_path(root)).ok()?;
    let info = LockInfo::decode(&text)?;
    (info.pid != std::process::id() && info.is_fresh(now_ms())).then_some(info)
}

fn write_lock(path: &Path, info: &LockInfo) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Written in place: the file is tiny, and a reader that sees a torn
    // write simply treats the lock as unreadable for one beat.
    fs::write(path, info.encode())
}

/// A lock held by this process. Dropping it releases the lock.
pub struct ProjectLock {
    path: PathBuf,
    info: LockInfo,
    stop: Arc<AtomicBool>,
    /// Another process took the project while we held it (set by the heartbeat).
    taken_over: Arc<Mutex<Option<LockInfo>>>,
    beat: Option<JoinHandle<()>>,
}

impl ProjectLock {
    /// Takes the lock on `root`. The caller decides what to do about a lock
    /// that [`inspect`] reports **before** calling this: acquiring always
    /// succeeds (it overwrites).
    pub fn acquire(root: &Path, owner: &str) -> io::Result<Self> {
        let path = lock_path(root);
        let started = now_ms();
        let info = LockInfo {
            owner: owner.to_owned(),
            pid: std::process::id(),
            started_ms: started,
            heartbeat_ms: started,
        };
        write_lock(&path, &info)?;
        let stop = Arc::new(AtomicBool::new(false));
        let taken_over = Arc::new(Mutex::new(None));
        let beat = {
            let (stop, taken, path, mut mine) =
                (stop.clone(), taken_over.clone(), path.clone(), info.clone());
            thread::Builder::new()
                .name("orbit-project-lock".into())
                .spawn(move || {
                    // Sleeps in small slices so that Drop does not wait a whole beat.
                    let slice = Duration::from_millis(100);
                    let mut waited = Duration::ZERO;
                    while !stop.load(Ordering::Relaxed) {
                        thread::sleep(slice);
                        waited += slice;
                        if waited < HEARTBEAT {
                            continue;
                        }
                        waited = Duration::ZERO;
                        // Someone else wrote the file: report it, stop beating.
                        let current = fs::read_to_string(&path)
                            .ok()
                            .and_then(|t| LockInfo::decode(&t));
                        if let Some(other) = current.filter(|c| c.pid != mine.pid) {
                            if let Ok(mut slot) = taken.lock() {
                                *slot = Some(other);
                            }
                            return;
                        }
                        mine.heartbeat_ms = now_ms();
                        let _ = write_lock(&path, &mine);
                    }
                })
                .ok()
        };
        Ok(Self {
            path,
            info,
            stop,
            taken_over,
            beat,
        })
    }

    pub fn info(&self) -> &LockInfo {
        &self.info
    }

    /// Who took the project from us, once (`None` while we still hold it).
    pub fn taken_over_by(&self) -> Option<LockInfo> {
        self.taken_over.lock().ok().and_then(|slot| slot.clone())
    }
}

impl Drop for ProjectLock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.beat.take() {
            let _ = handle.join();
        }
        // Only remove what is ours: a forced takeover left another owner's file.
        let ours = fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| LockInfo::decode(&t))
            .is_some_and(|c| c.pid == self.info.pid);
        if ours {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orbit_lock_{tag}_{}_{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn encodes_and_decodes() {
        let info = LockInfo {
            owner: "editor".into(),
            pid: 42,
            started_ms: 1,
            heartbeat_ms: 2,
        };
        assert_eq!(LockInfo::decode(&info.encode()), Some(info));
        assert_eq!(LockInfo::decode("garbage"), None);
    }

    #[test]
    fn freshness_follows_the_heartbeat() {
        let info = LockInfo {
            owner: "editor".into(),
            pid: 1,
            started_ms: 0,
            heartbeat_ms: 100_000,
        };
        assert!(info.is_fresh(100_000 + 5_000));
        assert!(!info.is_fresh(100_000 + 13_000));
    }

    #[test]
    fn own_lock_is_not_a_conflict_and_is_released_on_drop() {
        let dir = temp_dir("own");
        let lock = ProjectLock::acquire(&dir, "editor").unwrap();
        assert!(dir.join(LOCK_FILE).is_file());
        assert!(inspect(&dir).is_none(), "our own lock is not a conflict");
        drop(lock);
        assert!(!dir.join(LOCK_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_process_lock_is_reported_until_stale() {
        let dir = temp_dir("other");
        let other = LockInfo {
            owner: "headless-mcp".into(),
            pid: std::process::id().wrapping_add(1),
            started_ms: now_ms(),
            heartbeat_ms: now_ms(),
        };
        write_lock(&lock_path(&dir), &other).unwrap();
        assert_eq!(inspect(&dir), Some(other.clone()));
        let stale = LockInfo {
            heartbeat_ms: now_ms() - 60_000,
            ..other
        };
        write_lock(&lock_path(&dir), &stale).unwrap();
        assert!(inspect(&dir).is_none(), "a stale lock is ignored");
        let _ = fs::remove_dir_all(&dir);
    }
}
