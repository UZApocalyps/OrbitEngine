//! Core actions: file, edit and view menus. The first built-in plugin, so
//! its menus come first.

use egui::{Key, Modifiers};
use orbit_editor_api::{
    ActionDef, EditorPlugin, EditorRegistry, EditorRequest, Lang, Plugin, PluginManifest,
    ThemeMode, icons,
};

use super::ids;

pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::CORE, "Core actions", "0.1.0")
    }
}

const CTRL: Modifiers = Modifiers::COMMAND;
const CTRL_SHIFT: Modifiers = Modifiers {
    command: true,
    shift: true,
    ..Modifiers::NONE
};

impl EditorPlugin for CorePlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.menu("file", "menu.file", 10);
        reg.menu("edit", "menu.edit", 20);
        reg.menu("view", "menu.view", 30);
        reg.menu("tools", "menu.tools", 40);

        // ---- File
        reg.action(
            ActionDef::new("new", "action.new_project", |c| {
                c.request(EditorRequest::NewProjectDialog);
            })
            .icon(icons::FILE_PLUS)
            .category("category.file")
            .shortcut(CTRL, Key::N)
            .menu("file", 100),
        );
        reg.action(
            ActionDef::new("open", "action.open_project", |c| {
                c.request(EditorRequest::PickOpenProject);
            })
            .icon(icons::FOLDER_OPEN)
            .category("category.file")
            .shortcut(CTRL, Key::O)
            .menu("file", 110),
        );
        reg.action(
            ActionDef::new("open_story", "action.open_story", |c| {
                c.request(EditorRequest::PickOpenStory);
            })
            .icon(icons::FILM_SLATE)
            .category("category.file")
            .shortcut(CTRL_SHIFT, Key::O)
            .menu("file", 120),
        );
        reg.action(
            ActionDef::new("convert_story", "action.convert_story", |c| {
                c.request(EditorRequest::ConvertStory);
            })
            .icon(icons::FILM_SLATE)
            .category("category.file")
            .menu("file", 205)
            .enabled(|c| {
                c.project_info()
                    .is_some_and(|p| orbit_project::orbit_story_name(&p.story_file).is_some())
            }),
        );
        reg.action(
            ActionDef::new("save", "action.save", |c| c.request(EditorRequest::Save))
                .icon(icons::FLOPPY_DISK)
                .category("category.file")
                .shortcut(CTRL, Key::S)
                .menu("file", 200)
                .enabled(|c| c.project_info().is_some_and(|p| p.modified)),
        );
        reg.action(
            ActionDef::new("close", "action.close_project", |c| {
                c.request(EditorRequest::CloseProject);
            })
            .icon(icons::X_CIRCLE)
            .category("category.file")
            .shortcut(CTRL, Key::W)
            .menu("file", 210)
            .enabled(|c| c.has_project()),
        );
        reg.action(
            ActionDef::new("quit", "action.quit", |c| c.request(EditorRequest::Quit))
                .icon(icons::SIGN_OUT)
                .category("category.file")
                .shortcut(CTRL, Key::Q)
                .menu("file", 900),
        );

        // ---- Edit
        reg.action(
            ActionDef::new("undo", "action.undo", |c| {
                if let Err(e) = c.undo() {
                    let message = c.describe(&e);
                    c.error(message);
                }
            })
            .label_with(|c| match c.undo_label() {
                Some(label) => c.tf("action.undo_label", &[("label", &c.undo_text(label))]),
                None => c.t("action.undo"),
            })
            .icon(icons::ARROW_COUNTER_CLOCKWISE)
            .category("category.edit")
            .shortcut(CTRL, Key::Z)
            .menu("edit", 100)
            .enabled(|c| c.can_undo()),
        );
        reg.action(
            ActionDef::new("redo", "action.redo", |c| {
                if let Err(e) = c.redo() {
                    let message = c.describe(&e);
                    c.error(message);
                }
            })
            .label_with(|c| match c.redo_label() {
                Some(label) => c.tf("action.redo_label", &[("label", &c.undo_text(label))]),
                None => c.t("action.redo"),
            })
            .icon(icons::ARROW_CLOCKWISE)
            .category("category.edit")
            .shortcut(CTRL, Key::Y)
            .also_shortcut(CTRL_SHIFT, Key::Z)
            .menu("edit", 110)
            .enabled(|c| c.can_redo()),
        );

        // ---- View
        reg.action(
            ActionDef::new("palette", "action.palette", |c| {
                c.request(EditorRequest::OpenPalette);
            })
            .icon(icons::COMMAND)
            .category("category.view")
            .shortcut(CTRL, Key::K)
            .also_shortcut(CTRL_SHIFT, Key::P)
            .menu("view", 100),
        );
        reg.action(
            ActionDef::new("toggle_theme", "action.toggle_theme", |c| {
                let next = match c.state().theme {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
                c.request(EditorRequest::SetTheme(next));
            })
            .icon(icons::PALETTE)
            .category("category.view")
            .menu("view", 110),
        );
        for lang in Lang::ALL {
            reg.action(
                ActionDef::new(
                    format!("language.{}", lang.code()),
                    "action.language",
                    move |c| c.request(EditorRequest::SetLanguage(lang)),
                )
                .label_with(move |c| c.tf("action.language", &[("language", &lang.native_name())]))
                .menu_group("menu.language")
                .menu_label(lang.native_name())
                .checked(move |c| c.lang() == lang)
                .icon(icons::TRANSLATE)
                .category("category.view")
                .menu("view", 130 + lang as i32),
            );
        }
        reg.action(
            ActionDef::new("experimental_3d", "action.experimental_3d", |c| {
                let on = !orbit_core::experimental::preference_3d();
                orbit_core::experimental::set_preference_3d(on);
                c.set_pref(
                    super::PREF_PLUGIN,
                    super::PREF_3D,
                    if on { "1" } else { "0" },
                );
            })
            .checked(|_| orbit_core::experimental::enabled_3d())
            .icon(icons::CUBE)
            .category("category.view")
            .menu("view", 125),
        );
        reg.action(
            ActionDef::new("auto_validation", "action.auto_validation", |c| {
                let on = !c.state().validation.auto;
                c.state_mut().validation.auto = on;
                c.set_pref(
                    super::PREF_PLUGIN,
                    super::PREF_AUTO_VALIDATION,
                    if on { "1" } else { "0" },
                );
            })
            .checked(|c| c.state().validation.auto)
            .icon(icons::CHECK_CIRCLE)
            .category("category.view")
            .menu("view", 126),
        );
        reg.action(
            ActionDef::new("validate", "action.validate", |c| {
                c.state_mut().validation.requested = true;
            })
            .icon(icons::CHECK_CIRCLE)
            .category("category.edit")
            .shortcut(egui::Modifiers::CTRL, Key::F7)
            .menu("edit", 399)
            .enabled(|c| c.has_project()),
        );        reg.action(
            ActionDef::new("reset_layout", "action.reset_layout", |c| {
                c.request(EditorRequest::ResetLayout);
            })
            .icon(icons::LAYOUT)
            .category("category.view")
            .menu("view", 120),
        );
    }
}
