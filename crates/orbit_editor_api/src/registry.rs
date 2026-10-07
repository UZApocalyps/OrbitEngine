//! The editor registry: everything plugins register on the UI side.

use egui::{Key, KeyboardShortcut, Modifiers};
use orbit_plugin_api::{PluginError, ProjectRegistry};

use crate::EditorPlugin;
use crate::context::EditorContext;
use crate::i18n::{Lang, Table, Translator, parse_table};

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
    /// Area whose tab group receives the panel when it is opened while not
    /// in the layout (a `Hidden` panel, a closed tab). Defaults to
    /// `default_dock`, or `Center` for hidden panels; see [`Self::opens_in`].
    pub home: DockSlot,
    pub ui: PanelUi,
    /// Called (before `ui`) on the first frame the panel is drawn after not
    /// being on screen: refresh what was left stale while hidden.
    pub on_shown: Option<PanelHook>,
    /// Called once the panel was not drawn for a whole frame (another tab
    /// of its group was chosen, it was closed): release what only the
    /// visible panel needs.
    pub on_hidden: Option<PanelHook>,
}

type PanelHook = Box<dyn Fn(&mut EditorContext<'_>)>;

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
            home: if default_dock == DockSlot::Hidden {
                DockSlot::Center
            } else {
                default_dock
            },
            ui: Box::new(ui),
            on_shown: None,
            on_hidden: None,
        }
    }

    /// See [`PanelDef::on_shown`].
    pub fn on_shown(mut self, hook: impl Fn(&mut EditorContext<'_>) + 'static) -> Self {
        self.on_shown = Some(Box::new(hook));
        self
    }

    /// See [`PanelDef::on_hidden`].
    pub fn on_hidden(mut self, hook: impl Fn(&mut EditorContext<'_>) + 'static) -> Self {
        self.on_hidden = Some(Box::new(hook));
        self
    }

    /// Opens the panel as a tab of the `slot` area instead of the default
    /// one (`Hidden` is ignored).
    pub fn opens_in(mut self, slot: DockSlot) -> Self {
        if slot != DockSlot::Hidden {
            self.home = slot;
        }
        self
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
    /// Text key of the group shown in the palette (`category.file`...).
    pub category: String,
    pub shortcut: Option<KeyboardShortcut>,
    /// Further shortcuts running the same action (`Ctrl+Y` next to `Ctrl+Shift+Z`).
    pub extra_shortcuts: Vec<KeyboardShortcut>,
    /// Replaces `label` when it depends on the state ("Undo: ..."); it
    /// translates by itself (`ctx.t`).
    pub label_fn: Option<LabelFn>,
    pub menu: Option<MenuSlot>,
    /// Text key of a submenu this action's menu entry goes into (the
    /// entries sharing it are grouped; the first one places the submenu).
    pub menu_group: Option<String>,
    /// Text (key or literal) shown in the menu instead of `label` (the
    /// palette keeps `label`), for entries inside a submenu.
    pub menu_label: Option<String>,
    /// Shows a check mark in the menu while this is true (radio items).
    pub checked: Option<Enabled>,
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
            menu_group: None,
            menu_label: None,
            checked: None,
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

    /// Puts the menu entry in a submenu titled by this text key.
    pub fn menu_group(mut self, group: impl Into<String>) -> Self {
        self.menu_group = Some(group.into());
        self
    }

    /// Text shown for the menu entry instead of the label.
    pub fn menu_label(mut self, label: impl Into<String>) -> Self {
        self.menu_label = Some(label.into());
        self
    }

    /// Marks the menu entry with a check while `checked` holds.
    pub fn checked(mut self, checked: impl Fn(&EditorContext<'_>) -> bool + 'static) -> Self {
        self.checked = Some(Box::new(checked));
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
    /// Label to display now, in the current language (`label` is a text
    /// key; `label_fn` translates by itself).
    pub fn label(&self, ctx: &EditorContext<'_>) -> String {
        match &self.def.label_fn {
            Some(label) => label(ctx),
            None => ctx.t(&self.def.label),
        }
    }

    /// Palette group in the current language.
    pub fn category(&self, ctx: &EditorContext<'_>) -> String {
        ctx.t(&self.def.category)
    }

    /// Every shortcut that runs this action, main one first.
    pub fn shortcuts(&self) -> impl Iterator<Item = &KeyboardShortcut> {
        self.def.shortcut.iter().chain(&self.def.extra_shortcuts)
    }
}

/// Shared flags between the host and the interface editor panel, which
/// owns its own document (`ui/` folder) outside the story.
#[derive(Default, Debug)]
pub struct UiEditorLink {
    /// The panel has unsaved edits (counts in the project's "modified").
    pub dirty: std::cell::Cell<bool>,
    /// The 3D scene editor has unsaved edits (counts in the project's
    /// "modified"; the open scene is saved with the project).
    pub scene3d_dirty: std::cell::Cell<bool>,
    /// The panel has the keyboard focus (last click was inside it): the
    /// host routes Ctrl+Z / Ctrl+Y to it instead of the story.
    pub focus: std::cell::Cell<bool>,
    /// Undo (-1) / redo (+1) steps the host asked for, applied at next draw.
    pub history: std::cell::Cell<i32>,
}

/// Everything registered by plugins, project side included.
#[derive(Default)]
pub struct EditorRegistry {
    /// See [`UiEditorLink`].
    pub ui_link: std::rc::Rc<UiEditorLink>,
    /// The project-side registry (node types, importers, MCP tools, hooks).
    pub project: ProjectRegistry,
    current: String,
    panels: Vec<RegisteredPanel>,
    actions: Vec<RegisteredAction>,
    menus: Vec<MenuDef>,
    inspectors: Vec<(String, InspectorDef)>,
    /// Text tables registered by plugins, merged by the host into its translator.
    texts: Vec<(Lang, Table)>,
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

    /// Registers the text table (RON map `{ "key": "text" }`) of a plugin for
    /// one language. Keys should start with the plugin's own prefix. Panel
    /// titles, action labels, categories and menu labels given to the
    /// registry are such keys (a plain literal is displayed as is).
    pub fn texts(&mut self, lang: Lang, ron_text: &str) {
        match parse_table(ron_text) {
            Ok(table) => self.texts.push((lang, table)),
            Err(e) => self.errors.push(format!(
                "text table ({}) of `{}` is invalid: {e}",
                lang.code(),
                self.current
            )),
        }
    }

    /// The text tables plugins registered.
    pub fn text_tables(&self) -> &[(Lang, Table)] {
        &self.texts
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

    /// Actions of a menu, sorted by `order` then id.
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
                .then_with(|| a.def.id.cmp(&b.def.id))
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
/// Modifier and key names come from the `shortcut.*` text keys.
pub fn shortcut_text(texts: &Translator, shortcut: &KeyboardShortcut) -> String {
    let m = shortcut.modifiers;
    let mut parts: Vec<String> = Vec::new();
    if m.command || m.ctrl {
        parts.push(texts.get("shortcut.ctrl"));
    }
    if m.alt {
        parts.push(texts.get("shortcut.alt"));
    }
    if m.shift {
        parts.push(texts.get("shortcut.shift"));
    }
    let name = shortcut.logical_key.symbol_or_name();
    let key = format!("shortcut.key.{name}");
    parts.push(if texts.has(&key) {
        texts.get(&key)
    } else {
        name.to_owned()
    });
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
        let texts = Translator::new(Lang::Fr);
        texts
            .add_ron(
                Lang::Fr,
                r#"{ "shortcut.ctrl": "Ctrl", "shortcut.alt": "Alt", "shortcut.shift": "Maj" }"#,
            )
            .unwrap();
        assert_eq!(shortcut_text(&texts, &s), "Ctrl+Maj+P");
        let s = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        assert_eq!(shortcut_text(&texts, &s), "Ctrl+S");
    }
}
