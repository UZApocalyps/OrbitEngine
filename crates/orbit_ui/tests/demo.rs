//! Global property of the demo's UI bundle (`examples/stories/ui/`).

use std::path::Path;

use orbit_ui::UiBundle;

// Demo: global property only (the ui/ folder loads and validates without issue).
#[test]
fn demo_ui_bundle_still_validates() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stories/ui");
    let bundle = UiBundle::load_dir(&dir).expect("the demo ui/ folder loads");
    let issues = bundle.validate();
    assert!(issues.is_empty(), "{issues:#?}");
}
