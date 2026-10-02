//! The editor registry: everything plugins register on the UI side.

use egui::{Key, KeyboardShortcut, Modifiers};
use orbit_plugin_api::{PluginError, ProjectRegistry};

use crate::EditorPlugin;
use crate::context::EditorContext;

/// Where a panel opens in the default layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockSlot {
    Left,
    Center,
    Right,
    /// Above the `Right` panels (the preview).
    RightTop,
    Bottom,
    /// Not in the default layout; opened from the View menu or the palette.
    Hidden,
}

type PanelUi = Box<dyn Fn(&mut egui::Ui, &mut EditorContext<'_>)>;

/// A dockable panel.
pub struct PanelDef {
    /// Local id; the registered id is `plugin_id.id`.
    pub id: String,
    pub title: String,
    /// Icon glyph (see [`crate::icons`]), empty for none.
    pub icon: &'static str,
    pub default_dock: DockSlot,
    pub ui: PanelUi,
}

impl PanelDef {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        icon: &'static str,
        default_dock: DockSlot,
        ui: impl Fn(&mut egui::Ui, &mut EditorContext<'_>) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            icon,
            default_dock,
            ui: Box::new(ui),
        }
    }
}

/// A top-level menu. Ids are global so that a plugin can add items to `file`,
/// `edit`, `view`, `tools` or `help`, or create its own menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuDef {
    pub id: String,
    pub label: String,
    /// Position in the menu bar.
    pub order: i32,
}

/// Where an action appears in a menu. Items are sorted by `order`; a
/// separator is drawn when `order / 100` changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuSlot {
    pub menu: String,
    pub order: i32,
}

type Enabled = Box<dyn Fn(&EditorContext<'_>) -> bool>;
type LabelFn = Box<dyn Fn(&EditorContext<'_>) -> String>;
type Run = Box<dyn Fn(&mut EditorContext<'_>)>;

/// An editor command: one entry shared by the palette, menus and shortcuts.
pub struct ActionDef {
    /// Local id; the registered id is `plugin_id.id`.
    pub id: String,
    pub label: String,
    pub icon: &'static str,
    /// Group shown in the palette ("Fichier", "Affichage"...).
    pub category: String,
    pub shortcut: Option<KeyboardShortcut>,
    /// Further shortcuts running the same action (`Ctrl+Y` next to `Ctrl+Shift+Z`).
    pub extra_shortcuts: Vec<KeyboardShortcut>,
    /// Replaces `label` when it depends on the state ("Annuler : ...").
    pub label_fn: Option<LabelFn>,
    pub menu: Option<MenuSlot>,
    pub enabled: Enabled,
    pub run: Run,
}

impl ActionDef {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        run: impl Fn(&mut EditorContext<'_>) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: "",
            category: String::new(),
            shortcut: None,
            extra_shortcuts: Vec::new(),
            label_fn: None,
            menu: None,
            enabled: Box::new(|_| true),
            run: Box::new(run),
        }
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = icon;
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    pub fn shortcut(mut self, modifiers: Modifiers, key: Key) -> Self {
        self.shortcut = Some(KeyboardShortcut::new(modifiers, key));
        self
    }

    pub fn also_shortcut(mut self, modifiers: Modifiers, key: Key) -> Self {
        self.extra_shortcuts
            .push(KeyboardShortcut::new(modifiers, key));
        self
    }

    pub fn label_with(mut self, label: impl Fn(&EditorContext<'_>) -> String + 'static) -> Self {
        self.label_fn = Some(Box::new(label));
        self
    }

    pub fn menu(mut self, menu: impl Into<String>, order: i32) -> Self {
        self.menu = Some(MenuSlot {
            menu: menu.into(),
            order,
        });
        self
    }

    pub fn enabled(mut self, enabled: impl Fn(&EditorContext<'_>) -> bool + 'static) -> Self {
        self.enabled = Box::new(enabled);
        self
    }
}

type InspectorUi = Box<dyn Fn(&mut egui::Ui, &mut EditorContext<'_>, orbit_core::NodeId)>;

/// Inspector for one kind of selectable element.
pub struct InspectorDef {
    /// `Say`, `Label`... for built-in nodes; `plugin_id.kind` for custom ones.
    pub kind: String,
    pub ui: InspectorUi,
}

impl InspectorDef {
    pub fn new(
        kind: impl Into<String>,
        ui: impl Fn(&mut egui::Ui, &mut EditorContext<'_>, orbit_core::NodeId) + 'static,
    ) -> Self {
        Self {
            kind: kind.into(),
            ui: Box::new(ui),
        }
    }
}

/// A panel with its owner.
pub struct RegisteredPanel {
    pub plugin_id: String,
    pub def: PanelDef,
}

/// An action with its owner (`def.id` is the full id).
pub struct RegisteredAction {
    pub plugin_id: String,
    pub def: ActionDef,
}

impl RegisteredAction {
    /// Label to display now.
    pub fn label(&self, ctx: &EditorContext<'_>) -> String {
        match &self.def.label_fn {
            Some(label) => label(ctx),
            None => self.def.label.clone(),
        }
    }

    /// Every shortcut that runs this action, main one first.
    pub fn shortcuts(&self) -> impl Iterator<Item = &KeyboardShortcut> {
        self.def.shortcut.iter().chain(&self.def.extra_shortcuts)
    }
}

/// Everything registered by plugins, project side included.
#[derive(Default)]
pub struct EditorRegistry {
    /// The project-side registry (node types, importers, MCP tools, hooks).
    pub project: ProjectRegistry,
    current: String,
    panels: Vec<RegisteredPanel>,
    actions: Vec<RegisteredAction>,
    menus: Vec<MenuDef>,
    inspectors: Vec<(String, InspectorDef)>,
    /// Registration problems (duplicates...), reported in the console.
    pub errors: Vec<String>,
}

impl EditorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs both halves of a plugin.
    pub fn install(&mut self, plugin: &dyn EditorPlugin) -> Result<(), PluginError> {
        let manifest = plugin.manifest();
        self.project.begin(&manifest)?;
        self.current = manifest.id;
        plugin.register(&mut self.project);
        plugin.register_ui(self);
        Ok(())
    }

    fn full_id(&self, local: &str) -> String {
        format!("{}.{local}", self.current)
    }

    pub fn panel(&mut self, mut def: PanelDef) {
        def.id = self.full_id(&def.id);
        if self.find_panel(&def.id).is_some() {
            self.errors
                .push(format!("panel `{}` registered twice", def.id));
            return;
        }
        self.panels.push(RegisteredPanel {
            plugin_id: self.current.clone(),
            def,
        });
    }

    pub fn action(&mut self, mut def: ActionDef) {
        def.id = self.full_id(&def.id);
        if self.find_action(&def.id).is_some() {
            self.errors
                .push(format!("action `{}` registered twice", def.id));
            return;
        }
        self.actions.push(RegisteredAction {
            plugin_id: self.current.clone(),
            def,
        });
    }

    /// Declares a menu; declaring an existing id again is a no-op.
    pub fn menu(&mut self, id: impl Into<String>, label: impl Into<String>, order: i32) {
        let id = id.into();
        if self.menus.iter().all(|m| m.id != id) {
            self.menus.push(MenuDef {
                id,
                label: label.into(),
                order,
            });
        }
    }

    pub fn inspector(&mut self, def: InspectorDef) {
        if self.inspectors.iter().any(|(_, i)| i.kind == def.kind) {
            self.errors
                .push(format!("inspector for `{}` registered twice", def.kind));
            return;
        }
        self.inspectors.push((self.current.clone(), def));
    }

    pub fn find_panel(&self, id: &str) -> Option<&RegisteredPanel> {
        self.panels.iter().find(|p| p.def.id == id)
    }

    pub fn find_action(&self, id: &str) -> Option<&RegisteredAction> {
        self.actions.iter().find(|a| a.def.id == id)
    }

    pub fn panels(&self) -> &[RegisteredPanel] {
        &self.panels
    }

    pub fn actions(&self) -> &[RegisteredAction] {
        &self.actions
    }

    /// Menus sorted by `order`.
    pub fn menus(&self) -> Vec<&MenuDef> {
        let mut menus: Vec<&MenuDef> = self.menus.iter().collect();
        menus.sort_by_key(|m| m.order);
        menus
    }

    /// Actions of a menu, sorted by `order` then label.
    pub fn menu_actions(&self, menu: &str) -> Vec<&RegisteredAction> {
        let mut items: Vec<&RegisteredAction> = self
            .actions
            .iter()
            .filter(|a| a.def.menu.as_ref().is_some_and(|m| m.menu == menu))
            .collect();
        items.sort_by(|a, b| {
            let (oa, ob) = (a.def.menu.as_ref(), b.def.menu.as_ref());
            oa.map(|m| m.order)
                .cmp(&ob.map(|m| m.order))
                .then_with(|| a.def.label.cmp(&b.def.label))
        });
        items
    }

    /// Inspector registered for a node kind key.
    pub fn inspector_for(&self, kind: &str) -> Option<&InspectorDef> {
        self.inspectors
            .iter()
            .map(|(_, i)| i)
            .find(|i| i.kind == kind)
    }
}

/// `Ctrl+Shift+P`-style text for a shortcut (Windows / Linux wording).
pub fn shortcut_text(shortcut: &KeyboardShortcut) -> String {
    let m = shortcut.modifiers;
    let mut parts: Vec<String> = Vec::new();
    if m.command || m.ctrl {
        parts.push("Ctrl".into());
    }
    if m.alt {
        parts.push("Alt".into());
    }
    if m.shift {
        parts.push("Maj".into());
    }
    parts.push(shortcut.logical_key.symbol_or_name().to_owned());
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use orbit_plugin_api::{Plugin, PluginManifest};

    use super::*;
    use crate::context::EditorState;

    struct Demo;

    impl Plugin for Demo {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("demo.tools", "Demo", "0.1.0")
        }
    }

    impl EditorPlugin for Demo {
        fn register_ui(&self, reg: &mut EditorRegistry) {
            reg.menu("tools", "Outils", 40);
            reg.panel(PanelDef::new(
                "panel",
                "Demo",
                "",
                DockSlot::Hidden,
                |ui, _| {
                    ui.label("hi");
                },
            ));
            reg.action(
                ActionDef::new("hello", "Dire bonjour", |ctx| ctx.info("bonjour"))
                    .category("Outils")
                    .shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::H)
                    .menu("tools", 10),
            );
            // Duplicates are reported, not fatal.
            reg.action(ActionDef::new("hello", "Encore", |_| {}));
        }
    }

    #[test]
    fn ids_are_namespaced_and_duplicates_reported() {
        let mut reg = EditorRegistry::new();
        reg.install(&Demo).unwrap();
        assert!(reg.find_panel("demo.tools.panel").is_some());
        assert!(reg.find_action("demo.tools.hello").is_some());
        assert_eq!(reg.errors.len(), 1);
        assert_eq!(reg.menu_actions("tools").len(), 1);
        assert!(reg.project.has_plugin("demo.tools"));
    }

    #[test]
    fn actions_run_through_the_context() {
        let mut reg = EditorRegistry::new();
        reg.install(&Demo).unwrap();
        let mut state = EditorState::default();
        {
            let mut ctx = EditorContext::new(None, &mut state, &reg);
            let action = reg.find_action("demo.tools.hello").unwrap();
            assert!((action.def.enabled)(&ctx));
            (action.def.run)(&mut ctx);
        }
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].message, "bonjour");
    }

    #[test]
    fn shortcut_text_is_readable() {
        let s = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
        assert_eq!(shortcut_text(&s), "Ctrl+Maj+P");
        let s = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        assert_eq!(shortcut_text(&s), "Ctrl+S");
    }
}
