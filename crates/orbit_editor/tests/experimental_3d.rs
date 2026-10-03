//! The experimental 3D switch: 3D entries are absent by default and present
//! once it is on. One test, because the switch is process-wide.

use orbit_core::experimental;
use orbit_editor::builtin::{drafts, manual};
use orbit_editor_api::Lang;
use orbit_mcp::host::{ToolRunner, is_3d_tool};
use orbit_plugin_api::ProjectRegistry;

fn has_3d_insertable() -> bool {
    drafts::builtin_insertables()
        .iter()
        .any(|i| matches!(i.key.as_str(), "ShowScene3d" | "Camera3d" | "HideScene3d"))
}

fn has_3d_chapter() -> bool {
    manual::visible_chapters(Lang::En)
        .iter()
        .any(|(stem, _)| *stem == "09-3d-scenes")
}

#[test]
fn three_d_entries_follow_the_switch() {
    assert!(std::env::var_os(experimental::ENV_3D).is_none());
    let runner = ToolRunner::new();
    let registry = ProjectRegistry::default();
    let list = || runner.list(&registry);

    assert!(!experimental::enabled_3d());
    assert!(!has_3d_insertable());
    assert!(!has_3d_chapter());
    assert!(!list().iter().any(|t| is_3d_tool(&t.name)));
    // Hidden, not removed: the dispatch table still knows the tools.
    assert!(is_3d_tool("list_scenes3d"));

    experimental::set_preference_3d(true);
    assert!(experimental::enabled_3d());
    assert!(has_3d_insertable());
    assert!(has_3d_chapter());
    assert!(list().iter().any(|t| t.name == "list_scenes3d"));

    experimental::set_preference_3d(false);
    assert!(!has_3d_insertable());
}
