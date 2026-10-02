//! A large story for the graph panel (`orbit-editor --generate-graph <n>
//! <file.ron>`) and tests: many scenes linked by choices, conditions, scene
//! jumps and calls, valid by construction.

use orbit_core::{
    BinOp, ChoiceOption, Expr, Node, NodeId, NodeKind, Scene, SceneId, Story, Value, VarDecl,
};

/// A story of about `blocks` graph blocks: scenes of one or two blocks each,
/// linked by choices (2 or 3 options), conditions, jumps, calls and endings,
/// with a few links going back. Deterministic.
pub fn generate_graph_story(blocks: usize) -> Story {
    let blocks = blocks.max(6);
    let scenes = (blocks * 4 / 5).max(5);
    let mut story = Story::new(format!("Graphe de {blocks} blocs"));
    story
        .variables
        .push(VarDecl::game("affinity", Value::Int(0)));

    // Ids first, so that any scene can target any other.
    let labels: Vec<NodeId> = (0..scenes).map(|_| NodeId::new()).collect();
    let scene_ids: Vec<SceneId> = (0..scenes).map(|_| SceneId::new()).collect();
    let routine = NodeId::new();
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rnd = |m: usize| -> usize {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % m as u64) as usize
    };
    // A scene a little later in the list (or the start when there is none).
    let ahead = |from: usize, step: usize, jitter: usize| -> NodeId {
        if from + 1 >= scenes {
            labels[0]
        } else {
            labels[(from + 1 + step + jitter).min(scenes - 1)]
        }
    };
    for i in 0..scenes {
        let mut nodes = vec![Node {
            id: labels[i],
            kind: NodeKind::Label {
                name: format!("s{i}"),
            },
        }];
        for k in 0..=i % 3 {
            let text_id = story.strings.add(format!(
                "Scène {i}, réplique {k} : il se passe quelque chose."
            ));
            nodes.push(Node {
                id: NodeId::new(),
                kind: NodeKind::Say {
                    speaker: None,
                    text_id,
                },
            });
        }
        let ending = if i + 1 >= scenes { 4 } else { i % 5 };
        match ending {
            0 | 3 => {
                let count = if ending == 0 { 2 } else { 3 };
                let mut options = Vec::new();
                for k in 0..count {
                    let target = if k == count - 1 && i % 7 == 3 {
                        labels[rnd(i + 1)] // a link going back
                    } else {
                        ahead(i, k, if k == 0 { 0 } else { rnd(2) })
                    };
                    options.push(ChoiceOption {
                        text_id: story
                            .strings
                            .add(format!("Option {} de la scène {i}", k + 1)),
                        target,
                        condition: (k == 2).then(|| {
                            Expr::binary(BinOp::Gt, Expr::var("affinity"), Expr::lit(1i64))
                        }),
                    });
                }
                nodes.push(Node {
                    id: NodeId::new(),
                    kind: NodeKind::Choice { options },
                });
            }
            1 => {
                nodes.push(Node {
                    id: NodeId::new(),
                    kind: NodeKind::If {
                        condition: Expr::binary(BinOp::Gt, Expr::var("affinity"), Expr::lit(0i64)),
                        then_target: ahead(i, 0, 0),
                        else_target: Some(ahead(i, 1, rnd(2))),
                    },
                });
                nodes.push(Node {
                    id: NodeId::new(),
                    kind: NodeKind::End,
                });
            }
            2 => {
                nodes.push(Node {
                    id: NodeId::new(),
                    kind: NodeKind::Call { label: routine },
                });
                nodes.push(Node {
                    id: NodeId::new(),
                    kind: NodeKind::JumpScene {
                        scene: scene_ids[(i + 1).min(scenes - 1)],
                        label: None,
                    },
                });
            }
            _ => nodes.push(Node {
                id: NodeId::new(),
                kind: NodeKind::End,
            }),
        }
        // A second block in every fourth scene that ends the story: an inner
        // label reached by a jump, just before the end.
        if i % 4 == 1 && matches!(nodes.last().map(|n| &n.kind), Some(NodeKind::End)) {
            let inner = NodeId::new();
            let at = nodes.len() - 1;
            nodes.insert(
                at,
                Node {
                    id: NodeId::new(),
                    kind: NodeKind::Jump { target: inner },
                },
            );
            nodes.insert(
                at + 1,
                Node {
                    id: inner,
                    kind: NodeKind::Label {
                        name: format!("inner_{i}"),
                    },
                },
            );
        }
        story.scenes.push(Scene {
            id: scene_ids[i],
            name: format!("Scène {i}"),
            nodes,
        });
    }
    // The routine called by some scenes.
    let text_id = story.strings.add("Un petit moment partagé.");
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "Routine".into(),
        nodes: vec![
            Node {
                id: routine,
                kind: NodeKind::Label {
                    name: "routine".into(),
                },
            },
            Node {
                id: NodeId::new(),
                kind: NodeKind::Say {
                    speaker: None,
                    text_id,
                },
            },
            Node {
                id: NodeId::new(),
                kind: NodeKind::Return,
            },
        ],
    });
    story
}

#[cfg(test)]
mod tests {
    use orbit_project::graph::GraphModel;

    use super::*;

    #[test]
    fn generated_graph_is_valid_and_sized() {
        for blocks in [6, 40, 200] {
            let story = generate_graph_story(blocks);
            let report = orbit_core::validate(&story);
            assert!(report.errors.is_empty(), "{:?}", report.errors);
            let model = GraphModel::build(&story);
            assert!(
                model.blocks.len() >= blocks * 3 / 4,
                "{} blocks for {blocks}",
                model.blocks.len()
            );
            assert!(!model.edges().is_empty());
        }
    }
}
