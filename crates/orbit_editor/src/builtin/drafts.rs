//! New nodes with sensible defaults, by type key: what "insert a line of
//! this type" creates. Pure functions over the story, so they are tested
//! without a window.

use orbit_core::{
    AssetKind, CharacterId, ChoiceOption, Expr, NodeKind, Position, SceneId, Story, StringId,
    Value,
};
use orbit_editor_api::{EditorRegistry, icons};
use orbit_project::ops::{NodeDraft, unique_name};

/// Why a draft could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftError {
    /// The project has no asset of this kind yet (the user must add one).
    NeedAsset(AssetKind),
    /// A message for the user.
    Message(String),
}

/// An entry of the "insert" menus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insertable {
    /// `Say`, `Choice`... or `plugin_id.kind`.
    pub key: String,
    pub label: String,
    pub icon: &'static str,
    pub group: &'static str,
}

fn item(key: &str, label: &str, icon: &'static str, group: &'static str) -> Insertable {
    Insertable {
        key: key.to_owned(),
        label: label.to_owned(),
        icon,
        group,
    }
}

/// Built-in node types, grouped for the menus.
pub fn builtin_insertables() -> Vec<Insertable> {
    vec![
        item("Say", "Réplique", icons::CHAT_TEXT, "Dialogue"),
        item("Choice", "Choix du joueur", icons::GIT_BRANCH, "Dialogue"),
        item("Background", "Décor", icons::IMAGE, "Scène"),
        item("Show", "Afficher un personnage", icons::USER_PLUS, "Scène"),
        item("Hide", "Masquer un personnage", icons::USER_MINUS, "Scène"),
        item("PlayMusic", "Jouer une musique", icons::MUSIC_NOTE, "Son"),
        item("StopMusic", "Arrêter la musique", icons::STOP, "Son"),
        item("PlaySound", "Jouer un son", icons::SPEAKER_HIGH, "Son"),
        item("Wait", "Attendre", icons::TIMER, "Scène"),
        item("SetVar", "Modifier une variable", icons::FUNCTION, "Logique"),
        item("If", "Condition", icons::GIT_FORK, "Logique"),
        item("Label", "Étiquette", icons::FLAG, "Navigation"),
        item("Jump", "Saut vers une étiquette", icons::ARROW_BEND_UP_RIGHT, "Navigation"),
        item("Call", "Appeler une routine", icons::PHONE_CALL, "Navigation"),
        item("Return", "Retour de routine", icons::ARROW_U_UP_LEFT, "Navigation"),
        item("End", "Fin de l'histoire", icons::FLAG_CHECKERED, "Navigation"),
    ]
}

/// Built-in types plus the ones registered by plugins.
pub fn all_insertables(registry: &EditorRegistry) -> Vec<Insertable> {
    let mut items = builtin_insertables();
    for t in registry.project.node_types() {
        items.push(Insertable {
            key: format!("{}.{}", t.plugin_id, t.def.kind),
            label: t.def.label.clone(),
            icon: icons::PUZZLE_PIECE,
            group: "Plugins",
        });
    }
    items
}

/// Speaker of the closest `Say` before `index` in the scene.
fn previous_speaker(story: &Story, scene: SceneId, index: usize) -> Option<CharacterId> {
    story.scene(scene)?.nodes[..index.min(story.scene(scene)?.nodes.len())]
        .iter()
        .rev()
        .find_map(|n| match &n.kind {
            NodeKind::Say { speaker, .. } => Some(*speaker),
            _ => None,
        })
        .flatten()
}

fn label_names(story: &Story) -> Vec<String> {
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| match &n.kind {
            NodeKind::Label { name } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn first_asset(story: &Story, kind: AssetKind) -> Result<orbit_core::AssetId, DraftError> {
    story
        .assets
        .assets
        .iter()
        .find(|a| a.kind == kind)
        .map(|a| a.id)
        .ok_or(DraftError::NeedAsset(kind))
}

fn stage_character(
    story: &Story,
    scene: SceneId,
    index: usize,
) -> Result<CharacterId, DraftError> {
    previous_speaker(story, scene, index)
        .filter(|c| story.character(*c).is_some())
        .or_else(|| story.characters.first().map(|c| c.id))
        .ok_or_else(|| {
            DraftError::Message(
                "Créez d'abord un personnage (panneau Personnages) pour l'afficher.".to_owned(),
            )
        })
}

/// The drafts inserted by "insert a node of type `key` at `index` of
/// `scene`". The first draft is the node the user asked for; some types come
/// with helpers (labels for a choice, a condition or a jump).
pub fn drafts_for(
    story: &Story,
    registry: &EditorRegistry,
    key: &str,
    scene: SceneId,
    index: usize,
) -> Result<Vec<NodeDraft>, DraftError> {
    let mut taken = label_names(story);
    let mut fresh_label = |base: &str| -> NodeDraft {
        let name = unique_name(base, taken.iter().map(String::as_str));
        taken.push(name.clone());
        NodeDraft::label(name)
    };
    Ok(match key {
        "Say" => vec![NodeDraft::say(previous_speaker(story, scene, index), "")],
        "Choice" => {
            // The whole skeleton: two branches that join again.
            let end = fresh_label("suite");
            let (a, b) = (fresh_label("choix_a"), fresh_label("choix_b"));
            let speaker = previous_speaker(story, scene, index);
            let (text_a, text_b) = (StringId::new(), StringId::new());
            let choice = NodeDraft {
                node: orbit_core::Node {
                    id: orbit_core::NodeId::new(),
                    kind: NodeKind::Choice {
                        options: vec![
                            ChoiceOption {
                                text_id: text_a,
                                target: a.node.id,
                                condition: None,
                            },
                            ChoiceOption {
                                text_id: text_b,
                                target: b.node.id,
                                condition: None,
                            },
                        ],
                    },
                },
                strings: vec![(text_a, "Option A".into()), (text_b, "Option B".into())],
            };
            let join = |target| NodeDraft::plain(NodeKind::Jump { target });
            let (jump_a, jump_b) = (join(end.node.id), join(end.node.id));
            vec![
                choice,
                a,
                NodeDraft::say(speaker, ""),
                jump_a,
                b,
                NodeDraft::say(speaker, ""),
                jump_b,
                end,
            ]
        }
        "Background" => vec![NodeDraft::plain(NodeKind::Background {
            asset: first_asset(story, AssetKind::Image)?,
            transition: None,
        })],
        "Show" => {
            let character = stage_character(story, scene, index)?;
            let def = story.character(character);
            let expression = def
                .and_then(|c| c.expressions.first())
                .map(|e| e.name.clone())
                .unwrap_or_default();
            let position = def
                .and_then(|c| c.default_position.clone())
                .unwrap_or(Position::Center);
            vec![NodeDraft::plain(NodeKind::Show {
                character,
                expression,
                position,
                transition: None,
            })]
        }
        "Hide" => vec![NodeDraft::plain(NodeKind::Hide {
            character: stage_character(story, scene, index)?,
            transition: None,
        })],
        "PlayMusic" => vec![NodeDraft::plain(NodeKind::PlayMusic {
            asset: first_asset(story, AssetKind::Music)?,
            fade_in: 0.0,
            looping: true,
        })],
        "StopMusic" => vec![NodeDraft::plain(NodeKind::StopMusic { fade_out: 1.0 })],
        "PlaySound" => vec![NodeDraft::plain(NodeKind::PlaySound {
            asset: first_asset(story, AssetKind::Sound)?,
        })],
        "Wait" => vec![NodeDraft::plain(NodeKind::Wait { seconds: 1.0 })],
        "SetVar" => {
            let (var, value) = match story.variables.first() {
                Some(v) => (v.name.clone(), v.initial.clone()),
                None => ("variable".to_owned(), Value::Int(0)),
            };
            vec![NodeDraft::plain(NodeKind::SetVar {
                var,
                value: Expr::Lit(value),
            })]
        }
        "If" => {
            let then = fresh_label("alors");
            vec![
                NodeDraft::plain(NodeKind::If {
                    condition: Expr::lit(true),
                    then_target: then.node.id,
                    else_target: None,
                }),
                then,
            ]
        }
        "Label" => vec![fresh_label("etiquette")],
        "Jump" | "Call" => {
            let nearest = story.scene(scene).and_then(|s| {
                s.nodes[..index.min(s.nodes.len())]
                    .iter()
                    .rev()
                    .find(|n| matches!(n.kind, NodeKind::Label { .. }))
                    .map(|n| n.id)
            });
            let existing = nearest.or_else(|| {
                story
                    .scenes
                    .iter()
                    .flat_map(|s| &s.nodes)
                    .find(|n| matches!(n.kind, NodeKind::Label { .. }))
                    .map(|n| n.id)
            });
            let (target, helper) = match existing {
                Some(t) => (t, None),
                None => {
                    let l = fresh_label("suite");
                    (l.node.id, Some(l))
                }
            };
            let kind = if key == "Jump" {
                NodeKind::Jump { target }
            } else {
                NodeKind::Call { label: target }
            };
            let mut v = vec![NodeDraft::plain(kind)];
            v.extend(helper);
            v
        }
        "Return" => vec![NodeDraft::plain(NodeKind::Return)],
        "End" => vec![NodeDraft::plain(NodeKind::End)],
        other => {
            let t = registry
                .project
                .node_types()
                .iter()
                .find(|t| format!("{}.{}", t.plugin_id, t.def.kind) == other)
                .ok_or_else(|| DraftError::Message(format!("Type de nœud inconnu : {other}")))?;
            vec![NodeDraft::plain(NodeKind::Custom {
                plugin_id: t.plugin_id.clone(),
                kind: t.def.kind.clone(),
                version: 1,
                data: (t.def.default_data)(),
            })]
        }
    })
}
