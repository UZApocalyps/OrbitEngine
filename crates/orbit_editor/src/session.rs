//! An open project: the `Project` (the only document state), where it
//! lives, what is on disk, and autosave / recovery bookkeeping.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use orbit_core::Story;
use orbit_project::{ChangeEvent, Project, load_story, save_story};

use crate::storage::{ProjectLocation, Recovery, RecoveryMeta};

/// Default delay between two autosaves while the project is modified.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// A story to save, owned: written off the UI thread (see
/// [`Session::save_snapshot`]).
pub struct SaveSnapshot {
    storage: crate::storage::FsStorage,
    file: String,
    pub story: Story,
    pub revision: u64,
}

impl SaveSnapshot {
    /// Serializes and writes the story (atomically).
    pub fn write(&self) -> Result<(), String> {
        save_story(&self.storage, &self.file, &self.story).map_err(|e| e.to_string())
    }
}

pub struct Session {
    pub project: Project,
    pub location: ProjectLocation,
    pub name: String,
    /// The story as last read from / written to the project files.
    saved: Story,
    /// Change events not yet delivered to the editor.
    changes: Arc<Mutex<Vec<ChangeEvent>>>,
    checked_revision: u64,
    modified: bool,
    recovery: Recovery,
    autosaved_revision: u64,
    last_autosave: Instant,
}

impl Session {
    pub fn open(location: ProjectLocation, story: Story, recovery_dir: std::path::PathBuf) -> Self {
        let name = location
            .manifest
            .as_ref()
            .map_or_else(|| story.title.clone(), |m| m.name.clone());
        let mut session = Self {
            project: Project::new(story.clone()),
            location,
            name,
            saved: story,
            changes: Arc::default(),
            checked_revision: 0,
            modified: false,
            recovery: Recovery::new(recovery_dir),
            autosaved_revision: 0,
            last_autosave: Instant::now(),
        };
        session.attach();
        session
    }

    /// Subscribes to the project's change events.
    fn attach(&mut self) {
        let queue = self.changes.clone();
        self.project.subscribe(move |event| {
            if let Ok(mut queue) = queue.lock() {
                queue.push(event.clone());
            }
        });
    }

    /// Change events since the last call.
    pub fn drain_changes(&self) -> Vec<ChangeEvent> {
        self.changes
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }

    /// Number of change events waiting to be delivered (not consumed).
    pub fn pending_changes(&self) -> usize {
        self.changes.lock().map_or(0, |q| q.len())
    }

    /// The nodes touched by the events queued after the first `from` ones,
    /// without consuming them (what one MCP call changed).
    pub fn touched_since(&self, from: usize) -> Vec<orbit_core::NodeId> {
        let mut out = Vec::new();
        if let Ok(queue) = self.changes.lock() {
            for event in queue.iter().skip(from) {
                for id in &event.touched {
                    if !out.contains(id) {
                        out.push(*id);
                    }
                }
            }
        }
        out
    }

    /// Whether the story differs from what is on disk. Cached per revision.
    pub fn modified(&mut self) -> bool {
        let revision = self.project.revision();
        if revision != self.checked_revision {
            self.checked_revision = revision;
            self.modified = self.project.story() != &self.saved;
        }
        self.modified
    }

    /// The story as last read from / written to the project files.
    pub fn saved_story(&self) -> &Story {
        &self.saved
    }

    /// Replaces the story by what the file holds now (it was changed by
    /// another tool). Undo history starts empty; the project is unmodified.
    pub fn reload_from_disk(&mut self) -> Result<(), String> {
        let story = load_story(&self.location.storage(), &self.location.story_file)
            .map_err(|e| e.to_string())?;
        self.project = Project::new(story.clone());
        self.attach();
        self.saved = story;
        self.checked_revision = u64::MAX; // force the comparison
        self.autosaved_revision = self.project.revision();
        self.recovery.clear();
        Ok(())
    }

    /// Writes the story to its file (atomically) and forgets recovery files.
    pub fn save(&mut self) -> Result<(), String> {
        let snapshot = self.save_snapshot();
        snapshot.write()?;
        self.saved_snapshot(snapshot);
        Ok(())
    }

    /// What a save writes, taken on the UI thread (a copy of the story): the
    /// serialization and the write can then run in the background
    /// ([`SaveSnapshot::write`]), and [`Session::saved_snapshot`] records it.
    pub fn save_snapshot(&self) -> SaveSnapshot {
        SaveSnapshot {
            storage: self.location.storage(),
            file: self.location.story_file.clone(),
            story: self.project.story().clone(),
            revision: self.project.revision(),
        }
    }

    /// `snapshot` was written: it is what is on disk now. Edits made while it
    /// was written keep the project modified.
    pub fn saved_snapshot(&mut self, snapshot: SaveSnapshot) {
        self.saved = snapshot.story;
        if self.project.revision() == snapshot.revision {
            self.checked_revision = snapshot.revision;
            self.modified = false;
            self.recovery.clear();
        } else {
            self.checked_revision = u64::MAX; // compare again
        }
    }

    // ------------------------------------------------------------ recovery

    /// A snapshot left by a session that did not end cleanly.
    pub fn pending_recovery(&self) -> Option<RecoveryMeta> {
        self.recovery
            .pending()
            .filter(|m| m.story_file == self.location.story_file)
    }

    /// Replaces the story by the recovered snapshot. The result counts as
    /// modified (it differs from the file) and can be saved or discarded;
    /// undo history starts empty.
    pub fn restore_recovery(&mut self) -> Result<(), String> {
        let story = self.recovery.load()?;
        self.project = Project::new(story);
        self.attach();
        self.autosaved_revision = self.project.revision();
        self.checked_revision = u64::MAX; // force the comparison
        Ok(())
    }

    pub fn discard_recovery(&self) {
        self.recovery.clear();
    }

    /// An autosave is needed and its delay has elapsed.
    pub fn autosave_due(&mut self, now: Instant, interval: Duration) -> bool {
        self.needs_autosave() && now.duration_since(self.last_autosave) >= interval
    }

    /// When the next autosave becomes due, if one is pending.
    pub fn autosave_wake(&mut self, interval: Duration) -> Option<Instant> {
        self.needs_autosave().then(|| self.last_autosave + interval)
    }

    fn needs_autosave(&mut self) -> bool {
        self.modified() && self.project.revision() != self.autosaved_revision
    }

    pub fn autosave(&mut self, now: Instant) -> Result<(), String> {
        // Even a failed attempt waits a full interval before the next one.
        self.last_autosave = now;
        let meta = RecoveryMeta {
            story_file: self.location.story_file.clone(),
            saved_at_ms: orbit_platform::unix_time_ms(),
        };
        self.recovery.write(self.project.story(), &meta)?;
        self.autosaved_revision = self.project.revision();
        Ok(())
    }

    /// Clean end of the session: nothing to recover any more.
    pub fn close(mut self) {
        if !self.modified() {
            self.recovery.clear();
        }
    }
}
