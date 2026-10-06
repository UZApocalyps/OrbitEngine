//! Achievements of a project (M7-05): the editor, the CLI and the MCP server
//! all go through these functions (schema v16, `Story::achievements`).

use orbit_core::{Achievement, AchievementId, AssetId, Node, NodeId, NodeKind, SceneId, StringId};

use crate::ops::reject;
use crate::{Command, Project, ProjectError};

/// Defines an achievement: its title and description are new strings of the
/// source table. Returns its id.
pub fn add_achievement(
    project: &mut Project,
    title: &str,
    description: &str,
    hidden: bool,
    icon: Option<AssetId>,
) -> Result<AchievementId, ProjectError> {
    if title.trim().is_empty() {
        return Err(reject(
            "achievement_no_title",
            "Un succès a besoin d'un titre.",
        ));
    }
    check_icon(project, icon)?;
    let id = AchievementId::new();
    let (title_id, description_id) = (StringId::new(), StringId::new());
    let index = project.story().achievements.len();
    project.transaction_labeled("Ajouter un succès", |tx| {
        tx.execute(Command::SetString {
            id: title_id,
            text: Some(title.to_owned()),
        })?;
        tx.execute(Command::SetString {
            id: description_id,
            text: Some(description.to_owned()),
        })?;
        tx.execute(Command::SetAchievement {
            id,
            achievement: Some(Achievement {
                id,
                title_id,
                description_id,
                hidden,
                icon,
                locked_icon: None,
            }),
            index: Some(index),
        })
    })?;
    Ok(id)
}

fn check_icon(project: &Project, icon: Option<AssetId>) -> Result<(), ProjectError> {
    let Some(icon) = icon else { return Ok(()) };
    match project.story().assets.get(icon) {
        Some(e) if e.kind == orbit_core::AssetKind::Image => Ok(()),
        Some(_) => Err(reject(
            "achievement_icon_kind",
            "L'icône d'un succès doit être une image.",
        )),
        None => Err(reject(
            "achievement_icon_unknown",
            "L'icône du succès n'est pas une ressource déclarée.",
        )),
    }
}

/// Edits an achievement: `None` keeps a field. `icon` is `Some(None)` to
/// remove the icon.
pub fn update_achievement(
    project: &mut Project,
    id: AchievementId,
    title: Option<&str>,
    description: Option<&str>,
    hidden: Option<bool>,
    icon: Option<Option<AssetId>>,
) -> Result<(), ProjectError> {
    let current = project
        .story()
        .achievements
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .ok_or(crate::CommandError::UnknownAchievement(id))?;
    if title.is_some_and(|t| t.trim().is_empty()) {
        return Err(reject(
            "achievement_no_title",
            "Un succès a besoin d'un titre.",
        ));
    }
    if let Some(icon) = icon {
        check_icon(project, icon)?;
    }
    let (title_id, description_id) = (current.title_id, current.description_id);
    let index = project.story().achievements.iter().position(|a| a.id == id);
    project.transaction_labeled("Modifier un succès", |tx| {
        if let Some(t) = title {
            tx.execute(Command::SetString {
                id: title_id,
                text: Some(t.to_owned()),
            })?;
        }
        if let Some(d) = description {
            tx.execute(Command::SetString {
                id: description_id,
                text: Some(d.to_owned()),
            })?;
        }
        tx.execute(Command::SetAchievement {
            id,
            achievement: Some(Achievement {
                hidden: hidden.unwrap_or(current.hidden),
                icon: icon.unwrap_or(current.icon),
                ..current
            }),
            index,
        })
    })
}

/// Sets (`Some`) or removes (`None`) the icon shown while achievement
/// `id` is locked (`Achievement::locked_icon`; without one the icon is used).
pub fn set_achievement_locked_icon(
    project: &mut Project,
    id: AchievementId,
    locked_icon: Option<AssetId>,
) -> Result<(), ProjectError> {
    let current = project
        .story()
        .achievements
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .ok_or(crate::CommandError::UnknownAchievement(id))?;
    check_icon(project, locked_icon)?;
    let index = project.story().achievements.iter().position(|a| a.id == id);
    project.transaction_labeled("Modifier un succès", |tx| {
        tx.execute(Command::SetAchievement {
            id,
            achievement: Some(Achievement {
                locked_icon,
                ..current
            }),
            index,
        })
    })
}

/// Deletes an achievement and its two strings. Refused while an `Unlock`
/// node names it.
pub fn delete_achievement(project: &mut Project, id: AchievementId) -> Result<(), ProjectError> {
    let Some(a) = project.story().achievements.iter().find(|a| a.id == id).cloned() else {
        return Err(crate::CommandError::UnknownAchievement(id).into());
    };
    project.transaction_labeled("Supprimer un succès", |tx| {
        tx.execute(Command::SetAchievement {
            id,
            achievement: None,
            index: None,
        })?;
        for string in [a.title_id, a.description_id] {
            tx.execute(Command::SetString {
                id: string,
                text: None,
            })?;
        }
        Ok(())
    })
}

/// Nodes that unlock `id`.
pub fn unlock_nodes(story: &orbit_core::Story, id: AchievementId) -> Vec<NodeId> {
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter(|n| matches!(&n.kind, NodeKind::Unlock { achievement } if *achievement == id))
        .map(|n| n.id)
        .collect()
}

/// Inserts an `Unlock` node at `index` of `scene`. Returns its id.
pub fn insert_unlock(
    project: &mut Project,
    scene: SceneId,
    index: usize,
    achievement: AchievementId,
) -> Result<NodeId, ProjectError> {
    if !project.story().achievements.iter().any(|a| a.id == achievement) {
        return Err(crate::CommandError::UnknownAchievement(achievement).into());
    }
    let id = NodeId::new();
    project.execute(Command::InsertNode {
        scene,
        index,
        node: Node {
            id,
            kind: NodeKind::Unlock { achievement },
        },
    })?;
    Ok(id)
}

