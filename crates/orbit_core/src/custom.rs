//! Open data carried by plugin-defined nodes (`NodeKind::Custom`).
//!
//! A deliberately small value tree instead of `ron::Value`: it serializes to
//! readable RON *and* to `postcard` (which is not self-describing), with
//! explicit variant names, so a node whose plugin is absent survives a
//! load / save round trip unchanged.

use serde::{Deserialize, Serialize};

/// A JSON-like value. Maps keep insertion order (a `Vec` of pairs) so saved
/// files stay diff-friendly and stable.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum CustomData {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<CustomData>),
    Map(Vec<(String, CustomData)>),
}

impl CustomData {
    /// Value of `key` when `self` is a map.
    pub fn get(&self, key: &str) -> Option<&CustomData> {
        match self {
            CustomData::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            CustomData::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            CustomData::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            CustomData::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_lookup() {
        let data = CustomData::Map(vec![
            ("a".into(), CustomData::Int(3)),
            ("b".into(), CustomData::Str("x".into())),
        ]);
        assert_eq!(data.get("a").and_then(CustomData::as_int), Some(3));
        assert_eq!(data.get("b").and_then(CustomData::as_str), Some("x"));
        assert!(data.get("c").is_none());
        assert!(CustomData::Null.get("a").is_none());
    }
}
