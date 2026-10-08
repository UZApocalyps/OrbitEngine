//! The menu bar stays short: entries are grouped in submenus, and every
//! action stays reachable from its menu.

use orbit_editor::builtin;
use orbit_editor_api::EditorRegistry;

fn registry() -> EditorRegistry {
    let mut reg = EditorRegistry::new();
    let shared = builtin::Shared::default();
    for p in builtin::plugins_with(&shared).iter() {
        reg.install(p.as_ref()).unwrap();
    }
    let v = builtin::PanelActions::for_registry(&reg);
    reg.install(&v).unwrap();
    reg
}

/// Distinct top-level entries of a menu: leaves plus first path keys.
fn top_level(reg: &EditorRegistry, menu: &str) -> Vec<String> {
    let mut top: Vec<String> = Vec::new();
    for a in reg.menu_actions(menu) {
        let key = match a.def.menu_group.as_deref() {
            Some(g) => g.split('/').next().unwrap().to_owned(),
            None => a.def.id.clone(),
        };
        if !top.contains(&key) {
            top.push(key);
        }
    }
    top
}

#[test]
fn view_menu_is_grouped_and_complete() {
    let reg = registry();
    let top = top_level(&reg, "view");
    assert!(top.len() <= 8, "View has {} top-level entries: {top:?}", top.len());

    // Every panel can still be shown, and the former loose entries remain.
    for panel in reg.panels() {
        let id = format!("orbit.core.views.show.{}", panel.def.id);
        let a = reg
            .find_action(&id)
            .unwrap_or_else(|| panic!("no show action for {id}"));
        assert!(a.def.menu.as_ref().is_some_and(|m| m.menu == "view"), "{id}");
        assert!(a.def.menu_group.is_some(), "{id} is not grouped");
    }
    for id in [
        "orbit.core.palette",
        "orbit.core.toggle_theme",
        "orbit.core.reset_layout",
        "orbit.core.experimental_3d",
        "orbit.core.auto_validation",
        "orbit.core.language.en",
        "orbit.core.language.fr",
        "orbit.console.clear",
        "orbit.profiler.show_hitches",
    ] {
        let a = reg.find_action(id).unwrap_or_else(|| panic!("missing {id}"));
        assert!(a.def.menu.as_ref().is_some_and(|m| m.menu == "view"), "{id}");
    }
}

#[test]
fn other_menus_stay_short() {
    let reg = registry();
    for menu in ["file", "edit", "tools", "help"] {
        let top = top_level(&reg, menu);
        println!("{menu}: {top:?}");
        assert!(top.len() <= 8, "{menu} has {} entries: {top:?}", top.len());
    }
    // Nothing was dropped by the grouping.
    assert_eq!(reg.menu_actions("file").len(), 11);
    assert_eq!(reg.menu_actions("edit").len(), 21);
}
