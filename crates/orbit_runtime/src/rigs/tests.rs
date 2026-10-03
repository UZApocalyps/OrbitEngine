use std::collections::BTreeMap;

use orbit_anim::rig::{
    Attachment, AttachmentId, Bone, BoneId, PreparedRig, RegionAttachment, Rig, RigClip, RigTrack, Skin, Slot,
    SlotId, Xform,
};
use orbit_anim::{Angle, Key, Loop, Rgba, Track};
use orbit_core::{
    AssetEntry, AssetId, AssetKind, Character, CharacterId, Color, Expression, LoopMode, Motion, RiggedMotion,
    Story,
};

use super::*;
use crate::stage::{AnimClock, RIG_BLEND, StageCharacter, StageState};

fn rotate(from: f32, to: f32, duration: f32) -> RigClip {
    let mut c = RigClip::new(duration, Loop::Forever);
    c.tracks.push(RigTrack::BoneRotate {
        bone: BoneId::new("arm"),
        track: Track::new(vec![Key::new(0.0, Angle(from)), Key::new(duration, Angle(to))]),
    });
    c
}

/// root, an arm bone and a region on it; `idle` 0 -> 90 degrees in 1 s,
/// `wave` 0 -> -90 in 2 s.
fn arm_rig() -> Rig {
    let bone = |id: &str, parent: Option<&str>, y: f32| Bone {
        id: BoneId::new(id),
        name: id.into(),
        parent: parent.map(BoneId::new),
        rest: Xform::translate(0.0, y),
        length: 10.0,
    };
    let mut attachments = BTreeMap::new();
    attachments.insert(
        AttachmentId::new("hand"),
        Attachment::Region(RegionAttachment {
            image: "assets/hand.png".into(),
            size: [20.0, 10.0],
            xform: Xform::IDENTITY,
        }),
    );
    let mut clips = BTreeMap::new();
    clips.insert("idle".to_owned(), rotate(0.0, 90.0, 1.0));
    clips.insert("wave".to_owned(), rotate(0.0, -90.0, 2.0));
    Rig {
        bones: vec![bone("root", None, 0.0), bone("arm", Some("root"), -100.0)],
        slots: vec![Slot {
            id: SlotId::new("s"),
            name: "hand".into(),
            bone: BoneId::new("arm"),
            attachment: Some(AttachmentId::new("hand")),
            color: Rgba::new(1.0, 1.0, 1.0, 1.0),
        }],
        skins: vec![Skin { name: "default".into(), attachments }],
        clips,
        ..Rig::default()
    }
}

struct Fixture {
    story: Story,
    prepared: PreparedRig,
    hero: CharacterId,
    poster: AssetId,
    rig: AssetId,
}

fn rigged(rig: AssetId, clip: &str, loop_mode: LoopMode) -> Option<Motion> {
    Some(Motion::Rigged(RiggedMotion { rig, skin: None, clip: clip.into(), loop_mode }))
}

fn fixture() -> Fixture {
    let mut story = Story::new("rigs");
    let poster = AssetId::new();
    let rig = AssetId::new();
    let other_rig = AssetId::new();
    for (id, kind, path) in [
        (poster, AssetKind::Image, "assets/poster.png"),
        (rig, AssetKind::Rig, "assets/rigs/hero.rig.ron"),
        (other_rig, AssetKind::Rig, "assets/rigs/other.rig.ron"),
    ] {
        story.assets.assets.push(AssetEntry { id, kind, path: path.into() });
    }
    let hero = CharacterId::new();
    let name_id = story.strings.add("Hero");
    let expr = |name: &str, motion| Expression { name: name.into(), image: poster, motion };
    story.characters.push(Character {
        id: hero,
        name_id,
        color: Color::rgb(255, 255, 255),
        expressions: vec![
            expr("idle", rigged(rig, "idle", LoopMode::Loop)),
            expr("wave", rigged(rig, "wave", LoopMode::Once)),
            expr("elsewhere", rigged(other_rig, "idle", LoopMode::Loop)),
            expr("still", None),
        ],
        default_position: None,
        default_scale: 1.0,
    });
    Fixture { story, prepared: PreparedRig::new(arm_rig()).unwrap(), hero, poster, rig }
}

fn stage(f: &Fixture, expression: &str) -> StageState {
    StageState {
        characters: vec![StageCharacter {
            character: f.hero,
            expression: expression.into(),
            image: f.poster,
            x: 0.5,
            y: 1.0,
            scale: [1.0, 1.0],
            flip_x: false,
            flip_y: false,
        }],
        ..StageState::default()
    }
}

fn arm_rotation(f: &Fixture, stage: &StageState, clock: Option<&AnimClock>) -> f32 {
    let c = &stage.characters[0];
    let expression = f.story.character(c.character).unwrap().expression(&c.expression).unwrap();
    let motion = expression.rigged().expect("a rigged expression");
    let pose = stage_pose(&f.prepared, &f.story, c, motion, clock);
    pose.bones[f.prepared.bone_index("arm").unwrap()].rotation
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn loop_modes_map_the_clock_into_the_clip() {
    assert_eq!(clip_time(2.0, LoopMode::Loop, 5.0), 1.0);
    assert_eq!(clip_time(2.0, LoopMode::Once, 5.0), 2.0);
    assert_eq!(clip_time(2.0, LoopMode::Once, 0.5), 0.5);
    assert_eq!(clip_time(2.0, LoopMode::PingPong, 3.0), 1.0);
    assert_eq!(clip_time(2.0, LoopMode::PingPong, 4.5), 0.5);
    // Degenerate input holds the first pose.
    assert_eq!(clip_time(0.0, LoopMode::Loop, 1.0), 0.0);
    assert_eq!(clip_time(2.0, LoopMode::Loop, f32::NAN), 0.0);
    assert_eq!(clip_time(2.0, LoopMode::Loop, -1.0), 0.0);
}

#[test]
fn the_loop_mode_of_the_expression_overrides_the_clip_s_own() {
    let f = fixture();
    // `wave` is a Forever clip in the rig but `Once` in the expression: it
    // holds its last pose (-90) instead of looping.
    let s = stage(&f, "wave");
    let mut clock = AnimClock::default();
    clock.sync([&s]);
    clock.advance(5.0);
    assert!(near(arm_rotation(&f, &s, Some(&clock)), -90.0));
    // Unknown clip: setup pose, no panic.
    let mut g = fixture();
    if let Some(Motion::Rigged(r)) = &mut g.story.characters[0].expressions[0].motion {
        r.clip = "missing".into();
    }
    let s = stage(&g, "idle");
    assert!(near(arm_rotation(&g, &s, None), 0.0));
}

#[test]
fn a_pose_is_a_pure_function_of_the_clock_so_rollback_and_saves_need_no_state() {
    let f = fixture();
    let s = stage(&f, "idle");
    // Played for real: the clock advances frame by frame.
    let mut played = AnimClock::default();
    played.sync([&s]);
    for dt in [0.1, 0.05, 0.1] {
        played.advance(dt);
        played.sync([&s]);
    }
    // After a rollback / a load the stage is rebuilt and the clock is
    // derived from it: the same moment gives the same pose.
    let rebuilt = AnimClock::at(&s, 0.25);
    let a = arm_rotation(&f, &s, Some(&played));
    let b = arm_rotation(&f, &s, Some(&rebuilt));
    assert!(near(a, 22.5), "{a}");
    assert!(near(a, b));
    // Nothing is stored per frame: asking twice changes nothing.
    assert!(near(arm_rotation(&f, &s, Some(&played)), a));
}

#[test]
fn changing_expression_on_the_same_rig_cross_fades_the_clips() {
    let f = fixture();
    let mut s = stage(&f, "idle");
    let mut clock = AnimClock::default();
    clock.sync([&s]);
    clock.advance(0.5);
    assert!(near(arm_rotation(&f, &s, Some(&clock)), 45.0));

    // `Show` of the same character with another expression.
    s.characters[0].expression = "wave".into();
    clock.sync([&s]);
    let fade = clock.rig_fade(f.hero, "wave").expect("a fade starts");
    assert_eq!(fade.previous, "idle");
    assert!(near(fade.weight, 0.0));
    // At the change itself the pose is still the old one.
    assert!(near(arm_rotation(&f, &s, Some(&clock)), 45.0));

    // Halfway through the fade: idle is at 0.6 s (54), wave at 0.1 s (-4.5).
    clock.advance(RIG_BLEND * 0.5);
    let fade = clock.rig_fade(f.hero, "wave").unwrap();
    assert!(near(fade.weight, 0.5));
    assert!(near(fade.previous_elapsed, 0.6));
    assert!(near(arm_rotation(&f, &s, Some(&clock)), (54.0 + -4.5) * 0.5));

    // Fade over: the new clip alone (0.4 s into wave, 2 s clip: -18).
    clock.advance(RIG_BLEND * 0.5 + 0.2);
    assert!(clock.rig_fade(f.hero, "wave").is_none());
    assert!(near(arm_rotation(&f, &s, Some(&clock)), -18.0));
}

#[test]
fn no_fade_towards_another_rig_a_still_or_a_new_arrival() {
    let f = fixture();
    let mut s = stage(&f, "idle");
    let mut clock = AnimClock::default();
    clock.sync([&s]);
    clock.advance(0.3);
    // Another rig: the fade is recorded by the clock but the poses are not
    // mixed (different skeletons).
    s.characters[0].expression = "elsewhere".into();
    clock.sync([&s]);
    let fade = clock.rig_fade(f.hero, "elsewhere");
    assert!(fade.is_some());
    let c = &s.characters[0];
    let other = f.story.character(f.hero).unwrap().expression("elsewhere").unwrap().rigged().unwrap();
    let pose = stage_pose(&f.prepared, &f.story, c, other, Some(&clock));
    let none = stage_pose(&f.prepared, &f.story, c, other, None);
    assert_eq!(pose, none, "the clip of the other rig is shown as is");
    // A character leaving and coming back restarts without a fade.
    clock.sync([&StageState::default()]);
    s.characters[0].expression = "idle".into();
    clock.sync([&s]);
    assert!(clock.rig_fade(f.hero, "idle").is_none());
    assert_eq!(clock.elapsed(f.hero, "idle"), 0.0);
}

#[test]
fn reduced_motion_holds_the_first_pose_and_never_fades() {
    let f = fixture();
    let mut s = stage(&f, "idle");
    let mut clock = AnimClock::default();
    clock.set_reduced_motion(true);
    clock.sync([&s]);
    clock.advance(0.5);
    assert!(near(arm_rotation(&f, &s, Some(&clock)), 0.0));
    s.characters[0].expression = "wave".into();
    clock.sync([&s]);
    assert!(clock.rig_fade(f.hero, "wave").is_none());
    // Frame animations keep their own clock under reduced motion.
    assert!(clock.elapsed(f.hero, "wave") >= 0.0);
    clock.set_reduced_motion(false);
    clock.advance(0.5);
    assert!(near(arm_rotation(&f, &s, Some(&clock)), -22.5));
}

#[cfg(not(target_arch = "wasm32"))]
mod gpu {
    use orbit_render2d::orbit_gfx::Gfx;
    use orbit_render2d::{Renderer2d, Scene, TextureOptions};

    use super::*;
    use crate::loading::AssetStore;
    use crate::runtime::stage_scene;

    fn renderer() -> Option<Renderer2d> {
        let gfx = pollster::block_on(Gfx::headless((64, 64))).ok()?;
        Renderer2d::new(gfx).ok()
    }

    fn solid(r: &mut Renderer2d, name: &str) -> orbit_render2d::TextureId {
        let pixels: Vec<u8> = [255u8, 0, 0, 255].iter().copied().cycle().take(4 * 4 * 4).collect();
        r.create_texture(name, 4, 4, &pixels, TextureOptions::COLOR).unwrap()
    }

    #[test]
    fn a_rigged_character_is_drawn_from_the_rig_and_falls_back_to_the_poster() {
        let Some(mut r) = renderer() else {
            eprintln!("no GPU adapter: skipping rigged stage test");
            return;
        };
        let f = fixture();
        let mut store = AssetStore::default();
        let poster = solid(&mut r, "assets/poster.png");
        store.insert_texture("assets/poster.png", poster);

        // Rig not loaded yet: the poster is drawn (one sprite).
        let s = stage(&f, "idle");
        let mut scene = Scene::new();
        stage_scene(&r, &store, &f.story, &s, None, None, None, &mut scene);
        assert_eq!(scene.sprites().len(), 1);

        // Rig loaded but its picture is not: still the poster, no gap.
        store.insert_rig("assets/rigs/hero.rig.ron", std::sync::Arc::new(PreparedRig::new(arm_rig()).unwrap()));
        let mut scene = Scene::new();
        stage_scene(&r, &store, &f.story, &s, None, None, None, &mut scene);
        assert_eq!(scene.sprites().len(), 1);
        assert_eq!(scene.sprites()[0].texture, poster);

        // With its picture: the region follows the arm bone.
        let hand = solid(&mut r, "assets/hand.png");
        store.insert_rig_texture("assets/hand.png", hand);
        let clock = AnimClock::at(&s, 0.5);
        let mut scene = Scene::new();
        stage_scene(&r, &store, &f.story, &s, Some(&clock), None, None, &mut scene);
        assert_eq!(scene.sprites().len(), 1);
        let hand = &scene.sprites()[0];
        assert!(near(hand.rotation, 45f32.to_radians()), "{}", hand.rotation);
        // Still expressions are unaffected.
        let s = stage(&f, "still");
        let mut scene = Scene::new();
        stage_scene(&r, &store, &f.story, &s, None, None, None, &mut scene);
        assert_eq!(scene.sprites().len(), 1);
        let _ = f.rig;
    }
}
