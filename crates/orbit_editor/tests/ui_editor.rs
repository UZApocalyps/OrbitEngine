//! The interface editor panel (`orbit.ui`) in the whole editor, headless:
//! it opens, draws and settles with and without a `ui/` folder, and shows
//! the diagnostics of a broken one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor_api::EditorRequest;
use orbit_project::MANIFEST_FILE;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-editor-uied-{tag}-{}-{:?}",
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

fn demo_project(root: &Path) -> PathBuf {
    let project = root.join("demo-project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stories/demo.ron"),
        project.join("story.ron"),
    )
    .unwrap();
    std::fs::write(
        project.join(MANIFEST_FILE),
        "(format: 1, name: \"Demo\", story: \"story.ron\", plugins: [])",
    )
    .unwrap();
    project
}

fn frame(core: &mut EditorCore, ctx: &Context, time: f64) -> Duration {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1360.0, 820.0))),
        time: Some(time),
        ..RawInput::default()
    };
    let output = ctx.run_ui(input, |ui| core.ui(ui));
    let delay = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map_or(Duration::MAX, |v| v.repaint_delay);
    output.drop_without_applying_deltas();
    delay
}

fn settle(core: &mut EditorCore, ctx: &Context) {
    for i in 0..80 {
        if frame(core, ctx, f64::from(i) * 0.016) != Duration::ZERO && i > 2 {
            return;
        }
    }
    panic!("the UI never settled: it would redraw forever");
}

fn open_panel(core: &mut EditorCore, ctx: &Context) {
    core.state
        .requests
        .push(EditorRequest::RunAction("orbit.ui.open".to_owned()));
    settle(core, ctx);
    assert!(
        orbit_editor::dock::open_panels(&core.dock).contains(&"orbit.ui.editor".to_owned()),
        "the action shows the panel"
    );
}

fn core_with_project(dir: &TempDir) -> (EditorCore, Context, PathBuf) {
    let ctx = Context::default();
    let project = demo_project(dir.path());
    let mut core = EditorCore::new(
        CoreOptions::with_language(orbit_editor_api::Lang::En),
        &ctx,
    );
    core.open_project(&project);
    settle(&mut core, &ctx);
    (core, ctx, project)
}

#[test]
fn the_panel_registers_and_opens_without_a_ui_folder() {
    let dir = TempDir::new("none");
    let (mut core, ctx, project) = core_with_project(&dir);
    assert!(core.registry.find_panel("orbit.ui.editor").is_some());
    assert!(core.registry.find_action("orbit.ui.open").is_some());
    open_panel(&mut core, &ctx);
    settle(&mut core, &ctx);
    assert!(!project.join("ui").exists(), "drawing the panel writes nothing");
    assert!(core.state.preview.ui_request.is_none());
}

#[test]
fn the_panel_draws_a_project_with_stock_screens_and_a_broken_splash() {
    let dir = TempDir::new("stock");
    let (mut core, ctx, project) = core_with_project(&dir);
    // The stock bundle on disk, plus a splash that cannot be valid.
    let mut bundle = orbit_ui::UiBundle::builtin();
    bundle.splashes.push(orbit_ui::SplashDef {
        content: orbit_ui::SplashContent::Image("missing.png".into()),
        duration: 0.0,
        fade_in: 0.5,
        fade_out: 0.5,
        skippable: true,
        background: [0, 0, 0, 255],
    });
    std::fs::create_dir_all(project.join("ui")).unwrap();
    for s in &bundle.screens {
        std::fs::write(
            project.join("ui").join(format!("{}.screen.ron", s.id)),
            orbit_ui::asset::to_ron(s).unwrap(),
        )
        .unwrap();
    }
    std::fs::write(
        project.join("ui/theme.ron"),
        bundle.theme.as_ref().unwrap().to_ron().unwrap(),
    )
    .unwrap();
    std::fs::write(
        project.join("ui/ui.ron"),
        "(splashes: [(content: Image(\"missing.png\"), duration: 0.0)])",
    )
    .unwrap();
    // Re-open so the panel reads the folder.
    core.open_project(&project);
    open_panel(&mut core, &ctx);
    settle(&mut core, &ctx);
    // Nothing was asked of the preview until the author turns it on.
    assert!(core.state.preview.ui_request.is_none());
}

#[test]
fn an_unreadable_folder_is_reported_not_fatal() {
    let dir = TempDir::new("broken");
    let (mut core, ctx, project) = core_with_project(&dir);
    std::fs::create_dir_all(project.join("ui")).unwrap();
    std::fs::write(project.join("ui/bad.screen.ron"), "(id: ").unwrap();
    core.open_project(&project);
    open_panel(&mut core, &ctx);
    settle(&mut core, &ctx);
}

#[test]
fn unsaved_interface_edits_count_as_modified_and_ask_before_switching_project() {
    use orbit_editor::core::{Dialog, Pending, UnsavedChoice};
    let dir = TempDir::new("dirty");
    let (mut core, ctx, project) = core_with_project(&dir);
    assert!(!core.is_modified());
    // What the panel publishes while its document has unsaved edits.
    core.registry.ui_link.dirty.set(true);
    assert!(core.is_modified());
    settle(&mut core, &ctx);
    assert!(core.state.project.as_ref().unwrap().modified);

    core.open_project(&project);
    assert_eq!(
        core.dialog,
        Some(Dialog::ConfirmUnsaved {
            then: Pending::Open(project.clone())
        })
    );
    core.dialog = None;
    // Discarding goes through and forgets the flag.
    core.resolve_unsaved(Pending::Open(project), UnsavedChoice::Discard);
    assert!(!core.registry.ui_link.dirty.get());
    assert!(!core.is_modified());
}

#[test]
fn undo_and_redo_go_to_the_interface_editor_while_it_has_the_focus() {
    let dir = TempDir::new("undo");
    let (mut core, ctx, _project) = core_with_project(&dir);
    let key = |key: egui::Key, shift: bool| egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            command: true,
            ctrl: true,
            shift,
            ..Default::default()
        },
    };
    let press = |core: &mut EditorCore, ctx: &Context, event: egui::Event| {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1360.0, 820.0))),
            time: Some(10.0),
            events: vec![event],
            ..RawInput::default()
        };
        let output = ctx.run_ui(input, |ui| core.ui(ui));
        output.drop_without_applying_deltas();
    };
    // No focus: the panel is not asked.
    press(&mut core, &ctx, key(egui::Key::Z, false));
    assert_eq!(core.registry.ui_link.history.get(), 0);
    // Focus: Ctrl+Z is an undo for it, Ctrl+Shift+Z a redo.
    core.registry.ui_link.focus.set(true);
    press(&mut core, &ctx, key(egui::Key::Z, false));
    assert_eq!(core.registry.ui_link.history.get(), -1);
    core.registry.ui_link.focus.set(true);
    press(&mut core, &ctx, key(egui::Key::Z, true));
    assert_eq!(core.registry.ui_link.history.get(), 0, "undo then redo");
}
