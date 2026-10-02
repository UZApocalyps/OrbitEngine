//! Headless tests of the M3e tools: asset import (copy, id, manifest, undo,
//! refusals), rename with undo, the case lint, the file watcher feeding the
//! editor, the story file changed on disk, live validation latency, the
//! reveal of problems and search hits. No window, no GPU.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_core::{AssetId, AssetKind, NodeId, NodeKind, Story};
use orbit_editor::assets::{self, AssetIssue};
use orbit_editor::core::{CoreOptions, Dialog, EditorCore};
use orbit_editor_api::{EditorContext, Level, Location, Selection};
use orbit_project::{Command, ops};

// ------------------------------------------------------------------ helpers

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-assets-it-{tag}-{}-{:?}",
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

fn options() -> CoreOptions {
    CoreOptions {
        watch_interval: Duration::from_millis(20),
        ..CoreOptions::with_language(orbit_editor_api::Lang::Fr)
    }
}

fn new_project(tag: &str) -> (EditorCore, Context, TempDir, PathBuf) {
    let dir = TempDir::new(tag);
    let ctx = Context::default();
    let mut core = EditorCore::new(options(), &ctx);
    let root = orbit_editor::storage::create_project(
        dir.path(),
        "Projet",
        &orbit_editor::i18n::translator(orbit_editor_api::Lang::Fr),
    )
    .unwrap();
    core.open_project(&root);
    assert!(core.has_project());
    (core, ctx, dir, root)
}

fn frame(core: &mut EditorCore, ctx: &Context, time: f64) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1360.0, 820.0))),
        time: Some(time),
        ..RawInput::default()
    };
    ctx.run_ui(input, |ui| core.ui(ui))
        .drop_without_applying_deltas();
}

fn frames(core: &mut EditorCore, ctx: &Context, n: usize) {
    for i in 0..n {
        frame(core, ctx, i as f64 * 0.016);
    }
}

/// Runs frames (sleeping a little between them) until `done`, at most `max`.
fn frames_until(
    core: &mut EditorCore,
    ctx: &Context,
    max: Duration,
    mut done: impl FnMut(&mut EditorCore) -> bool,
) -> Option<Duration> {
    let started = Instant::now();
    let mut t = 0.0;
    while started.elapsed() < max {
        frame(core, ctx, t);
        t += 0.016;
        if done(core) {
            return Some(started.elapsed());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    None
}

fn with_ctx<R>(core: &mut EditorCore, f: impl FnOnce(&mut EditorContext<'_>) -> R) -> R {
    let session = core.session.as_mut().expect("a project is open");
    let mut ectx = EditorContext::new(Some(&mut session.project), &mut core.state, &core.registry);
    f(&mut ectx)
}

fn story(core: &EditorCore) -> &Story {
    core.story().unwrap()
}

fn write_png(path: &Path, w: u32, h: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbaImage::from_pixel(w, h, image::Rgba([40, 90, 200, 255]))
        .save(path)
        .unwrap();
}

fn write_wav(path: &Path, seconds: f32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..(8000.0 * seconds) as usize {
        w.write_sample(((i % 80) as i16 - 40) * 200).unwrap();
    }
    w.finalize().unwrap();
}

fn undo_label(core: &mut EditorCore) -> Option<String> {
    with_ctx(core, |c| c.undo_label().map(str::to_owned))
}

// ------------------------------------------------------------------- import

#[test]
fn dropping_a_png_and_a_wav_imports_them_with_stable_ids_as_one_undo_step() {
    let (mut core, ctx, dir, root) = new_project("import");
    let outside = dir.path().join("outside");
    write_png(&outside.join("My Room (final).PNG"), 64, 48);
    write_wav(&outside.join("ding.wav"), 0.5);
    let before = story(&core).clone();

    core.import_dropped(vec![
        outside.join("My Room (final).PNG"),
        outside.join("ding.wav"),
    ]);
    frames(&mut core, &ctx, 2);

    let assets = story(&core).assets.assets.clone();
    assert_eq!(assets.len(), 2, "{assets:?}");
    let image = assets.iter().find(|a| a.kind == AssetKind::Image).unwrap();
    let sound = assets.iter().find(|a| a.kind == AssetKind::Sound).unwrap();
    // A safe name in a sub-folder per type; the files are copies inside the project.
    assert_eq!(image.path, "assets/images/My_Room_final.png");
    assert_eq!(sound.path, "assets/sounds/ding.wav");
    assert!(root.join(&image.path).is_file());
    assert!(root.join(&sound.path).is_file());
    assert!(
        outside.join("ding.wav").is_file(),
        "the source is left alone"
    );
    // Valid in the editor's eyes: the ids resolve and the files exist.
    assert!(assets::lint_asset_files(&root, story(&core)).is_empty());
    assert_eq!(
        undo_label(&mut core).as_deref(),
        Some("Importer 2 ressources")
    );
    assert_eq!(core.state.selected_asset, Some(assets[0].id));

    // One undo removes both manifest entries; redo brings back the same ids.
    let ids: Vec<AssetId> = assets.iter().map(|a| a.id).collect();
    core.run_action_now("orbit.core.undo", &ctx);
    assert_eq!(*story(&core), before);
    core.run_action_now("orbit.core.redo", &ctx);
    let again: Vec<AssetId> = story(&core).assets.assets.iter().map(|a| a.id).collect();
    assert_eq!(again, ids);
}

#[test]
fn a_file_with_the_wrong_content_or_format_is_refused_without_a_trace() {
    let (mut core, ctx, dir, root) = new_project("refuse");
    let texts = orbit_editor::i18n::translator(orbit_editor_api::Lang::Fr);
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("fake.png"), b"definitely not a png").unwrap();
    std::fs::write(outside.join("notes.txt"), b"hello").unwrap();
    std::fs::write(outside.join("broken.wav"), b"RIFF....WAVEjunk").unwrap();
    let full = outside.join("good.png");
    write_png(&full, 32, 32);
    let bytes = std::fs::read(&full).unwrap();
    std::fs::write(outside.join("truncated.png"), &bytes[..bytes.len() / 2]).unwrap();

    let report = with_ctx(&mut core, |c| {
        orbit_editor::builtin::import_paths(
            c,
            &[
                outside.join("fake.png"),
                outside.join("notes.txt"),
                outside.join("broken.wav"),
                outside.join("truncated.png"),
                outside.join("missing.png"),
                full.clone(),
            ],
        )
    });
    // Only the good one got in; the others say why.
    assert_eq!(report.added(), 1);
    assert_eq!(report.failed.len(), 5);
    assert!(
        report
            .failed
            .iter()
            .any(|(p, e)| p.ends_with("notes.txt")
                && e.message(&texts).contains("non pris en charge"))
    );
    assert!(
        report
            .failed
            .iter()
            .any(|(p, e)| p.ends_with("fake.png") && e.message(&texts).contains("contenu invalide"))
    );
    assert_eq!(story(&core).assets.assets.len(), 1);
    let images: Vec<_> = std::fs::read_dir(root.join("assets/images"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(images.len(), 1, "nothing was copied for the refused files");
    assert!(!root.join("assets/sounds").exists());
    // The refusals are reported to the user (toasts), one per file.
    let errors = core
        .state
        .toasts
        .iter()
        .filter(|t| t.level == Level::Error)
        .count();
    assert_eq!(errors, 5);
    frames(&mut core, &ctx, 1);
}

#[test]
fn names_never_collide_even_by_case_and_a_folder_is_expanded() {
    let (mut core, _ctx, dir, root) = new_project("names");
    let a = dir.path().join("a");
    let b = dir.path().join("b/sub");
    write_png(&a.join("Hero.png"), 8, 8);
    write_png(&b.join("hero.png"), 8, 8);
    core.import_dropped(vec![a.join("Hero.png")]);
    // A whole folder dropped at once.
    core.import_dropped(vec![dir.path().join("b")]);
    let paths: Vec<&str> = story(&core)
        .assets
        .assets
        .iter()
        .map(|x| x.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["assets/images/Hero.png", "assets/images/hero-2.png"]
    );
    let lowered: std::collections::HashSet<String> =
        paths.iter().map(|p| p.to_lowercase()).collect();
    assert_eq!(lowered.len(), paths.len());
    assert!(root.join("assets/images/hero-2.png").is_file());
}

#[test]
fn a_file_already_in_assets_is_declared_in_place_and_never_twice() {
    let (mut core, _ctx, _dir, root) = new_project("inplace");
    write_png(&root.join("assets/images/room.png"), 8, 8);
    core.import_dropped(vec![root.join("assets/images/room.png")]);
    assert_eq!(story(&core).assets.assets.len(), 1);
    assert_eq!(story(&core).assets.assets[0].path, "assets/images/room.png");
    let id = story(&core).assets.assets[0].id;
    // Dropping it again finds the existing declaration.
    core.import_dropped(vec![root.join("assets/images/room.png")]);
    assert_eq!(story(&core).assets.assets.len(), 1);
    assert_eq!(story(&core).assets.assets[0].id, id);
    assert_eq!(
        std::fs::read_dir(root.join("assets/images"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn destinations_cannot_leave_the_project() {
    let (mut core, _ctx, dir, root) = new_project("sandbox");
    let texts = orbit_editor::i18n::translator(orbit_editor_api::Lang::Fr);
    write_png(&root.join("assets/images/a.png"), 8, 8);
    core.import_dropped(vec![root.join("assets/images/a.png")]);
    let id = story(&core).assets.assets[0].id;
    let mut journal = assets::MoveJournal::default();
    for evil in [
        "../escape.png",
        "assets/../../escape.png",
        "/etc/passwd.png",
        "C:/Windows/escape.png",
        "assets\\images\\x.png",
    ] {
        let result = with_ctx(&mut core, |c| {
            c.ops(|p| assets::move_asset(p, &root, &mut journal, id, evil, &texts))
        });
        assert!(result.is_err(), "{evil} must be refused");
    }
    assert!(!dir.path().join("escape.png").exists());
    assert!(root.join("assets/images/a.png").is_file());
    assert_eq!(story(&core).assets.assets[0].path, "assets/images/a.png");
    // A changed extension is refused as well.
    let result = with_ctx(&mut core, |c| {
        c.ops(|p| assets::move_asset(p, &root, &mut journal, id, "assets/images/a.jpg", &texts))
    });
    assert!(result.is_err());
}

// ---------------------------------------------------------- rename, usage

fn background_using(core: &mut EditorCore, asset: AssetId) -> NodeId {
    let scene = with_ctx(core, |c| c.ops(|p| ops::add_scene(p, "Scène")).unwrap());
    let draft = ops::NodeDraft::plain(NodeKind::Background {
        asset,
        transition: None,
    });
    let id = draft.node.id;
    with_ctx(core, |c| {
        c.ops(|p| ops::insert_drafts(p, scene, 0, vec![draft], "bg"))
            .unwrap();
    });
    id
}

#[test]
fn renaming_an_asset_moves_the_file_keeps_references_and_undo_moves_it_back() {
    let (mut core, ctx, _dir, root) = new_project("rename");
    let texts = orbit_editor::i18n::translator(orbit_editor_api::Lang::Fr);
    write_png(&root.join("assets/images/old.png"), 8, 8);
    core.import_dropped(vec![root.join("assets/images/old.png")]);
    let id = story(&core).assets.assets[0].id;
    let node = background_using(&mut core, id);
    frames(&mut core, &ctx, 2);

    let moves = core.shared.moves.clone();
    with_ctx(&mut core, |c| {
        c.ops(|p| {
            assets::move_asset(
                p,
                &root,
                &mut moves.borrow_mut(),
                id,
                "assets/images/new.png",
                &texts,
            )
        })
        .unwrap();
    });
    assert!(root.join("assets/images/new.png").is_file());
    assert!(!root.join("assets/images/old.png").exists());
    let entry = story(&core).assets.get(id).unwrap();
    assert_eq!(entry.path, "assets/images/new.png");
    // The node still points at the same id: nothing broke.
    assert!(matches!(
        story(&core).node(node).unwrap().kind,
        NodeKind::Background { asset, .. } if asset == id
    ));
    assert!(orbit_core::validate(story(&core)).errors.is_empty());
    assert!(assets::lint_asset_files(&root, story(&core)).is_empty());

    // Undo: the manifest path comes back and so does the file.
    core.run_action_now("orbit.core.undo", &ctx);
    frames(&mut core, &ctx, 2);
    assert_eq!(
        story(&core).assets.get(id).unwrap().path,
        "assets/images/old.png"
    );
    assert!(root.join("assets/images/old.png").is_file());
    assert!(!root.join("assets/images/new.png").exists());
    // Redo: both again.
    core.run_action_now("orbit.core.redo", &ctx);
    frames(&mut core, &ctx, 2);
    assert!(root.join("assets/images/new.png").is_file());
    assert!(!root.join("assets/images/old.png").exists());
}

#[test]
fn a_used_asset_cannot_be_removed_by_a_node_or_by_a_character_expression() {
    let (mut core, _ctx, _dir, root) = new_project("usage");
    write_png(&root.join("assets/images/bg.png"), 8, 8);
    write_png(&root.join("assets/images/ana.png"), 8, 8);
    core.import_dropped(vec![
        root.join("assets/images/bg.png"),
        root.join("assets/images/ana.png"),
    ]);
    let id_of = |core: &EditorCore, name: &str| {
        story(core)
            .assets
            .assets
            .iter()
            .find(|a| a.path.ends_with(name))
            .unwrap()
            .id
    };
    let (bg, ana) = (id_of(&core, "bg.png"), id_of(&core, "ana.png"));
    background_using(&mut core, bg);
    let character = with_ctx(&mut core, |c| {
        c.ops(|p| ops::add_character(p, "Ana", orbit_core::Color::WHITE))
            .unwrap()
    });
    let mut def = story(&core).character(character).unwrap().clone();
    def.expressions.push(orbit_core::Expression {
        name: "happy".into(),
        image: ana,
        motion: None,
    });
    with_ctx(&mut core, |c| {
        c.exec(Command::SetCharacter {
            id: character,
            character: Some(def),
            index: None,
        })
        .unwrap();
    });
    for id in [bg, ana] {
        assert!(with_ctx(&mut core, |c| c.ops(|p| ops::remove_asset(p, id))).is_err());
        assert!(story(&core).assets.get(id).is_some());
    }
}

// --------------------------------------------------------------- case lint

#[test]
fn the_lint_reports_missing_files_and_case_differences() {
    let (mut core, ctx, _dir, root) = new_project("lint");
    write_png(&root.join("assets/images/bg.png"), 8, 8);
    // The manifest says `Bg.png` (works on Windows, not on Linux), and a
    // second asset points at a file that is not there.
    for (path, kind) in [
        ("assets/images/Bg.png", AssetKind::Image),
        ("assets/images/ghost.png", AssetKind::Image),
    ] {
        with_ctx(&mut core, |c| {
            c.ops(|p| ops::add_asset(p, kind, path)).unwrap();
        });
    }
    let issues = assets::lint_asset_files(&root, story(&core));
    assert!(issues.iter().any(|i| matches!(
        i,
        AssetIssue::CaseMismatch { path, actual, .. }
            if path == "assets/images/Bg.png" && actual == "assets/images/bg.png"
    )));
    assert!(issues.iter().any(|i| matches!(
        i,
        AssetIssue::Missing { path, .. } if path == "assets/images/ghost.png"
    )));
    // Two paths that differ only by case.
    with_ctx(&mut core, |c| {
        c.exec(Command::SetAsset {
            id: AssetId::new(),
            entry: Some(orbit_core::AssetEntry {
                id: AssetId::new(),
                kind: AssetKind::Image,
                path: "assets/images/BG.png".into(),
            }),
            index: None,
        })
        .unwrap_or(());
    });
    // The Problems list carries them, with the asset as location.
    frames(&mut core, &ctx, 3);
    let problems = &core.state.diagnostics.problems;
    assert!(problems.iter().any(|p| p.level == Level::Warn
        && p.message.contains("Casse différente")
        && matches!(p.location, Some(Location::Asset(_)))));
    assert!(problems.iter().any(|p| p.level == Level::Error
        && p.message.contains("introuvable")
        && matches!(p.location, Some(Location::Asset(_)))));
}

// -------------------------------------------------------------- the watcher

#[test]
fn files_changed_on_disk_refresh_the_editor() {
    let (mut core, ctx, _dir, root) = new_project("watch");
    write_png(&root.join("assets/images/a.png"), 8, 8);
    core.import_dropped(vec![root.join("assets/images/a.png")]);
    // The creation of the file above is itself noticed by the watcher: wait
    // for that before looking at the counters.
    let first = frames_until(&mut core, &ctx, Duration::from_secs(10), |c| {
        c.asset_epoch > 0
    });
    assert!(first.is_some(), "the added file was not noticed");
    frames(&mut core, &ctx, 2);
    let epoch = core.state.asset_epoch;
    let preview_epoch = core.asset_epoch;
    // Another tool replaces the image.
    write_png(&root.join("assets/images/a.png"), 40, 40);
    let seen = frames_until(&mut core, &ctx, Duration::from_secs(10), |c| {
        c.asset_epoch != preview_epoch
    });
    assert!(seen.is_some(), "the modified file was not noticed");
    assert!(core.state.asset_epoch > epoch);
    assert!(
        core.state
            .console
            .entries()
            .iter()
            .any(|e| e.message.contains("modifié sur le disque"))
    );

    // The file disappears: the problem shows up in the list without any edit.
    std::fs::remove_file(root.join("assets/images/a.png")).unwrap();
    let appeared = frames_until(&mut core, &ctx, Duration::from_secs(10), |c| {
        c.state
            .diagnostics
            .problems
            .iter()
            .any(|p| p.message.contains("Fichier introuvable"))
    });
    assert!(appeared.is_some(), "a deleted asset file is not reported");
}

#[test]
fn a_story_file_changed_outside_proposes_a_reload_with_a_conflict_when_modified() {
    let (mut core, ctx, _dir, root) = new_project("external");
    frames(&mut core, &ctx, 2);
    let story_path = root.join(&core.state.project.as_ref().unwrap().story_file);

    // Someone else edits the file: the project is clean, a reload is proposed.
    let mut other = story(&core).clone();
    other.title = "Titre changé dehors".into();
    std::fs::write(&story_path, other.to_ron().unwrap()).unwrap();
    let asked = frames_until(&mut core, &ctx, Duration::from_secs(10), |c| {
        matches!(c.dialog, Some(Dialog::ExternalChange { .. }))
    });
    assert!(asked.is_some(), "no reload proposed");
    assert!(matches!(
        core.dialog,
        Some(Dialog::ExternalChange { conflict: false })
    ));
    core.dialog = None;
    core.resolve_external_change(true);
    assert_eq!(story(&core).title, "Titre changé dehors");
    assert!(!core.is_modified());

    // Now with unsaved edits in the editor: the question is a conflict.
    with_ctx(&mut core, |c| {
        c.ops(|p| ops::add_scene(p, "Ma scène locale")).unwrap();
    });
    assert!(core.is_modified());
    let mut again = story(&core).clone();
    again.title = "Encore dehors".into();
    again.scenes.clear();
    std::fs::write(&story_path, again.to_ron().unwrap()).unwrap();
    let asked = frames_until(&mut core, &ctx, Duration::from_secs(10), |c| {
        matches!(c.dialog, Some(Dialog::ExternalChange { .. }))
    });
    assert!(asked.is_some());
    assert!(matches!(
        core.dialog,
        Some(Dialog::ExternalChange { conflict: true })
    ));
    // "Keep mine": nothing is lost, and the same state is not asked again.
    core.dialog = None;
    core.resolve_external_change(false);
    assert!(
        story(&core)
            .scenes
            .iter()
            .any(|s| s.name == "Ma scène locale")
    );
    frames_until(&mut core, &ctx, Duration::from_millis(300), |_| false);
    assert!(core.dialog.is_none());
    // Saving then overwrites the outside edit, and that is not an outside change.
    assert!(core.save());
    frames_until(&mut core, &ctx, Duration::from_millis(300), |_| false);
    assert!(core.dialog.is_none(), "the editor's own save was reported");
}

// ------------------------------------------------------- live validation

#[test]
fn an_error_is_displayed_within_300_ms_on_a_ten_thousand_node_story() {
    let dir = TempDir::new("latency");
    let ctx = Context::default();
    let mut core = EditorCore::new(options(), &ctx);
    let root = orbit_editor::storage::create_project(
        dir.path(),
        "Grand",
        &orbit_editor::i18n::translator(orbit_editor_api::Lang::Fr),
    )
    .unwrap();
    let story_file = root.join("story.orbit");
    let big = orbit_editor::bench::generate_story(10_000);
    std::fs::write(&story_file, big.to_ron().unwrap()).unwrap();
    core.open_project(&root);
    let ready = frames_until(&mut core, &ctx, Duration::from_secs(20), |c| {
        c.state.diagnostics.revision == 0 && !c.state.diagnostics.problems.is_empty()
    });
    assert!(ready.is_some(), "the first validation never arrived");
    let baseline = core.state.diagnostics.error_count();
    let nodes: usize = story(&core).scenes.iter().map(|s| s.nodes.len()).sum();
    assert!(nodes >= 10_000, "{nodes}");

    // Break a node after a pause (the usual case: the first edit of a
    // burst is validated at once): a background whose asset is unknown.
    std::thread::sleep(Duration::from_millis(150));
    let target = story(&core).scenes[0].nodes[5000].id;
    let edit = Instant::now();
    with_ctx(&mut core, |c| {
        c.exec(Command::SetNodeKind {
            id: target,
            kind: NodeKind::Background {
                asset: AssetId::new(),
                transition: None,
            },
            expected_version: None,
        })
        .unwrap();
    });
    let shown = frames_until(&mut core, &ctx, Duration::from_secs(5), |c| {
        c.state
            .diagnostics
            .of(target)
            .iter()
            .any(|(l, _)| *l == Level::Error)
    });
    let latency = edit.elapsed();
    assert!(shown.is_some(), "the error never appeared");
    assert_eq!(core.state.diagnostics.error_count(), baseline + 1);
    let (run, edit_to_publish) = core.validation_timings();
    eprintln!(
        "10k nodes: validation {run:?}, edit -> findings published {edit_to_publish:?}, \
         edit -> error visible {latency:?}"
    );
    assert!(
        latency < Duration::from_millis(300),
        "error shown after {latency:?}"
    );

    // Right after a validation (worst case for the debounce): another error
    // still shows up in well under 300 ms.
    let second = story(&core).scenes[0].nodes[6000].id;
    let edit = Instant::now();
    with_ctx(&mut core, |c| {
        c.exec(Command::SetNodeKind {
            id: second,
            kind: NodeKind::Background {
                asset: AssetId::new(),
                transition: None,
            },
            expected_version: None,
        })
        .unwrap();
    });
    let shown = frames_until(&mut core, &ctx, Duration::from_secs(5), |c| {
        c.state
            .diagnostics
            .of(second)
            .iter()
            .any(|(l, _)| *l == Level::Error)
    });
    let burst_latency = edit.elapsed();
    assert!(shown.is_some());
    eprintln!("10k nodes: second error visible after {burst_latency:?}");
    assert!(
        burst_latency < Duration::from_millis(300),
        "{burst_latency:?}"
    );

    // Typing is debounced: a burst of 50 edits costs a handful of validations.
    let mut last = Instant::now();
    for i in 0..50 {
        let node = story(&core).scenes[0].nodes[200 + i].id;
        with_ctx(&mut core, |c| {
            c.exec(Command::SetNodeKind {
                id: node,
                kind: NodeKind::Wait { seconds: 0.5 },
                expected_version: None,
            })
            .unwrap();
        });
        let started = Instant::now();
        frame(&mut core, &ctx, i as f64 * 0.016);
        // A frame never waits for validation.
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "frame {i} blocked for {:?}",
            started.elapsed()
        );
        last = started;
    }
    let _ = last;
}

// ------------------------------------------------- reveal (problems, search)

#[test]
fn revealing_a_problem_selects_the_node_and_brings_the_scene_forward() {
    let (mut core, ctx, _dir, _root) = new_project("reveal");
    let scene = with_ctx(&mut core, |c| {
        c.ops(|p| ops::add_scene(p, "Acte 2")).unwrap()
    });
    let draft = ops::NodeDraft::plain(NodeKind::Background {
        asset: AssetId::new(),
        transition: None,
    });
    let node = draft.node.id;
    with_ctx(&mut core, |c| {
        c.ops(|p| ops::insert_drafts(p, scene, 0, vec![draft], "bg"))
            .unwrap();
    });
    frames(&mut core, &ctx, 2);
    let problem = core
        .state
        .diagnostics
        .problems
        .iter()
        .find(|p| p.location == Some(Location::Node(node)))
        .expect("the unknown asset is a problem")
        .clone();
    assert_eq!(problem.level, Level::Error);

    core.state.selection = None;
    core.state.collapsed_scenes.insert(scene);
    with_ctx(&mut core, |c| {
        orbit_editor::builtin::reveal::reveal(c, problem.location.unwrap());
    });
    assert_eq!(core.state.selection, Some(Selection::Node(node)));
    assert!(
        !core.state.collapsed_scenes.contains(&scene),
        "scene unfolded"
    );
    assert_eq!(core.state.current_scene, Some(scene));
    let wanted = orbit_editor::builtin::panels::SCENE.to_owned();
    assert!(
        core.state
            .requests
            .iter()
            .any(|r| matches!(r, orbit_editor_api::EditorRequest::ShowPanel(p) if *p == wanted))
    );
    // The description tells where it is.
    let text = with_ctx(&mut core, |c| {
        orbit_editor::builtin::reveal::describe(c, &Location::Node(node))
    });
    assert!(text.starts_with("Acte 2"), "{text}");
}

#[test]
fn the_new_panels_and_actions_are_registered() {
    let ctx = Context::default();
    let core = EditorCore::new(options(), &ctx);
    for panel in [
        orbit_editor::builtin::panels::LIBRARY,
        orbit_editor::builtin::panels::PROBLEMS,
        orbit_editor::builtin::panels::SEARCH,
    ] {
        assert!(core.registry.find_panel(panel).is_some(), "{panel}");
    }
    for action in [
        "orbit.search.open",
        "orbit.library.import",
        "orbit.problems.next",
    ] {
        assert!(core.registry.find_action(action).is_some(), "{action}");
    }
    assert!(core.registry.project.importer_for("png").is_some());
    assert!(core.registry.project.importer_for("JPG").is_some());
    assert!(core.registry.project.importer_for("ogg").is_some());
    assert!(core.registry.project.importer_for("wav").is_some());
    assert!(core.registry.project.importer_for("mp3").is_none());
}

#[test]
fn no_two_actions_share_a_shortcut() {
    let ctx = Context::default();
    let core = EditorCore::new(options(), &ctx);
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for action in core.registry.actions() {
        for shortcut in action.shortcuts() {
            let key = orbit_editor_api::shortcut_text(&core.state.i18n, shortcut);
            if let Some(other) = seen.insert(key.clone(), action.def.id.clone()) {
                panic!("{key} is bound to both {other} and {}", action.def.id);
            }
        }
    }
}

// ------------------------------------------------------- the demo, as is

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Opens `story` the way the editor does and returns the error and warning
/// messages of its diagnostics.
fn problems_of(story: &Path) -> Vec<(Level, String)> {
    let ctx = Context::default();
    let mut core = EditorCore::new(options(), &ctx);
    core.open_project(story);
    assert!(core.has_project(), "the story opens");
    frames(&mut core, &ctx, 4);
    core.state
        .diagnostics
        .problems
        .iter()
        .map(|p| (p.level, p.message.clone()))
        .collect()
}

#[test]
fn the_demo_opened_as_a_story_file_has_no_error_in_its_diagnostics() {
    let problems = problems_of(&repo_root().join("examples/stories/demo.ron"));
    let errors: Vec<_> = problems.iter().filter(|p| p.0 == Level::Error).collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn the_demo_copied_deep_in_a_folder_tree_has_no_error_either() {
    let dir = TempDir::new("demo-deep");
    let deep = dir.path().join("a/.claude/worktrees/some-long-name/b");
    copy_dir(&repo_root().join("examples"), &deep.join("examples"));
    copy_dir(&repo_root().join("assets"), &deep.join("assets"));
    let problems = problems_of(&deep.join("examples/stories/demo.ron"));
    let errors: Vec<_> = problems.iter().filter(|p| p.0 == Level::Error).collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn the_lint_accepts_an_empty_root_and_a_relative_story_path() {
    // Tests run in the crate folder: `Cargo.toml` exists there.
    let mut story = Story::new("t");
    story.assets.assets.push(orbit_core::AssetEntry {
        id: AssetId::new(),
        kind: AssetKind::Image,
        path: "Cargo.toml".into(),
    });
    assert!(assets::lint_asset_files(Path::new(""), &story).is_empty());
    // The demo opened by a relative path, `/` or `\` (the way a shell on
    // Windows passes it), has no error.
    for rel in [
        "../../examples/stories/demo.ron",
        r"..\..\examples\stories\demo.ron",
    ] {
        if rel.contains('\\') && !cfg!(windows) {
            continue;
        }
        let errors: Vec<_> = problems_of(Path::new(rel))
            .into_iter()
            .filter(|p| p.0 == Level::Error)
            .collect();
        assert!(errors.is_empty(), "{rel}: {errors:#?}");
    }
}
