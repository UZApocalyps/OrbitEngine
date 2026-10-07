//! The rigged character fixture (M5B-08): `examples/stories/test_rigged.ron`
//! with `examples/fixtures/assets/robo.rig.ron` and its pictures, played
//! natively through the real asset loader and renderer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use orbit_anim::rig::{
    Attachment, AttachmentId, Bone, BoneId, MeshAttachment, RegionAttachment, Rig, RigClip, RigTrack, Skin, Slot,
    SlotId, Xform, save_rig,
};
use orbit_anim::{Angle, Key, Loop, Rgba, Track, Vec2};
use orbit_core::Story;

const STORY: &str = include_str!("../../../../examples/stories/test_rigged.ron");

fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/fixtures/assets")
}

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

const POSTER: [u32; 2] = [160, 360];

/// The pictures are plain shapes: easy to tell apart in a read-back.
fn picture(w: u32, h: u32, color: [u8; 4], shape: &str) -> image::RgbaImage {
    image::RgbaImage::from_fn(w, h, |x, y| {
        let (cx, cy) = (x as f32 - w as f32 / 2.0 + 0.5, y as f32 - h as f32 / 2.0 + 0.5);
        let inside = match shape {
            "disc" => (cx / (w as f32 / 2.0)).powi(2) + (cy / (h as f32 / 2.0)).powi(2) <= 1.0,
            _ => true,
        };
        if inside { image::Rgba(color) } else { image::Rgba([0, 0, 0, 0]) }
    })
}

fn track_angle(bone: &str, keys: &[(f32, f32)]) -> RigTrack {
    RigTrack::BoneRotate {
        bone: BoneId::new(bone),
        track: Track::new(keys.iter().map(|&(t, v)| Key::new(t, Angle(v))).collect()),
    }
}

/// The rig: legs, body, a head mesh bending at the neck and an arm, over
/// the poster's pixel space (origin: bottom-center). Clips `idle` (breathing)
/// and `wave` (the arm goes up and waves).
fn robo() -> Rig {
    let bone = |id: &str, parent: Option<&str>, x: f32, y: f32| Bone {
        id: BoneId::new(id),
        name: id.into(),
        parent: parent.map(BoneId::new),
        rest: Xform::translate(x, y),
        length: 40.0,
    };
    let region = |image: &str, size: [f32; 2], x: f32, y: f32| {
        Attachment::Region(RegionAttachment { image: image.into(), size, xform: Xform::translate(x, y) })
    };
    let mut default = BTreeMap::new();
    default.insert(AttachmentId::new("legs"), region("fixtures/assets/robo_arm.png", [60.0, 110.0], 0.0, -55.0));
    default.insert(AttachmentId::new("body"), region("fixtures/assets/robo_body.png", [100.0, 160.0], 0.0, -50.0));
    default.insert(
        AttachmentId::new("head"),
        Attachment::Mesh(MeshAttachment {
            image: "fixtures/assets/robo_head.png".into(),
            vertices: vec![[-40.0, -350.0], [40.0, -350.0], [-40.0, -270.0], [40.0, -270.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            triangles: vec![[0, 1, 2], [2, 1, 3]],
            weights: vec![
                vec![(BoneId::new("head"), 1.0)],
                vec![(BoneId::new("head"), 1.0)],
                vec![(BoneId::new("head"), 0.5), (BoneId::new("torso"), 0.5)],
                vec![(BoneId::new("head"), 0.5), (BoneId::new("torso"), 0.5)],
            ],
        }),
    );
    default.insert(AttachmentId::new("arm"), region("fixtures/assets/robo_arm.png", [24.0, 90.0], 0.0, 40.0));
    let mut alt = default.clone();
    alt.insert(AttachmentId::new("body"), region("fixtures/assets/robo_arm.png", [100.0, 160.0], 0.0, -50.0));

    let slot = |id: &str, bone: &str| Slot {
        id: SlotId::new(id),
        name: id.into(),
        bone: BoneId::new(bone),
        attachment: Some(AttachmentId::new(id)),
        color: Rgba::new(1.0, 1.0, 1.0, 1.0),
    };
    let mut clips = BTreeMap::new();
    let mut idle = RigClip::new(2.0, Loop::Forever);
    idle.tracks.push(RigTrack::BoneTranslate {
        bone: BoneId::new("torso"),
        track: Track::new(vec![
            Key::new(0.0, Vec2::new(0.0, 0.0)),
            Key::new(1.0, Vec2::new(0.0, -6.0)),
            Key::new(2.0, Vec2::new(0.0, 0.0)),
        ]),
    });
    idle.tracks.push(track_angle("head", &[(0.0, 0.0), (1.0, 4.0), (2.0, 0.0)]));
    clips.insert("idle".to_owned(), idle);
    let mut wave = RigClip::new(1.2, Loop::Forever);
    wave.tracks.push(track_angle("arm", &[(0.0, 0.0), (0.3, -140.0), (0.6, -100.0), (0.9, -140.0), (1.2, 0.0)]));
    clips.insert("wave".to_owned(), wave);
    Rig {
        bones: vec![
            bone("root", None, 0.0, 0.0),
            bone("torso", Some("root"), 0.0, -140.0),
            bone("head", Some("torso"), 0.0, -130.0),
            bone("arm", Some("torso"), 58.0, -110.0),
        ],
        slots: vec![slot("legs", "root"), slot("body", "torso"), slot("head", "head"), slot("arm", "arm")],
        skins: vec![
            Skin { name: "default".into(), attachments: default },
            Skin { name: "alt".into(), attachments: alt },
        ],
        clips,
        ..Rig::default()
    }
}

/// Fixture generators run only with `ORBIT_REGENERATE_FIXTURES=1` and refuse
/// to overwrite an existing file unless it is `force`.
fn regen_allowed(out: &std::path::Path) -> bool {
    let mode = std::env::var("ORBIT_REGENERATE_FIXTURES").unwrap_or_default();
    if mode != "1" && mode != "force" {
        eprintln!("skipped: set ORBIT_REGENERATE_FIXTURES=1 to regenerate the frozen fixtures");
        return false;
    }
    if out.exists() && mode != "force" {
        eprintln!("skipped: {} exists; ORBIT_REGENERATE_FIXTURES=force overwrites it", out.display());
        return false;
    }
    true
}

/// Writes the fixture files from the code above. Run once with
/// `cargo test -p orbit_runtime generate_rig_fixture -- --ignored`; the files
/// are committed and the tests below read them.
#[test]
#[ignore = "writes the fixture files; run once with --ignored"]
fn generate_rig_fixture() {
    if !regen_allowed(&assets_dir().join("robo.rig.ron")) {
        return;
    }
    let dir = assets_dir();
    let save = |name: &str, img: image::RgbaImage| img.save(dir.join(name)).unwrap();
    // The poster: a flat silhouette of the whole character.
    let mut poster = picture(POSTER[0], POSTER[1], [0, 0, 0, 0], "rect");
    for (x0, y0, x1, y1, c) in [
        (50, 250, 110, 360, [70u8, 70, 90, 255]),
        (30, 90, 130, 250, [59, 110, 165, 255]),
        (120, 90, 144, 180, [230, 140, 50, 255]),
        (40, 10, 120, 90, [240, 200, 60, 255]),
    ] {
        for y in y0..y1 {
            for x in x0..x1 {
                poster.put_pixel(x, y, image::Rgba(c));
            }
        }
    }
    save("robo_poster.png", poster);
    save("robo_body.png", picture(100, 160, [59, 110, 165, 255], "rect"));
    save("robo_arm.png", picture(24, 90, [230, 140, 50, 255], "rect"));
    save("robo_head.png", picture(80, 80, [240, 200, 60, 255], "disc"));
    std::fs::write(dir.join("robo.rig.ron"), save_rig(&robo()).unwrap()).unwrap();
}

/// The demo's rig (`assets/demo/alice_sad.rig.ron`): the picture of
/// Alice's sad expression on one bone that breathes and sways a little, so
/// a still drawing comes alive without any extra art. Run once with
/// `cargo test -p orbit_runtime generate_demo_rig -- --ignored`.
#[test]
#[ignore = "writes the demo rig; run once with --ignored"]
fn generate_demo_rig() {
    if !regen_allowed(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/demo/alice_sad.rig.ron")) {
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/demo");
    let (w, h) = image::image_dimensions(dir.join("alice_sad.png")).unwrap();
    let mut attachments = BTreeMap::new();
    attachments.insert(
        AttachmentId::new("body"),
        Attachment::Region(RegionAttachment {
            image: "assets/demo/alice_sad.png".into(),
            size: [w as f32, h as f32],
            xform: Xform::translate(0.0, -(h as f32) / 2.0),
        }),
    );
    let mut breathe = RigClip::new(3.0, Loop::Forever);
    breathe.tracks.push(RigTrack::BoneScale {
        bone: BoneId::new("body"),
        track: Track::new(vec![
            Key::new(0.0, Vec2::new(1.0, 1.0)),
            Key::new(1.5, Vec2::new(1.006, 1.018)),
            Key::new(3.0, Vec2::new(1.0, 1.0)),
        ]),
    });
    breathe.tracks.push(track_angle("body", &[(0.0, 0.0), (0.75, 0.6), (1.5, 0.0), (2.25, -0.6), (3.0, 0.0)]));
    let mut clips = BTreeMap::new();
    clips.insert("breathe".to_owned(), breathe);
    let rig = Rig {
        bones: vec![Bone {
            id: BoneId::new("body"),
            name: "body".into(),
            parent: None,
            rest: Xform::IDENTITY,
            length: h as f32,
        }],
        slots: vec![Slot {
            id: SlotId::new("body"),
            name: "body".into(),
            bone: BoneId::new("body"),
            attachment: Some(AttachmentId::new("body")),
            color: Rgba::new(1.0, 1.0, 1.0, 1.0),
        }],
        skins: vec![Skin { name: "default".into(), attachments }],
        clips,
        ..Rig::default()
    };
    std::fs::write(dir.join("alice_sad.rig.ron"), save_rig(&rig).unwrap()).unwrap();
}

#[test]
fn the_fixture_story_loads_validates_and_its_rig_is_sound() {
    let story = Story::from_ron(STORY).expect("the fixture story is valid");
    assert_eq!(story.schema_version, orbit_core::CURRENT_SCHEMA_VERSION);
    assert!(orbit_core::validate(&story).warnings.is_empty());
    // The rig file on disk equals the one the generator builds (so the two
    // cannot drift) and has no problem against the project files.
    let on_disk = orbit_anim::rig::load_rig(&std::fs::read_to_string(assets_dir().join("robo.rig.ron")).unwrap())
        .unwrap()
        .rig;
    assert_eq!(on_disk, orbit_anim::rig::load_rig(&save_rig(&robo()).unwrap()).unwrap().rig);
    assert!(orbit_anim::rig::PreparedRig::new(on_disk).unwrap().warnings.is_empty());
    for e in &story.characters[0].expressions {
        let m = e.rigged().expect("rigged");
        assert!(["idle", "wave"].contains(&m.clip.as_str()));
        assert_eq!(story.assets.get(m.rig).unwrap().kind, orbit_core::AssetKind::Rig);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::time::Duration;

    use orbit_assets::AssetLoader;
    use orbit_core::{CharacterId, StageFx};
    use orbit_render2d::orbit_gfx::Gfx;
    use orbit_render2d::{ReadbackStatus, Renderer2d, Scene, TextureId};

    use super::*;
    use crate::loading::{AssetStore, finish_loading, start_loading};
    use crate::runtime::stage_scene;
    use crate::stage::{AnimClock, StageCharacter, StageState};

    fn renderer() -> Option<Renderer2d> {
        let gfx = pollster::block_on(Gfx::headless((64, 64))).ok()?;
        Renderer2d::new(gfx).ok()
    }

    /// Loads the story's assets the way the player does (loader, pending
    /// handles, finish).
    fn load(r: &mut Renderer2d, story: &Story) -> AssetStore {
        let loader = AssetLoader::new(examples_dir().to_string_lossy());
        let mut store = AssetStore::default();
        let pending = start_loading(&loader, story, &store);
        for _ in 0..5_000 {
            if pending.iter().all(|p| p.is_done()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(pending.iter().all(|p| p.is_done()), "assets loaded");
        let failures = finish_loading(r, &mut store, pending);
        assert!(failures.is_empty(), "{failures:?}");
        store
    }

    fn stage(hero: CharacterId, expression: &str, story: &Story) -> StageState {
        StageState {
            characters: vec![StageCharacter {
                character: hero,
                expression: expression.into(),
                image: story.characters[0].expression(expression).unwrap().image,
                x: 0.5,
                y: 1.0,
                scale: [1.0, 1.0],
                flip_x: false,
                flip_y: false,
            }],
            ..StageState::default()
        }
    }

    fn draw(r: &mut Renderer2d, store: &AssetStore, story: &Story, s: &StageState, clock: &AnimClock) -> Vec<u8> {
        let mut scene = Scene::new();
        stage_scene(r, store, story, s, Some(clock), None, None, &mut scene);
        let target: TextureId = r.create_canvas_target("rig fixture").unwrap();
        r.render_to_target(target, &scene).unwrap();
        let mut readback = r.start_readback(target).unwrap();
        for _ in 0..10_000 {
            if let ReadbackStatus::Ready { rgba, .. } = r.poll_readback(&mut readback) {
                return rgba;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("read-back timed out");
    }

    /// The rig and its pictures load from a pak (the exported game / the web
    /// bundle) like from a folder.
    #[test]
    fn the_rig_and_its_pictures_load_from_a_pak() {
        let Some(mut r) = renderer() else {
            eprintln!("no GPU adapter: skipping the rig pak loading test");
            return;
        };
        let story = Story::from_ron(STORY).unwrap();
        let mut writer = orbit_pak::PakWriter::new(orbit_pak::Compression::Lz4);
        for name in ["bg_room.png", "robo_poster.png", "robo.rig.ron", "robo_body.png", "robo_arm.png", "robo_head.png"] {
            let bytes = std::fs::read(assets_dir().join(name)).unwrap();
            writer.add(&format!("fixtures/assets/{name}"), &bytes).unwrap();
        }
        let (bytes, _) = writer.to_bytes();
        let pak = orbit_assets::PakSource::from_memory(bytes).unwrap();
        let loader = AssetLoader::from_pak(pak, "");
        let mut store = AssetStore::default();
        let pending = start_loading(&loader, &story, &store);
        for _ in 0..5_000 {
            if pending.iter().all(|p| p.is_done()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let failures = finish_loading(&mut r, &mut store, pending);
        assert!(failures.is_empty(), "{failures:?}");
        assert!(store.rig("fixtures/assets/robo.rig.ron").is_some());
        assert!(store.texture("fixtures/assets/robo_head.png").is_some());
    }

    /// A picture a rig draws that the story also declares as an image, after
    /// the rig (an eyelid only a clip shows): the rig uploads it first, then
    /// the image finishes under the same name. The texture must stay alive
    /// (regression: the exported game released it, blinks never showed).
    #[test]
    fn a_rig_picture_also_declared_after_the_rig_stays_alive() {
        let Some(mut r) = renderer() else {
            eprintln!("no GPU adapter: skipping the shared rig picture test");
            return;
        };
        let mut story = Story::from_ron(STORY).unwrap();
        let head = "fixtures/assets/robo_head.png";
        assert!(!story.assets.assets.iter().any(|a| a.path == head));
        story.assets.assets.push(orbit_core::AssetEntry {
            id: orbit_core::AssetId::new(),
            kind: orbit_core::AssetKind::Image,
            path: head.into(),
        });
        let store = load(&mut r, &story);
        let id = store.texture(head).expect("the picture is loaded");
        assert!(r.texture_size(id).is_some(), "the rig picture was released");
        assert_eq!(r.texture_by_name(head), Some(id));
    }

    fn differing(a: &[u8], b: &[u8]) -> usize {
        a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count()
    }

    #[test]
    fn the_rigged_character_plays_natively_and_cross_fades() {
        let Some(mut r) = renderer() else {
            eprintln!("no GPU adapter: skipping the rigged fixture read-back test");
            return;
        };
        let story = Story::from_ron(STORY).unwrap();
        let store = load(&mut r, &story);
        let hero = story.characters[0].id;
        assert!(store.rig("fixtures/assets/robo.rig.ron").is_some(), "the rig loaded with its pictures");
        for p in ["robo_body", "robo_arm", "robo_head"] {
            assert!(store.texture(&format!("fixtures/assets/{p}.png")).is_some(), "{p}");
        }

        let idle = stage(hero, "idle", &story);
        let mut clock = AnimClock::default();
        clock.sync([&idle]);
        let frame0 = draw(&mut r, &store, &story, &idle, &clock);
        // Same clock: same picture (deterministic, nothing stored per frame).
        assert_eq!(frame0, draw(&mut r, &store, &story, &idle, &clock));
        // Breathing: a second later the picture moved.
        clock.advance(1.0);
        let frame1 = draw(&mut r, &store, &story, &idle, &clock);
        assert!(differing(&frame0, &frame1) > 50, "the idle clip animates");

        // The rig is on screen at all: a different expression of the same
        // clock that raises the arm changes other pixels.
        let wave = stage(hero, "wave", &story);
        let mut clock = AnimClock::default();
        clock.sync([&idle]);
        clock.advance(1.0);
        let before = draw(&mut r, &store, &story, &idle, &clock);
        clock.sync([&wave]);
        let at_change = draw(&mut r, &store, &story, &wave, &clock);
        clock.advance(0.1);
        let mid_fade = draw(&mut r, &store, &story, &wave, &clock);
        clock.advance(0.5);
        let after = draw(&mut r, &store, &story, &wave, &clock);
        // At the change instant the fade has not started: still the old pose.
        assert_eq!(differing(&before, &at_change), 0, "no jump at the expression change");
        assert!(differing(&at_change, &mid_fade) > 0, "the fade moves");
        assert!(differing(&mid_fade, &after) > 0, "the wave goes on");

        // Reduced motion: the first pose is held.
        let mut frozen = AnimClock::default();
        frozen.set_reduced_motion(true);
        frozen.sync([&idle]);
        let still0 = draw(&mut r, &store, &story, &idle, &frozen);
        frozen.advance(1.0);
        assert_eq!(still0, draw(&mut r, &store, &story, &idle, &frozen));
        assert_eq!(still0, frame0, "held pose = first pose");
        let _ = StageFx::default();
    }
}
