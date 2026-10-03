//! Achievements (M7-05) and voices (M7-06) through the project API: every
//! edit is undoable, validated, and refused when it would leave a dangling
//! reference.

use orbit_core::{AssetKind, NodeId, NodeKind, SceneId, Story, validate};
use orbit_project::achievements::{
    add_achievement, delete_achievement, insert_unlock, unlock_nodes, update_achievement,
};
use orbit_project::ops::{self, NodeDraft};
use orbit_project::voices::{apply_voice_plan, plan_voice_import, set_say_voice};
use orbit_project::{Command, CommandError, Project, ProjectError};

fn project_with_line() -> (Project, SceneId, NodeId) {
    let mut p = Project::new(Story::new("t"));
    let scene = ops::add_scene(&mut p, "intro").unwrap();
    let text = orbit_core::StringId::new();
    p.execute(Command::SetString {
        id: text,
        text: Some("Hello".into()),
    })
    .unwrap();
    let draft = NodeDraft::plain(NodeKind::Say {
        speaker: None,
        text_id: text,
        voice: None,
    });
    let node = draft.node.id;
    ops::insert_drafts(&mut p, scene, 0, vec![draft], "say").unwrap();
    (p, scene, node)
}

#[test]
fn an_achievement_is_added_edited_unlocked_and_deleted_with_undo() {
    let (mut p, scene, _) = project_with_line();
    let icon = ops::add_asset(&mut p, AssetKind::Image, "assets/ui/medal.png").unwrap();
    let id = add_achievement(&mut p, "First words", "Say hello.", true, Some(icon)).unwrap();
    let story = p.story().clone();
    let a = &story.achievements[0];
    assert_eq!(story.text(a.title_id), Some("First words"));
    assert!(a.hidden);
    assert_eq!(a.icon, Some(icon));
    // Never unlocked: a warning, no error.
    let report = validate(&story);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.iter().any(|w| matches!(w, orbit_core::ValidationWarning::AchievementNeverUnlocked(a) if *a == id)));

    let unlock = insert_unlock(&mut p, scene, 1, id).unwrap();
    assert_eq!(unlock_nodes(p.story(), id), vec![unlock]);
    let report = validate(p.story());
    assert!(report.errors.is_empty());
    assert!(!report.warnings.iter().any(|w| matches!(w, orbit_core::ValidationWarning::AchievementNeverUnlocked(_))));

    update_achievement(&mut p, id, Some("Hello!"), None, Some(false), Some(None)).unwrap();
    let a = &p.story().achievements[0];
    assert_eq!(p.story().text(a.title_id), Some("Hello!"));
    assert!(!a.hidden && a.icon.is_none());

    // Refused while an Unlock node names it; the icon asset is free again.
    assert!(matches!(
        delete_achievement(&mut p, id),
        Err(ProjectError::Command(CommandError::AchievementInUse(..)))
    ));
    ops::delete_nodes(&mut p, &[unlock]).unwrap();
    let before = p.story().clone();
    delete_achievement(&mut p, id).unwrap();
    assert!(p.story().achievements.is_empty());
    assert!(p.undo().unwrap());
    assert_eq!(*p.story(), before);
}

#[test]
fn the_icon_of_an_achievement_protects_its_asset_and_must_be_an_image() {
    let (mut p, ..) = project_with_line();
    let sound = ops::add_asset(&mut p, AssetKind::Sound, "assets/sounds/a.wav").unwrap();
    assert!(add_achievement(&mut p, "A", "", false, Some(sound)).is_err());
    assert!(add_achievement(&mut p, " ", "", false, None).is_err());
    let icon = ops::add_asset(&mut p, AssetKind::Image, "assets/ui/m.png").unwrap();
    add_achievement(&mut p, "A", "", false, Some(icon)).unwrap();
    assert!(ops::remove_asset(&mut p, icon).is_err());
    // A string in use cannot be removed either.
    let title = p.story().achievements[0].title_id;
    assert!(matches!(
        p.execute(Command::SetString {
            id: title,
            text: None
        }),
        Err(ProjectError::Command(CommandError::StringUsedByAchievement(..)))
    ));
}

#[test]
fn a_voice_is_set_registers_the_sound_and_protects_it() {
    let (mut p, _, node) = project_with_line();
    set_say_voice(&mut p, node, Some("voice/line1.ogg")).unwrap();
    let story = p.story().clone();
    let NodeKind::Say { voice: Some(v), .. } = &story.node(node).unwrap().kind else {
        panic!("no voice");
    };
    assert_eq!(story.asset_path(*v), Some("voice/line1.ogg"));
    assert_eq!(story.assets.get(*v).unwrap().kind, AssetKind::Sound);
    assert!(validate(&story).errors.is_empty());
    // In use by the line.
    assert!(ops::remove_asset(&mut p, *v).is_err());
    // One undo undoes the registration and the voice together.
    assert!(p.undo().unwrap());
    assert!(p.story().assets.assets.is_empty());
    set_say_voice(&mut p, node, Some("voice/line1.ogg")).unwrap();
    set_say_voice(&mut p, node, None).unwrap();
    assert!(matches!(
        p.story().node(node).unwrap().kind,
        NodeKind::Say { voice: None, .. }
    ));
    // Not a line / bad path.
    let scene = p.story().scenes[0].id;
    let jump = NodeDraft::plain(NodeKind::End);
    let end = jump.node.id;
    ops::insert_drafts(&mut p, scene, 1, vec![jump], "end").unwrap();
    assert!(set_say_voice(&mut p, end, Some("voice/a.ogg")).is_err());
    assert!(set_say_voice(&mut p, node, Some("../a.ogg")).is_err());
}

#[test]
fn the_batch_import_matches_files_by_line_id_language_folder_and_csv() {
    let (mut p, _, node) = project_with_line();
    let text = match &p.story().node(node).unwrap().kind {
        NodeKind::Say { text_id, .. } => *text_id,
        _ => unreachable!(),
    };
    let files = vec![
        format!("voice/{node}.ogg"),
        format!("voice/fr/{text}.wav"),
        "voice/readme.txt".to_owned(),
        "voice/01ARZ3NDEKTSV4RRFFQ69G5FAV.ogg".to_owned(),
        format!("voice\\en\\{node}.ogg"),
    ];
    let plan = plan_voice_import(p.story(), &files, None);
    // `en` is the source language: that file replaces the first for the node.
    assert_eq!(plan.matches.len(), 2);
    assert_eq!(plan.unmatched.len(), 2);
    let fr = plan
        .matches
        .iter()
        .find(|m| m.language.as_deref() == Some("fr"))
        .unwrap();
    assert_eq!((fr.node, fr.text_id), (node, text));

    let csv = format!("line,file,language\n{node},voice/csv.ogg\n{text},voice/de.ogg,de\nnope,x.ogg\n");
    let plan = plan_voice_import(p.story(), &[], Some(&csv));
    assert_eq!(plan.matches.len(), 2);
    assert_eq!(plan.unmatched.len(), 1, "{:?}", plan.unmatched);

    let before = p.story().clone();
    let report = apply_voice_plan(&mut p, &plan).unwrap();
    assert_eq!(report.source_voices, 1);
    assert_eq!(report.translated["de"].len(), 1);
    let NodeKind::Say { voice: Some(v), .. } = &p.story().node(node).unwrap().kind else {
        panic!("no voice");
    };
    assert_eq!(p.story().asset_path(*v), Some("voice/csv.ogg"));
    // The German voice is registered too (for its language file).
    assert!(p.story().assets.assets.iter().any(|a| a.path == "voice/de.ogg"));
    assert!(p.undo().unwrap());
    assert_eq!(*p.story(), before);
}
