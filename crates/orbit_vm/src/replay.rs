//! Scene replay (M7-04) and what a playthrough unlocks for the gallery and
//! the replay menu (M7-03).
//!
//! A normal VM records, outside its [`crate::VmState`] (so saves and
//! rollback never see it), like the read lines:
//!
//! * the scenes entered, each with the playthrough variables it was first
//!   entered with (`scene_visits`): the replay menu lists seen scenes, and a
//!   replay starts from those values;
//! * the images shown as a background (`shown_images`): the gallery unlocks.
//!
//! The host persists them (`orbit_save::PersistentData`).
//!
//! A replay VM ([`Vm::replay`]) starts at the first node of one scene with
//! its own copy of the variables and records nothing. Calls into other
//! scenes run normally; leaving the scene otherwise (a jump to another
//! scene, the end of the scene or of the story) ends the replay: `step()`
//! returns [`StepResult::End`]. The host must not save or persist anything
//! from a replay VM ([`Vm::is_replay`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use orbit_core::{AssetId, SceneId, Story, Value, VarScope};

use crate::error::VmError;
use crate::event::StepResult;
use crate::state::Pending;
use crate::vm::Vm;

/// What a VM tracks about scenes and images (not part of the state).
#[derive(Clone, Debug, Default)]
pub(crate) struct Tracking {
    /// Index of the scene of the last executed node.
    current: Option<usize>,
    /// Replay mode: the scene being replayed.
    replay: Option<SceneId>,
    /// Scenes entered, with the playthrough variables at the first entry.
    visits: BTreeMap<SceneId, BTreeMap<String, Value>>,
    /// Images shown as a background.
    images: BTreeSet<AssetId>,
}

impl Vm {
    /// A VM replaying `scene` (M7-04): it starts at the scene's first node
    /// with the playthrough variables of `vars` — the values recorded when
    /// the scene was first entered ([`Vm::scene_visits`]) — or, for the
    /// variables missing there (or of another type), their declared initial
    /// values. Persistent variables start from `persistent` (a copy: the host
    /// must not write them back). Nothing is recorded (seen scenes, images).
    ///
    /// Empty or unknown scene: an error.
    pub fn replay(
        story: impl Into<Arc<Story>>,
        scene: SceneId,
        vars: Option<&BTreeMap<String, Value>>,
        persistent: &BTreeMap<String, Value>,
    ) -> Result<Self, VmError> {
        let story: Arc<Story> = story.into();
        let entry = story
            .scenes
            .iter()
            .find(|s| s.id == scene)
            .ok_or(VmError::UnknownScene(scene))?
            .nodes
            .first()
            .map(|n| n.id)
            .ok_or(VmError::EmptyStory)?;
        let mut vm = Vm::new(story, entry)?;
        if let Some(vars) = vars {
            for decl in vm.story.variables.iter().filter(|v| v.scope == VarScope::Game) {
                if let Some(v) = vars
                    .get(&decl.name)
                    .filter(|v| v.value_type() == decl.initial.value_type())
                {
                    vm.state.vars.insert(decl.name.clone(), v.clone());
                }
            }
        }
        vm.set_persistent_vars(persistent);
        vm.tracking.replay = Some(scene);
        Ok(vm)
    }

    /// The scene this VM replays (`None`: a normal playthrough).
    pub fn replay_scene(&self) -> Option<SceneId> {
        self.tracking.replay
    }

    /// Whether this VM replays a scene: nothing it does may be saved.
    pub fn is_replay(&self) -> bool {
        self.tracking.replay.is_some()
    }

    /// Scenes entered (this session and, through
    /// [`Vm::set_scene_visits`], before), with the playthrough variables of
    /// the first entry.
    pub fn scene_visits(&self) -> &BTreeMap<SceneId, BTreeMap<String, Value>> {
        &self.tracking.visits
    }

    /// Replaces the recorded scene visits (loaded from persistent storage).
    pub fn set_scene_visits(&mut self, visits: BTreeMap<SceneId, BTreeMap<String, Value>>) {
        self.tracking.visits = visits;
    }

    /// Images shown as a background (the gallery unlocks).
    pub fn shown_images(&self) -> &BTreeSet<AssetId> {
        &self.tracking.images
    }

    /// Replaces the recorded images (loaded from persistent storage).
    pub fn set_shown_images(&mut self, images: BTreeSet<AssetId>) {
        self.tracking.images = images;
    }

    pub(crate) fn record_image(&mut self, asset: AssetId) {
        if self.tracking.replay.is_none() {
            self.tracking.images.insert(asset);
        }
    }

    /// Called before running a node of scene `index`: records a first visit,
    /// or ends a replay that leaves its scene (returns the `End` to report).
    pub(crate) fn enter_scene(&mut self, index: usize) -> Option<StepResult> {
        if self.tracking.current == Some(index) {
            return None;
        }
        let id = self.story.scenes[index].id;
        match self.tracking.replay {
            // A call into another scene comes back: only a real exit ends.
            Some(replayed) if replayed != id && self.state.call_stack.is_empty() => {
                self.state.pc = None;
                self.state.pending = Pending::None;
                self.tracking.current = None;
                return Some(StepResult::End);
            }
            Some(_) => {}
            None => {
                if !self.tracking.visits.contains_key(&id) {
                    self.tracking.visits.insert(id, self.state.vars.clone());
                }
            }
        }
        self.tracking.current = Some(index);
        None
    }
}
