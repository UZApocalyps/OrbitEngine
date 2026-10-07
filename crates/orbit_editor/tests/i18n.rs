//! The editor's interface languages: first-launch choice, persistence, hot
//! switching, registrations that are all keys, plugin text tables.

use std::path::{Path, PathBuf};

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_core::{Node, NodeId, NodeKind};
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor_api::{EditorRequest, Lang, Level};
use orbit_project::Command;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-editor-i18n-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn frame(core: &mut EditorCore, ctx: &Context, time: f64) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
        time: Some(time),
        ..RawInput::default()
    };
    let output = ctx.run_ui(input, |ui| core.ui(ui));
    output.drop_without_applying_deltas();
}

fn with_config(dir: &Path, language: Option<Lang>) -> CoreOptions {
    CoreOptions {
        config_dir: Some(dir.to_owned()),
        language,
        ..CoreOptions::default()
    }
}

/// A project whose first scene jumps to a label that does not exist.
fn project_with_a_broken_jump(core: &mut EditorCore, parent: &Path) {
    let texts = orbit_editor::i18n::translator(Lang::En);
    let root = orbit_editor::storage::create_project(parent, "Broken", &texts).unwrap();
    core.open_project(&root);
    let session = core.session.as_mut().unwrap();
    let scene = session.project.story().scenes[0].id;
    session
        .project
        .execute(Command::InsertNode {
            scene,
            index: 0,
            node: Node {
                id: NodeId::new(),
                kind: NodeKind::Jump {
                    target: NodeId::new(),
                },
            },
        })
        .unwrap();
}

#[test]
fn the_language_follows_the_system_on_first_launch() {
    // `Lang::from_system` is what `EditorCore` asks of the platform.
    assert_eq!(Lang::from_system(Some("fr-FR")), Lang::Fr);
    assert_eq!(Lang::from_system(Some("fr")), Lang::Fr);
    assert_eq!(Lang::from_system(Some("en-US")), Lang::En);
    assert_eq!(Lang::from_system(Some("de-DE")), Lang::En);
    assert_eq!(Lang::from_system(None), Lang::En);
}

#[test]
fn the_choice_is_remembered_and_the_debug_option_is_not() {
    let dir = TempDir::new("persist");
    let ctx = Context::default();
    {
        let mut core = EditorCore::new(with_config(dir.path(), Some(Lang::Fr)), &ctx);
        assert_eq!(core.lang(), Lang::Fr);
        // `--lang` alone does not write the choice...
        core.persist();
    }
    let ctx = Context::default();
    {
        let mut core = EditorCore::new(with_config(dir.path(), Some(Lang::En)), &ctx);
        assert_eq!(core.lang(), Lang::En);
        // ...picking a language in the menu does.
        core.state
            .requests
            .push(EditorRequest::SetLanguage(Lang::Fr));
        core.pump(&ctx);
        assert_eq!(core.lang(), Lang::Fr);
    }
    let ctx = Context::default();
    let core = EditorCore::new(with_config(dir.path(), None), &ctx);
    assert_eq!(
        core.lang(),
        Lang::Fr,
        "the saved choice wins over the system"
    );
}

#[test]
fn switching_the_language_is_immediate() {
    let ctx = Context::default();
    let mut core = EditorCore::new(CoreOptions::with_language(Lang::Fr), &ctx);
    frame(&mut core, &ctx, 0.0);
    assert_eq!(core.state.t("menu.file"), "Fichier");
    assert_eq!(
        core.state.t("orbit.sample.hello"),
        "Plugin d'essai : dire bonjour"
    );

    core.run_action_now("orbit.core.language.en", &ctx);
    frame(&mut core, &ctx, 0.1);
    assert_eq!(core.lang(), Lang::En);
    assert_eq!(core.state.t("menu.file"), "File");
    assert_eq!(
        core.state.t("orbit.sample.hello"),
        "Sample plugin: say hello"
    );
    // The title of the panels (the dock tabs) is resolved when drawn.
    let title = |core: &EditorCore| {
        let panel = core
            .registry
            .find_panel(orbit_editor::builtin::panels::WELCOME)
            .unwrap();
        core.state.t(&panel.def.title)
    };
    assert_eq!(title(&core), "Welcome");

    core.run_action_now("orbit.core.language.fr", &ctx);
    assert_eq!(title(&core), "Bienvenue");
}

#[test]
fn every_registered_text_is_a_key_in_both_languages() {
    let ctx = Context::default();
    let core = EditorCore::new(CoreOptions::with_language(Lang::En), &ctx);
    let reg = &core.registry;
    let (en, fr) = (
        orbit_editor::i18n::translator(Lang::En),
        orbit_editor::i18n::translator(Lang::Fr),
    );
    for t in [&en, &fr] {
        for (lang, table) in reg.text_tables() {
            if *lang == t.lang() {
                t.add_table(*lang, table.clone());
            }
        }
    }
    let mut missing = Vec::new();
    let mut check = |what: &str, id: &str, key: &str| {
        if key.is_empty() {
            return;
        }
        for t in [&en, &fr] {
            if !t.has(key) {
                missing.push(format!("{what} `{id}`: `{key}` ({})", t.lang().code()));
            }
        }
    };
    for panel in reg.panels() {
        check("panel", &panel.def.id, &panel.def.title);
    }
    for action in reg.actions() {
        check("action label", &action.def.id, &action.def.label);
        check("action category", &action.def.id, &action.def.category);
        if let Some(group) = &action.def.menu_group {
            check("action menu group", &action.def.id, group);
        }
    }
    for menu in reg.menus() {
        check("menu", &menu.id, &menu.label);
    }
    assert!(
        missing.is_empty(),
        "texts given to the registry that are not keys:\n{}",
        missing.join("\n")
    );
}

#[test]
fn shown_shortcuts_are_translated() {
    let en = orbit_editor::i18n::translator(Lang::En);
    let fr = orbit_editor::i18n::translator(Lang::Fr);
    let shortcut = egui::KeyboardShortcut::new(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Delete,
    );
    assert_eq!(
        orbit_editor_api::shortcut_text(&en, &shortcut),
        "Ctrl+Shift+Del"
    );
    assert_eq!(
        orbit_editor_api::shortcut_text(&fr, &shortcut),
        "Ctrl+Maj+Suppr"
    );
}

#[test]
fn the_problems_are_written_again_in_the_new_language() {
    let dir = TempDir::new("problems");
    let ctx = Context::default();
    let mut core = EditorCore::new(CoreOptions::with_language(Lang::Fr), &ctx);
    project_with_a_broken_jump(&mut core, dir.path());
    for i in 0..4 {
        frame(&mut core, &ctx, f64::from(i) * 0.016);
    }
    core.settle_diagnostics();
    let first = core.state.diagnostics.problems.first().cloned().unwrap();
    assert_eq!(first.level, Level::Error);
    assert!(
        first.message.starts_with("Cible introuvable"),
        "{}",
        first.message
    );

    core.run_action_now("orbit.core.language.en", &ctx);
    for i in 0..4 {
        frame(&mut core, &ctx, 1.0 + f64::from(i) * 0.016);
    }
    core.settle_diagnostics();
    let first = core.state.diagnostics.problems.first().cloned().unwrap();
    assert!(
        first.message.starts_with("Target not found"),
        "{}",
        first.message
    );
}

#[test]
fn a_new_project_template_and_errors_follow_the_language() {
    let dir = TempDir::new("template");
    let fr = orbit_editor::i18n::translator(Lang::Fr);
    let en = orbit_editor::i18n::translator(Lang::En);
    let story_fr = orbit_editor::storage::minimal_story("Jeu", &fr);
    let story_en = orbit_editor::storage::minimal_story("Game", &en);
    assert_eq!(story_fr.scenes[0].name, "Introduction");
    assert_eq!(story_en.scenes[0].name, "Introduction");
    let first = |story: &orbit_core::Story| match &story.scenes[0].nodes[0].kind {
        NodeKind::Say { text_id, .. } => story.text(*text_id).unwrap().to_owned(),
        other => panic!("{other:?}"),
    };
    assert_eq!(first(&story_fr), "Bienvenue dans « Jeu ».");
    assert_eq!(first(&story_en), "Welcome to “Game”.");

    let err = orbit_editor::storage::create_project(dir.path(), "  ", &en).unwrap_err();
    assert_eq!(err, "the project name is empty");
    let err = orbit_editor::storage::create_project(dir.path(), "  ", &fr).unwrap_err();
    assert_eq!(err, "le nom du projet est vide");
}

#[test]
fn refusals_of_the_project_layer_are_translated() {
    let en = orbit_editor::i18n::translator(Lang::En);
    let fr = orbit_editor::i18n::translator(Lang::Fr);
    let mut project = orbit_project::Project::new(orbit_core::Story::new("t"));
    let err = orbit_project::ops::paste_nodes(
        &mut project,
        &orbit_project::ops::NodeClip::default(),
        orbit_core::SceneId::new(),
        0,
    )
    .unwrap_err();
    assert_eq!(
        orbit_editor_api::describe_project_error(&fr, &err),
        "Le presse-papiers est vide."
    );
    assert_eq!(
        orbit_editor_api::describe_project_error(&en, &err),
        "The clipboard is empty."
    );
    assert_eq!(
        orbit_editor_api::describe_undo_label(&en, "Coller"),
        "Paste"
    );
    assert_eq!(
        orbit_editor_api::describe_undo_label(&en, "Anything else"),
        "Anything else"
    );
}
