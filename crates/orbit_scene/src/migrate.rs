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
const STEPS: &[Step] = &[];

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
        assert!(matches!(migrate(1, "(a)").unwrap(), Cow::Borrowed(_)));
        assert!(migrate(SCENE3D_VERSION + 1, "(a)").is_err());
        assert!(migrate(0, "(a)").is_err());
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
