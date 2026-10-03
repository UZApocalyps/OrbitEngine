//! Stable identifiers (ULID newtypes).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

/// Generates a fresh ULID using wall-clock time (via `web-time`, wasm safe)
/// and OS randomness.
fn new_ulid() -> Ulid {
    let ms = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut buf = [0u8; 16];
    // Randomness is required for uniqueness; failing to get any is unrecoverable.
    getrandom::fill(&mut buf).expect("OS random source unavailable");
    // ULID has 80 random bits; mask the rest.
    let random = u128::from_le_bytes(buf) & ((1u128 << 80) - 1);
    Ulid::from_parts(ms, random)
}

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Ulid);

        impl $name {
            /// Creates a brand new unique id.
            pub fn new() -> Self {
                Self(new_ulid())
            }

            /// Builds an id from its raw 128-bit value (useful for tests).
            pub const fn from_u128(v: u128) -> Self {
                Self(Ulid(v))
            }

            /// Raw 128-bit value.
            pub const fn as_u128(&self) -> u128 {
                self.0.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.0)
            }
        }

        // Serialized as the canonical 26-character ULID string.
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = String::deserialize(deserializer)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }

        #[cfg(feature = "schema")]
        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({
                    "type": "string",
                    "pattern": "^[0-9A-HJKMNP-TV-Z]{26}$",
                    "description": "Stable id: a 26-character ULID string."
                })
            }
        }

        impl FromStr for $name {
            type Err = ulid::DecodeError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ulid::from_string(s).map(Self)
            }
        }
    };
}

id_type!(
    /// Stable id of a story node. Saves, localization and diffs depend on it.
    NodeId
);
id_type!(
    /// Stable id of a scene.
    SceneId
);
id_type!(
    /// Stable id of a localizable string.
    StringId
);
id_type!(
    /// Stable id of a character definition.
    CharacterId
);
id_type!(
    /// Stable id of an asset (image, music, sound...). Nodes never reference
    /// a raw file path, only an `AssetId` resolved through the manifest.
    AssetId
);

id_type!(
    /// Stable id of a node inside a 3D scene or prefab (`.scene3d.ron`).
    Node3dId
);

id_type!(
    /// Stable id of an achievement (schema v16). Unlocks are persisted by id.
    AchievementId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_roundtrip_through_string() {
        let a = NodeId::new();
        let b = NodeId::new();
        assert_ne!(a, b);
        let parsed: NodeId = a.to_string().parse().unwrap();
        assert_eq!(a, parsed);
    }

    #[test]
    fn invalid_string_is_rejected() {
        assert!("not-a-ulid".parse::<NodeId>().is_err());
    }
}
