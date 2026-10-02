//! The text view panel (`orbit.textview`) in the whole editor, headless.

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
            "orbit-editor-textview-{tag}-{}-{:?}",
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

use orbit_editor::builtin::textview::{TextViewState, state_id};

fn core_with_project(dir: &TempDir) -> (EditorCore, Context) {
    let ctx = Context::default();
    let project = demo_project(dir.path());
    let mut core = EditorCore::new(
        CoreOptions::with_language(orbit_editor_api::Lang::En),
        &ctx,
    );
    core.open_project(&project);
    settle(&mut core, &ctx);
    core.state
        .requests
        .push(EditorRequest::ShowPanel("orbit.textview.textview".to_owned()));
    settle(&mut core, &ctx);
    (core, ctx)
}

fn get(ctx: &Context) -> TextViewState {
    ctx.data(|d| d.get_temp::<TextViewState>(state_id()))
        .expect("the panel was drawn")
}

fn set(ctx: &Context, f: impl FnOnce(&mut TextViewState)) {
    let mut st = get(ctx);
    f(&mut st);
    ctx.data_mut(|d| d.insert_temp(state_id(), st));
}

fn story_text(core: &EditorCore) -> String {
    orbit_dsl::print_story(core.story().unwrap())
}

#[test]
fn the_panel_shows_the_story_as_text_without_problems() {
    let dir = TempDir::new("show");
    let (mut core, ctx) = core_with_project(&dir);
    set(&ctx, |s| s.whole_story = true);
    settle(&mut core, &ctx);
    let st = get(&ctx);
    assert_eq!(st.text, story_text(&core));
    assert!(st.diagnostics.is_empty());
    assert!(!st.dirty());
    assert!(core.state.text_problems.is_empty());
}

#[test]
fn auto_apply_changes_the_story_in_one_undo_step() {
    let dir = TempDir::new("auto");
    let (mut core, ctx) = core_with_project(&dir);
    set(&ctx, |s| s.whole_story = true);
    settle(&mut core, &ctx);
    let before = story_text(&core);
    assert!(before.contains("Welcome back, runner"));
    set(&ctx, |s| {
        s.auto_apply = true;
        s.text = s.text.replace("Welcome back, runner", "Hello, runner");
    });
    frame(&mut core, &ctx, 20.0);
    frame(&mut core, &ctx, 21.0);
    let after = story_text(&core);
    assert!(after.contains("Hello, runner") && !after.contains("Welcome back, runner"));
    // The view reloaded the canonical text and is clean.
    let st = get(&ctx);
    assert!(!st.dirty() && st.text == after);
    let project = &mut core.session.as_mut().unwrap().project;
    assert!(project.undo().unwrap());
    assert_eq!(story_text(&core), before, "one undo step reverts the text edit");
}

#[test]
fn nothing_is_applied_without_an_explicit_action() {
    let dir = TempDir::new("manual");
    let (mut core, ctx) = core_with_project(&dir);
    set(&ctx, |s| {
        s.whole_story = true;
        s.auto_apply = false;
    });
    settle(&mut core, &ctx);
    let before = story_text(&core);
    set(&ctx, |s| s.text = s.text.replace("Welcome back, runner", "Hello, runner"));
    frame(&mut core, &ctx, 30.0);
    frame(&mut core, &ctx, 40.0);
    assert_eq!(story_text(&core), before);
    assert!(get(&ctx).dirty());
}

#[test]
fn syntax_errors_are_listed_inline_and_in_problems_and_block_apply() {
    let dir = TempDir::new("syntax");
    let (mut core, ctx) = core_with_project(&dir);
    set(&ctx, |s| s.whole_story = true);
    settle(&mut core, &ctx);
    let before = story_text(&core);
    set(&ctx, |s| {
        s.auto_apply = true;
        s.text.push_str("\n@@@ garbage ((\n");
    });
    frame(&mut core, &ctx, 50.0);
    frame(&mut core, &ctx, 51.0);
    let st = get(&ctx);
    assert!(!st.diagnostics.is_empty());
    assert!(st.diagnostics[0].line > 0);
    assert_eq!(core.state.text_problems.len(), st.diagnostics.len());
    assert_eq!(story_text(&core), before, "a broken text changes nothing");
    // Fixing the text clears the findings.
    set(&ctx, |s| s.text = s.text.replace("\n@@@ garbage ((\n", ""));
    frame(&mut core, &ctx, 52.0);
    assert!(get(&ctx).diagnostics.is_empty());
    assert!(core.state.text_problems.is_empty());
}

#[test]
fn a_change_elsewhere_refreshes_a_clean_text() {
    let dir = TempDir::new("refresh");
    let (mut core, ctx) = core_with_project(&dir);
    set(&ctx, |s| s.whole_story = true);
    settle(&mut core, &ctx);
    core.edit_nth_say(0, "Changed elsewhere").unwrap();
    settle(&mut core, &ctx);
    let st = get(&ctx);
    assert!(st.text.contains("Changed elsewhere"));
    assert!(!st.conflict);
}

#[test]
fn a_change_elsewhere_never_clobbers_unsaved_text() {
    let dir = TempDir::new("conflict");
    let (mut core, ctx) = core_with_project(&dir);
    set(&ctx, |s| {
        s.whole_story = true;
        s.auto_apply = true;
    });
    settle(&mut core, &ctx);
    // Unapplied edit with a syntax error (so auto-apply cannot fire).
    set(&ctx, |s| s.text.push_str("\n@@@ mine\n"));
    frame(&mut core, &ctx, 60.0);
    core.edit_nth_say(0, "Changed elsewhere").unwrap();
    frame(&mut core, &ctx, 61.0);
    frame(&mut core, &ctx, 62.0);
    let st = get(&ctx);
    assert!(st.conflict, "a conflict notice is shown");
    assert!(st.text.contains("@@@ mine"), "the user's text is kept");
    assert!(!st.text.contains("Changed elsewhere"));
}
