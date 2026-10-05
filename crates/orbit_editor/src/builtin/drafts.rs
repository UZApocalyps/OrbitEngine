//! New nodes with sensible defaults, by type key: what "insert a line of
//! this type" creates. Pure functions over the story, so they are tested
//! without a window.

use orbit_core::{
    AssetKind, CharacterId, ChoiceOption, Expr, NodeKind, Position, SceneId, Story, StringId, Value,
};
use orbit_editor_api::{EditorRegistry, Translator, icons};
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
    /// A translation key (a plugin type: its own label, shown as is);
    /// see [`Insertable::label_text`].
    pub label: String,
    pub icon: &'static str,
    /// A translation key; see [`Insertable::group_text`].
    pub group: &'static str,
}

impl Insertable {
    /// The label in the language of `texts`.
    pub fn label_text(&self, texts: &Translator) -> String {
        texts.t(&self.label)
    }

    /// The group title in the language of `texts`.
    pub fn group_text(&self, texts: &Translator) -> String {
        texts.t(self.group)
    }
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
        item(
            "Say",
            "draft.label.say",
            icons::CHAT_TEXT,
            "draft.group.dialogue",
        ),
        item(
            "Choice",
            "draft.label.choice",
            icons::GIT_BRANCH,
            "draft.group.dialogue",
        ),
        item(
            "Background",
            "draft.label.background",
            icons::IMAGE,
            "draft.group.scene",
        ),
        item(
            "Show",
            "draft.label.show",
            icons::USER_PLUS,
            "draft.group.scene",
        ),
        item(
            "Hide",
            "draft.label.hide",
            icons::USER_MINUS,
            "draft.group.scene",
        ),
        item(
            "PlayMusic",
            "draft.label.play_music",
            icons::MUSIC_NOTE,
            "draft.group.sound",
        ),
        item(
            "StopMusic",
            "draft.label.stop_music",
            icons::STOP,
            "draft.group.sound",
        ),
        item(
            "PlaySound",
            "draft.label.play_sound",
            icons::SPEAKER_HIGH,
            "draft.group.sound",
        ),
        item(
            "Wait",
            "draft.label.wait",
            icons::TIMER,
            "draft.group.scene",
        ),
        item(
            "Effect",
            "draft.label.effect",
            icons::SPARKLE,
            "draft.group.scene",
        ),
        item(
            "Timeline",
            "draft.label.timeline",
            icons::FILM_STRIP,
            "draft.group.scene",
        ),
        item(
            "BackgroundLayers",
            "draft.label.background_layers",
            icons::STACK,
            "draft.group.scene",
        ),
        item(
            "Particles",
            "draft.label.particles",
            icons::CLOUD_RAIN,
            "draft.group.scene",
        ),
        item(
            "StopParticles",
            "draft.label.stop_particles",
            icons::STOP_CIRCLE,
            "draft.group.scene",
        ),
        item(
            "Script",
            "draft.label.script",
            icons::CODE,
            "draft.group.logic",
        ),
        item(
            "Unlock",
            "draft.label.unlock",
            icons::TROPHY,
            "draft.group.logic",
        ),
        item(
            "OpenScreen",
            "draft.label.open_screen",
            icons::BROWSER,
            "draft.group.scene",
        ),
        item(
            "CloseScreen",
            "draft.label.close_screen",
            icons::X_SQUARE,
            "draft.group.scene",
        ),
        item(
            "ShowScene3d",
            "draft.label.show_scene3d",
            icons::CUBE,
            "draft.group.scene",
        ),
        item(
            "Camera3d",
            "draft.label.camera3d",
            icons::VIDEO_CAMERA,
            "draft.group.scene",
        ),
        item(
            "HideScene3d",
            "draft.label.hide_scene3d",
            icons::EYE_SLASH,
            "draft.group.scene",
        ),
        item(
            "SetVar",
            "draft.label.set_var",
            icons::FUNCTION,
            "draft.group.logic",
        ),
        item("If", "draft.label.if", icons::GIT_FORK, "draft.group.logic"),
        item(
            "Label",
            "draft.label.label",
            icons::FLAG,
            "draft.group.navigation",
        ),
        item(
            "Jump",
            "draft.label.jump",
            icons::ARROW_BEND_UP_RIGHT,
            "draft.group.navigation",
        ),
        item(
            "JumpScene",
            "draft.label.jump_scene",
            icons::ARROW_SQUARE_OUT,
            "draft.group.navigation",
        ),
        item(
            "Call",
            "draft.label.call",
            icons::PHONE_CALL,
            "draft.group.navigation",
        ),
        item(
            "Return",
            "draft.label.return",
            icons::ARROW_U_UP_LEFT,
            "draft.group.navigation",
        ),
        item(
            "End",
            "draft.label.end",
            icons::FLAG_CHECKERED,
            "draft.group.navigation",
        ),
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
            group: "draft.group.plugins",
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
    texts: &Translator,
    scene: SceneId,
    index: usize,
) -> Result<CharacterId, DraftError> {
    previous_speaker(story, scene, index)
        .filter(|c| story.character(*c).is_some())
        .or_else(|| story.characters.first().map(|c| c.id))
        .ok_or_else(|| DraftError::Message(texts.t("draft.need_character")))
}

/// The drafts inserted by "insert a node of type `key` at `index` of
/// `scene`". The first draft is the node the user asked for; some types come
/// with helpers (labels for a choice, a condition or a jump).
pub fn drafts_for(
    story: &Story,
    registry: &EditorRegistry,
    texts: &Translator,
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
                strings: vec![
                    (text_a, texts.t("draft.option_a")),
                    (text_b, texts.t("draft.option_b")),
                ],
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
            let character = stage_character(story, texts, scene, index)?;
            let def = story.character(character);
            let expression = def
                .and_then(|c| c.expressions.first())
                .map(|e| e.name.clone())
                .unwrap_or_default();
            let position = def
                .and_then(|c| c.default_position.clone())
                .unwrap_or(Position::Center);
            vec![NodeDraft::plain(NodeKind::Show {
                scale: None,
                scale_y: None,
                flip_x: false,
                flip_y: false,
                character,
                expression,
                position,
                transition: None,
            })]
        }
        "Hide" => vec![NodeDraft::plain(NodeKind::Hide {
            character: stage_character(story, texts, scene, index)?,
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
        "Effect" => vec![NodeDraft::plain(NodeKind::Effect(orbit_core::Effect::Shake {
            target: orbit_core::AnimTarget::Camera,
            amplitude: 0.02,
            duration: 0.5,
            ease: None,
            wait: false,
        }))],
        "BackgroundLayers" => {
            let image = first_asset(story, AssetKind::Image)?;
            vec![NodeDraft::plain(NodeKind::BackgroundLayers {
                layers: vec![
                    orbit_core::BgLayer::new(image, 0.0),
                    orbit_core::BgLayer::new(image, 0.5),
                ],
                transition: None,
            })]
        }
        "Particles" => vec![NodeDraft::plain(NodeKind::Particles(orbit_core::Emitter::preset(
            orbit_core::ParticlePreset::Rain,
        )))],
        "Script" => {
            let ok = fresh_label("script");
            vec![
                NodeDraft::plain(NodeKind::Script {
                    source: "branch(\"ok\");
".to_owned(),
                    branches: vec![orbit_core::ScriptBranch {
                        name: "ok".to_owned(),
                        target: ok.node.id,
                    }],
                    on_error: None,
                }),
                ok,
            ]
        }
        "Unlock" => {
            let achievement = story
                .achievements
                .first()
                .map(|a| a.id)
                .ok_or_else(|| DraftError::Message(texts.t("draft.need_achievement")))?;
            vec![NodeDraft::plain(NodeKind::Unlock { achievement })]
        }
        "StopParticles" => vec![NodeDraft::plain(NodeKind::StopParticles { name: String::new() })],
        "OpenScreen" => vec![NodeDraft::plain(NodeKind::OpenScreen {
            screen: String::new(),
            wait: false,
            result: None,
            params: Vec::new(),
        })],
        "CloseScreen" => vec![NodeDraft::plain(NodeKind::CloseScreen {
            screen: String::new(),
        })],
        "ShowScene3d" => vec![NodeDraft::plain(NodeKind::ShowScene3d {
            scene: first_asset(story, AssetKind::Scene3d)?,
            camera: None,
            transition: None,
        })],
        "Camera3d" => vec![NodeDraft::plain(NodeKind::Camera3d {
            camera: String::new(),
            transition: None,
        })],
        "HideScene3d" => vec![NodeDraft::plain(NodeKind::HideScene3d { transition: None })],
        "Timeline" => vec![NodeDraft::plain(NodeKind::Timeline {
            duration: 2.0,
            looping: orbit_core::AnimLoop::Once,
            wait: false,
            tracks: vec![orbit_core::StoryTrack {
                target: orbit_core::AnimTarget::Background,
                prop: orbit_core::AnimProp::Offset,
                data: orbit_core::TrackData::Vec2(orbit_core::Track::new(vec![
                    orbit_core::Key::new(0.0, orbit_core::Vec2::new(0.0, 0.0)),
                    orbit_core::Key::new(2.0, orbit_core::Vec2::new(0.1, 0.0)),
                ])),
            }],
        })],
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
        "JumpScene" => {
            // The scene after this one, else the first other scene.
            let scenes = &story.scenes;
            let here = scenes.iter().position(|s| s.id == scene);
            let target = here
                .and_then(|i| scenes.get(i + 1))
                .or_else(|| {
                    scenes
                        .iter()
                        .find(|s| Some(s.id) != here.map(|i| scenes[i].id))
                })
                .ok_or_else(|| DraftError::Message(texts.t("draft.need_scene")))?;
            vec![NodeDraft::plain(NodeKind::JumpScene {
                scene: target.id,
                label: None,
            })]
        }
        "Return" => vec![NodeDraft::plain(NodeKind::Return)],
        "End" => vec![NodeDraft::plain(NodeKind::End)],
        other => {
            let t = registry
                .project
                .node_types()
                .iter()
                .find(|t| format!("{}.{}", t.plugin_id, t.def.kind) == other)
                .ok_or_else(|| {
                    DraftError::Message(texts.tf("draft.unknown_type", &[("type", &other)]))
                })?;
            vec![NodeDraft::plain(NodeKind::Custom {
                plugin_id: t.plugin_id.clone(),
                kind: t.def.kind.clone(),
                version: 1,
                data: (t.def.default_data)(),
            })]
        }
    })
}
