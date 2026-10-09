//! Merged story → `orbit_project` commands.
//!
//! The text never replaces the IR: [`diff`] computes the commands that turn
//! the base story into the parsed one, and [`apply`] runs them as a single
//! undoable transaction, so the editor's undo stack, events and per-node
//! versions all see a text edit like any other edit.

use std::collections::HashSet;

use orbit_core::{NodeId, NodeKind, SceneId, Story};
use orbit_project::{Command, Project, ProjectError};

use crate::parser::{Diagnostic, Scope, parse};

/// Why [`apply`] did nothing.
#[derive(Debug)]
pub enum ApplyError {
    /// The text does not parse or resolve.
    Syntax(Vec<Diagnostic>),
    /// The parsed story cannot be reached with commands (e.g. a deleted
    /// label is still targeted from elsewhere).
    Command(ProjectError),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::Syntax(d) => {
                for (i, d) in d.iter().enumerate() {
                    if i > 0 {
                        f.write_str("\n")?;
                    }
                    write!(f, "{d}")?;
                }
                Ok(())
            }
            ApplyError::Command(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// Commands turning `base` into `target`, where `target` only differs from
/// `base` in its scenes and string table (what [`parse`] produces).
/// Applying them in order to a project holding `base` yields `target`.
pub fn diff(base: &Story, target: &Story) -> Result<Vec<Command>, ProjectError> {
    let mut scratch = Project::new(base.clone());
    let mut out = Vec::new();
    let mut run = |p: &mut Project, c: Command| -> Result<(), ProjectError> {
        p.execute(c.clone())?;
        out.push(c);
        Ok(())
    };

    // 1. New and changed strings (nothing refuses them).
    for (id, text) in &target.strings.entries {
        if base.strings.get(*id) != Some(text.as_str()) {
            run(
                &mut scratch,
                Command::SetString {
                    id: *id,
                    text: Some(text.clone()),
                },
            )?;
        }
    }

    // 2. New scenes (empty, appended), renames.
    for scene in &target.scenes {
        match base.scene(scene.id) {
            None => {
                let index = scratch.story().scenes.len();
                run(
                    &mut scratch,
                    Command::InsertScene {
                        index,
                        scene: orbit_core::Scene {
                            id: scene.id,
                            name: scene.name.clone(),
                            nodes: Vec::new(),
                        },
                    },
                )?;
            }
            Some(old) if old.name != scene.name => run(
                &mut scratch,
                Command::RenameScene {
                    id: scene.id,
                    name: scene.name.clone(),
                },
            )?,
            Some(_) => {}
        }
    }

    // 3. New nodes, appended to their scene with their final content. A new
    //    node targeting another new node is first inserted as `End`, then
    //    given its kind once every new node exists: undoing then clears the
    //    reference before removing its target (a menu before its labels).
    let mut deferred = Vec::new();
    for scene in &target.scenes {
        for node in &scene.nodes {
            if base.node(node.id).is_none() {
                let index = scratch.story().scene(scene.id).map_or(0, |s| s.nodes.len());
                let forward = node
                    .kind
                    .targets()
                    .iter()
                    .any(|t| base.node(*t).is_none());
                let mut inserted = node.clone();
                if forward {
                    inserted.kind = NodeKind::End;
                    deferred.push(Command::SetNodeKind {
                        id: node.id,
                        kind: node.kind.clone(),
                        expected_version: None,
                    });
                }
                run(
                    &mut scratch,
                    Command::InsertNode {
                        scene: scene.id,
                        index,
                        node: inserted,
                    },
                )?;
            }
        }
    }
    for cmd in deferred {
        run(&mut scratch, cmd)?;
    }

    // 4. Changed nodes. A label turning into something else must wait until
    //    nothing targets it any more.
    let mut late = Vec::new();
    for node in target.scenes.iter().flat_map(|s| &s.nodes) {
        let Some(old) = base.node(node.id) else {
            continue;
        };
        if old.kind == node.kind {
            continue;
        }
        let cmd = Command::SetNodeKind {
            id: node.id,
            kind: node.kind.clone(),
            expected_version: None,
        };
        if matches!(old.kind, NodeKind::Label { .. })
            && !matches!(node.kind, NodeKind::Label { .. })
        {
            late.push(cmd);
        } else {
            run(&mut scratch, cmd)?;
        }
    }

    // 5. Deleted nodes: first make them target nothing (they may target each
    //    other), then remove them.
    let kept: HashSet<NodeId> = target
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .map(|n| n.id)
        .collect();
    let deleted: Vec<NodeId> = base
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .map(|n| n.id)
        .filter(|id| !kept.contains(id))
        .collect();
    for id in &deleted {
        let kind = &scratch.story().node(*id).expect("not removed yet").kind;
        if !kind.targets().is_empty() || kind.target_scene().is_some() {
            run(
                &mut scratch,
                Command::SetNodeKind {
                    id: *id,
                    kind: NodeKind::End,
                    expected_version: None,
                },
            )?;
        }
    }
    for cmd in late {
        run(&mut scratch, cmd)?;
    }
    for id in deleted {
        run(
            &mut scratch,
            Command::RemoveNode {
                id,
                expected_version: None,
            },
        )?;
    }

    // 6. Node order (including moves between scenes).
    for scene in &target.scenes {
        for (index, node) in scene.nodes.iter().enumerate() {
            let (si, ni) = scratch.story().locate(node.id).expect("placed above");
            if scratch.story().scenes[si].id != scene.id || ni != index {
                run(
                    &mut scratch,
                    Command::MoveNode {
                        id: node.id,
                        scene: scene.id,
                        index,
                        expected_version: None,
                    },
                )?;
            }
        }
    }

    // 7. Deleted scenes (now empty of anything kept), then scene order.
    let kept_scenes: HashSet<SceneId> = target.scenes.iter().map(|s| s.id).collect();
    for scene in &base.scenes {
        if !kept_scenes.contains(&scene.id) {
            run(&mut scratch, Command::RemoveScene { id: scene.id })?;
        }
    }
    for (index, scene) in target.scenes.iter().enumerate() {
        let at = scratch.story().scenes.iter().position(|s| s.id == scene.id);
        if at != Some(index) {
            run(
                &mut scratch,
                Command::MoveScene {
                    id: scene.id,
                    index,
                },
            )?;
        }
    }

    // 8. Strings no line uses any more.
    for id in base.strings.entries.keys() {
        if !target.strings.entries.contains_key(id) {
            run(
                &mut scratch,
                Command::SetString {
                    id: *id,
                    text: None,
                },
            )?;
        }
    }
    Ok(out)
}

/// Parses `text` against the project's story and applies the result as one
/// undoable transaction labeled `"Text edit"`. Returns the number of commands
/// run (0 when the text changes nothing). On error the project is untouched.
pub fn apply(project: &mut Project, text: &str, scope: Scope) -> Result<usize, ApplyError> {
    let target = parse(text, project.story(), scope).map_err(ApplyError::Syntax)?;
    let commands = diff(project.story(), &target).map_err(ApplyError::Command)?;
    let n = commands.len();
    project
        .transaction_labeled("Text edit", |tx| {
            for c in commands {
                tx.execute(c)?;
            }
            Ok(())
        })
        .map_err(ApplyError::Command)?;
    Ok(n)
}
