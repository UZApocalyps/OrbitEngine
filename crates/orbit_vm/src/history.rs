//! Rollback history (checkpoints) and dialogue backlog.
//!
//! A [`Checkpoint`] is a [`VmState`] captured *right after* an interaction
//! (a `Say` or a `Choice`) was presented, so `state.shown` is what the player
//! sees and `state.scene` is the declarative scene at that moment. Rolling
//! back restores that state and hands a [`Presentation`] to the host, which
//! rebuilds the screen from it: no node is re-run and no transition replayed.
//!
//! The history is a bounded ring (default [`DEFAULT_HISTORY_CAPACITY`]) with a
//! cursor. Rolling back moves the cursor towards the past and keeps the future
//! entries, so `roll_forward` (or simply stepping again) can replay them. The
//! future is cut as soon as the player makes a *different* choice.

use std::collections::VecDeque;

use orbit_core::{CharacterId, NodeId, StringId};
use serde::{Deserialize, Serialize};

use crate::event::PresentationEvent;
use crate::state::{Interaction, SceneState, VmState};

/// Default number of checkpoints kept for rollback.
pub const DEFAULT_HISTORY_CAPACITY: usize = 100;
/// Default number of lines kept in the backlog.
pub const DEFAULT_BACKLOG_CAPACITY: usize = 500;

/// A rollback point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Position of this interaction in the timeline (0 = first one). Stable
    /// while the entry exists; used to refer to it from the backlog.
    pub seq: u64,
    /// State right after the interaction was presented.
    pub state: VmState,
    /// Rollback cannot go back further than the most recent barrier.
    pub barrier: bool,
    /// For a `Choice`: the visible option picked from here, if any.
    pub chosen: Option<usize>,
}

/// A displayed line, for the history screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacklogEntry {
    pub seq: u64,
    pub node: NodeId,
    pub speaker: Option<CharacterId>,
    pub string_id: StringId,
}

/// A backlog entry as returned to the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BacklogItem {
    pub node: NodeId,
    pub speaker: Option<CharacterId>,
    pub string_id: StringId,
    /// `Some(seq)` when "go back to this line" is possible: pass it to
    /// `Vm::rollback_to`. `None` for lines that fell out of the ring or lie
    /// before a barrier.
    pub checkpoint: Option<u64>,
}

/// Serializable form of the history (what a save may embed). The last
/// checkpoint is the current one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RollbackHistory {
    pub checkpoints: Vec<Checkpoint>,
    pub backlog: Vec<BacklogEntry>,
    pub next_seq: u64,
}

/// Everything the host needs to re-display a moment of the story.
#[derive(Clone, Debug, PartialEq)]
pub struct Presentation {
    /// Background, characters and music to show.
    pub scene: SceneState,
    /// The line or choice to show, if any.
    pub interaction: Option<Interaction>,
    /// `scene` as cut-style events (no transitions). The host clears the
    /// stage and music, then applies them like any other event.
    pub events: Vec<PresentationEvent>,
    /// The line was already read (always `true` for a rollback).
    pub already_read: bool,
    /// The timeline position of the interaction, when it is a checkpoint.
    pub checkpoint: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct History {
    pub entries: VecDeque<Checkpoint>,
    pub cursor: usize,
    pub backlog: VecDeque<BacklogEntry>,
    pub next_seq: u64,
    pub capacity: usize,
    pub backlog_capacity: usize,
    pub barrier_pending: bool,
    /// The VM has run past the interaction at `cursor` (it is waiting, ended
    /// or in error): the screen still shows `cursor`, so a rollback of one
    /// step re-shows it instead of going before it.
    pub ahead: bool,
}

impl Default for History {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            cursor: 0,
            backlog: VecDeque::new(),
            next_seq: 0,
            capacity: DEFAULT_HISTORY_CAPACITY,
            backlog_capacity: DEFAULT_BACKLOG_CAPACITY,
            barrier_pending: false,
            ahead: false,
        }
    }
}

fn same_interaction(a: &Option<Interaction>, b: &Option<Interaction>) -> bool {
    match (a, b) {
        (Some(Interaction::Say { node: x, .. }), Some(Interaction::Say { node: y, .. }))
        | (Some(Interaction::Choice { node: x, .. }), Some(Interaction::Choice { node: y, .. }))
        | (Some(Interaction::Screen { node: x, .. }), Some(Interaction::Screen { node: y, .. })) => {
            x == y
        }
        _ => false,
    }
}

impl History {
    /// Drops everything and keeps capacities.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.cursor = 0;
        self.backlog.clear();
        self.next_seq = 0;
        self.barrier_pending = false;
        self.ahead = false;
    }

    fn current(&self) -> Option<&Checkpoint> {
        self.entries.get(self.cursor)
    }

    /// Cuts the entries after the cursor (a different choice was made).
    fn truncate_future(&mut self) {
        let Some(seq) = self.current().map(|c| c.seq) else {
            return;
        };
        self.entries.truncate(self.cursor + 1);
        self.next_seq = seq + 1;
        while self.backlog.back().is_some_and(|b| b.seq > seq) {
            self.backlog.pop_back();
        }
    }

    /// A variable changed outside of a node: records it in the checkpoint
    /// on screen and cuts the known future.
    pub fn patch_cursor_var(&mut self, name: &str, value: &orbit_core::Value) {
        self.patch_cursor(|state| {
            state.vars.insert(name.to_owned(), value.clone());
        });
    }

    /// The state changed outside of a node (a variable, a screen closed by
    /// the player): applies the change to the checkpoint on screen and cuts
    /// the known future.
    pub fn patch_cursor(&mut self, patch: impl FnOnce(&mut VmState)) {
        if self.entries.is_empty() {
            return;
        }
        let index = self.cursor.min(self.entries.len() - 1);
        self.cursor = index;
        patch(&mut self.entries[index].state);
        self.truncate_future();
    }

    /// The screen the story waited for was closed: like an answered choice,
    /// one rollback step reopens it, and the known future (which followed
    /// from another closing) is cut.
    pub fn on_screen_closed(&mut self) {
        self.ahead = true;
        if !self.entries.is_empty() {
            self.cursor = self.cursor.min(self.entries.len() - 1);
            self.truncate_future();
        }
    }

    /// Index of the oldest entry rollback may reach.
    pub fn floor(&self) -> usize {
        if self.entries.is_empty() {
            return 0;
        }
        (0..=self.cursor.min(self.entries.len() - 1))
            .rev()
            .find(|&i| self.entries[i].barrier)
            .unwrap_or(0)
    }

    /// The VM jumped to another scene from outside a node: the checkpoint on
    /// screen stays the rollback point, the known future is cut, and one
    /// rollback step re-shows it.
    pub fn on_goto(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.cursor = self.cursor.min(self.entries.len() - 1);
        self.truncate_future();
        self.ahead = true;
    }

    /// Called when an interaction was just presented by `step()`.
    pub fn record(&mut self, state: &VmState) {
        self.ahead = false;
        let say = match &state.shown {
            Some(Interaction::Say {
                node,
                speaker,
                string_id,
            }) => Some((*node, *speaker, *string_id)),
            _ => None,
        };

        if self.capacity > 0 {
            // Replaying a known future: just move the cursor.
            if let Some(next) = self.entries.get(self.cursor + 1)
                && !self.entries.is_empty()
                && same_interaction(&next.state.shown, &state.shown)
            {
                self.cursor += 1;
                return;
            }
            if !self.entries.is_empty() {
                self.truncate_future();
            }
            let seq = self.next_seq;
            self.next_seq += 1;
            let barrier = std::mem::take(&mut self.barrier_pending);
            self.entries.push_back(Checkpoint {
                seq,
                state: state.clone(),
                barrier,
                chosen: None,
            });
            while self.entries.len() > self.capacity {
                self.entries.pop_front();
            }
            self.cursor = self.entries.len() - 1;
            if let Some((node, speaker, string_id)) = say {
                self.push_backlog(seq, node, speaker, string_id);
            }
        } else if let Some((node, speaker, string_id)) = say {
            let seq = self.next_seq;
            self.next_seq += 1;
            self.push_backlog(seq, node, speaker, string_id);
        }
    }

    fn push_backlog(
        &mut self,
        seq: u64,
        node: NodeId,
        speaker: Option<CharacterId>,
        string_id: StringId,
    ) {
        self.backlog.push_back(BacklogEntry {
            seq,
            node,
            speaker,
            string_id,
        });
        while self.backlog.len() > self.backlog_capacity {
            self.backlog.pop_front();
        }
    }

    /// Called by `choose` with the pending choice node and the picked index.
    pub fn on_choose(&mut self, node: NodeId, index: usize) {
        // The screen now shows an answered choice: a rollback of one step
        // re-shows it.
        self.ahead = true;
        let at_choice = matches!(
            self.current().and_then(|c| c.state.shown.as_ref()),
            Some(Interaction::Choice { node: n, .. }) if *n == node
        );
        if !at_choice {
            return;
        }
        if self.entries[self.cursor].chosen == Some(index) {
            return; // same branch: the known future stays valid
        }
        self.truncate_future();
        self.entries[self.cursor].chosen = Some(index);
    }

    /// Cursor index a rollback of `steps` would reach, if it moves anything.
    pub fn rollback_target(&self, steps: usize) -> Option<usize> {
        if steps == 0 || self.entries.is_empty() {
            return None;
        }
        let steps = if self.ahead { steps - 1 } else { steps };
        let target = self.cursor.saturating_sub(steps).max(self.floor());
        (target != self.cursor || self.ahead).then_some(target)
    }

    pub fn forward_target(&self, steps: usize) -> Option<usize> {
        if steps == 0 || self.entries.is_empty() {
            return None;
        }
        let target = (self.cursor + steps).min(self.entries.len() - 1);
        (target != self.cursor).then_some(target)
    }

    /// Moves the cursor and returns the state to restore.
    pub fn move_to(&mut self, index: usize) -> VmState {
        self.cursor = index;
        self.ahead = false;
        self.barrier_pending = false;
        self.entries[index].state.clone()
    }

    pub fn index_of(&self, seq: u64) -> Option<usize> {
        let first = self.entries.front()?.seq;
        let idx = usize::try_from(seq.checked_sub(first)?).ok()?;
        self.entries.get(idx).filter(|c| c.seq == seq).map(|_| idx)
    }

    /// Backlog up to the cursor, oldest first.
    pub fn backlog_items(&self) -> Vec<BacklogItem> {
        let cursor_seq = self.current().map(|c| c.seq);
        let floor_seq = self.entries.get(self.floor()).map(|c| c.seq);
        self.backlog
            .iter()
            .filter(|b| cursor_seq.is_none_or(|c| b.seq <= c))
            .map(|b| BacklogItem {
                node: b.node,
                speaker: b.speaker,
                string_id: b.string_id,
                checkpoint: floor_seq
                    .filter(|f| b.seq >= *f && self.index_of(b.seq).is_some())
                    .map(|_| b.seq),
            })
            .collect()
    }

    pub fn snapshot(&self, max_checkpoints: usize, max_backlog: usize) -> RollbackHistory {
        let Some(cur) = self.current() else {
            return RollbackHistory {
                checkpoints: Vec::new(),
                backlog: self.backlog.iter().cloned().collect(),
                next_seq: self.next_seq,
            };
        };
        let end = self.cursor + 1;
        let start = end.saturating_sub(max_checkpoints.max(1));
        let checkpoints: Vec<Checkpoint> = self.entries.range(start..end).cloned().collect();
        let cur_seq = cur.seq;
        let mut backlog: Vec<BacklogEntry> = self
            .backlog
            .iter()
            .filter(|b| b.seq <= cur_seq)
            .cloned()
            .collect();
        let excess = backlog.len().saturating_sub(max_backlog);
        backlog.drain(..excess);
        RollbackHistory {
            checkpoints,
            backlog,
            next_seq: cur_seq + 1,
        }
    }
}
