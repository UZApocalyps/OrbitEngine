//! In-game notifications ("toasts"): the [`Notify`] node and the `[var]`
//! interpolation of their text.
//!
//! A notification is transient: it is not part of the VM state, saves or
//! rollback; the host shows it once when the node runs.

use serde::{Deserialize, Serialize};

use crate::ids::{AssetId, StringId};

/// A non-blocking notification (`NodeKind::Notify`): the story goes on at
/// once while the host shows a toast.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Notify {
    /// Localized text (markup allowed, `[variable]` replaced by its value).
    pub text_id: StringId,
    /// An image shown left of the text.
    #[serde(default)]
    pub icon: Option<AssetId>,
    /// A sound played with the toast (else the style's / theme's default).
    #[serde(default)]
    pub sound: Option<AssetId>,
    /// Seconds on screen (else the style's / theme's duration).
    #[serde(default)]
    pub duration: Option<f32>,
    /// A named notification style of the theme (`None`: `default`).
    #[serde(default)]
    pub style: Option<String>,
}

impl Notify {
    /// The assets the node refers to (icon, sound).
    pub fn assets(&self) -> Vec<AssetId> {
        self.icon.iter().chain(&self.sound).copied().collect()
    }
}

/// Replaces every `[name]` of `text` with `value(name)`; `[[` is a literal
/// `[`. A name `value` does not know (or a malformed bracket) is kept as
/// written, so a typo shows on screen. Only names made of letters, digits,
/// `_` and `.` are looked up.
pub fn interpolate_vars(text: &str, mut value: impl FnMut(&str) -> Option<String>) -> String {
    if !text.contains('[') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('[') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        if let Some(after) = tail.strip_prefix('[') {
            out.push('[');
            rest = after;
            continue;
        }
        match tail.find(']') {
            Some(end)
                if end > 0
                    && tail[..end]
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.') =>
            {
                let name = &tail[..end];
                match value(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('[');
                        out.push_str(name);
                        out.push(']');
                    }
                }
                rest = &tail[end + 1..];
            }
            _ => {
                out.push('[');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_replaces_known_names_only() {
        let v = |n: &str| (n == "gold").then(|| "12".to_owned());
        assert_eq!(interpolate_vars("You have [gold] coins", v), "You have 12 coins");
        assert_eq!(interpolate_vars("[nope] [gold]", v), "[nope] 12");
        assert_eq!(interpolate_vars("[[gold] [ x ] [", v), "[gold] [ x ] [");
        assert_eq!(interpolate_vars("no brackets", v), "no brackets");
    }
}
