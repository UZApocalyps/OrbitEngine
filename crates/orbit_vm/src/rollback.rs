//! Player-facing rollback, roll-forward and backlog (`impl Vm`).

use std::collections::VecDeque;

use orbit_core::Story;

use crate::error::VmError;
use crate::event::PresentationEvent;
use crate::history::{BacklogItem, Presentation, RollbackHistory};
use crate::state::{SceneState, VmState};
use crate::vm::Vm;

impl Vm {
    /// `true` if [`Vm::rollback`] would move back at least one step.
    pub fn can_rollback(&self) -> bool {
        self.history.rollback_target(1).is_some()
    }

    /// `true` if [`Vm::roll_forward`] would move at least one step.
    pub fn can_roll_forward(&self) -> bool {
        self.history.forward_target(1).is_some()
    }

    /// Goes back `steps` interactions (lines or choices) and restores the VM
    /// to that moment. Stops at the oldest checkpoint, or at a barrier.
    /// Returns what to re-display, or `None` (nothing changed) when there is
    /// nowhere to go.
    ///
    /// The VM is then waiting exactly where it was at that interaction:
    /// `step()` moves forward again (replaying the known future as long as
    /// the choices are not changed), `choose()` answers a re-displayed choice.
    /// Persistent variables and the set of read lines are not affected.
    pub fn rollback(&mut self, steps: usize) -> Option<Presentation> {
        let target = self.history.rollback_target(steps)?;
        Some(self.move_to(target))
    }

    /// Goes forward `steps` interactions through the history kept by a
    /// previous rollback (nothing to do if the player has already made a
    /// different choice since).
    pub fn roll_forward(&mut self, steps: usize) -> Option<Presentation> {
        let target = self.history.forward_target(steps)?;
        Some(self.move_to(target))
    }

    /// Goes to the checkpoint `seq` (from [`BacklogItem::checkpoint`]).
    pub fn rollback_to(&mut self, seq: u64) -> Option<Presentation> {
        let index = self.history.index_of(seq)?;
        if index < self.history.floor() {
            return None;
        }
        Some(self.move_to(index))
    }

    fn move_to(&mut self, index: usize) -> Presentation {
        let state = self.history.move_to(index);
        self.state = state;
        self.fault = None;
        self.events.clear();
        self.presentation()
    }

    /// Marks a checkpoint barrier: the next interaction becomes the oldest
    /// point rollback can reach (e.g. after an irreversible event).
    pub fn checkpoint_barrier(&mut self) {
        self.history.barrier_pending = true;
    }

    /// What is on screen now, as plain data: the host rebuilds the display
    /// from it after `restore` / `restore_with_history` (a load).
    pub fn presentation(&self) -> Presentation {
        let checkpoint = self.history.entries.get(self.history.cursor).and_then(|c| {
            (c.state.shown == self.state.shown && self.state.shown.is_some()).then_some(c.seq)
        });
        Presentation {
            events: scene_events(&self.story, &self.state.scene),
            scene: self.state.scene.clone(),
            interaction: self.state.shown.clone(),
            already_read: true,
            checkpoint,
        }
    }

    /// The lines shown so far, oldest first, up to the current position
    /// (rolled-back lines are not included, rolled-forward ones are).
    pub fn backlog(&self) -> Vec<BacklogItem> {
        self.history.backlog_items()
    }

    /// Number of checkpoints currently kept.
    pub fn history_len(&self) -> usize {
        self.history.entries.len()
    }

    /// Maximum number of checkpoints (default 100). `0` disables rollback.
    pub fn history_capacity(&self) -> usize {
        self.history.capacity
    }

    pub fn set_history_capacity(&mut self, capacity: usize) {
        self.history.capacity = capacity;
        while self.history.entries.len() > capacity {
            self.history.entries.pop_front();
            self.history.cursor = self.history.cursor.saturating_sub(1);
        }
    }

    /// Maximum number of backlog lines (default 500).
    pub fn set_backlog_capacity(&mut self, capacity: usize) {
        self.history.backlog_capacity = capacity;
        while self.history.backlog.len() > capacity {
            self.history.backlog.pop_front();
        }
    }

    /// Forgets all checkpoints and the backlog.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Serializable copy of the history up to the current position, bounded
    /// to the last `max_checkpoints` checkpoints and `max_backlog` lines.
    pub fn history_snapshot(&self, max_checkpoints: usize, max_backlog: usize) -> RollbackHistory {
        self.history.snapshot(max_checkpoints, max_backlog)
    }

    /// Like `restore`, but also reinstates a saved history, so the player can
    /// still roll back after loading. The last checkpoint is the current one.
    /// Fails (VM untouched) if a checkpoint references unknown nodes.
    pub fn restore_with_history(
        &mut self,
        state: VmState,
        history: RollbackHistory,
    ) -> Result<(), VmError> {
        for checkpoint in &history.checkpoints {
            self.validate_state(&checkpoint.state)?;
        }
        let mut state = state;
        self.validate_state(&state)?;
        self.reconcile(&mut state);
        let mut checkpoints = history.checkpoints;
        for checkpoint in &mut checkpoints {
            self.reconcile(&mut checkpoint.state);
        }

        self.history.clear();
        if self.history.capacity > 0 {
            let excess = checkpoints.len().saturating_sub(self.history.capacity);
            checkpoints.drain(..excess);
        } else {
            checkpoints.clear();
        }
        let last_seq = checkpoints.last().map(|c| c.seq);
        self.history.next_seq = history.next_seq.max(last_seq.map_or(0, |s| s + 1));
        self.history.ahead = checkpoints.last().is_some_and(|c| c.state != state);
        self.history.cursor = checkpoints.len().saturating_sub(1);
        self.history.entries = checkpoints.into();
        let mut backlog: VecDeque<_> = history.backlog.into();
        while backlog.len() > self.history.backlog_capacity {
            backlog.pop_front();
        }
        self.history.backlog = backlog;
        if self.history.entries.is_empty() && state.shown.is_some() {
            self.history.record(&state);
        }
        self.state = state;
        self.fault = None;
        self.events.clear();
        Ok(())
    }
}

/// `scene` as cut-style events (see [`Presentation::events`]). Elements the
/// story cannot resolve are skipped.
pub fn scene_events(story: &Story, scene: &SceneState) -> Vec<PresentationEvent> {
    let mut events = Vec::new();
    if !scene.bg_layers.is_empty() {
        events.push(PresentationEvent::BackgroundLayers {
            layers: scene.bg_layers.clone(),
            transition: None,
        });
    } else if let Some(asset) = scene.background {
        events.push(PresentationEvent::Background {
            asset,
            transition: None,
        });
    }
    // After the background: a `Background` event clears the 3D layer.
    if let Some(s) = &scene.scene3d {
        events.push(PresentationEvent::Scene3d {
            scene: s.scene,
            camera: s.camera.clone(),
            transition: None,
        });
    }
    for c in &scene.characters {
        let Some(image) = story
            .character(c.character)
            .and_then(|def| def.expression(&c.expression))
            .map(|e| e.image)
        else {
            continue;
        };
        let Some((x, y)) = story.resolve_position(&c.position) else {
            continue;
        };
        events.push(PresentationEvent::Show {
            character: c.character,
            expression: c.expression.clone(),
            image,
            position: c.position.clone(),
            x,
            y,
            scale: c.scale,
            flip_x: c.flip_x,
            flip_y: c.flip_y,
            transition: None,
        });
    }
    if let Some(music) = &scene.music {
        events.push(PresentationEvent::PlayMusic {
            asset: music.asset,
            fade_in: 0.0,
            looping: music.looping,
        });
    }
    events
}
