//! Frame-time budgets of the editor (headless, no GPU): the first frames
//! after opening a project, the first story timeline, the first edit after
//! load and typing in the inspector must stay under the hitch threshold, and
//! typing must not touch the disk. Run with `--nocapture` to see the times.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_core::{NodeKind, Story};
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor_api::{EditorContext, EditorRequest, Selection};
use orbit_project::{Command, MANIFEST_FILE};

/// Longest acceptable frame: under the 100 ms hitch threshold in release,
/// with room for unoptimized code in debug.
fn budget() -> Duration {
    if cfg!(debug_assertions) { Duration::from_millis(150) } else { Duration::from_millis(50) }
}

fn profile() -> &'static str {
    if cfg!(debug_assertions) { "debug" } else { "release" }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("orbit-hitch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A project holding a generated story of `nodes` nodes.
fn project(root: &Path, nodes: usize) -> PathBuf {
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let story = orbit_editor::bench::generate_story(nodes);
    std::fs::write(project.join("story.orbit"), story.to_ron().unwrap()).unwrap();
    std::fs::write(
        project.join(MANIFEST_FILE),
        "(format: 1, name: \"Hitch\", story: \"story.orbit\", plugins: [])",
    )
    .unwrap();
    project
}

struct Editor {
    core: EditorCore,
    ctx: Context,
    time: f64,
}

impl Editor {
    fn open(project: &Path, config: &Path) -> Self {
        let ctx = Context::default();
        let options = CoreOptions {
            config_dir: Some(config.to_owned()),
            ..CoreOptions::with_language(orbit_editor_api::Lang::En)
        };
        let mut core = EditorCore::new(options, &ctx);
        core.open_project(project);
        assert!(core.has_project());
        Self { core, ctx, time: 0.0 }
    }

    /// One whole editor frame (UI and its bookkeeping), timed.
    fn frame(&mut self) -> Duration {
        self.time += 0.016;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0))),
            time: Some(self.time),
            ..RawInput::default()
        };
        orbit_profile::begin_frame();
        let t = Instant::now();
        let core = &mut self.core;
        self.ctx.run_ui(input, |ui| core.ui(ui)).drop_without_applying_deltas();
        let took = t.elapsed();
        orbit_profile::finish_frame();
        if std::env::var_os("ORBIT_HITCH_DEBUG").is_some() {
            let f = orbit_profile::frames(1).pop().unwrap();
            eprintln!("{}", orbit_profile::Hitch::of(&f, 0).summary());
        }
        took
    }

    /// The longest of `n` frames.
    fn worst_of(&mut self, n: usize) -> Duration {
        (0..n).map(|_| self.frame()).max().unwrap_or_default()
    }

    fn story(&self) -> &Story {
        self.core.story().unwrap()
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[test]
fn first_frames_timeline_first_edit_and_typing_stay_within_budget() {
    let dir = TempDir::new("budget");
    let project = project(&dir.0, 2_000);
    let config = dir.0.join("config");
    let mut ed = Editor::open(&project, &config);
    let mut report = Vec::new();
    let mut worst = Duration::ZERO;
    let mut check = |name: &str, d: Duration, report: &mut Vec<String>| {
        report.push(format!("{name}: {:.1} ms", ms(d)));
        worst = worst.max(d);
    };

    // First frames after load (fonts, layout of every panel, first
    // validation started on a worker).
    let first = ed.frame();
    report.push(format!("first frame (fonts, glyph atlas): {:.1} ms", ms(first)));
    // The project is read and parsed on a worker; the frame that installs
    // it lays out every panel on the 2000-node story.
    let started = Instant::now();
    let mut install = Duration::ZERO;
    while ed.core.story().is_none() {
        assert!(started.elapsed() < Duration::from_secs(60), "the project never opened");
        install = install.max(ed.frame());
        std::thread::sleep(Duration::from_millis(1));
    }
    check("frames until the project is installed (worst)", install, &mut report);
    let next = ed.worst_of(5);
    check("5 frames after load (worst)", next, &mut report);

    // First story timeline.
    ed.core
        .state
        .requests
        .push(EditorRequest::ShowPanel("orbit.timeline.timeline".to_owned()));
    let timeline = ed.worst_of(4);
    check("first timeline open (4 frames, worst)", timeline, &mut report);

    // First edit after load: a line's text, then the frames that show it.
    let line = ed
        .story()
        .scenes[0]
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Say { .. }))
        .map(|n| n.id)
        .unwrap();
    ed.core.state.selection = Some(Selection::Node(line));
    ed.worst_of(2);
    let writes_before = orbit_editor::writer::submitted();
    let Some(NodeKind::Say { text_id, .. }) = ed.story().node(line).map(|n| n.kind.clone()) else {
        unreachable!()
    };
    // What the inspector's text field does on each keystroke.
    let mut typed = String::from("Line");
    let mut keystroke = |ed: &mut Editor, c: char| -> Duration {
        let t = Instant::now();
        typed.push(c);
        let session = ed.core.session.as_mut().unwrap();
        let mut ectx = EditorContext::new(Some(&mut session.project), &mut ed.core.state, &ed.core.registry);
        ectx.exec_merged(
            Command::SetString { id: text_id, text: Some(typed.clone()) },
            &format!("node:{line}:text"),
        )
        .unwrap();
        t.elapsed() + ed.frame()
    };
    let first_edit = keystroke(&mut ed, 'a');
    check("first edit after load (command + frame)", first_edit, &mut report);

    // Typing: one keystroke per frame, no disk access.
    let typing = "hello world, typing in the inspector"
        .chars()
        .map(|c| keystroke(&mut ed, c))
        .max()
        .unwrap();
    check("typing, worst keystroke frame", typing, &mut report);
    let writes = orbit_editor::writer::submitted() - writes_before;
    assert!(ed.core.is_modified());

    // Disk writes the UI thread used to do itself, before and after.
    // Recovery snapshot (autosave): serialize + two fsync'd writes, versus a
    // copy of the story handed to the background writer.
    let recovery = orbit_editor::storage::Recovery::new(dir.0.join("recovery-sync"));
    let meta = orbit_editor::storage::RecoveryMeta { story_file: "story.orbit".into(), saved_at_ms: 0 };
    let t = Instant::now();
    recovery.write(ed.story(), &meta).unwrap();
    let autosave_before = t.elapsed();
    let t = Instant::now();
    recovery.write_in_background(ed.story().clone(), meta);
    let autosave_after = t.elapsed();
    report.push(format!(
        "autosave on the UI thread: before {:.1} ms (to_ron + 2 fsync'd writes), after {:.2} ms (story copy)",
        ms(autosave_before),
        ms(autosave_after)
    ));
    // Editor config (dock layout, prefs), saved after each layout change.
    let text = std::fs::read(config.join("config.ron")).unwrap_or_else(|_| vec![b' '; 4096]);
    let t = Instant::now();
    orbit_editor::storage::write_atomic(&dir.0.join("config-sync.ron"), &text).unwrap();
    let config_before = t.elapsed();
    let t = Instant::now();
    ed.core.persist();
    let config_after = t.elapsed();
    report.push(format!(
        "editor config save on the UI thread: before {:.1} ms (fsync'd write), after {:.2} ms (queued)",
        ms(config_before),
        ms(config_after)
    ));
    orbit_editor::writer::flush();
    worst = worst.max(autosave_after).max(config_after);

    eprintln!("editor frames ({}, 2000-node story):\n  {}", profile(), report.join("\n  "));
    // Other tests of this binary do not run in parallel with this one
    // (it is the only test), so every submitted write is ours.
    assert_eq!(writes, 0, "typing queued {writes} disk writes");
    assert!(worst <= budget(), "a frame took {:.1} ms (budget {:?}):\n{}", ms(worst), budget(), report.join("\n"));
    ed.core.shutdown();
}
