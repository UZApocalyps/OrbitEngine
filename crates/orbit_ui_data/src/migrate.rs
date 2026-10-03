//! Versioning of screen documents. Like the story schema, migrations work
//! on the RON text and run one version step at a time. There is only
//! version 1 so far: the table is the skeleton future versions plug into.

use std::borrow::Cow;

use crate::widget::UI_VERSION;

/// The `ui_version` of a document (1 when the field is missing).
pub fn read_version(source: &str) -> Result<u32, String> {
    // A tiny scan keeps the header independent of the rest of the schema,
    // which an older document may not match anymore.
    let Some(at) = source.find("ui_version") else {
        return Ok(1);
    };
    let rest = source[at + "ui_version".len()..].trim_start();
    let rest = rest
        .strip_prefix(':')
        .ok_or("`ui_version` must be followed by `:`")?
        .trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits
        .parse()
        .map_err(|_| "`ui_version` must be a number".to_owned())
}

/// One migration step: rewrites a document from version `N` to `N + 1`.
pub type Step = fn(&str) -> Result<String, String>;

/// Steps indexed by source version: `STEPS[0]` migrates v1 -> v2.
const STEPS: &[Step] = &[v1_to_v2, v2_to_v3, v3_to_v4];

/// v3 -> v4 (UI-14 / UI-15 / UI-16, ADR 0016): `ScreenAsset::hud`,
/// `transition_in` / `transition_out`, `WidgetKind::Repeat`, the
/// `BindPath::{Item, ItemIndex, ItemKey, Param}` bindings and
/// `UiAction::CloseWith`. All optional or new variants (the plain `Close`
/// is unchanged), so a document only changes its version number.
fn v3_to_v4(source: &str) -> Result<String, String> {
    Ok(bump_version(source, 3, 4))
}

/// v1 -> v2 (UI-12): everything added is optional (`col_span`, `row_span`,
/// `Fill::NineSlice`, `Dropdown::values`, `Theme::fonts`), so a document
/// only changes its version number.
fn v1_to_v2(source: &str) -> Result<String, String> {
    Ok(bump_version(source, 1, 2))
}

/// v2 -> v3 (M7-07): `SettingKey` gained `ScreenReader`, `TextScale`,
/// `HighContrast` and `ReducedMotion` (new bindings only; no existing
/// document changes meaning), so a document only changes its version number.
fn v2_to_v3(source: &str) -> Result<String, String> {
    Ok(bump_version(source, 2, 3))
}

/// Rewrites the first `ui_version: from` to `ui_version: to` (a document
/// without the field already reads as the current version).
fn bump_version(source: &str, from: u32, to: u32) -> String {
    let Some(at) = source.find("ui_version") else {
        return source.to_owned();
    };
    let after = at + "ui_version".len();
    let tail = &source[after..];
    let Some(colon) = tail.find(':') else {
        return source.to_owned();
    };
    let rest = &tail[colon + 1..];
    let skipped = rest.len() - rest.trim_start().len();
    let digits_at = after + colon + 1 + skipped;
    let digits: String = source[digits_at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits != from.to_string() {
        return source.to_owned();
    }
    format!(
        "{}{to}{}",
        &source[..digits_at],
        &source[digits_at + digits.len()..]
    )
}

/// Brings `source` (declared at `from`) up to [`UI_VERSION`].
pub fn migrate(from: u32, source: &str) -> Result<Cow<'_, str>, String> {
    migrate_with(from, source, UI_VERSION, STEPS)
}

fn migrate_with<'a>(
    from: u32,
    source: &'a str,
    current: u32,
    steps: &[Step],
) -> Result<Cow<'a, str>, String> {
    if from == 0 || from > current {
        return Err(format!(
            "ui version {from} is not supported (this build reads 1..={current})"
        ));
    }
    let mut text = Cow::Borrowed(source);
    for version in from..current {
        let step = steps
            .get(version as usize - 1)
            .ok_or_else(|| format!("no migration from ui version {version}"))?;
        text = Cow::Owned(step(&text)?);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_defaults_to_one_and_is_read_when_present() {
        assert_eq!(read_version("(id: \"a\")").unwrap(), 1);
        assert_eq!(read_version("(ui_version: 7, id: \"a\")").unwrap(), 7);
    }

    #[test]
    fn current_version_is_untouched() {
        let doc = format!("(ui_version: {UI_VERSION})");
        assert!(matches!(migrate(UI_VERSION, &doc).unwrap(), Cow::Borrowed(_)));
    }

    #[test]
    fn v1_documents_are_stamped_with_the_new_version() {
        let out = migrate(1, "(ui_version: 1, id: \"a\")").unwrap();
        assert_eq!(out, "(ui_version: 4, id: \"a\")");
        let out = migrate(2, "(ui_version: 2, id: \"a\")").unwrap();
        assert_eq!(out, "(ui_version: 4, id: \"a\")");
        // A document without the field is already current.
        assert_eq!(migrate(1, "(id: \"a\")").unwrap(), "(id: \"a\")");
    }

    #[test]
    fn future_and_zero_versions_are_refused() {
        assert!(migrate(UI_VERSION + 1, "()").is_err());
        assert!(migrate(0, "()").is_err());
    }

    #[test]
    fn steps_run_in_order() {
        fn a(s: &str) -> Result<String, String> {
            Ok(format!("{s}a"))
        }
        fn b(s: &str) -> Result<String, String> {
            Ok(format!("{s}b"))
        }
        assert_eq!(migrate_with(1, "x", 3, &[a, b]).unwrap(), "xab");
        assert!(migrate_with(1, "x", 4, &[a, b]).is_err(), "missing step");
    }
}
