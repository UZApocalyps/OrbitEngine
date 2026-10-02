//! Core actions: file, edit and view menus. The first built-in plugin, so
//! its menus come first.

use egui::{Key, Modifiers};
use orbit_editor_api::{
    ActionDef, EditorPlugin, EditorRegistry, EditorRequest, Plugin, PluginManifest, ThemeMode,
    icons,
};

use super::ids;

pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::CORE, "Actions de base", "0.1.0")
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
        reg.menu("file", "Fichier", 10);
        reg.menu("edit", "Édition", 20);
        reg.menu("view", "Affichage", 30);
        reg.menu("tools", "Outils", 40);

        // ---- File
        reg.action(
            ActionDef::new("new", "Nouveau projet…", |c| {
                c.request(EditorRequest::NewProjectDialog);
            })
            .icon(icons::FILE_PLUS)
            .category("Fichier")
            .shortcut(CTRL, Key::N)
            .menu("file", 100),
        );
        reg.action(
            ActionDef::new("open", "Ouvrir un projet…", |c| {
                c.request(EditorRequest::PickOpenProject);
            })
            .icon(icons::FOLDER_OPEN)
            .category("Fichier")
            .shortcut(CTRL, Key::O)
            .menu("file", 110),
        );
        reg.action(
            ActionDef::new("open_story", "Ouvrir une histoire (.ron)…", |c| {
                c.request(EditorRequest::PickOpenStory);
            })
            .icon(icons::FILM_SLATE)
            .category("Fichier")
            .shortcut(CTRL_SHIFT, Key::O)
            .menu("file", 120),
        );
        reg.action(
            ActionDef::new("save", "Enregistrer", |c| c.request(EditorRequest::Save))
                .icon(icons::FLOPPY_DISK)
                .category("Fichier")
                .shortcut(CTRL, Key::S)
                .menu("file", 200)
                .enabled(|c| c.project_info().is_some_and(|p| p.modified)),
        );
        reg.action(
            ActionDef::new("close", "Fermer le projet", |c| {
                c.request(EditorRequest::CloseProject);
            })
            .icon(icons::X_CIRCLE)
            .category("Fichier")
            .shortcut(CTRL, Key::W)
            .menu("file", 210)
            .enabled(|c| c.has_project()),
        );
        reg.action(
            ActionDef::new("quit", "Quitter", |c| c.request(EditorRequest::Quit))
                .icon(icons::SIGN_OUT)
                .category("Fichier")
                .shortcut(CTRL, Key::Q)
                .menu("file", 900),
        );

        // ---- Edit
        reg.action(
            ActionDef::new("undo", "Annuler", |c| {
                if let Err(e) = c.undo() {
                    c.error(e.to_string());
                }
            })
            .label_with(|c| match c.undo_label() {
                Some(label) => format!("Annuler : {label}"),
                None => "Annuler".to_owned(),
            })
            .icon(icons::ARROW_COUNTER_CLOCKWISE)
            .category("Édition")
            .shortcut(CTRL, Key::Z)
            .menu("edit", 100)
            .enabled(|c| c.can_undo()),
        );
        reg.action(
            ActionDef::new("redo", "Rétablir", |c| {
                if let Err(e) = c.redo() {
                    c.error(e.to_string());
                }
            })
            .label_with(|c| match c.redo_label() {
                Some(label) => format!("Rétablir : {label}"),
                None => "Rétablir".to_owned(),
            })
            .icon(icons::ARROW_CLOCKWISE)
            .category("Édition")
            .shortcut(CTRL, Key::Y)
            .also_shortcut(CTRL_SHIFT, Key::Z)
            .menu("edit", 110)
            .enabled(|c| c.can_redo()),
        );

        // ---- View
        reg.action(
            ActionDef::new("palette", "Palette de commandes", |c| {
                c.request(EditorRequest::OpenPalette);
            })
            .icon(icons::COMMAND)
            .category("Affichage")
            .shortcut(CTRL, Key::K)
            .also_shortcut(CTRL_SHIFT, Key::P)
            .menu("view", 100),
        );
        reg.action(
            ActionDef::new("toggle_theme", "Basculer le thème clair / sombre", |c| {
                let next = match c.state().theme {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
                c.request(EditorRequest::SetTheme(next));
            })
            .icon(icons::PALETTE)
            .category("Affichage")
            .menu("view", 110),
        );
        reg.action(
            ActionDef::new("reset_layout", "Réinitialiser la disposition", |c| {
                c.request(EditorRequest::ResetLayout);
            })
            .icon(icons::LAYOUT)
            .category("Affichage")
            .menu("view", 120),
        );
    }
}
