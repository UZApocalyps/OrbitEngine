//! `ui_version` 10 (list overflow): frozen version 9 documents
//! (`examples/fixtures/compat/ui_v9/`, never regenerate) keep loading, mean
//! the same (their generated lists scroll when the rows do not fit) and
//! write none of the new fields.

use orbit_ui_data::{Overflow, UI_VERSION, WidgetKind, from_ron, to_ron};

const GALLERY_V9: &str = include_str!("../../../examples/fixtures/compat/ui_v9/gallery.screen.ron");

#[test]
fn v9_screens_load_as_version_10_and_default_to_scrolling() {
    assert!(GALLERY_V9.contains("ui_version: 9"));
    let screen = from_ron(GALLERY_V9).unwrap();
    assert_eq!(screen.ui_version, UI_VERSION);
    let as_current = GALLERY_V9.replace("ui_version: 9", &format!("ui_version: {UI_VERSION}"));
    assert_eq!(from_ron(&as_current).unwrap(), screen);
    let mut lists = 0;
    screen.root.walk(&mut |w| {
        if let WidgetKind::Repeat { overflow, .. } = &w.kind {
            assert_eq!(*overflow, Overflow::Scroll);
            lists += 1;
        }
    });
    assert_eq!(lists, 1);
    // The default is not written, so the document reads back identical.
    let text = to_ron(&screen).unwrap();
    assert!(!text.contains("overflow"), "{text}");
    assert_eq!(from_ron(&text).unwrap(), screen);
}

#[test]
fn a_chosen_overflow_round_trips() {
    for (overflow, written) in [(Overflow::Pages, "overflow: Pages"), (Overflow::Off, "overflow: Off")] {
        let mut screen = from_ron(GALLERY_V9).unwrap();
        screen.root.children.iter_mut().for_each(|w| {
            if let WidgetKind::Repeat { overflow: o, .. } = &mut w.kind {
                *o = overflow;
            }
        });
        let text = to_ron(&screen).unwrap();
        assert!(text.contains(written), "{text}");
        assert_eq!(from_ron(&text).unwrap(), screen);
    }
}
