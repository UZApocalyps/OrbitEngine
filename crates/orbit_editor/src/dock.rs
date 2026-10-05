//! The dock: default layout from the panel registry, layout sanity after
//! loading, opening panels and tool workspaces as tabs, and the `TabViewer`
//! that draws plugin panels.
//!
//! Layout: every area of the window is one tab group (explorer on the left;
//! documents in the center; preview over the Inspector | Characters |
//! Variables | Curves | Scripts group on the right; Console | Problems |
//! Timeline | Translations | ... at the bottom). Panels outside the default
//! layout open as a tab of their home area; tool workspaces (rig editor, 3D,
//! UI editor) open their main panel in the center and their side panels as
//! one sub-tab group under it. Tabs can be dragged, re-ordered and split; the
//! layout is saved with the user config.

use egui::{Id, Ui, WidgetText};
use egui_dock::{DockState, Node, NodeIndex, NodePath, Split, SurfaceIndex, TabViewer};
use orbit_editor_api::{DockSlot, EditorContext, EditorRegistry};

/// Version of the default layout. A saved layout from another version is
/// replaced by the default one (the panel set or grouping changed).
pub const LAYOUT_VERSION: u32 = 2;

/// Preferred tab order inside each area; panels not listed keep their
/// registration order after the listed ones.
const TAB_ORDER: &[&str] = &[
    // Center
    "orbit.welcome.welcome",
    "orbit.scene.scene",
    "orbit.graph.graph",
    "orbit.sceneview.sceneview",
    // Right
    "orbit.inspector.inspector",
    "orbit.characters.characters",
    "orbit.variables.variables",
    "orbit.curves.curves",
    "orbit.scripts.scripts",
    // Bottom
    "orbit.console.console",
    "orbit.problems.problems",
    "orbit.timeline.timeline",
    "orbit.translations.translations",
    "orbit.search.search",
    "orbit.library.library",
    "orbit.assistant.assistant",
];

fn order_key(id: &str) -> usize {
    TAB_ORDER.iter().position(|t| *t == id).unwrap_or(TAB_ORDER.len())
}

/// Default layout: explorer on the left, center area, inspector group on
/// the right, bottom tab group under the center. Panels without a slot
/// (`Hidden`) are not part of it.
pub fn default_layout(registry: &EditorRegistry) -> DockState<String> {
    let tabs = |slot: DockSlot| -> Vec<String> {
        let mut ids: Vec<String> = registry
            .panels()
            .iter()
            .filter(|p| p.def.default_dock == slot)
            .map(|p| p.def.id.clone())
            .collect();
        // Stable: unlisted panels keep the registration order.
        ids.sort_by_key(|id| order_key(id));
        ids
    };
    let (left, center, right, right_top, bottom) = (
        tabs(DockSlot::Left),
        tabs(DockSlot::Center),
        tabs(DockSlot::Right),
        tabs(DockSlot::RightTop),
        tabs(DockSlot::Bottom),
    );

    let mut dock = DockState::new(center);
    let main = dock.main_surface_mut();
    // `split_*(parent, fraction, tabs)`: `fraction` is the share of the first
    // child (left / top): the new node for `split_left`, the old one for
    // `split_right` and `split_below`.
    let mut middle = NodeIndex::root();
    if !left.is_empty() {
        let [old, _] = main.split_left(middle, 0.17, left);
        middle = old;
    }
    if !right.is_empty() || !right_top.is_empty() {
        // The right column is wide enough for a 16:9 preview over the inspector.
        let width = if right_top.is_empty() { 0.76 } else { 0.58 };
        let [old, column] = main.split_right(middle, width, right);
        middle = old;
        if !right_top.is_empty() {
            main.split_above(column, 0.5, right_top);
        }
    }
    if !bottom.is_empty() {
        let [old, _] = main.split_below(middle, 0.66, bottom);
        middle = old;
    }
    // The center starts focused.
    main.set_focused_node(middle);
    dock
}

/// Drops tabs of panels that no longer exist (a plugin was removed) and
/// duplicates; an empty result falls back to the default layout.
pub fn sanitize(dock: &mut DockState<String>, registry: &EditorRegistry) {
    let mut seen = std::collections::HashSet::new();
    dock.retain_tabs(|tab| registry.find_panel(tab).is_some() && seen.insert(tab.clone()));
    if dock.iter_all_tabs().next().is_none() {
        *dock = default_layout(registry);
    }
}

/// The saved layout when it is usable, else the default one: a missing
/// layout, one saved by another layout version (older editors opened tools
/// as loose tabs) or one left empty after dropping unknown panels.
pub fn restore(
    saved: Option<DockState<String>>,
    version: u32,
    registry: &EditorRegistry,
) -> DockState<String> {
    let mut dock = match saved {
        Some(dock) if version == LAYOUT_VERSION => dock,
        _ => default_layout(registry),
    };
    sanitize(&mut dock, registry);
    dock
}

/// Adds `id` to the tab group standing for `slot`: the leaf holding a panel
/// whose default area is `slot` (main window first), else a related area.
/// A closed side area is rebuilt next to the center group; otherwise falls
/// back to the center group, the focused leaf, then any leaf.
fn place_in_slot(dock: &mut DockState<String>, registry: &EditorRegistry, slot: DockSlot, id: &str) {
    let find = |dock: &DockState<String>, slot: DockSlot| {
        let mut leaves: Vec<_> = dock.iter_leaves().collect();
        // Main surface first: floating windows are a last resort.
        leaves.sort_by_key(|(path, _)| path.surface != SurfaceIndex::main());
        leaves.into_iter().find_map(|(path, leaf)| {
            leaf.tabs()
                .iter()
                .any(|t| {
                    registry
                        .find_panel(t)
                        .is_some_and(|p| p.def.default_dock == slot)
                })
                .then_some(path)
        })
    };
    let related = match slot {
        DockSlot::RightTop => Some(DockSlot::Right),
        DockSlot::Right => Some(DockSlot::RightTop),
        _ => None,
    };
    if let Some(found) = find(dock, slot).or_else(|| related.and_then(|r| find(dock, r))) {
        append(dock, Some(found), id);
        return;
    }
    // The whole area was closed: rebuild it beside the center group
    // (`fraction` as in `default_layout`).
    let side = match slot {
        DockSlot::Left => Some((Split::Left, 0.17)),
        DockSlot::Right | DockSlot::RightTop => Some((Split::Right, 0.76)),
        DockSlot::Bottom => Some((Split::Below, 0.66)),
        DockSlot::Center | DockSlot::Hidden => None,
    };
    if let (Some((split, fraction)), Some(center)) = (side, find(dock, DockSlot::Center)) {
        // egui_dock refuses empty leaves: the new group starts with the tab.
        dock.split(center, split, fraction, Node::leaf_with(vec![id.to_owned()]));
        return;
    }
    let leaf = find(dock, DockSlot::Center)
        .or_else(|| dock.focused_leaf())
        .or_else(|| dock.iter_leaves().next().map(|(p, _)| p));
    append(dock, leaf, id);
}

/// Makes an open panel the visible tab of its group and focuses the group.
fn focus(dock: &mut DockState<String>, id: &str) -> bool {
    let Some(path) = dock.find_tab(&id.to_owned()) else {
        return false;
    };
    let _ = dock.set_active_tab(path);
    dock.set_focused_node_and_surface(NodePath {
        surface: path.surface,
        node: path.node,
    });
    true
}

fn append(dock: &mut DockState<String>, leaf: Option<NodePath>, id: &str) {
    match leaf.and_then(|path| dock.leaf_mut(path).ok()) {
        Some(leaf) => leaf.append_tab(id.to_owned()),
        None => dock.push_to_first_leaf(id.to_owned()),
    }
}

fn node_path(dock: &DockState<String>, id: &str) -> Option<NodePath> {
    dock.find_tab(&id.to_owned()).map(|p| NodePath {
        surface: p.surface,
        node: p.node,
    })
}

/// Brings a panel to the front; when the layout does not hold it (closed
/// tab, hidden panel) it becomes a tab of its home area's group.
pub fn show_panel(dock: &mut DockState<String>, registry: &EditorRegistry, id: &str) {
    if focus(dock, id) {
        return;
    }
    let home = registry
        .find_panel(id)
        .map_or(DockSlot::Center, |p| p.def.home);
    place_in_slot(dock, registry, home, id);
    focus(dock, id);
}

/// Opens a tool workspace: `ids[0]` is shown like [`show_panel`]; the other
/// panels join the group already holding one of them, or else form a new
/// tab group split under the main panel. Open panels are not moved (the
/// user's arrangement wins). The main panel ends focused.
pub fn show_workspace(dock: &mut DockState<String>, registry: &EditorRegistry, ids: &[String]) {
    let Some((main, rest)) = ids.split_first() else {
        return;
    };
    show_panel(dock, registry, main);
    let mut missing: Vec<String> = Vec::new();
    for id in rest {
        if id != main && dock.find_tab(id).is_none() && !missing.contains(id) {
            missing.push(id.clone());
        }
    }
    if !missing.is_empty() {
        let group = rest.iter().find_map(|id| node_path(dock, id));
        match (group, node_path(dock, main)) {
            (Some(group), _) => {
                for id in &missing {
                    append(dock, Some(group), id);
                }
            }
            (None, Some(main_leaf)) => {
                dock.split(main_leaf, Split::Below, 0.62, Node::leaf_with(missing));
            }
            (None, None) => {
                for id in &missing {
                    append(dock, None, id);
                }
            }
        }
        if let Some(first) = rest.first() {
            focus(dock, first);
        }
    }
    focus(dock, main);
}

/// Ids of the panels currently in the layout.
pub fn open_panels(dock: &DockState<String>) -> Vec<String> {
    dock.iter_all_tabs().map(|(_, t)| t.clone()).collect()
}

/// Draws the panels of the registry inside the dock.
pub struct DockViewer<'a, 'b> {
    pub ctx: &'a mut EditorContext<'b>,
}

impl TabViewer for DockViewer<'_, '_> {
    type Tab = String;

    fn id(&mut self, tab: &mut String) -> Id {
        Id::new(("orbit_panel", tab.as_str()))
    }

    fn title(&mut self, tab: &mut String) -> WidgetText {
        match self.ctx.registry().find_panel(tab) {
            Some(panel) if panel.def.icon.is_empty() => self.ctx.t(&panel.def.title).into(),
            Some(panel) => format!("{}  {}", panel.def.icon, self.ctx.t(&panel.def.title)).into(),
            None => format!("? {tab}").into(),
        }
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut String) {
        match self.ctx.registry().find_panel(tab) {
            // Each panel gets its own id scope: two panels using an auto-id
            // widget (a `ScrollArea`...) would otherwise share its state.
            Some(panel) => {
                let _span = orbit_profile::is_enabled()
                    .then(|| orbit_profile::Scope::new(orbit_profile::intern(tab)));
                ui.push_id(("orbit_panel_ui", tab.as_str()), |ui| {
                    (panel.def.ui)(ui, self.ctx);
                });
            }
            None => {
                ui.label(self.ctx.tf("dock.unknown_panel", &[("id", tab)]));
            }
        }
    }

    // Panels manage their own scrolling (virtualized lists).
    fn scroll_bars(&self, _tab: &String) -> [bool; 2] {
        [false, false]
    }
}

#[cfg(test)]
mod tests {
    use orbit_editor_api::{EditorPlugin, PanelDef, Plugin, PluginManifest};

    use super::*;

    struct Panels;

    impl Plugin for Panels {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("t.panels", "Panels", "1")
        }
    }

    impl EditorPlugin for Panels {
        fn register_ui(&self, reg: &mut EditorRegistry) {
            for (id, slot) in [
                ("explorer", DockSlot::Left),
                ("welcome", DockSlot::Center),
                ("inspector", DockSlot::Right),
                ("console", DockSlot::Bottom),
                ("extra", DockSlot::Hidden),
            ] {
                reg.panel(PanelDef::new(id, id, "", slot, |_, _| {}));
            }
        }
    }

    fn registry() -> EditorRegistry {
        let mut reg = EditorRegistry::new();
        reg.install(&Panels).unwrap();
        reg
    }

    #[test]
    fn default_layout_holds_every_non_hidden_panel() {
        let reg = registry();
        let dock = default_layout(&reg);
        let mut tabs = open_panels(&dock);
        tabs.sort();
        assert_eq!(
            tabs,
            [
                "t.panels.console",
                "t.panels.explorer",
                "t.panels.inspector",
                "t.panels.welcome"
            ]
        );
    }

    #[test]
    fn layout_survives_a_ron_round_trip() {
        let reg = registry();
        let dock = default_layout(&reg);
        let text = ron::to_string(&dock).unwrap();
        let back: DockState<String> = ron::from_str(&text).unwrap();
        assert_eq!(open_panels(&back), open_panels(&dock));
    }

    #[test]
    fn sanitize_drops_unknown_and_duplicate_tabs() {
        let reg = registry();
        let mut dock = DockState::new(vec![
            "t.panels.welcome".to_owned(),
            "gone.panel".to_owned(),
            "t.panels.welcome".to_owned(),
        ]);
        sanitize(&mut dock, &reg);
        assert_eq!(open_panels(&dock), ["t.panels.welcome"]);

        let mut all_gone = DockState::new(vec!["gone.panel".to_owned()]);
        sanitize(&mut all_gone, &reg);
        assert_eq!(open_panels(&all_gone).len(), 4); // back to the default layout
    }

    #[test]
    fn show_panel_adds_a_closed_panel_and_focuses_an_open_one() {
        let reg = registry();
        let mut dock = default_layout(&reg);
        show_panel(&mut dock, &reg, "t.panels.extra");
        assert!(open_panels(&dock).contains(&"t.panels.extra".to_owned()));
        let count = open_panels(&dock).len();
        show_panel(&mut dock, &reg, "t.panels.console");
        assert_eq!(open_panels(&dock).len(), count);
    }

    fn group_of(dock: &DockState<String>, id: &str) -> Vec<String> {
        let path = dock.find_tab(&id.to_owned()).expect(id);
        dock.leaf(NodePath {
            surface: path.surface,
            node: path.node,
        })
        .unwrap()
        .tabs()
        .to_vec()
    }

    #[test]
    fn hidden_panels_open_as_a_tab_of_their_home_area() {
        let reg = registry();
        let mut dock = default_layout(&reg);
        // Focus the bottom group: a hidden panel still goes to the center.
        show_panel(&mut dock, &reg, "t.panels.console");
        show_panel(&mut dock, &reg, "t.panels.extra");
        assert!(group_of(&dock, "t.panels.extra").contains(&"t.panels.welcome".to_owned()));
        // A closed bottom panel comes back to the bottom group.
        let path = dock.find_tab(&"t.panels.console".to_owned()).unwrap();
        dock.remove_tab(path);
        show_panel(&mut dock, &reg, "t.panels.welcome");
        show_panel(&mut dock, &reg, "t.panels.console");
        assert_eq!(group_of(&dock, "t.panels.console"), ["t.panels.console"]);
    }

    struct Tool;

    impl Plugin for Tool {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("t.tool", "Tool", "1")
        }
    }

    impl EditorPlugin for Tool {
        fn register_ui(&self, reg: &mut EditorRegistry) {
            for id in ["view", "tree", "props"] {
                reg.panel(PanelDef::new(id, id, "", DockSlot::Hidden, |_, _| {}));
            }
            reg.panel(
                PanelDef::new("perf", "perf", "", DockSlot::Hidden, |_, _| {})
                    .opens_in(DockSlot::Bottom),
            );
        }
    }

    fn tool_registry() -> EditorRegistry {
        let mut reg = registry();
        reg.install(&Tool).unwrap();
        reg
    }

    fn tool_ids() -> Vec<String> {
        ["t.tool.view", "t.tool.tree", "t.tool.props"]
            .map(str::to_owned)
            .to_vec()
    }

    #[test]
    fn a_workspace_opens_its_side_panels_as_one_sub_tab_group() {
        let reg = tool_registry();
        let mut dock = default_layout(&reg);
        show_workspace(&mut dock, &reg, &tool_ids());
        assert!(group_of(&dock, "t.tool.view").contains(&"t.panels.welcome".to_owned()));
        assert_eq!(group_of(&dock, "t.tool.tree"), ["t.tool.tree", "t.tool.props"]);
        assert_eq!(dock.find_active_focused().map(|(_, t)| t.clone()).as_deref(), Some("t.tool.view"));

        // Opening it again does not duplicate anything.
        let count = open_panels(&dock).len();
        show_workspace(&mut dock, &reg, &tool_ids());
        assert_eq!(open_panels(&dock).len(), count);

        // A closed sub-panel comes back into its group.
        let path = dock.find_tab(&"t.tool.props".to_owned()).unwrap();
        dock.remove_tab(path);
        show_workspace(&mut dock, &reg, &tool_ids());
        assert_eq!(group_of(&dock, "t.tool.tree"), ["t.tool.tree", "t.tool.props"]);
        show_workspace(&mut dock, &reg, &[]);
    }

    #[test]
    fn opens_in_routes_a_hidden_panel_to_another_area() {
        let reg = tool_registry();
        let mut dock = default_layout(&reg);
        show_panel(&mut dock, &reg, "t.tool.perf");
        assert!(group_of(&dock, "t.tool.perf").contains(&"t.panels.console".to_owned()));
    }

    #[test]
    fn restore_resets_layouts_of_another_version() {
        let reg = registry();
        let old = DockState::new(vec!["t.panels.welcome".to_owned()]);
        let kept = restore(Some(old.clone()), LAYOUT_VERSION, &reg);
        assert_eq!(open_panels(&kept), ["t.panels.welcome"]);
        let reset = restore(Some(old), 0, &reg);
        assert_eq!(open_panels(&reset).len(), 4);
        assert_eq!(open_panels(&restore(None, LAYOUT_VERSION, &reg)).len(), 4);
    }

    #[test]
    fn default_layout_orders_known_tabs() {
        let mut reg = EditorRegistry::new();
        struct Bottom;
        impl Plugin for Bottom {
            fn manifest(&self) -> PluginManifest {
                PluginManifest::new("orbit.problems", "P", "1")
            }
        }
        impl EditorPlugin for Bottom {
            fn register_ui(&self, reg: &mut EditorRegistry) {
                reg.panel(PanelDef::new("problems", "p", "", DockSlot::Bottom, |_, _| {}));
            }
        }
        reg.install(&Bottom).unwrap();
        reg.install(&Panels).unwrap();
        let dock = default_layout(&reg);
        assert_eq!(
            group_of(&dock, "orbit.problems.problems"),
            ["orbit.problems.problems", "t.panels.console"]
        );
    }
}
