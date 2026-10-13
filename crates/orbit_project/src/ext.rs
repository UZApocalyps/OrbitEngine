//! Story-level commands (scenes, node moves, characters, variables, assets).
//!
//! Kept apart from the node-level commands of `command.rs`: [`Command::apply`]
//! delegates here for these variants. Every application returns its exact
//! inverse, like the others.

use orbit_core::{AchievementId, AssetEntry, AssetId, CharacterId, NodeKind, Story, VarDecl};

use crate::command::{Applied, Command, CommandError};

/// Inserts `item` at `index` (clamped to the end when `None`).
fn insert_at<T>(list: &mut Vec<T>, index: Option<usize>, item: T) {
    let at = index.unwrap_or(list.len()).min(list.len());
    list.insert(at, item);
}

/// Node (if any) that still displays or uses character `id`.
fn character_user(story: &Story, id: CharacterId) -> Option<orbit_core::NodeId> {
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .find(|n| match &n.kind {
            NodeKind::Say { speaker, .. } => *speaker == Some(id),
            NodeKind::Show { character, .. } | NodeKind::Hide { character, .. } => *character == id,
            _ => false,
        })
        .map(|n| n.id)
}

/// What still uses an asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetUser {
    /// A node (background, music, sound).
    Node(orbit_core::NodeId),
    /// An expression (sprite) of a character.
    Character(CharacterId),
    /// The icon of an achievement (M7-05).
    Achievement(AchievementId),
}

/// Everything that uses asset `id`: nodes first, then characters whose
/// expressions point at it.
pub fn asset_users(story: &Story, id: AssetId) -> Vec<AssetUser> {
    let nodes = story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter(|n| match &n.kind {
            NodeKind::Background { asset, .. }
            | NodeKind::PlayMusic { asset, .. }
            | NodeKind::PlaySound { asset }
            | NodeKind::ShowScene3d { scene: asset, .. } => *asset == id,
            NodeKind::BackgroundLayers { layers, .. } => layers.iter().any(|l| l.asset == id),
            NodeKind::Hotspots(map) => map.assets().contains(&id),
            NodeKind::Notify(n) => n.assets().contains(&id),
            NodeKind::Particles(e) => e.sprite == Some(id),
            NodeKind::Say { voice, .. } => *voice == Some(id),
            _ => false,
        })
        .map(|n| AssetUser::Node(n.id));
    let characters = story
        .characters
        .iter()
        .filter(|c| c.expressions.iter().any(|e| e.assets().contains(&id)))
        .map(|c| AssetUser::Character(c.id));
    let achievements = story
        .achievements
        .iter()
        .filter(|a| a.icon == Some(id) || a.locked_icon == Some(id))
        .map(|a| AssetUser::Achievement(a.id));
    nodes.chain(characters).chain(achievements).collect()
}

/// [`asset_users`] of every asset at once, in one pass over the story (for
/// the questions about many assets: a bulk removal, a usage column). Same
/// order per asset as [`asset_users`]; unused assets are absent.
pub fn asset_users_index(story: &Story) -> std::collections::HashMap<AssetId, Vec<AssetUser>> {
    let mut out: std::collections::HashMap<AssetId, Vec<AssetUser>> = Default::default();
    let mut add = |id: AssetId, user: AssetUser| {
        let users = out.entry(id).or_default();
        // A user's references come in a row: comparing with the last is enough.
        if users.last() != Some(&user) {
            users.push(user);
        }
    };
    for n in story.scenes.iter().flat_map(|s| &s.nodes) {
        let user = AssetUser::Node(n.id);
        match &n.kind {
            NodeKind::Background { asset, .. }
            | NodeKind::PlayMusic { asset, .. }
            | NodeKind::PlaySound { asset }
            | NodeKind::ShowScene3d { scene: asset, .. } => add(*asset, user),
            NodeKind::BackgroundLayers { layers, .. } => layers.iter().for_each(|l| add(l.asset, user)),
            NodeKind::Hotspots(map) => map.assets().into_iter().for_each(|a| add(a, user)),
            NodeKind::Notify(n) => n.assets().into_iter().for_each(|a| add(a, user)),
            NodeKind::Particles(e) => e.sprite.into_iter().for_each(|s| add(s, user)),
            NodeKind::Say { voice: Some(v), .. } => add(*v, user),
            _ => {}
        }
    }
    for c in &story.characters {
        for e in &c.expressions {
            for a in e.assets() {
                add(a, AssetUser::Character(c.id));
            }
        }
    }
    for a in &story.achievements {
        for icon in [a.icon, a.locked_icon].into_iter().flatten() {
            add(icon, AssetUser::Achievement(a.id));
        }
    }
    out
}

pub(crate) fn apply(cmd: &Command, story: &mut Story) -> Result<Applied, CommandError> {
    match cmd {
        Command::InsertScene { index, scene } => {
            if story.scenes.iter().any(|s| s.id == scene.id) {
                return Err(CommandError::DuplicateScene(scene.id));
            }
            if *index > story.scenes.len() {
                return Err(CommandError::IndexOutOfBounds {
                    index: *index,
                    len: story.scenes.len(),
                });
            }
            if let Some(dup) = scene.nodes.iter().find(|n| story.locate(n.id).is_some()) {
                return Err(CommandError::DuplicateNode(dup.id));
            }
            let touched = scene.nodes.iter().map(|n| n.id).collect();
            story.scenes.insert(*index, scene.clone());
            Ok(Applied {
                inverse: Command::RemoveScene { id: scene.id },
                touched,
            })
        }
        Command::RemoveScene { id } => {
            let index = story
                .scenes
                .iter()
                .position(|s| s.id == *id)
                .ok_or(CommandError::UnknownScene(*id))?;
            // Nodes of other scenes must not target the nodes being removed.
            let inside: Vec<_> = story.scenes[index].nodes.iter().map(|n| n.id).collect();
            for (si, scene) in story.scenes.iter().enumerate() {
                if si == index {
                    continue;
                }
                for node in &scene.nodes {
                    if let Some(hit) = node.kind.targets().into_iter().find(|t| inside.contains(t))
                    {
                        return Err(CommandError::StillReferenced(hit, node.id));
                    }
                    if node.kind.target_scene() == Some(*id) {
                        return Err(CommandError::SceneStillTargeted(*id, node.id));
                    }
                }
            }
            let scene = story.scenes.remove(index);
            Ok(Applied {
                touched: scene.nodes.iter().map(|n| n.id).collect(),
                inverse: Command::InsertScene { index, scene },
            })
        }
        Command::RenameScene { id, name } => {
            let scene = story
                .scenes
                .iter_mut()
                .find(|s| s.id == *id)
                .ok_or(CommandError::UnknownScene(*id))?;
            let old = std::mem::replace(&mut scene.name, name.clone());
            Ok(Applied {
                inverse: Command::RenameScene { id: *id, name: old },
                touched: Vec::new(),
            })
        }
        Command::MoveScene { id, index } => {
            let from = story
                .scenes
                .iter()
                .position(|s| s.id == *id)
                .ok_or(CommandError::UnknownScene(*id))?;
            if *index >= story.scenes.len() {
                return Err(CommandError::IndexOutOfBounds {
                    index: *index,
                    len: story.scenes.len(),
                });
            }
            let scene = story.scenes.remove(from);
            story.scenes.insert(*index, scene);
            Ok(Applied {
                inverse: Command::MoveScene {
                    id: *id,
                    index: from,
                },
                touched: Vec::new(),
            })
        }
        Command::SetPositions { positions } => {
            let old = std::mem::replace(&mut story.positions, positions.clone());
            Ok(Applied {
                inverse: Command::SetPositions { positions: old },
                touched: Vec::new(),
            })
        }
        Command::SetCurve { name, curve } => {
            if name.trim().is_empty() {
                return Err(CommandError::EmptyCurveName);
            }
            let old = match curve {
                Some(c) => {
                    if matches!(c, orbit_core::Curve::Named(_)) {
                        return Err(CommandError::CurveChain(name.clone()));
                    }
                    story.curves.insert(name.clone(), c.clone())
                }
                None => story.curves.remove(name),
            };
            Ok(Applied {
                inverse: Command::SetCurve {
                    name: name.clone(),
                    curve: old,
                },
                touched: Vec::new(),
            })
        }
        Command::MoveNode {
            id,
            scene,
            index,
            expected_version: _,
        } => {
            let (si, ni) = story.locate(*id).ok_or(CommandError::UnknownNode(*id))?;
            let to = story
                .scenes
                .iter()
                .position(|s| s.id == *scene)
                .ok_or(CommandError::UnknownScene(*scene))?;
            let remaining = story.scenes[to].nodes.len() - usize::from(si == to);
            if *index > remaining {
                return Err(CommandError::IndexOutOfBounds {
                    index: *index,
                    len: remaining,
                });
            }
            let old_scene = story.scenes[si].id;
            let node = story.scenes[si].nodes.remove(ni);
            story.scenes[to].nodes.insert(*index, node);
            Ok(Applied {
                inverse: Command::MoveNode {
                    id: *id,
                    scene: old_scene,
                    index: ni,
                    expected_version: None,
                },
                touched: vec![*id],
            })
        }
        Command::SetNodeKind {
            id,
            kind,
            expected_version: _,
        } => {
            let node = story.node(*id).ok_or(CommandError::UnknownNode(*id))?;
            // A label that is still targeted cannot become another kind.
            if matches!(node.kind, NodeKind::Label { .. })
                && !matches!(kind, NodeKind::Label { .. })
                && let Some(by) = story
                    .scenes
                    .iter()
                    .flat_map(|s| &s.nodes)
                    .find(|n| n.kind.targets().contains(id))
            {
                return Err(CommandError::StillReferenced(*id, by.id));
            }
            let node = story.node_mut(*id).ok_or(CommandError::UnknownNode(*id))?;
            let old = std::mem::replace(&mut node.kind, kind.clone());
            Ok(Applied {
                inverse: Command::SetNodeKind {
                    id: *id,
                    kind: old,
                    expected_version: None,
                },
                touched: vec![*id],
            })
        }
        Command::SetCharacter {
            id,
            character,
            index,
        } => {
            let existing = story.characters.iter().position(|c| c.id == *id);
            match (existing, character) {
                (Some(pos), Some(new)) => {
                    let old = std::mem::replace(&mut story.characters[pos], new.clone());
                    Ok(Applied {
                        inverse: Command::SetCharacter {
                            id: *id,
                            character: Some(old),
                            index: Some(pos),
                        },
                        touched: Vec::new(),
                    })
                }
                (None, Some(new)) => {
                    insert_at(&mut story.characters, *index, new.clone());
                    Ok(Applied {
                        inverse: Command::SetCharacter {
                            id: *id,
                            character: None,
                            index: None,
                        },
                        touched: Vec::new(),
                    })
                }
                (Some(pos), None) => {
                    if let Some(user) = character_user(story, *id) {
                        return Err(CommandError::CharacterInUse(*id, user));
                    }
                    let old = story.characters.remove(pos);
                    Ok(Applied {
                        inverse: Command::SetCharacter {
                            id: *id,
                            character: Some(old),
                            index: Some(pos),
                        },
                        touched: Vec::new(),
                    })
                }
                (None, None) => Err(CommandError::UnknownCharacter(*id)),
            }
        }
        Command::SetVariable { name, decl, index } => {
            let existing = story.variables.iter().position(|v| &v.name == name);
            match (existing, decl) {
                (Some(pos), Some(new)) => {
                    let old = std::mem::replace(&mut story.variables[pos], new.clone());
                    Ok(Applied {
                        inverse: Command::SetVariable {
                            // Keyed by the *new* name: the replacement may rename.
                            name: new.name.clone(),
                            decl: Some(old),
                            index: Some(pos),
                        },
                        touched: Vec::new(),
                    })
                }
                (None, Some(new)) => {
                    insert_at(&mut story.variables, *index, new.clone());
                    Ok(Applied {
                        inverse: Command::SetVariable {
                            name: name.clone(),
                            decl: None,
                            index: None,
                        },
                        touched: Vec::new(),
                    })
                }
                (Some(pos), None) => {
                    let old: VarDecl = story.variables.remove(pos);
                    Ok(Applied {
                        inverse: Command::SetVariable {
                            name: name.clone(),
                            decl: Some(old),
                            index: Some(pos),
                        },
                        touched: Vec::new(),
                    })
                }
                (None, None) => Err(CommandError::UnknownVariable(name.clone())),
            }
        }
        Command::SetAsset { id, entry, index } => {
            let existing = story.assets.assets.iter().position(|a| a.id == *id);
            match (existing, entry) {
                (Some(pos), Some(new)) => {
                    let old = std::mem::replace(&mut story.assets.assets[pos], new.clone());
                    Ok(Applied {
                        inverse: Command::SetAsset {
                            id: *id,
                            entry: Some(old),
                            index: Some(pos),
                        },
                        touched: Vec::new(),
                    })
                }
                (None, Some(new)) => {
                    insert_at(&mut story.assets.assets, *index, new.clone());
                    Ok(Applied {
                        inverse: Command::SetAsset {
                            id: *id,
                            entry: None,
                            index: None,
                        },
                        touched: Vec::new(),
                    })
                }
                (Some(pos), None) => {
                    match asset_users(story, *id).first() {
                        Some(AssetUser::Node(user)) => {
                            return Err(CommandError::AssetInUse(*id, *user));
                        }
                        Some(AssetUser::Character(user)) => {
                            return Err(CommandError::AssetInUseByCharacter(*id, *user));
                        }
                        Some(AssetUser::Achievement(user)) => {
                            return Err(CommandError::AssetInUseByAchievement(*id, *user));
                        }
                        None => {}
                    }
                    let old: AssetEntry = story.assets.assets.remove(pos);
                    Ok(Applied {
                        inverse: Command::SetAsset {
                            id: *id,
                            entry: Some(old),
                            index: Some(pos),
                        },
                        touched: Vec::new(),
                    })
                }
                (None, None) => Err(CommandError::UnknownAsset(*id)),
            }
        }
        Command::SetAchievement {
            id,
            achievement,
            index,
        } => {
            let existing = story.achievements.iter().position(|a| a.id == *id);
            match (existing, achievement) {
                (Some(pos), Some(new)) => {
                    let old = std::mem::replace(&mut story.achievements[pos], new.clone());
                    Ok(Applied {
                        inverse: Command::SetAchievement {
                            id: *id,
                            achievement: Some(old),
                            index: Some(pos),
                        },
                        touched: Vec::new(),
                    })
                }
                (None, Some(new)) => {
                    insert_at(&mut story.achievements, *index, new.clone());
                    Ok(Applied {
                        inverse: Command::SetAchievement {
                            id: *id,
                            achievement: None,
                            index: None,
                        },
                        touched: Vec::new(),
                    })
                }
                (Some(pos), None) => {
                    if let Some(user) = story.scenes.iter().flat_map(|s| &s.nodes).find(
                        |n| matches!(&n.kind, NodeKind::Unlock { achievement } if achievement == id),
                    ) {
                        return Err(CommandError::AchievementInUse(*id, user.id));
                    }
                    let old = story.achievements.remove(pos);
                    Ok(Applied {
                        inverse: Command::SetAchievement {
                            id: *id,
                            achievement: Some(old),
                            index: Some(pos),
                        },
                        touched: Vec::new(),
                    })
                }
                (None, None) => Err(CommandError::UnknownAchievement(*id)),
            }
        }
        // Node-level commands are handled by `Command::apply` itself.
        _ => unreachable!("not a story-level command"),
    }
}

#[cfg(test)]
mod tests {
    use orbit_core::{
        AssetKind, Character, Color, Node, NodeId, NodeKind, Scene, SceneId, Story, StringId, Value,
    };

    use crate::{Command, CommandError, Project, ProjectError};

    fn scene(name: &str, nodes: Vec<Node>) -> Scene {
        Scene {
            id: SceneId::new(),
            name: name.into(),
            nodes,
        }
    }

    fn node(kind: NodeKind) -> Node {
        Node {
            id: NodeId::new(),
            kind,
        }
    }

    #[test]
    fn scenes_insert_rename_remove_and_undo() {
        let mut p = Project::new(Story::new("t"));
        let s = scene("a", vec![node(NodeKind::End)]);
        let id = s.id;
        p.execute(Command::InsertScene { index: 0, scene: s })
            .unwrap();
        p.execute(Command::RenameScene {
            id,
            name: "b".into(),
        })
        .unwrap();
        assert_eq!(p.story().scenes[0].name, "b");
        p.undo().unwrap();
        assert_eq!(p.story().scenes[0].name, "a");
        p.redo().unwrap();
        p.execute(Command::RemoveScene { id }).unwrap();
        assert!(p.story().scenes.is_empty());
        p.undo().unwrap();
        assert_eq!(p.story().scenes[0].nodes.len(), 1);
        // Versions of the restored nodes exist again.
        let node_id = p.story().scenes[0].nodes[0].id;
        assert!(p.node_version(node_id).is_some());
        assert!(matches!(
            p.execute(Command::RenameScene {
                id: SceneId::new(),
                name: "x".into()
            }),
            Err(ProjectError::Command(CommandError::UnknownScene(_)))
        ));
        assert!(matches!(
            p.execute(Command::InsertScene {
                index: 5,
                scene: scene("z", vec![])
            }),
            Err(ProjectError::Command(CommandError::IndexOutOfBounds { .. }))
        ));
    }

    #[test]
    fn removing_a_scene_targeted_from_another_is_refused() {
        let label = node(NodeKind::Label { name: "l".into() });
        let jump = node(NodeKind::Jump { target: label.id });
        let a = scene("a", vec![jump]);
        let b = scene("b", vec![label]);
        let b_id = b.id;
        let mut story = Story::new("t");
        story.scenes = vec![a, b];
        let mut p = Project::new(story);
        let err = p.execute(Command::RemoveScene { id: b_id }).unwrap_err();
        assert!(matches!(
            err,
            ProjectError::Command(CommandError::StillReferenced(..))
        ));
        assert_eq!(p.story().scenes.len(), 2);
    }

    #[test]
    fn nodes_move_between_and_within_scenes() {
        let (n1, n2, n3) = (
            node(NodeKind::End),
            node(NodeKind::Return),
            node(NodeKind::End),
        );
        let ids = [n1.id, n2.id, n3.id];
        let (a, b) = (scene("a", vec![n1, n2]), scene("b", vec![n3]));
        let (a_id, b_id) = (a.id, b.id);
        let mut story = Story::new("t");
        story.scenes = vec![a, b];
        let mut p = Project::new(story);
        let before = p.story().clone();
        let v = p.node_version(ids[0]).unwrap();
        p.execute(Command::MoveNode {
            id: ids[0],
            scene: b_id,
            index: 1,
            expected_version: Some(v),
        })
        .unwrap();
        assert_eq!(p.story().scenes[0].nodes.len(), 1);
        assert_eq!(p.story().scenes[1].nodes[1].id, ids[0]);
        // Same scene: the index counts the scene without the moved node.
        p.execute(Command::MoveNode {
            id: ids[1],
            scene: a_id,
            index: 0,
            expected_version: None,
        })
        .unwrap();
        assert!(
            p.execute(Command::MoveNode {
                id: ids[1],
                scene: a_id,
                index: 2,
                expected_version: None,
            })
            .is_err()
        );
        // A stale version is a conflict.
        assert!(matches!(
            p.execute(Command::MoveNode {
                id: ids[0],
                scene: a_id,
                index: 0,
                expected_version: Some(v),
            }),
            Err(ProjectError::Command(CommandError::VersionConflict { .. }))
        ));
        p.undo().unwrap();
        p.undo().unwrap();
        assert_eq!(p.story(), &before);
    }

    #[test]
    fn characters_variables_assets_and_node_kinds_round_trip() {
        let mut p = Project::new(Story::new("t"));
        let name_id = StringId::new();
        p.execute(Command::SetString {
            id: name_id,
            text: Some("Al".into()),
        })
        .unwrap();
        let id = orbit_core::CharacterId::new();
        let ch = Character {
            default_scale: 1.0,
            id,
            name_id,
            color: Color::WHITE,
            expressions: vec![],
            default_position: None,
        };
        p.execute(Command::SetCharacter {
            id,
            character: Some(ch.clone()),
            index: None,
        })
        .unwrap();
        let mut recolored = ch.clone();
        recolored.color = Color::BLACK;
        p.execute(Command::SetCharacter {
            id,
            character: Some(recolored),
            index: None,
        })
        .unwrap();
        assert_eq!(p.story().characters[0].color, Color::BLACK);
        p.undo().unwrap();
        assert_eq!(p.story().characters[0], ch);
        p.undo().unwrap();
        assert!(p.story().characters.is_empty());
        p.redo().unwrap();
        // A character used by a node cannot be removed.
        let s = scene(
            "s",
            vec![node(NodeKind::Hide {
                character: id,
                transition: None,
            })],
        );
        p.execute(Command::InsertScene { index: 0, scene: s })
            .unwrap();
        assert!(matches!(
            p.execute(Command::SetCharacter {
                id,
                character: None,
                index: None
            }),
            Err(ProjectError::Command(CommandError::CharacterInUse(..)))
        ));

        let decl = orbit_core::VarDecl::game("x", Value::Int(1));
        p.execute(Command::SetVariable {
            name: "x".into(),
            decl: Some(decl),
            index: None,
        })
        .unwrap();
        p.execute(Command::SetVariable {
            name: "x".into(),
            decl: None,
            index: None,
        })
        .unwrap();
        assert!(p.story().variables.is_empty());
        p.undo().unwrap();
        assert_eq!(p.story().variables.len(), 1);
        assert!(matches!(
            p.execute(Command::SetVariable {
                name: "nope".into(),
                decl: None,
                index: None
            }),
            Err(ProjectError::Command(CommandError::UnknownVariable(_)))
        ));

        let asset = orbit_core::AssetEntry {
            id: orbit_core::AssetId::new(),
            kind: AssetKind::Image,
            path: "assets/a.png".into(),
        };
        p.execute(Command::SetAsset {
            id: asset.id,
            entry: Some(asset.clone()),
            index: None,
        })
        .unwrap();
        let bg = node(NodeKind::Background {
            asset: asset.id,
            transition: None,
        });
        let bg_id = bg.id;
        let scene_id = p.story().scenes[0].id;
        p.execute(Command::InsertNode {
            scene: scene_id,
            index: 0,
            node: bg,
        })
        .unwrap();
        assert!(matches!(
            p.execute(Command::SetAsset {
                id: asset.id,
                entry: None,
                index: None
            }),
            Err(ProjectError::Command(CommandError::AssetInUse(..)))
        ));
        // SetNodeKind replaces the content and keeps the id.
        p.execute(Command::SetNodeKind {
            id: bg_id,
            kind: NodeKind::End,
            expected_version: None,
        })
        .unwrap();
        assert_eq!(p.story().node(bg_id).unwrap().kind, NodeKind::End);
        p.undo().unwrap();
        assert!(matches!(
            p.story().node(bg_id).unwrap().kind,
            NodeKind::Background { .. }
        ));
    }

    #[test]
    fn group_merges_undo_entries_and_rolls_back_on_error() {
        let mut p = Project::new(Story::new("t"));
        let add = |name: &str| Command::InsertScene {
            index: 0,
            scene: scene(name, vec![]),
        };
        p.execute(add("first")).unwrap();
        let ok: Result<(), ProjectError> = p.group("two scenes", |p| {
            p.execute(add("a"))?;
            p.execute(add("b"))
        });
        ok.unwrap();
        assert_eq!(p.story().scenes.len(), 3);
        assert_eq!(p.undo_label(), Some("two scenes"));
        p.undo().unwrap();
        assert_eq!(p.story().scenes.len(), 1);
        p.redo().unwrap();
        assert_eq!(p.story().scenes.len(), 3);
        p.undo().unwrap();

        // A failure rolls everything back.
        let failed: Result<(), ProjectError> = p.group("boom", |p| {
            p.execute(add("c"))?;
            p.execute(Command::RemoveScene { id: SceneId::new() })
        });
        assert!(failed.is_err());
        assert_eq!(p.story().scenes.len(), 1);
        assert_eq!(p.undo_label(), Some("Créer une scène"));
        assert!(!p.can_redo());

        // Nothing applied: the redo stack survives.
        let mut p2 = Project::new(Story::new("t"));
        p2.execute(add("x")).unwrap();
        p2.undo().unwrap();
        let _: Result<(), ProjectError> = p2.group("noop", |_| Ok(()));
        assert!(p2.can_redo());
        let _: Result<(), ProjectError> = p2.group("fail", |_| {
            Err(ProjectError::Command(CommandError::UnknownScene(
                SceneId::new(),
            )))
        });
        assert!(p2.can_redo());
    }

    #[test]
    fn animation_frames_count_as_asset_users() {
        use crate::AssetUser;
        use orbit_core::{AnimFrames, Animation, AssetId, Expression, LoopMode, Motion};
        let mut story = Story::new("t");
        let (poster, f1, f2) = (AssetId::new(), AssetId::new(), AssetId::new());
        let id = orbit_core::CharacterId::new();
        story.characters.push(Character {
            default_scale: 1.0,
            id,
            name_id: StringId::new(),
            color: Color::WHITE,
            expressions: vec![Expression {
                name: "wave".into(),
                image: poster,
                motion: Some(Motion::Animation(Animation {
                    frames: AnimFrames::Sequence(vec![poster, f1, f2]),
                    fps: 8.0,
                    loop_mode: LoopMode::Once,
                })),
            }],
            default_position: None,
        });
        // Export keeps an asset only when it has a user: every frame must.
        for a in [poster, f1, f2] {
            assert_eq!(super::asset_users(&story, a), vec![AssetUser::Character(id)]);
        }
        assert!(super::asset_users(&story, AssetId::new()).is_empty());
        let e = &story.characters[0].expressions[0];
        assert_eq!(e.assets(), vec![poster, f1, f2]);
    }
}
