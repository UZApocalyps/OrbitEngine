//! Data that lives outside any playthrough: persistent variables, the set of
//! read lines, and player preferences.
//!
//! Both are RON files (`persistent.ron`, `prefs.ron`): human readable and
//! tolerant to evolution thanks to `#[serde(default)]` on every field (an
//! old file simply lacks the new fields). They are independent of the saves:
//! deleting every slot never resets them.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use orbit_core::{NodeId, Value};
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
}

impl PersistentData {
    pub const FORMAT_VERSION: u32 = 1;

    /// Collects the persistent state from a running VM.
    pub fn from_vm(vm: &Vm) -> Self {
        Self {
            format_version: Self::FORMAT_VERSION,
            vars: vm.persistent_vars().clone(),
            read_nodes: vm.read_nodes().clone(),
        }
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
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            volumes: BusVolumes::default(),
            text_speed: 40.0,
            auto_delay: 2.0,
            skip_mode: SkipMode::default(),
            fullscreen: false,
            language: None,
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
