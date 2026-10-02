//! Slots (manual, autosave rotation, quicksave) on top of a [`Storage`].

use std::fmt;
use std::sync::Arc;

use orbit_core::Story;

use crate::compat::{Compat, check_compat, check_meta};
use crate::data::{SaveData, SaveMeta};
use crate::error::SaveError;
use crate::persistent::{PERSISTENT_KEY, PREFS_KEY, PersistentData, Preferences};
use crate::storage::Storage;

/// A save slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SlotId {
    /// Manual slot, numbered from 1.
    Manual(u32),
    /// Autosave; `Auto(0)` is the most recent, `Auto(1)` the one before...
    Auto(u32),
    Quick,
}

impl SlotId {
    fn key(self) -> String {
        match self {
            SlotId::Manual(n) => format!("saves/manual_{n:03}.sav"),
            SlotId::Auto(n) => format!("saves/auto_{n}.sav"),
            SlotId::Quick => "saves/quick.sav".to_owned(),
        }
    }
}

impl fmt::Display for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlotId::Manual(n) => write!(f, "manual #{n}"),
            SlotId::Auto(n) => write!(f, "autosave #{n}"),
            SlotId::Quick => f.write_str("quicksave"),
        }
    }
}

/// Slot counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveConfig {
    /// Number of manual slots (`Manual(1)..=Manual(n)`).
    pub manual_slots: u32,
    /// Number of autosaves kept in rotation.
    pub autosave_slots: u32,
}

impl Default for SaveConfig {
    fn default() -> Self {
        Self {
            manual_slots: 20,
            autosave_slots: 3,
        }
    }
}

/// What a slot contains, for the slot list.
#[derive(Clone, Debug, PartialEq)]
pub enum SlotState {
    Empty,
    /// A readable save, with its verdict against the current story
    /// (`Compat::Incompatible` slots are shown greyed out).
    Occupied {
        meta: Box<SaveMeta>,
        compat: Compat,
    },
    /// Unreadable data (damaged file). Offer to delete it.
    Corrupt(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlotInfo {
    pub slot: SlotId,
    pub state: SlotState,
}

/// A loaded save and its compatibility verdict (never `Incompatible`: those
/// are errors).
#[derive(Clone, Debug)]
pub struct LoadedSave {
    pub data: SaveData,
    pub compat: Compat,
}

/// High level save API for one game.
///
/// Storage layout (keys): `saves/manual_NNN.sav`, `saves/auto_K.sav`,
/// `saves/quick.sav`, `persistent.ron`, `prefs.ron`.
pub struct SaveManager {
    storage: Box<dyn Storage>,
    story: Arc<Story>,
    config: SaveConfig,
}

impl SaveManager {
    pub fn new(storage: Box<dyn Storage>, story: Arc<Story>, config: SaveConfig) -> Self {
        Self {
            storage,
            story,
            config,
        }
    }

    pub fn config(&self) -> SaveConfig {
        self.config
    }

    pub fn storage(&self) -> &dyn Storage {
        self.storage.as_ref()
    }

    /// Every slot the game offers, in display order: quicksave, autosaves
    /// (newest first), manual slots. Reads only the metadata of each save
    /// unless the story changed since (then it also checks the state).
    /// Never fails because of a bad slot: those are reported per slot.
    pub fn list(&self) -> Result<Vec<SlotInfo>, SaveError> {
        let mut slots = vec![SlotId::Quick];
        slots.extend((0..self.config.autosave_slots).map(SlotId::Auto));
        slots.extend((1..=self.config.manual_slots).map(SlotId::Manual));
        slots
            .into_iter()
            .map(|slot| {
                let state = match self.storage.read(&slot.key())? {
                    None => SlotState::Empty,
                    Some(bytes) => self.inspect(&bytes),
                };
                Ok(SlotInfo { slot, state })
            })
            .collect()
    }

    fn inspect(&self, bytes: &[u8]) -> SlotState {
        let meta = match SaveData::decode_meta(bytes) {
            Ok(meta) => meta,
            // A save from a newer game version is valid, just unusable here.
            Err(SaveError::Incompatible(reason)) => {
                return SlotState::Corrupt(reason);
            }
            Err(e) => return SlotState::Corrupt(e.to_string()),
        };
        let compat = match check_meta(&self.story, meta.format_version, meta.story_hash) {
            Some(verdict) => verdict,
            None => match SaveData::decode(bytes) {
                Ok(data) => check_compat(&self.story, &data),
                Err(e) => return SlotState::Corrupt(e.to_string()),
            },
        };
        SlotState::Occupied {
            meta: Box::new(meta),
            compat,
        }
    }

    fn check_slot(&self, slot: SlotId) -> Result<(), SaveError> {
        let ok = match slot {
            SlotId::Manual(n) => (1..=self.config.manual_slots).contains(&n),
            SlotId::Auto(n) => n < self.config.autosave_slots,
            SlotId::Quick => true,
        };
        if ok {
            Ok(())
        } else {
            Err(SaveError::InvalidSlot(slot))
        }
    }

    /// Writes `data` to `slot` (atomically; on failure, e.g. quota exceeded,
    /// the previous content of the slot is kept).
    pub fn save(&self, slot: SlotId, data: &SaveData) -> Result<(), SaveError> {
        self.check_slot(slot)?;
        self.storage.write(&slot.key(), &data.encode()?)?;
        Ok(())
    }

    pub fn quicksave(&self, data: &SaveData) -> Result<(), SaveError> {
        self.save(SlotId::Quick, data)
    }

    /// Stores `data` as the newest autosave: older ones shift by one and the
    /// oldest is dropped. Shifting happens oldest-first, so a failure at any
    /// point (e.g. quota) never loses the newest existing autosave.
    pub fn autosave(&self, data: &SaveData) -> Result<(), SaveError> {
        let k = self.config.autosave_slots;
        if k == 0 {
            return Err(SaveError::InvalidSlot(SlotId::Auto(0)));
        }
        // Encode first: if it fails nothing has moved.
        let bytes = data.encode()?;
        for i in (0..k - 1).rev() {
            if let Some(older) = self.storage.read(&SlotId::Auto(i).key())? {
                self.storage.write(&SlotId::Auto(i + 1).key(), &older)?;
            }
        }
        self.storage.write(&SlotId::Auto(0).key(), &bytes)?;
        Ok(())
    }

    /// Loads a slot. `Err(NotFound)` if empty, `Err(Corrupt)` if damaged,
    /// `Err(Incompatible)` if it cannot be used with the current story.
    /// `Ok` carries the verdict (`Compatible` or `CompatibleWithWarnings`).
    pub fn load(&self, slot: SlotId) -> Result<LoadedSave, SaveError> {
        self.check_slot(slot)?;
        let bytes = self
            .storage
            .read(&slot.key())?
            .ok_or(SaveError::NotFound(slot))?;
        let data = SaveData::decode(&bytes)?;
        match check_compat(&self.story, &data) {
            Compat::Incompatible(reason) => Err(SaveError::Incompatible(reason)),
            compat => Ok(LoadedSave { data, compat }),
        }
    }

    pub fn delete(&self, slot: SlotId) -> Result<(), SaveError> {
        self.check_slot(slot)?;
        self.storage.delete(&slot.key())?;
        Ok(())
    }

    /// The most recently created loadable save (for a "Continue" button).
    pub fn latest(&self) -> Result<Option<(SlotId, SaveMeta)>, SaveError> {
        Ok(self
            .list()?
            .into_iter()
            .filter_map(|info| match info.state {
                SlotState::Occupied { meta, compat } if compat.is_loadable() => {
                    Some((info.slot, *meta))
                }
                _ => None,
            })
            .max_by_key(|(_, meta)| meta.created_at_ms))
    }

    // ------------------------------------------------- persistent & prefs

    /// Persistent variables and read lines. A missing file gives the empty
    /// default; a damaged one is an error (the host can fall back to the
    /// default and overwrite it on the next save).
    pub fn load_persistent(&self) -> Result<PersistentData, SaveError> {
        match self.storage.read(PERSISTENT_KEY)? {
            None => Ok(PersistentData::default()),
            Some(bytes) => PersistentData::from_ron(&utf8(&bytes)?),
        }
    }

    pub fn save_persistent(&self, data: &PersistentData) -> Result<(), SaveError> {
        self.storage
            .write(PERSISTENT_KEY, data.to_ron()?.as_bytes())?;
        Ok(())
    }

    /// Preferences; defaults when missing. Values are sanitized.
    pub fn load_prefs(&self) -> Result<Preferences, SaveError> {
        match self.storage.read(PREFS_KEY)? {
            None => Ok(Preferences::default()),
            Some(bytes) => Preferences::from_ron(&utf8(&bytes)?),
        }
    }

    pub fn save_prefs(&self, prefs: &Preferences) -> Result<(), SaveError> {
        self.storage.write(PREFS_KEY, prefs.to_ron()?.as_bytes())?;
        Ok(())
    }
}

fn utf8(bytes: &[u8]) -> Result<String, SaveError> {
    String::from_utf8(bytes.to_vec()).map_err(|_| SaveError::Corrupt("not valid UTF-8".into()))
}
