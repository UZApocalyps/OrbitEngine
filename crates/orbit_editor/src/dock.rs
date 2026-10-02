//! The dock: default layout from the panel registry, layout sanity after
//! loading, and the `TabViewer` that draws plugin panels.

use egui::{Id, Ui, WidgetText};
use egui_dock::{DockState, NodeIndex, TabViewer};
use orbit_editor_api::{DockSlot, EditorContext, EditorRegistry};

/// Default layout: explorer on the left, center area, inspector on the
/// right, console under the center. Panels without a slot (`Hidden`) are not
/// part of it.
pub fn default_layout(registry: &EditorRegistry) -> DockState<String> {
    let tabs = |slot: DockSlot| -> Vec<String> {
        registry
            .panels()
            .iter()
            .filter(|p| p.def.default_dock == slot)
            .map(|p| p.def.id.clone())
            .collect()
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
    let _ = middle;
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

/// Brings a panel to the front; adds it to the focused area when the layout
/// does not hold it (closed tab, hidden panel).
pub fn show_panel(dock: &mut DockState<String>, id: &str) {
    if let Some(path) = dock.find_tab(&id.to_owned()) {
        let _ = dock.set_active_tab(path);
        dock.set_focused_node_and_surface(egui_dock::NodePath {
            surface: path.surface,
            node: path.node,
        });
    } else {
        dock.push_to_focused_leaf(id.to_owned());
    }
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
        show_panel(&mut dock, "t.panels.extra");
        assert!(open_panels(&dock).contains(&"t.panels.extra".to_owned()));
        let count = open_panels(&dock).len();
        show_panel(&mut dock, "t.panels.console");
        assert_eq!(open_panels(&dock).len(), count);
    }
}
