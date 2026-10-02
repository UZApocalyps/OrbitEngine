//! UI-01: the serialized formats must not change when types move between
//! crates. The files in `tests/golden/` were produced by the pre-extraction
//! `orbit_ui` code (`UiBundle::to_ron`, `Theme::to_ron`, `asset::to_ron`);
//! `orbit_ui_data` must reproduce them byte for byte, and read them back.

use orbit_ui_data::{ScreenAsset, Theme, UiBundle, from_ron, to_ron};

/// RON pretty-printing follows the platform newline; compare modulo CRLF.
fn norm(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn golden(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    std::fs::read_to_string(p).unwrap().replace("\r\n", "\n")
}

#[test]
fn builtin_bundle_serializes_identically() {
    assert_eq!(norm(&UiBundle::builtin().to_ron().unwrap()), golden("builtin_bundle.ron"));
}

#[test]
fn default_theme_serializes_identically() {
    assert_eq!(norm(&Theme::default().to_ron().unwrap()), golden("default_theme.ron"));
}

#[test]
fn stock_screens_serialize_identically() {
    for s in &UiBundle::builtin().screens {
        let name = format!("{}.screen.ron", s.id);
        assert_eq!(norm(&to_ron(s).unwrap()), golden(&name), "{name}");
    }
}

#[test]
fn golden_files_parse_back_to_the_same_values() {
    let bundle = UiBundle::from_ron(&golden("builtin_bundle.ron")).unwrap();
    assert_eq!(bundle, UiBundle::builtin());
    let theme = Theme::from_ron(&golden("default_theme.ron")).unwrap();
    assert_eq!(theme, Theme::default());
    let screen: ScreenAsset = from_ron(&golden("pause.screen.ron")).unwrap();
    assert_eq!(&screen, UiBundle::builtin().screen("pause").unwrap());
}
