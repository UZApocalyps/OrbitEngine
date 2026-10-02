//! Headless tests of the whole editor logic (no window, no GPU): plugins,
//! palette, projects, undo / redo, save, recovery, layout, idle behaviour.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_core::{CustomData, NodeKind, Story};
use orbit_editor::core::{CoreOptions, Dialog, EditorCore, Pending, UnsavedChoice};
use orbit_editor::palette;
use orbit_editor_api::{EditorContext, EditorRequest, Selection, ThemeMode};
use orbit_project::{Command, MANIFEST_FILE};

// ------------------------------------------------------------------ helpers

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-editor-it-{tag}-{}-{:?}",
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

fn demo_story_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stories/demo.ron")
}

/// A project folder with a manifest and a copy of the demo story.
fn demo_project(root: &Path) -> PathBuf {
    let project = root.join("demo-project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::copy(demo_story_path(), project.join("story.orbit")).unwrap();
    std::fs::write(
        project.join(MANIFEST_FILE),
        "(format: 1, name: \"Demo\", story: \"story.orbit\", plugins: [])",
    )
    .unwrap();
    project
}

fn options(config: Option<&Path>) -> CoreOptions {
    CoreOptions {
        config_dir: config.map(Path::to_owned),
        ..CoreOptions::with_language(orbit_editor_api::Lang::Fr)
    }
}

/// Runs one UI frame; returns egui's requested repaint delay.
fn frame(core: &mut EditorCore, ctx: &Context, time: f64, size: (f32, f32)) -> Duration {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(size.0, size.1))),
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

/// Frames until egui stops asking for repaints (bounded).
fn settle(core: &mut EditorCore, ctx: &Context, size: (f32, f32)) -> usize {
    for i in 0..80 {
        if frame(core, ctx, i as f64 * 0.016, size) != Duration::ZERO && i > 2 {
            return i;
        }
    }
    panic!("the UI never settled: it would redraw forever");
}

fn first_say(core: &EditorCore) -> orbit_core::NodeId {
    core.story()
        .unwrap()
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .find(|n| matches!(n.kind, NodeKind::Say { .. }))
        .unwrap()
        .id
}

fn node_count(core: &EditorCore) -> usize {
    core.story()
        .unwrap()
        .scenes
        .iter()
        .map(|s| s.nodes.len())
        .sum()
}

// -------------------------------------------------------------------- tests

#[test]
fn builtin_plugins_register_panels_actions_and_the_sample_plugin() {
    let ctx = Context::default();
    let core = EditorCore::new(options(None), &ctx);
    let reg = &core.registry;
    assert!(reg.errors.is_empty(), "{:?}", reg.errors);
    for panel in [
        "orbit.explorer.explorer",
        "orbit.inspector.inspector",
        "orbit.console.console",
        "orbit.welcome.welcome",
        "orbit.preview.preview",
        "orbit.sample.panel",
    ] {
        assert!(reg.find_panel(panel).is_some(), "missing panel {panel}");
    }
    for action in [
        "orbit.core.save",
        "orbit.core.undo",
        "orbit.core.redo",
        "orbit.core.palette",
        "orbit.sample.hello",
    ] {
        assert!(reg.find_action(action).is_some(), "missing action {action}");
    }
    // The sample plugin's node type and an inspector for `Say` are registered.
    assert!(reg.project.node_type("orbit.sample", "note").is_some());
    assert!(reg.inspector_for("Say").is_some());
    // A panel-opening action exists for every panel.
    assert!(
        reg.find_action("orbit.core.views.show.orbit.console.console")
            .is_some()
    );
    // Undo and redo carry both historical shortcuts.
    let redo = reg.find_action("orbit.core.redo").unwrap();
    assert_eq!(redo.shortcuts().count(), 2);
    // Dock default layout: the hidden sample panel is not part of it.
    let tabs = orbit_editor::dock::open_panels(&core.dock);
    assert!(tabs.contains(&"orbit.explorer.explorer".to_owned()));
    assert!(!tabs.contains(&"orbit.sample.panel".to_owned()));
}

#[test]
fn palette_finds_the_sample_action_and_the_custom_node_round_trips() {
    let ctx = Context::default();
    let mut core = EditorCore::new(options(None), &ctx);

    let found: Vec<String> = {
        let mut state = std::mem::take(&mut core.state);
        let items = {
            let ectx = EditorContext::new(None, &mut state, &core.registry);
            palette::search(&core.registry, &ectx, "essai bonj")
        };
        core.state = state;
        items.into_iter().map(|i| i.action_id).collect()
    };
    assert_eq!(
        found.first().map(String::as_str),
        Some("orbit.sample.hello")
    );

    // Running it through the palette's path shows a toast.
    core.run_action_now("orbit.sample.hello", &ctx);
    assert!(
        core.state
            .toasts
            .iter()
            .any(|t| t.message.contains("Bonjour"))
    );

    // A custom node of the sample plugin survives an RON round trip.
    let dir = TempDir::new("custom");
    let project = demo_project(dir.path());
    core.open_project(&project);
    core.state.selection = Some(Selection::Node(first_say(&core)));
    let before = node_count(&core);
    core.run_action_now("orbit.sample.insert_note", &ctx);
    assert_eq!(node_count(&core), before + 1);
    let story = core.story().unwrap().clone();
    let custom = story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .find(|n| matches!(&n.kind, NodeKind::Custom { plugin_id, .. } if plugin_id == "orbit.sample"))
        .expect("the note node was inserted");
    let NodeKind::Custom { data, .. } = &custom.kind else {
        unreachable!()
    };
    assert_eq!(data.get("text"), Some(&CustomData::Str("Une note".into())));
    let back = Story::from_ron(&story.to_ron().unwrap()).unwrap();
    assert_eq!(back, story);
    // The plugin's validator finds nothing wrong, and the hook saw the change.
    assert!(core.registry.project.validate_custom(&story).is_empty());
}

#[test]
fn edit_undo_redo_save_and_validate() {
    let ctx = Context::default();
    let dir = TempDir::new("edit");
    let project = demo_project(dir.path());
    let mut core = EditorCore::new(options(None), &ctx);
    core.open_project(&project);
    assert!(core.has_project());
    assert!(!core.is_modified());

    let say = first_say(&core);
    core.state.selection = Some(Selection::Node(say));
    let original = core.story().unwrap().clone();
    let nodes = node_count(&core);

    // Edit the text of the selected line through the editor context.
    {
        let session = core.session.as_mut().unwrap();
        let mut ectx =
            EditorContext::new(Some(&mut session.project), &mut core.state, &core.registry);
        let Some(NodeKind::Say { speaker, text_id }) =
            ectx.story().unwrap().node(say).map(|n| n.kind.clone())
        else {
            panic!("say expected")
        };
        let _ = text_id;
        for text in ["B", "Bo", "Bon", "Bonjour"] {
            let command = Command::SetSay {
                id: say,
                speaker,
                text: text.into(),
                expected_version: ectx.node_version(say),
            };
            ectx.exec_merged(command, "test").unwrap();
        }
    }
    core.after_frame(&ctx);
    assert!(core.is_modified());
    assert!(core.state.project.as_ref().unwrap().modified);
    assert!(core.window_title().starts_with('●'));

    // One undo reverts the whole typing session.
    core.run_action_now("orbit.core.undo", &ctx);
    assert_eq!(core.story().unwrap(), &original);
    assert!(!core.is_modified());
    core.run_action_now("orbit.core.redo", &ctx);
    assert!(core.is_modified());

    // Add a line, undo / redo it with the label in the menu.
    core.run_action_now("orbit.explorer.add_say", &ctx);
    assert_eq!(node_count(&core), nodes + 1);
    {
        let session = core.session.as_mut().unwrap();
        let ectx = EditorContext::new(Some(&mut session.project), &mut core.state, &core.registry);
        assert_eq!(ectx.undo_label(), Some("Ajouter une réplique"));
    }
    core.run_action_now("orbit.core.undo", &ctx);
    assert_eq!(node_count(&core), nodes);
    core.run_action_now("orbit.core.redo", &ctx);
    assert_eq!(node_count(&core), nodes + 1);

    // Delete it again (selection followed the new node), one undo step.
    core.run_action_now("orbit.explorer.delete", &ctx);
    assert_eq!(node_count(&core), nodes);
    core.run_action_now("orbit.core.undo", &ctx);
    assert_eq!(node_count(&core), nodes + 1);

    // Save: file on disk equals the session story and is valid.
    let edited = core.story().unwrap().clone();
    core.run_action_now("orbit.core.save", &ctx);
    assert!(!core.is_modified());
    let on_disk = Story::from_ron(&std::fs::read_to_string(project.join("story.orbit")).unwrap())
        .expect("saved story validates");
    assert_eq!(on_disk, edited);
    assert!(orbit_core::validate(&on_disk).errors.is_empty());
    // No temporary file is left next to it.
    let leftovers: Vec<_> = std::fs::read_dir(&project)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn shortcut_undo_is_reachable_through_requests_and_selection_follows() {
    let ctx = Context::default();
    let dir = TempDir::new("selection");
    let project = demo_project(dir.path());
    let mut core = EditorCore::new(options(None), &ctx);
    core.open_project(&project);
    core.state.selection = Some(Selection::Node(first_say(&core)));
    core.run_action_now("orbit.explorer.add_say", &ctx);
    let Some(Selection::Node(added)) = core.state.selection else {
        panic!("the new line is selected")
    };
    core.state.selection = None;
    core.run_action_now("orbit.core.undo", &ctx);
    core.after_frame(&ctx);
    // Undone: the line is gone and nothing dangling is selected.
    assert!(core.story().unwrap().node(added).is_none());
    assert_ne!(core.state.selection, Some(Selection::Node(added)));
    core.state
        .requests
        .push(EditorRequest::RunAction("orbit.core.redo".into()));
    core.pump(&ctx);
    core.after_frame(&ctx);
    // Redone: the selection follows the node that came back.
    assert_eq!(core.state.selection, Some(Selection::Node(added)));
}

#[test]
fn crash_then_relaunch_offers_recovery() {
    let ctx = Context::default();
    let dir = TempDir::new("recovery");
    let project = demo_project(dir.path());
    let config = dir.path().join("config");

    let mut opts = options(Some(&config));
    opts.autosave_interval = Duration::ZERO;
    let edited = {
        let mut core = EditorCore::new(opts.clone(), &ctx);
        core.open_project(&project);
        core.state.selection = Some(Selection::Node(first_say(&core)));
        core.run_action_now("orbit.explorer.add_say", &ctx);
        core.after_frame(&ctx); // autosave happens here
        assert!(core.is_modified());
        let edited = core.story().unwrap().clone();
        // The snapshot is next to the project, the project file is untouched.
        assert!(project.join(".orbit/recovery/story.recovery.ron").is_file());
        let on_disk = std::fs::read_to_string(project.join("story.orbit")).unwrap();
        assert_ne!(Story::from_ron(&on_disk).unwrap(), edited);
        edited
        // `core` is dropped without `shutdown()`: a crash.
    };

    // Relaunch: the recovery question is asked, "Restaurer" brings the work back.
    let mut core = EditorCore::new(opts.clone(), &ctx);
    core.open_project(&project);
    assert!(matches!(core.dialog, Some(Dialog::Recovery { .. })));
    assert!(!core.is_modified());
    core.dialog = None;
    core.resolve_recovery(true);
    assert_eq!(core.story().unwrap(), &edited);
    assert!(core.is_modified()); // restored work still has to be saved

    // Another crash and a relaunch where the user ignores the snapshot.
    let mut core2 = EditorCore::new(opts, &ctx);
    core2.open_project(&project);
    assert!(matches!(core2.dialog, Some(Dialog::Recovery { .. })));
    core2.dialog = None;
    core2.resolve_recovery(false);
    assert!(!core2.is_modified());
    assert!(!project.join(".orbit/recovery/story.recovery.ron").exists());
}

#[test]
fn a_clean_exit_leaves_no_recovery_file() {
    let ctx = Context::default();
    let dir = TempDir::new("clean");
    let project = demo_project(dir.path());
    let mut opts = options(None);
    opts.autosave_interval = Duration::ZERO;
    let mut core = EditorCore::new(opts, &ctx);
    core.open_project(&project);
    core.run_action_now("orbit.explorer.add_say", &ctx);
    core.after_frame(&ctx);
    assert!(project.join(".orbit/recovery/story.recovery.ron").is_file());
    core.run_action_now("orbit.core.save", &ctx);
    assert!(!project.join(".orbit/recovery/story.recovery.ron").exists());
    core.shutdown();
}

#[test]
fn closing_with_unsaved_changes_asks_first() {
    let ctx = Context::default();
    let dir = TempDir::new("close");
    let project = demo_project(dir.path());
    let mut core = EditorCore::new(options(None), &ctx);
    core.open_project(&project);
    assert!(
        core.request_close(),
        "nothing to save: the window can close"
    );
    let mut core = EditorCore::new(options(None), &ctx);
    core.open_project(&project);
    core.run_action_now("orbit.explorer.add_say", &ctx);
    core.after_frame(&ctx);

    assert!(!core.request_close());
    assert_eq!(
        core.dialog,
        Some(Dialog::ConfirmUnsaved {
            then: Pending::Quit
        })
    );
    // Cancel keeps everything; Discard quits and forgets the snapshot.
    core.resolve_unsaved(Pending::Quit, UnsavedChoice::Cancel);
    assert!(!core.take_quit());
    core.resolve_unsaved(Pending::Quit, UnsavedChoice::Discard);
    assert!(core.take_quit());

    // Save quits too, and the file has the new line.
    let mut core = EditorCore::new(options(None), &ctx);
    core.open_project(&project);
    let nodes = node_count(&core);
    core.run_action_now("orbit.explorer.add_say", &ctx);
    core.after_frame(&ctx);
    core.resolve_unsaved(Pending::Quit, UnsavedChoice::Save);
    assert!(core.take_quit());
    let on_disk =
        Story::from_ron(&std::fs::read_to_string(project.join("story.orbit")).unwrap()).unwrap();
    assert_eq!(
        on_disk.scenes.iter().map(|s| s.nodes.len()).sum::<usize>(),
        nodes + 1
    );
}

#[test]
fn opening_another_project_with_changes_asks_first() {
    let ctx = Context::default();
    let dir = TempDir::new("switch");
    let a = demo_project(dir.path());
    let b = orbit_editor::storage::create_project(
        dir.path(),
        "Autre",
        &orbit_editor::i18n::translator(orbit_editor_api::Lang::Fr),
    )
    .unwrap();
    let mut core = EditorCore::new(options(None), &ctx);
    core.open_project(&a);
    core.run_action_now("orbit.explorer.add_say", &ctx);
    core.after_frame(&ctx);
    core.open_project(&b);
    assert_eq!(
        core.dialog,
        Some(Dialog::ConfirmUnsaved {
            then: Pending::Open(b.clone())
        })
    );
    assert_eq!(core.state.project.as_ref().unwrap().name, "Demo");
    core.resolve_unsaved(Pending::Open(b.clone()), UnsavedChoice::Discard);
    assert_eq!(core.state.project.as_ref().unwrap().name, "Autre");
    // The discarded project's snapshot is gone.
    assert!(!a.join(".orbit/recovery/story.recovery.ron").exists());
}

#[test]
fn layout_and_theme_persist_between_launches() {
    let ctx = Context::default();
    let dir = TempDir::new("layout");
    let config = dir.path().join("config");
    {
        let mut core = EditorCore::new(options(Some(&config)), &ctx);
        // Open the hidden panel and switch theme; the layout is saved after the frame.
        core.run_action_now("orbit.core.views.show.orbit.sample.panel", &ctx);
        core.run_action_now("orbit.core.toggle_theme", &ctx);
        core.after_frame(&ctx);
        assert_eq!(core.state.theme, ThemeMode::Light);
    }
    let core = EditorCore::new(options(Some(&config)), &ctx);
    assert_eq!(core.state.theme, ThemeMode::Light);
    let tabs = orbit_editor::dock::open_panels(&core.dock);
    assert!(tabs.contains(&"orbit.sample.panel".to_owned()), "{tabs:?}");

    // "Réinitialiser la disposition" brings the default back.
    let mut core = core;
    core.run_action_now("orbit.core.reset_layout", &ctx);
    assert!(
        !orbit_editor::dock::open_panels(&core.dock).contains(&"orbit.sample.panel".to_owned())
    );
}

#[test]
fn recent_projects_are_remembered_and_dead_ones_forgotten() {
    let ctx = Context::default();
    let dir = TempDir::new("recent");
    let project = demo_project(dir.path());
    let config = dir.path().join("config");
    {
        let mut core = EditorCore::new(options(Some(&config)), &ctx);
        core.open_project(&project);
        assert_eq!(core.state.recent.len(), 1);
    }
    let mut core = EditorCore::new(options(Some(&config)), &ctx);
    assert_eq!(core.state.recent[0].path, project);
    // The project disappears: opening it from the list fails and removes it.
    std::fs::remove_dir_all(&project).unwrap();
    core.open_project(&project);
    assert!(core.state.recent.is_empty());
    assert!(
        core.state
            .toasts
            .iter()
            .any(|t| t.level == orbit_editor_api::Level::Error)
    );
}

#[test]
fn a_bare_story_file_opens_without_a_manifest() {
    let ctx = Context::default();
    let dir = TempDir::new("bare");
    let file = dir.path().join("demo.ron");
    std::fs::copy(demo_story_path(), &file).unwrap();
    let mut core = EditorCore::new(options(Some(&dir.path().join("config"))), &ctx);
    core.open_project(&file);
    let info = core.state.project.clone().unwrap();
    assert!(!info.has_manifest);
    assert_eq!(info.story_file, "demo.ron");
    // No `.orbit` folder is created next to a bare story file.
    assert!(!dir.path().join(".orbit").exists());
}

#[test]
fn ui_frames_run_and_settle_in_every_state() {
    let ctx = Context::default();
    let dir = TempDir::new("ui");
    let project = demo_project(dir.path());
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        for size in [
            (1360.0, 820.0),
            (1000.0, 600.0),
            (1360.0, 1000.0),
            (700.0, 500.0),
        ] {
            let mut opts = options(None);
            opts.theme = Some(mode);
            let mut core = EditorCore::new(opts, &ctx);
            // No project: welcome page.
            settle(&mut core, &ctx, size);
            // With a project and a selection: explorer, inspector, console.
            core.open_project(&project);
            core.state.selection = Some(Selection::Node(first_say(&core)));
            settle(&mut core, &ctx, size);
            // Palette open with a query, then a dialog.
            core.palette.open();
            core.palette.query = "ou".into();
            settle(&mut core, &ctx, size);
            core.palette.close();
            core.state.requests.push(EditorRequest::NewProjectDialog);
            settle(&mut core, &ctx, size);
            core.dialog = None;
        }
    }
}

#[test]
fn idle_ui_asks_for_no_more_frames() {
    let ctx = Context::default();
    let dir = TempDir::new("idle");
    let project = demo_project(dir.path());
    let mut core = EditorCore::new(options(None), &ctx);
    core.open_project(&project);
    core.state.selection = Some(Selection::Node(first_say(&core)));
    let size = (1360.0, 820.0);
    let settled_at = settle(&mut core, &ctx, size);
    // Ten more frames with no event: egui never asks to be redrawn.
    for i in 0..10 {
        let delay = frame(&mut core, &ctx, (settled_at + i) as f64 * 0.016, size);
        assert_eq!(delay, Duration::MAX, "idle frame {i} wants a repaint");
    }
    assert!(core.next_wake().is_none(), "nothing scheduled while idle");
}

// ---------------------------------------------------------------- shortcuts

fn ctrl(shift: bool) -> egui::Modifiers {
    // What egui-winit reports on Windows / Linux: `command` mirrors `ctrl`.
    egui::Modifiers {
        ctrl: true,
        command: true,
        shift,
        ..egui::Modifiers::NONE
    }
}

/// One frame with a key press (and release) event.
fn press(core: &mut EditorCore, ctx: &Context, key: egui::Key, modifiers: egui::Modifiers, t: f64) {
    for pressed in [true, false] {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1360.0, 820.0))),
            time: Some(t),
            events: vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers,
            }],
            ..RawInput::default()
        };
        let output = ctx.run_ui(input, |ui| core.ui(ui));
        output.drop_without_applying_deltas();
    }
}

#[test]
fn keyboard_shortcuts_drive_the_registry_actions() {
    use egui::Key;

    let ctx = Context::default();
    let dir = TempDir::new("keys");
    let project = demo_project(dir.path());
    let mut core = EditorCore::new(options(None), &ctx);
    core.open_project(&project);
    settle(&mut core, &ctx, (1360.0, 820.0));
    let nodes = node_count(&core);

    // Ctrl+Enter adds a line, Ctrl+Z undoes, Ctrl+Y and Ctrl+Shift+Z redo.
    press(&mut core, &ctx, Key::Enter, ctrl(false), 10.0);
    assert_eq!(node_count(&core), nodes + 1, "Ctrl+Enter");
    press(&mut core, &ctx, Key::Z, ctrl(false), 11.0);
    assert_eq!(node_count(&core), nodes, "Ctrl+Z");
    press(&mut core, &ctx, Key::Y, ctrl(false), 12.0);
    assert_eq!(node_count(&core), nodes + 1, "Ctrl+Y");
    press(&mut core, &ctx, Key::Z, ctrl(false), 13.0);
    press(&mut core, &ctx, Key::Z, ctrl(true), 14.0);
    assert_eq!(node_count(&core), nodes + 1, "Ctrl+Shift+Z");

    // Ctrl+S saves.
    assert!(core.is_modified());
    press(&mut core, &ctx, Key::S, ctrl(false), 15.0);
    assert!(!core.is_modified(), "Ctrl+S");

    // Ctrl+K and Ctrl+Shift+P toggle the palette.
    press(&mut core, &ctx, Key::K, ctrl(false), 16.0);
    assert!(core.palette.is_open(), "Ctrl+K");
    press(&mut core, &ctx, Key::K, ctrl(false), 17.0);
    assert!(!core.palette.is_open(), "Ctrl+K again closes");
    press(&mut core, &ctx, Key::P, ctrl(true), 18.0);
    assert!(core.palette.is_open(), "Ctrl+Shift+P");
}

// ------------------------------------------------------------------ preview

/// One frame with raw events (pointer, keys); the preview state is read from
/// `core.state.preview` afterwards.
fn frame_with(core: &mut EditorCore, ctx: &Context, time: f64, events: Vec<egui::Event>) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1360.0, 820.0))),
        time: Some(time),
        events,
        ..RawInput::default()
    };
    let output = ctx.run_ui(input, |ui| core.ui(ui));
    output.drop_without_applying_deltas();
}

fn key_event(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn click_events(pos: Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

/// A project with a running preview, as the host would publish it.
fn playing_preview(tag: &str) -> (EditorCore, Context, TempDir, egui::Rect) {
    let ctx = Context::default();
    let dir = TempDir::new(tag);
    let project = demo_project(dir.path());
    let mut core = EditorCore::new(options(None), &ctx);
    core.open_project(&project);
    settle(&mut core, &ctx, (1360.0, 820.0));
    core.state.preview.available = true;
    core.state.preview.status = orbit_editor_api::PreviewStatus::Playing;
    frame_with(&mut core, &ctx, 20.0, Vec::new());
    let rect = core
        .state
        .preview
        .picture_rect
        .expect("the preview panel is in the default layout and drawn");
    assert!(core.state.preview.visible, "the panel reports itself drawn");
    core.state.preview.input.clear();
    (core, ctx, dir, rect)
}

#[test]
fn the_preview_panel_is_in_the_default_layout_and_has_its_actions() {
    let ctx = Context::default();
    let core = EditorCore::new(options(None), &ctx);
    assert!(
        orbit_editor::dock::open_panels(&core.dock).contains(&"orbit.preview.preview".to_owned())
    );
    for action in [
        "orbit.preview.play_pause",
        "orbit.preview.stop",
        "orbit.preview.restart",
        "orbit.preview.play_from_here",
        "orbit.preview.mute",
        "orbit.preview.follow",
        "orbit.preview.fullscreen",
    ] {
        assert!(
            core.registry.find_action(action).is_some(),
            "missing {action}"
        );
    }
}

#[test]
fn preview_actions_post_commands_and_play_from_here_needs_a_node() {
    use orbit_editor_api::{PreviewCommand, PreviewStatus};

    let ctx = Context::default();
    let dir = TempDir::new("preview-actions");
    let project = demo_project(dir.path());
    let mut core = EditorCore::new(options(None), &ctx);

    let enabled = |core: &mut EditorCore, id: &str| {
        let action = core.registry.find_action(id).unwrap();
        let ectx = EditorContext::new(
            core.session.as_mut().map(|s| &mut s.project),
            &mut core.state,
            &core.registry,
        );
        (action.def.enabled)(&ectx)
    };
    // Without a project nothing can be played.
    assert!(!enabled(&mut core, "orbit.preview.play_pause"));

    core.open_project(&project);
    settle(&mut core, &ctx, (1360.0, 820.0));
    assert!(enabled(&mut core, "orbit.preview.play_pause"));

    // "Play from here" waits for a selected node.
    assert!(!enabled(&mut core, "orbit.preview.play_from_here"));
    let node = first_say(&core);
    core.state.selection = Some(Selection::Node(node));
    assert!(enabled(&mut core, "orbit.preview.play_from_here"));

    core.run_action_now("orbit.preview.play_from_here", &ctx);
    assert_eq!(
        core.state.preview.commands.pop(),
        Some(PreviewCommand::PlayFrom(node))
    );

    // Play when stopped, pause when playing.
    core.run_action_now("orbit.preview.play_pause", &ctx);
    assert_eq!(
        core.state.preview.commands.pop(),
        Some(PreviewCommand::Play)
    );
    core.state.preview.status = PreviewStatus::Playing;
    core.run_action_now("orbit.preview.play_pause", &ctx);
    assert_eq!(
        core.state.preview.commands.pop(),
        Some(PreviewCommand::Pause)
    );
    core.run_action_now("orbit.preview.mute", &ctx);
    assert_eq!(
        core.state.preview.commands.pop(),
        Some(PreviewCommand::SetMuted(true))
    );
}

#[test]
fn the_preview_gets_pointer_and_keys_only_while_it_has_the_focus() {
    use orbit_editor_api::PreviewInput;

    let (mut core, ctx, _dir, rect) = playing_preview("preview-focus");
    let center = rect.center();

    // No focus yet: keys and a wandering pointer go nowhere.
    frame_with(
        &mut core,
        &ctx,
        21.0,
        vec![egui::Event::PointerMoved(center)],
    );
    frame_with(
        &mut core,
        &ctx,
        22.0,
        vec![key_event(egui::Key::Enter, true)],
    );
    assert!(!core.state.preview.focused);
    assert!(
        core.state.preview.input.is_empty(),
        "nothing before the focus: {:?}",
        core.state.preview.input
    );

    // A press on the picture takes the focus and goes through, in virtual
    // pixels (the middle of the picture is the middle of the canvas).
    frame_with(&mut core, &ctx, 23.0, click_events(center, true));
    assert!(core.state.preview.focused);
    let input = std::mem::take(&mut core.state.preview.input);
    assert!(
        input.contains(&PreviewInput::Primary { pressed: true }),
        "{input:?}"
    );
    let moved: Vec<[f32; 2]> = input
        .iter()
        .filter_map(|i| match i {
            PreviewInput::PointerMoved(p) => Some(*p),
            _ => None,
        })
        .collect();
    assert!(!moved.is_empty());
    let [x, y] = moved[moved.len() - 1];
    assert!(
        (x - 960.0).abs() < 4.0 && (y - 540.0).abs() < 4.0,
        "{x} {y}"
    );
    frame_with(&mut core, &ctx, 24.0, click_events(center, false));
    core.state.preview.input.clear();

    // With the focus, keys the game knows are sent...
    frame_with(
        &mut core,
        &ctx,
        25.0,
        vec![key_event(egui::Key::Space, true)],
    );
    let input = std::mem::take(&mut core.state.preview.input);
    assert!(
        input.iter().any(|i| matches!(
            i,
            PreviewInput::Key {
                key: egui::Key::Space,
                pressed: true,
                ..
            }
        )),
        "{input:?}"
    );
    // ...and keys it does not know are not.
    frame_with(&mut core, &ctx, 26.0, vec![key_event(egui::Key::Z, true)]);
    assert!(core.state.preview.input.is_empty());

    // Clicking another widget (a row of the explorer) takes the focus away.
    let outside = Pos2::new(100.0, 140.0);
    frame_with(&mut core, &ctx, 27.0, click_events(outside, true));
    frame_with(&mut core, &ctx, 28.0, click_events(outside, false));
    core.state.preview.input.clear();
    assert!(!core.state.preview.focused);
    frame_with(
        &mut core,
        &ctx,
        29.0,
        vec![key_event(egui::Key::Enter, true)],
    );
    assert!(
        core.state.preview.input.is_empty(),
        "{:?}",
        core.state.preview.input
    );
}

#[test]
fn a_stopped_preview_takes_no_input_and_maximizing_swaps_the_layout() {
    let (mut core, ctx, _dir, rect) = playing_preview("preview-maximize");
    core.state.preview.status = orbit_editor_api::PreviewStatus::Stopped;
    frame_with(&mut core, &ctx, 30.0, click_events(rect.center(), true));
    assert!(
        core.state.preview.input.is_empty(),
        "the game is not running: {:?}",
        core.state.preview.input
    );

    // Full panel: the preview alone fills the window, then comes back.
    let before = rect.width();
    core.run_action_now("orbit.preview.fullscreen", &ctx);
    assert_eq!(
        core.state.maximized_panel.as_deref(),
        Some("orbit.preview.preview")
    );
    frame_with(&mut core, &ctx, 31.0, Vec::new());
    let big = core.state.preview.picture_rect.unwrap();
    assert!(big.width() > before * 1.5, "{} vs {before}", big.width());
    core.run_action_now("orbit.preview.fullscreen", &ctx);
    assert_eq!(core.state.maximized_panel, None);
    frame_with(&mut core, &ctx, 32.0, Vec::new());
    assert!((core.state.preview.picture_rect.unwrap().width() - before).abs() < 1.0);
}

#[test]
fn a_still_preview_does_not_make_the_ui_redraw() {
    // A game waiting for the player is a still texture: the UI around it
    // must settle like it does without a preview.
    let (mut core, ctx, _dir, _) = playing_preview("preview-idle");
    let settled = settle(&mut core, &ctx, (1360.0, 820.0));
    assert!(settled < 30);
    let delay = frame(&mut core, &ctx, 100.0, (1360.0, 820.0));
    assert_ne!(delay, Duration::ZERO, "no repaint wanted when idle");
}
