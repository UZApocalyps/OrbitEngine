//! 3D scenes as assets of the project (M6-11), headless: registration in the
//! asset library (import and "add to project" target), the camera list the
//! inspector reads from the scene file, the 3D findings of the Problems
//! panel, the "add node" palette and the unsaved-changes counter.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_core::{AssetKind, NodeId, NodeKind, Story};
use orbit_editor::assets::{self, AssetIssue};
use orbit_editor::builtin::drafts::{self, DraftError};
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor_api::{EditorContext, Lang};
use orbit_project::ops;
use orbit_scene::{CameraDef, Node3d, Node3dKind, Projection, Scene3d};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-s3d-it-{tag}-{}-{:?}",
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

fn new_project(tag: &str) -> (EditorCore, Context, TempDir, PathBuf) {
    let dir = TempDir::new(tag);
    let ctx = Context::default();
    let mut core = EditorCore::new(
        CoreOptions {
            watch_interval: Duration::from_millis(20),
            ..CoreOptions::with_language(Lang::En)
        },
        &ctx,
    );
    let root = orbit_editor::storage::create_project(
        dir.path(),
        "Project",
        &orbit_editor::i18n::translator(Lang::En),
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

fn with_ctx<R>(core: &mut EditorCore, f: impl FnOnce(&mut EditorContext<'_>) -> R) -> R {
    let session = core.session.as_mut().expect("a project is open");
    let mut ectx = EditorContext::new(Some(&mut session.project), &mut core.state, &core.registry);
    f(&mut ectx)
}

fn story(core: &EditorCore) -> &Story {
    core.story().unwrap()
}

fn scene_with_cameras(name: &str, cameras: &[&str]) -> Scene3d {
    let mut s = Scene3d::new(name);
    for c in cameras {
        s.nodes.push(Node3d::new(
            format!("node-{c}"),
            None,
            Node3dKind::Camera(CameraDef {
                name: (*c).to_owned(),
                projection: Projection::Perspective { fov_y: 0.9 },
                near: 0.1,
                far: 100.0,
            }),
        ));
    }
    s
}

fn write_scene(path: &Path, scene: &Scene3d) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, orbit_scene::to_ron(scene).unwrap()).unwrap();
}

fn declare(core: &mut EditorCore, rel: &str) -> orbit_core::AssetId {
    with_ctx(core, |c| {
        c.ops(|p| ops::add_asset(p, AssetKind::Scene3d, rel)).unwrap()
    })
}

fn insert(core: &mut EditorCore, kinds: Vec<NodeKind>) -> Vec<NodeId> {
    let scene = with_ctx(core, |c| c.ops(|p| ops::add_scene(p, "Intro")).unwrap());
    let drafts: Vec<_> = kinds.into_iter().map(ops::NodeDraft::plain).collect();
    with_ctx(core, |c| {
        c.ops(|p| ops::insert_drafts(p, scene, 0, drafts, "3d")).unwrap()
    })
}

// ------------------------------------------------------- library: import

#[test]
fn a_scene3d_file_imports_as_a_scene3d_asset_and_garbage_is_refused() {
    let (mut core, ctx, dir, root) = new_project("import");
    let outside = dir.path().join("outside");
    let good = outside.join("hall.scene3d.ron");
    write_scene(&good, &scene_with_cameras("Hall", &["main"]));
    let bad = outside.join("notes.ron");
    std::fs::write(&bad, "(this: is, not: a scene)").unwrap();

    core.import_dropped(vec![good, bad]);
    for i in 0..3 {
        frame(&mut core, &ctx, f64::from(i) * 0.016);
    }
    let assets = story(&core).assets.assets.clone();
    assert_eq!(assets.len(), 1, "{assets:?}");
    assert_eq!(assets[0].kind, AssetKind::Scene3d);
    assert!(assets[0].path.starts_with("assets/3d/"), "{}", assets[0].path);
    assert!(root.join(&assets[0].path).is_file());
}

// -------------------------------------------------- "add to project" target

#[test]
fn the_add_to_project_target_is_a_free_assets_3d_path_or_the_file_in_place() {
    let (mut core, _ctx, dir, root) = new_project("target");
    let outside = dir.path().join("elsewhere/room.scene3d.ron");
    write_scene(&outside, &scene_with_cameras("Salon 2", &["main"]));

    let t = assets::plan_scene3d_target(&root, story(&core), "Salon 2", Some(&outside));
    assert_eq!(t.rel, "assets/3d/Salon_2.scene3d.ron");
    assert!(!t.in_place);
    assert_eq!(t.abs, root.join("assets/3d/Salon_2.scene3d.ron"));

    // A taken name gets a number, on disk or in the manifest.
    write_scene(&t.abs, &scene_with_cameras("x", &["a"]));
    let again = assets::plan_scene3d_target(&root, story(&core), "Salon 2", None);
    assert_ne!(again.rel, t.rel);
    assert!(again.rel.starts_with("assets/3d/Salon_2"), "{}", again.rel);
    declare(&mut core, &again.rel);
    let third = assets::plan_scene3d_target(&root, story(&core), "Salon 2", None);
    assert!(![t.rel.clone(), again.rel.clone()].contains(&third.rel));

    // A file already under assets/ is declared where it is.
    let inside = root.join("assets/3d/mine.scene3d.ron");
    write_scene(&inside, &scene_with_cameras("Mine", &["main"]));
    let place = assets::plan_scene3d_target(&root, story(&core), "Mine", Some(&inside));
    assert!(place.in_place);
    assert_eq!(place.rel, "assets/3d/mine.scene3d.ron");
    let id = declare(&mut core, &place.rel);
    // Declaring twice returns the same asset.
    assert_eq!(declare(&mut core, &place.rel), id);
    assert_eq!(
        story(&core).assets.assets.iter().filter(|a| a.path == place.rel).count(),
        1
    );
    // A path outside assets/ is not an asset path.
    assert_eq!(assets::asset_rel_path(&root, &outside), None);
}

// --------------------------------------------------------- camera dropdown

#[test]
fn the_camera_list_comes_from_the_scene_file() {
    let (mut core, _ctx, _dir, root) = new_project("cams");
    write_scene(
        &root.join("assets/3d/room.scene3d.ron"),
        &scene_with_cameras("Room", &["wide", "closeup"]),
    );
    let id = declare(&mut core, "assets/3d/room.scene3d.ron");
    assert_eq!(
        assets::scene3d_cameras(&root, story(&core), id),
        Some(vec!["wide".to_owned(), "closeup".to_owned()])
    );
    // Not a 3D scene asset / unreadable file: no list (the inspector falls back to text).
    write_scene(&root.join("assets/3d/gone.scene3d.ron"), &scene_with_cameras("G", &["a"]));
    let gone = declare(&mut core, "assets/3d/gone.scene3d.ron");
    std::fs::remove_file(root.join("assets/3d/gone.scene3d.ron")).unwrap();
    assert_eq!(assets::scene3d_cameras(&root, story(&core), gone), None);
    assert_eq!(assets::scene3d_cameras(&root, story(&core), orbit_core::AssetId::new()), None);
}

#[test]
fn a_camera3d_reads_the_scene_shown_before_it() {
    let (mut core, _ctx, _dir, root) = new_project("shown");
    write_scene(&root.join("assets/3d/room.scene3d.ron"), &scene_with_cameras("R", &["a"]));
    let room = declare(&mut core, "assets/3d/room.scene3d.ron");
    let ids = insert(
        &mut core,
        vec![
            NodeKind::Camera3d { camera: String::new(), transition: None },
            NodeKind::ShowScene3d { scene: room, camera: None, transition: None },
            NodeKind::Camera3d { camera: "a".into(), transition: None },
            NodeKind::HideScene3d { transition: None },
            NodeKind::Camera3d { camera: "a".into(), transition: None },
        ],
    );
    let s = story(&core);
    assert_eq!(assets::shown_scene3d_before(s, ids[0]), None);
    assert_eq!(assets::shown_scene3d_before(s, ids[2]), Some(room));
    assert_eq!(assets::shown_scene3d_before(s, ids[4]), None);
}

// ------------------------------------------------------- Problems findings

#[test]
fn the_lint_reports_invalid_scenes_and_unknown_cameras() {
    let (mut core, _ctx, _dir, root) = new_project("lint");
    // A valid scene with one camera, an invalid file, a scene with a
    // duplicated node id.
    write_scene(&root.join("assets/3d/ok.scene3d.ron"), &scene_with_cameras("Ok", &["main"]));
    std::fs::write(root.join("assets/3d/broken.scene3d.ron"), "(oops").unwrap();
    let ok = declare(&mut core, "assets/3d/ok.scene3d.ron");
    let broken = declare(&mut core, "assets/3d/broken.scene3d.ron");
    let mut dup = scene_with_cameras("Dup", &["c"]);
    let copy = dup.nodes[0].clone();
    dup.nodes.push(copy);
    write_scene(&root.join("assets/3d/dup.scene3d.ron"), &dup);
    let dup_id = declare(&mut core, "assets/3d/dup.scene3d.ron");
    insert(
        &mut core,
        vec![
            NodeKind::ShowScene3d { scene: ok, camera: Some("main".into()), transition: None },
            NodeKind::Camera3d { camera: "nope".into(), transition: None },
            NodeKind::Camera3d { camera: String::new(), transition: None },
            NodeKind::ShowScene3d { scene: ok, camera: Some("ghost".into()), transition: None },
        ],
    );

    let issues = assets::lint_asset_files(&root, story(&core));
    let scene3d: Vec<_> = issues.iter().filter(|i| matches!(i, AssetIssue::Scene3d { .. })).collect();
    assert!(
        scene3d.iter().any(|i| i.asset() == broken && i.is_error()),
        "{issues:?}"
    );
    assert!(
        scene3d.iter().any(|i| i.asset() == dup_id && i.is_error()),
        "duplicate node id: {issues:?}"
    );
    assert!(!scene3d.iter().any(|i| i.asset() == ok), "{issues:?}");
    let missing: Vec<&str> = issues
        .iter()
        .filter_map(|i| match i {
            AssetIssue::CameraMissing { camera, .. } => Some(camera.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(missing, ["nope", "", "ghost"], "{issues:?}");

    // The messages are translated.
    let texts = orbit_editor::i18n::translator(Lang::En);
    for i in &issues {
        if matches!(i, AssetIssue::Scene3d { .. } | AssetIssue::CameraMissing { .. }) {
            let m = i.message(&texts);
            assert!(!m.contains("asset.issue"), "untranslated: {m}");
        }
    }
    let fr = orbit_editor::i18n::translator(Lang::Fr);
    let m = issues.iter().find(|i| matches!(i, AssetIssue::CameraMissing { .. })).unwrap().message(&fr);
    assert!(m.contains("caméra"), "{m}");
}

#[test]
fn problems_reach_the_diagnostics_of_the_editor() {
    let (mut core, ctx, _dir, root) = new_project("diag");
    std::fs::create_dir_all(root.join("assets/3d")).unwrap();
    std::fs::write(root.join("assets/3d/broken.scene3d.ron"), "(oops").unwrap();
    declare(&mut core, "assets/3d/broken.scene3d.ron");
    let started = std::time::Instant::now();
    let mut t = 0.0;
    loop {
        frame(&mut core, &ctx, t);
        t += 0.016;
        if core
            .state
            .diagnostics
            .problems
            .iter()
            .any(|p| p.message.contains("broken.scene3d.ron"))
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "no problem listed");
        std::thread::sleep(Duration::from_millis(2));
    }
}

// ------------------------------------------------------------------ palette

#[test]
fn the_palette_offers_the_three_3d_nodes() {
    let (mut core, _ctx, _dir, root) = new_project("palette");
    let keys: Vec<String> = drafts::builtin_insertables().into_iter().map(|i| i.key).collect();
    for k in ["ShowScene3d", "Camera3d", "HideScene3d"] {
        assert!(keys.iter().any(|x| x == k), "{k} missing from the palette");
    }
    let scene = with_ctx(&mut core, |c| c.ops(|p| ops::add_scene(p, "Intro")).unwrap());
    let texts = orbit_editor::i18n::translator(Lang::En);
    let build = |core: &EditorCore, key: &str| {
        drafts::drafts_for(story(core), &core.registry, &texts, key, scene, 0)
    };
    // Camera3d and HideScene3d need nothing; ShowScene3d needs a 3D scene asset.
    assert!(matches!(
        build(&core, "Camera3d").unwrap()[0].node.kind,
        NodeKind::Camera3d { .. }
    ));
    assert!(matches!(
        build(&core, "HideScene3d").unwrap()[0].node.kind,
        NodeKind::HideScene3d { .. }
    ));
    assert_eq!(
        build(&core, "ShowScene3d").unwrap_err(),
        DraftError::NeedAsset(AssetKind::Scene3d)
    );
    write_scene(&root.join("assets/3d/a.scene3d.ron"), &scene_with_cameras("A", &["main"]));
    let a = declare(&mut core, "assets/3d/a.scene3d.ron");
    match &build(&core, "ShowScene3d").unwrap()[0].node.kind {
        NodeKind::ShowScene3d { scene, camera: None, .. } => assert_eq!(*scene, a),
        other => panic!("{other:?}"),
    }
    // Every entry has a label in both languages.
    for lang in [Lang::En, Lang::Fr] {
        let t = orbit_editor::i18n::translator(lang);
        for k in ["ShowScene3d", "Camera3d", "HideScene3d"] {
            let item = drafts::builtin_insertables().into_iter().find(|i| i.key == k).unwrap();
            let label = item.label_text(&t);
            assert!(!label.starts_with("draft."), "{k}/{lang:?}: {label}");
        }
    }
}

// -------------------------------------------------------- unsaved counter

#[test]
fn an_unsaved_3d_scene_counts_as_modified() {
    let (mut core, ctx, _dir, _root) = new_project("dirty");
    assert!(!core.is_modified());
    // What the 3D editor publishes while its scene has unsaved edits.
    core.registry.ui_link.scene3d_dirty.set(true);
    assert!(core.is_modified());
    frame(&mut core, &ctx, 0.0);
    assert!(core.state.project.as_ref().unwrap().modified);
    core.registry.ui_link.scene3d_dirty.set(false);
    assert!(!core.is_modified());
}
