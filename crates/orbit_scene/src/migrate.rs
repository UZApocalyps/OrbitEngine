//! `scene3d_version` migration skeleton (text based, one step at a time,
//! like `orbit_ui_data::migrate`). Only version 1 exists so far.

use std::borrow::Cow;

use crate::SCENE3D_VERSION;

/// The `scene3d_version` of a document (1 when the field is missing).
pub fn read_version(source: &str) -> Result<u32, String> {
    let Some(at) = source.find("scene3d_version") else {
        return Ok(1);
    };
    let rest = source[at + "scene3d_version".len()..].trim_start();
    let rest = rest
        .strip_prefix(':')
        .ok_or("`scene3d_version` must be followed by `:`")?
        .trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits
        .parse()
        .map_err(|_| "`scene3d_version` must be a number".to_owned())
}

/// One step rewrites a document from version `N` to `N + 1`.
pub type Step = fn(&str) -> Result<String, String>;

/// `STEPS[0]` migrates v1 -> v2.
const STEPS: &[Step] = &[v1_to_v2];

/// v2 only adds `Node3dKind::Shape`: a v1 document is a valid v2 document
/// once its version number says so.
fn v1_to_v2(source: &str) -> Result<String, String> {
    set_version(source, 2)
}

/// Rewrites (or inserts) the `scene3d_version` number.
fn set_version(source: &str, to: u32) -> Result<String, String> {
    let Some(at) = source.find("scene3d_version") else {
        // No field (version 1 by default): insert it after the opening paren.
        let open = source.find('(').ok_or("not a scene3d document")?;
        return Ok(format!(
            "{}scene3d_version: {to}, {}",
            &source[..=open],
            &source[open + 1..]
        ));
    };
    let after = at + "scene3d_version".len();
    let colon = after + source[after..].find(':').ok_or("`scene3d_version` without `:`")?;
    let start = colon + 1 + (source[colon + 1..].len() - source[colon + 1..].trim_start().len());
    let end = start + source[start..].chars().take_while(char::is_ascii_digit).count();
    Ok(format!("{}{to}{}", &source[..start], &source[end..]))
}

pub fn migrate(from: u32, source: &str) -> Result<Cow<'_, str>, String> {
    migrate_with(from, source, SCENE3D_VERSION, STEPS)
}

fn migrate_with<'a>(
    from: u32,
    source: &'a str,
    current: u32,
    steps: &[Step],
) -> Result<Cow<'a, str>, String> {
    if from == 0 || from > current {
        return Err(format!(
            "scene3d version {from} is not supported (this build reads 1..={current})"
        ));
    }
    let mut text = Cow::Borrowed(source);
    for version in from..current {
        let step = steps
            .get(version as usize - 1)
            .ok_or_else(|| format!("no migration from scene3d version {version}"))?;
        text = Cow::Owned(step(&text)?);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_version_and_defaults_to_one() {
        assert_eq!(read_version("(scene3d_version: 3)").unwrap(), 3);
        assert_eq!(read_version("(name: \"a\")").unwrap(), 1);
        assert!(read_version("(scene3d_version: x)").is_err());
    }

    #[test]
    fn current_is_untouched_future_is_refused() {
        assert!(matches!(
            migrate(SCENE3D_VERSION, "(a)").unwrap(),
            Cow::Borrowed(_)
        ));
        assert!(migrate(SCENE3D_VERSION + 1, "(a)").is_err());
        assert!(migrate(0, "(a)").is_err());
    }

    #[test]
    fn v1_to_v2_only_changes_the_number() {
        assert_eq!(
            migrate(1, "(scene3d_version: 1, name: \"a\")").unwrap(),
            "(scene3d_version: 2, name: \"a\")"
        );
        assert_eq!(
            migrate(1, "(\n    scene3d_version:   1,\n)").unwrap(),
            "(\n    scene3d_version:   2,\n)"
        );
        // A v1 document without the field gets it.
        assert_eq!(
            migrate(1, "(name: \"a\")").unwrap(),
            "(scene3d_version: 2, name: \"a\")"
        );
    }

    /// Frozen `scene3d_version` 1 document (plan §1.3): loads, migrates to
    /// the current version and validates.
    #[test]
    fn the_frozen_v1_scene_still_loads() {
        let text = include_str!("../../../examples/fixtures/compat/scene3d_v1/room.scene3d.ron");
        assert_eq!(read_version(text).unwrap(), 1);
        let scene = crate::from_ron(text).unwrap();
        assert_eq!(scene.scene3d_version, SCENE3D_VERSION);
        assert_eq!(scene.name, "room");
        assert!(scene.nodes.len() >= 4);
        assert!(crate::validate(&scene).is_empty(), "{:?}", crate::validate(&scene));
    }

    #[test]
    fn steps_run_in_order() {
        fn s1(t: &str) -> Result<String, String> {
            Ok(format!("{t}+1"))
        }
        fn s2(t: &str) -> Result<String, String> {
            Ok(format!("{t}+2"))
        }
        assert_eq!(migrate_with(1, "x", 3, &[s1, s2]).unwrap(), "x+1+2");
        assert_eq!(migrate_with(2, "x", 3, &[s1, s2]).unwrap(), "x+2");
        assert!(migrate_with(1, "x", 3, &[s1]).is_err());
    }
}
