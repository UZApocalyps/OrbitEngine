//! Data that lives outside any playthrough: persistent variables, the set of
//! read lines, and player preferences.
//!
//! Both are RON files (`persistent.ron`, `prefs.ron`): human readable and
//! tolerant to evolution thanks to `#[serde(default)]` on every field (an
//! old file simply lacks the new fields). They are independent of the saves:
//! deleting every slot never resets them.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use orbit_core::{NodeId, SceneId, Value};
use orbit_vm::Vm;
use serde::{Deserialize, Serialize};

use crate::error::SaveError;

/// Storage key of [`PersistentData`].
pub const PERSISTENT_KEY: &str = "persistent.ron";
/// Storage key of [`Preferences`].
pub const PREFS_KEY: &str = "prefs.ron";

/// Progress shared by all playthroughs of a story.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistentData {
    #[serde(default)]
    pub format_version: u32,
    /// Values of the `VarScope::Persistent` variables.
    #[serde(default)]
    pub vars: BTreeMap<String, Value>,
    /// Lines already read in any playthrough (for "skip read text").
    #[serde(default)]
    pub read_nodes: BTreeSet<NodeId>,
    /// Scenes entered in any playthrough (the scene replay menu, M7-04),
    /// with the playthrough variables of the first entry (what a replay
    /// starts from). Files written before this field existed have none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub seen_scenes: BTreeMap<SceneId, BTreeMap<String, Value>>,
    /// Asset paths of the images shown as a background in any playthrough
    /// (the gallery unlocks, M7-03).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub unlocked_images: BTreeSet<String>,
}

impl PersistentData {
    pub const FORMAT_VERSION: u32 = 1;

    /// Collects the persistent state from a running VM. Never call it on a
    /// replay VM (`Vm::is_replay`): a replay must not write anything.
    pub fn from_vm(vm: &Vm) -> Self {
        let story = vm.story();
        Self {
            format_version: Self::FORMAT_VERSION,
            vars: vm.persistent_vars().clone(),
            read_nodes: vm.read_nodes().clone(),
            seen_scenes: vm.scene_visits().clone(),
            unlocked_images: vm
                .shown_images()
                .iter()
                .filter_map(|id| story.assets.get(*id).map(|a| a.path.clone()))
                .collect(),
        }
    }

    /// Whether the scene `id` was entered in a playthrough.
    pub fn has_seen(&self, id: SceneId) -> bool {
        self.seen_scenes.contains_key(&id)
    }

    /// Whether the image at `path` was shown in a playthrough.
    pub fn is_unlocked(&self, path: &str) -> bool {
        self.unlocked_images.contains(path)
    }

    /// Loads this data into `vm`. Variables that are no longer declared (or
    /// changed type) and read lines whose node was deleted are ignored.
    pub fn apply_to(&self, vm: &mut Vm) {
        vm.set_persistent_vars(&self.vars);
        let existing: HashSet<NodeId> = vm
            .story()
            .scenes
            .iter()
            .flat_map(|s| s.nodes.iter().map(|n| n.id))
            .collect();
        let read = self
            .read_nodes
            .iter()
            .filter(|n| existing.contains(n))
            .copied()
            .collect();
        vm.set_read_nodes(read);
        let story = vm.story().clone();
        let visits = self
            .seen_scenes
            .iter()
            .filter(|(id, _)| story.scene(**id).is_some())
            .map(|(id, vars)| (*id, vars.clone()))
            .collect();
        vm.set_scene_visits(visits);
        let images = story
            .assets
            .assets
            .iter()
            .filter(|a| self.unlocked_images.contains(&a.path))
            .map(|a| a.id)
            .collect();
        vm.set_shown_images(images);
    }

    pub fn to_ron(&self) -> Result<String, SaveError> {
        let cfg = ron::ser::PrettyConfig::new().indentor("  ".to_owned());
        ron::ser::to_string_pretty(self, cfg).map_err(|e| SaveError::Encode(e.to_string()))
    }

    pub fn from_ron(text: &str) -> Result<Self, SaveError> {
        ron::from_str(text).map_err(|e| SaveError::Corrupt(format!("persistent data: {e}")))
    }
}

/// What "skip" fast-forwards through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkipMode {
    /// Only lines already read (in any playthrough).
    #[default]
    ReadOnly,
    /// Every line.
    All,
}

/// Linear volumes in `0.0..=1.0` for each audio bus.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BusVolumes {
    pub master: f32,
    pub music: f32,
    pub sfx: f32,
    pub voice: f32,
}

impl Default for BusVolumes {
    fn default() -> Self {
        Self {
            master: 1.0,
            music: 0.8,
            sfx: 1.0,
            voice: 1.0,
        }
    }
}

/// Player preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub volumes: BusVolumes,
    /// Text reveal speed in characters per second; `0.0` shows lines
    /// instantly.
    pub text_speed: f32,
    /// Seconds to wait after a line is complete before auto-advancing.
    pub auto_delay: f32,
    pub skip_mode: SkipMode,
    pub fullscreen: bool,
    /// BCP 47 tag of the chosen language; `None` = the system / source
    /// language.
    pub language: Option<String>,
    /// Gamepad remapping: action name (see [`GAMEPAD_ACTIONS`]) to button
    /// name. Missing or unknown entries use the default binding, so files
    /// written before this field existed load unchanged.
    pub gamepad: BTreeMap<String, String>,
    /// Screen reader support (ADR 0017, M7-09): the accessibility tree and
    /// the announced dialogue lines. On by default: it is invisible, costs
    /// nothing until an assistive technology asks for it (desktop), and a
    /// blind player could not find the option to turn it on. Files written
    /// before this field existed load with it on.
    pub screen_reader: bool,
    /// Accessibility (M7-07): multiplier of the dialogue, choice and UI text
    /// sizes, inside [`TEXT_SCALE_RANGE`]. `1.0` = the theme's sizes (the
    /// value of files written before this field existed).
    pub text_scale: f32,
    /// Accessibility: replaces the theme colors by a high-contrast set.
    pub high_contrast: bool,
    /// Accessibility: finite animations, shakes and transitions are cut
    /// (the final state is reached at once).
    pub reduced_motion: bool,
}

/// Allowed [`Preferences::text_scale`] (the same range as
/// `orbit_ui_data::TEXT_SCALE_RANGE`).
pub const TEXT_SCALE_RANGE: [f32; 2] = [1.0, 1.6];

/// The remappable gamepad actions with their default button.
pub const GAMEPAD_ACTIONS: [(&str, &str); 7] = [
    ("advance", "South"),
    ("back", "East"),
    ("menu", "Start"),
    ("skip", "RightBumper"),
    ("auto", "North"),
    ("rollback", "LeftBumper"),
    ("history", "West"),
];

impl Default for Preferences {
    fn default() -> Self {
        Self {
            volumes: BusVolumes::default(),
            text_speed: 40.0,
            auto_delay: 2.0,
            skip_mode: SkipMode::default(),
            fullscreen: false,
            language: None,
            gamepad: BTreeMap::new(),
            screen_reader: true,
            text_scale: 1.0,
            high_contrast: false,
            reduced_motion: false,
        }
    }
}

impl Preferences {
    /// Clamps every value to a sane range (a hand-edited or corrupt file must
    /// not break the game): volumes in `0..=1`, speed and delay finite and
    /// non-negative, delay at most 60 s.
    pub fn sanitized(mut self) -> Self {
        fn unit(v: f32) -> f32 {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                1.0
            }
        }
        fn non_negative(v: f32, default: f32, max: f32) -> f32 {
            if v.is_finite() {
                v.clamp(0.0, max)
            } else {
                default
            }
        }
        self.volumes.master = unit(self.volumes.master);
        self.volumes.music = unit(self.volumes.music);
        self.volumes.sfx = unit(self.volumes.sfx);
        self.volumes.voice = unit(self.volumes.voice);
        self.text_speed = non_negative(self.text_speed, 40.0, 10_000.0);
        self.auto_delay = non_negative(self.auto_delay, 2.0, 60.0);
        self.text_scale = if self.text_scale.is_finite() {
            self.text_scale.clamp(TEXT_SCALE_RANGE[0], TEXT_SCALE_RANGE[1])
        } else {
            1.0
        };
        self
    }

    pub fn to_ron(&self) -> Result<String, SaveError> {
        let cfg = ron::ser::PrettyConfig::new().indentor("  ".to_owned());
        ron::ser::to_string_pretty(self, cfg).map_err(|e| SaveError::Encode(e.to_string()))
    }

    pub fn from_ron(text: &str) -> Result<Self, SaveError> {
        ron::from_str::<Self>(text)
            .map(Self::sanitized)
            .map_err(|e| SaveError::Corrupt(format!("preferences: {e}")))
    }
}
