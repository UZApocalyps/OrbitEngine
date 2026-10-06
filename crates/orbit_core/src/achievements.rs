//! Achievements (schema v16, M7-05) and the layout switch for game packs
//! written before them (also M7-06: the voice of a line).

use serde::{Deserialize, Deserializer, Serialize};

use crate::ids::{AchievementId, AssetId, StringId};

/// An achievement the player can unlock: shown by the data-driven
/// achievements screen and announced by a toast when a
/// [`crate::NodeKind::Unlock`] node unlocks it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Achievement {
    pub id: AchievementId,
    /// Title, a key of the string table (localizable).
    pub title_id: StringId,
    /// Description, a key of the string table (localizable).
    pub description_id: StringId,
    /// Hidden until unlocked: the screen shows a placeholder instead of the
    /// title and description.
    #[serde(default)]
    pub hidden: bool,
    /// Icon (an `Image` asset); `None`: a default medal.
    #[serde(default)]
    pub icon: Option<AssetId>,
    /// Icon shown while the achievement is locked (an Image asset);
    /// `None`: the `icon` (the screen and the stock look dim it). Added
    /// after schema v16 as an optional field (RON files without it load
    /// unchanged; game pack version 7 carries it, see
    /// [with_legacy_v16_layout]).
    #[serde(default, deserialize_with = "v17_field")]
    pub locked_icon: Option<AssetId>,
}

thread_local! {
    static LEGACY_V15_LAYOUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` (a binary decode of a game pack of version 1 to 5) with the
/// layout those packs were written with: before schema v16 `Say` had no
/// `voice`, `Story` no `achievements` and a translation no `voices`.
/// Postcard is not self-describing, so the reader has to be told; frozen,
/// never remove (backward compatibility).
pub fn with_legacy_v15_layout<R>(f: impl FnOnce() -> R) -> R {
    let before = LEGACY_V15_LAYOUT.with(|c| c.replace(true));
    let out = f();
    LEGACY_V15_LAYOUT.with(|c| c.set(before));
    out
}

/// Whether the binary decode in progress reads the layout of packs written
/// before schema v16 (see [`with_legacy_v15_layout`]).
pub fn is_legacy_v15_layout() -> bool {
    LEGACY_V15_LAYOUT.with(std::cell::Cell::get)
}

/// `serialize_with` of the fields added by schema v16: encoding in the
/// legacy layout (a pack that uses none of them, which older players must
/// keep reading) writes nothing for them (postcard writes no byte for a
/// unit).
pub fn v16_skip<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: Serialize,
{
    if !serializer.is_human_readable() && is_legacy_v15_layout() {
        return serializer.serialize_unit();
    }
    value.serialize(serializer)
}

/// `deserialize_with` of the fields added by schema v16: a binary decode of
/// an older layout does not read them (they take their default).
pub fn v16_field<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    if !deserializer.is_human_readable() && is_legacy_v15_layout() {
        return Ok(T::default());
    }
    T::deserialize(deserializer)
}

thread_local! {
    static LEGACY_V16_LAYOUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs  (a binary encode or decode of a game pack of version 6 or
/// older) with the layout of those packs: no `Achievement::locked_icon`.
/// Frozen, never remove (backward compatibility).
pub fn with_legacy_v16_layout<R>(f: impl FnOnce() -> R) -> R {
    let before = LEGACY_V16_LAYOUT.with(|c| c.replace(true));
    let out = f();
    LEGACY_V16_LAYOUT.with(|c| c.set(before));
    out
}

fn is_legacy_v16_layout() -> bool {
    LEGACY_V16_LAYOUT.with(std::cell::Cell::get) || is_legacy_v15_layout()
}

/// `deserialize_with` of the fields added after schema v16: a binary
/// decode of an older layout does not read them.
pub fn v17_field<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    if !deserializer.is_human_readable() && is_legacy_v16_layout() {
        return Ok(T::default());
    }
    T::deserialize(deserializer)
}
/// Written by hand: `locked_icon` is left out of RON files when unset and of
/// the binary layout of game packs older than version 7.
impl Serialize for Achievement {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let write_locked = if serializer.is_human_readable() {
            self.locked_icon.is_some()
        } else {
            !is_legacy_v16_layout()
        };
        let mut st = serializer.serialize_struct("Achievement", 5 + usize::from(write_locked))?;
        st.serialize_field("id", &self.id)?;
        st.serialize_field("title_id", &self.title_id)?;
        st.serialize_field("description_id", &self.description_id)?;
        st.serialize_field("hidden", &self.hidden)?;
        st.serialize_field("icon", &self.icon)?;
        if write_locked {
            st.serialize_field("locked_icon", &self.locked_icon)?;
        } else {
            st.skip_field("locked_icon")?;
        }
        st.end()
    }
}

/// Whether a game pack must carry the fields added after schema v16 (pack
/// version 7): an achievement with a locked icon.
pub fn uses_v17(achievements: &[Achievement]) -> bool {
    achievements.iter().any(|a| a.locked_icon.is_some())
}
