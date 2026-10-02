//! Asset manifest operations: removal is refused while something uses the
//! asset (a node **or** a character expression), renaming keeps the id.

use orbit_core::{AssetKind, Color, Story, validate};
use orbit_project::ops::{self, NodeDraft};
use orbit_project::{AssetUser, Command, CommandError, Project, ProjectError};

fn project() -> (Project, orbit_core::SceneId) {
    let mut p = Project::new(Story::new("t"));
    let scene = ops::add_scene(&mut p, "intro").unwrap();
    (p, scene)
}

/// Runs `edit`, then checks one undo restores the story exactly and one redo
/// reapplies it.
fn undoable(p: &mut Project, edit: impl FnOnce(&mut Project)) {
    let before = p.story().clone();
    edit(p);
    let after = p.story().clone();
    assert_ne!(before, after, "the edit changed nothing");
    assert!(p.undo().unwrap());
    assert_eq!(*p.story(), before);
    assert!(p.redo().unwrap());
    assert_eq!(*p.story(), after);
    assert!(p.undo().unwrap());
    assert_eq!(*p.story(), before);
}

#[test]
fn asset_removal_is_refused_while_a_node_or_a_character_expression_uses_it() {
    let (mut p, scene) = project();
    let bg = ops::add_asset(&mut p, AssetKind::Image, "assets/images/bg.png").unwrap();
    let sprite = ops::add_asset(&mut p, AssetKind::Image, "assets/images/ana.png").unwrap();
    let free = ops::add_asset(&mut p, AssetKind::Sound, "assets/sounds/ding.wav").unwrap();

    // Used by a node.
    let draft = NodeDraft::plain(orbit_core::NodeKind::Background {
        asset: bg,
        transition: None,
    });
    let node_id = draft.node.id;
    ops::insert_drafts(&mut p, scene, 0, vec![draft], "bg").unwrap();
    assert!(ops::remove_asset(&mut p, bg).is_err());
    // The command itself refuses too (what the MCP and plugins go through).
    assert!(matches!(
        p.execute(Command::SetAsset {
            id: bg,
            entry: None,
            index: None
        }),
        Err(ProjectError::Command(CommandError::AssetInUse(..)))
    ));

    // Used by a character expression only.
    let ana = ops::add_character(&mut p, "Ana", Color::WHITE).unwrap();
    let mut def = p.story().character(ana).unwrap().clone();
    def.expressions.push(orbit_core::Expression {
        name: "happy".into(),
        image: sprite,
        motion: None,
    });
    p.execute(Command::SetCharacter {
        id: ana,
        character: Some(def),
        index: None,
    })
    .unwrap();
    assert!(orbit_project::asset_users(p.story(), sprite).contains(&AssetUser::Character(ana)));
    let message = ops::remove_asset(&mut p, sprite).unwrap_err().to_string();
    assert!(message.contains("Ana"), "{message}");
    assert!(matches!(
        p.execute(Command::SetAsset {
            id: sprite,
            entry: None,
            index: None
        }),
        Err(ProjectError::Command(CommandError::AssetInUseByCharacter(
            ..
        )))
    ));
    assert!(p.story().assets.get(sprite).is_some());

    // An unused asset goes, and comes back on undo.
    undoable(&mut p, |p| ops::remove_asset(p, free).unwrap());
    // Once the node is gone the background asset can be removed.
    ops::delete_nodes(&mut p, &[node_id]).unwrap();
    undoable(&mut p, |p| ops::remove_asset(p, bg).unwrap());
}

#[test]
fn renaming_an_asset_path_keeps_its_id_and_refuses_collisions() {
    let (mut p, scene) = project();
    let a = ops::add_asset(&mut p, AssetKind::Image, "assets/images/a.png").unwrap();
    let _b = ops::add_asset(&mut p, AssetKind::Image, "assets/images/b.png").unwrap();
    let draft = NodeDraft::plain(orbit_core::NodeKind::Background {
        asset: a,
        transition: None,
    });
    ops::insert_drafts(&mut p, scene, 0, vec![draft], "bg").unwrap();
    undoable(&mut p, |p| {
        ops::set_asset_path(p, a, "assets/images/renamed.png").unwrap();
        assert_eq!(
            p.story().assets.get(a).unwrap().path,
            "assets/images/renamed.png"
        );
    });
    // References are by id: validation is still clean.
    assert!(validate(p.story()).errors.is_empty());
    assert!(ops::set_asset_path(&mut p, a, "assets/images/B.PNG").is_err());
    assert!(ops::set_asset_path(&mut p, a, "../escape.png").is_err());
    assert!(ops::set_asset_path(&mut p, a, "C:/abs.png").is_err());
    assert!(ops::set_asset_path(&mut p, a, "").is_err());
}
