use std::path::PathBuf;

use orbit_core::{
    AssetEntry, AssetId, AssetKind, Character, CharacterId, Color, Expression, LoopMode, Motion, RiggedMotion,
    Story,
};

use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orbit_rigs_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets/rigs")).unwrap();
    dir
}

const RIG: &str = r#"(
    rig_version: 1,
    bones: [(id: "root", name: "root")],
    slots: [(id: "s", name: "body", bone: "root", attachment: Some("body"))],
    skins: [(name: "default", attachments: {"body": Region((image: "assets/body.png", size: (10.0, 20.0), xform: (x: 0.0, y: -10.0, rotation: 0.0, sx: 1.0, sy: 1.0)))})],
    clips: {"idle": (duration: 1.0, looping: Forever, tracks: [])},
)"#;

fn story_with(rig_path: &str, clip: &str, skin: Option<&str>) -> Story {
    let mut story = Story::new("rigs");
    let poster = AssetId::new();
    let rig = AssetId::new();
    story.assets.assets.push(AssetEntry { id: poster, kind: AssetKind::Image, path: "assets/poster.png".into() });
    story.assets.assets.push(AssetEntry { id: rig, kind: AssetKind::Rig, path: rig_path.into() });
    let name_id = story.strings.add("Hero");
    story.characters.push(Character {
        id: CharacterId::new(),
        name_id,
        color: Color::rgb(1, 2, 3),
        expressions: vec![Expression {
            name: "idle".into(),
            image: poster,
            motion: Some(Motion::Rigged(RiggedMotion {
                rig,
                skin: skin.map(str::to_owned),
                clip: clip.into(),
                loop_mode: LoopMode::Loop,
            })),
        }],
        default_position: None,
        default_scale: 1.0,
    });
    story
}

#[test]
fn a_good_rig_has_no_problem_and_is_listed() {
    let dir = temp_dir("good");
    std::fs::write(dir.join("assets/rigs/hero.rig.ron"), RIG).unwrap();
    std::fs::write(dir.join("assets/rigs/spare.rig.ron"), RIG).unwrap();
    std::fs::write(dir.join("assets/body.png"), b"x").unwrap();
    let story = story_with("assets/rigs/hero.rig.ron", "idle", None);
    assert_eq!(rig_problems(&dir, &story), vec![]);

    let list = list_rigs(&dir, &story);
    assert_eq!(list.len(), 2);
    assert!(list[0].asset.is_some());
    let content = list[0].content.as_ref().unwrap();
    assert_eq!(content.clips, vec![("idle".to_owned(), 1.0)]);
    assert_eq!(content.skins, vec!["default".to_owned()]);
    assert!(content.missing_images.is_empty());
    assert_eq!(list[1].path, "assets/rigs/spare.rig.ron");
    assert!(list[1].asset.is_none(), "an undeclared file is listed too");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clip_skin_image_and_file_problems_are_reported() {
    let dir = temp_dir("bad");
    std::fs::write(dir.join("assets/rigs/hero.rig.ron"), RIG).unwrap();
    let story = story_with("assets/rigs/hero.rig.ron", "dance", Some("winter"));
    let kinds: Vec<_> = rig_problems(&dir, &story).into_iter().map(|p| p.kind).collect();
    assert!(kinds.contains(&RigProblemKind::UnknownClip("dance".into())));
    assert!(kinds.contains(&RigProblemKind::UnknownSkin("winter".into())));
    assert!(kinds.contains(&RigProblemKind::MissingImage("assets/body.png".into())));

    let missing = story_with("assets/rigs/none.rig.ron", "idle", None);
    let problems = rig_problems(&dir, &missing);
    assert!(matches!(problems[0].kind, RigProblemKind::Unreadable(_)));
    assert!(problems[0].to_string().contains("the still image is used"));

    std::fs::write(dir.join("assets/rigs/broken.rig.ron"), "(bones: [(id: \"a\", name: \"a\", parent: Some(\"zz\"))])")
        .unwrap();
    let broken = story_with("assets/rigs/broken.rig.ron", "idle", None);
    assert!(matches!(rig_problems(&dir, &broken)[0].kind, RigProblemKind::Unreadable(_)));
    // Paths cannot leave the project.
    assert!(read_rig(&dir, "../x.rig.ron").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
