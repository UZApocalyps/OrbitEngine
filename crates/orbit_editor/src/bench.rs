//! A large story generator, for performance work (`orbit-editor
//! --generate-story <n> <file.ron>`) and tests: one scene of `nodes` nodes
//! mixing dialogue, stage commands, variables, choices with branches and
//! jumps, valid by construction.

use orbit_core::{
    AssetEntry, AssetId, AssetKind, BinOp, Character, CharacterId, ChoiceOption, Color, Expr,
    Expression, Node, NodeId, NodeKind, Position, Scene, SceneId, Story, StringId, Value, VarDecl,
};

/// Builds a valid story of about `nodes` nodes (at least 20) in a single
/// scene named `Longue scène`, with four characters.
pub fn generate_story(nodes: usize) -> Story {
    let mut story = Story::new(format!("Histoire de {nodes} nœuds"));
    let image = |story: &mut Story, path: &str| {
        let id = AssetId::new();
        story.assets.assets.push(AssetEntry {
            id,
            kind: AssetKind::Image,
            path: path.to_owned(),
        });
        id
    };
    let bg = image(&mut story, "assets/demo/bg_room.png");
    let music = AssetId::new();
    story.assets.assets.push(AssetEntry {
        id: music,
        kind: AssetKind::Music,
        path: "assets/demo/music_theme.wav".into(),
    });
    let sprite = image(&mut story, "assets/demo/alice_neutral.png");
    let names = ["Alice", "Bob", "Clara", "David"];
    let colors = [
        Color::rgb(232, 120, 160),
        Color::rgb(90, 150, 230),
        Color::rgb(96, 190, 140),
        Color::rgb(236, 168, 70),
    ];
    let characters: Vec<CharacterId> = names
        .iter()
        .zip(colors)
        .map(|(name, color)| {
            let id = CharacterId::new();
            let name_id = story.strings.add(*name);
            story.characters.push(Character {
                default_scale: 1.0,
                id,
                name_id,
                color,
                expressions: vec![Expression {
                    name: "neutral".into(),
                    image: sprite,
                }],
                default_position: None,
            });
            id
        })
        .collect();
    story
        .variables
        .push(VarDecl::game("affinity", Value::Int(0)));

    let mut list: Vec<Node> = Vec::with_capacity(nodes + 16);
    let mut first = |kind: NodeKind| {
        list.push(Node {
            id: NodeId::new(),
            kind,
        });
    };
    first(NodeKind::Background {
        asset: bg,
        transition: None,
    });
    first(NodeKind::PlayMusic {
        asset: music,
        fade_in: 1.0,
        looping: true,
    });
    let mut block = 0usize;
    while list.len() < nodes {
        block += 1;
        let mut add = |kind: NodeKind| -> NodeId {
            let id = NodeId::new();
            list.push(Node { id, kind });
            id
        };
        let who = characters[block % characters.len()];
        add(NodeKind::Show {
            scale: None,
            scale_y: None,
            flip_x: false,
            flip_y: false,
            character: who,
            expression: "neutral".into(),
            position: if block.is_multiple_of(2) {
                Position::Left
            } else {
                Position::Right
            },
            transition: None,
        });
        for i in 0..8 {
            let text_id = story.strings.add(format!(
                "Bloc {block}, réplique {i} : un peu de texte pour que la ligne ressemble à une vraie réplique de visual novel."
            ));
            add(NodeKind::Say {
                speaker: (i % 5 != 4).then(|| characters[(block + i) % characters.len()]),
                text_id,
            });
        }
        add(NodeKind::SetVar {
            var: "affinity".into(),
            value: Expr::binary(BinOp::Add, Expr::var("affinity"), Expr::lit(1i64)),
        });
        // A choice with two branches that join again.
        let (a, b, end) = (NodeId::new(), NodeId::new(), NodeId::new());
        let (ta, tb) = (
            story.strings.add(format!("Option A du bloc {block}")),
            story.strings.add(format!("Option B du bloc {block}")),
        );
        add(NodeKind::Choice {
            options: vec![
                ChoiceOption {
                    text_id: ta,
                    target: a,
                    condition: None,
                },
                ChoiceOption {
                    text_id: tb,
                    target: b,
                    condition: Some(Expr::binary(
                        BinOp::Gt,
                        Expr::var("affinity"),
                        Expr::lit(2i64),
                    )),
                },
            ],
        });
        list.push(Node {
            id: a,
            kind: NodeKind::Label {
                name: format!("a_{block}"),
            },
        });
        let text: StringId = story.strings.add(format!("Branche A {block}"));
        list.push(Node {
            id: NodeId::new(),
            kind: NodeKind::Say {
                speaker: Some(who),
                text_id: text,
            },
        });
        list.push(Node {
            id: NodeId::new(),
            kind: NodeKind::Jump { target: end },
        });
        list.push(Node {
            id: b,
            kind: NodeKind::Label {
                name: format!("b_{block}"),
            },
        });
        let text = story.strings.add(format!("Branche B {block}"));
        list.push(Node {
            id: NodeId::new(),
            kind: NodeKind::Say {
                speaker: None,
                text_id: text,
            },
        });
        list.push(Node {
            id: end,
            kind: NodeKind::Label {
                name: format!("suite_{block}"),
            },
        });
        list.push(Node {
            id: NodeId::new(),
            kind: NodeKind::Hide {
                character: who,
                transition: None,
            },
        });
    }
    list.push(Node {
        id: NodeId::new(),
        kind: NodeKind::End,
    });
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "Longue scène".into(),
        nodes: list,
    });
    story
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_stories_are_valid_and_sized() {
        for n in [20, 500, 5000] {
            let story = generate_story(n);
            let count: usize = story.scenes.iter().map(|s| s.nodes.len()).sum();
            assert!(count >= n && count < n + 20, "{n} -> {count}");
            let report = orbit_core::validate(&story);
            assert!(report.errors.is_empty(), "{:?}", report.errors);
            // And it survives a RON round trip.
            let back = Story::from_ron(&story.to_ron().unwrap()).unwrap();
            assert_eq!(back.scenes[0].nodes.len(), count);
        }
    }
}
