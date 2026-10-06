//! Tests of the picture users index.

use std::path::PathBuf;

use orbit_core::{AssetEntry, AssetId, AssetKind, Story};

use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orbit_picusers_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets/rigs")).unwrap();
    std::fs::create_dir_all(dir.join("ui")).unwrap();
    dir
}

fn rig_text(image: &str, second: &str) -> String {
    format!(
        r#"(
    rig_version: 1,
    bones: [(id: "root", name: "root")],
    slots: [
        (id: "s1", name: "eyes", bone: "root", attachment: Some("a1")),
        (id: "s2", name: "mouth", bone: "root", attachment: Some("a2")),
    ],
    skins: [(name: "default", attachments: {{
        "a1": Region((image: "{image}", size: (10.0, 20.0), xform: (x: 0.0, y: 0.0, rotation: 0.0, sx: 1.0, sy: 1.0))),
        "a2": Region((image: "{second}", size: (10.0, 20.0), xform: (x: 0.0, y: 0.0, rotation: 0.0, sx: 1.0, sy: 1.0))),
    }})],
    clips: {{}},
)"#
    )
}

#[test]
fn index_finds_rigs_slots_story_and_ui_users() {
    let dir = temp_dir("index");
    std::fs::write(dir.join("assets/rigs/mira.rig.ron"), rig_text("assets/a.png", "assets/b.png")).unwrap();
    std::fs::write(dir.join("assets/rigs/mira_twin.rig.ron"), rig_text("assets/a.png", "assets/a.png")).unwrap();
    std::fs::write(dir.join("ui/theme.ron"), r#"(button: Some("assets/b.png"), font: "x.ttf")"#).unwrap();
    let mut story = Story::new("t");
    story.assets.assets.push(AssetEntry { id: AssetId::new(), kind: AssetKind::Image, path: "assets/a.png".into() });
    let idx = PictureUsers::build(&dir, &story, None);
    let a: Vec<String> = idx.users("assets\\a.png").iter().map(ToString::to_string).collect();
    assert_eq!(
        a,
        vec![
            "assets/rigs/mira.rig.ron / eyes",
            "assets/rigs/mira_twin.rig.ron / eyes",
            "assets/rigs/mira_twin.rig.ron / mouth"
        ]
    );
    let b = idx.users("assets/b.png");
    assert_eq!(b.len(), 2, "mira mouth and the ui theme: {b:?}");
    assert!(b.contains(&PictureUser::Ui("ui/theme.ron".into())));
    // Without the slot being edited, the others remain.
    assert_eq!(idx.others("assets/a.png", "assets/rigs/mira.rig.ron", "eyes").len(), 2);
    assert!(idx.others("assets/zzz.png", "assets/rigs/mira.rig.ron", "eyes").is_empty());
    // A rig skipped on disk is replaced by the caller's in-memory version.
    let skipped = PictureUsers::build(&dir, &story, Some("assets/rigs/mira.rig.ron"));
    assert_eq!(skipped.users("assets/a.png").len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn repoint_slot_leaves_the_other_slots_alone() {
    let rig = orbit_anim::rig::load_rig(&rig_text("assets/a.png", "assets/a.png")).unwrap().rig;
    let mut copy = rig.clone();
    assert_eq!(repoint_slot(&mut copy, "eyes", "assets/a.png", "assets/mira_eyes.png"), 1);
    let images: Vec<String> = copy.skins[0].attachments.values().map(|a| a.image().to_owned()).collect();
    assert_eq!(images, vec!["assets/mira_eyes.png", "assets/a.png"]);
    assert_eq!(repoint_slot(&mut copy, "nope", "assets/a.png", "x.png"), 0);
}
