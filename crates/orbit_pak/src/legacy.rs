//! The story layout of game packs written before schema v9 (pack versions 1
//! and 2). Postcard is not self-describing, so a pack made by an older build
//! can only be read with the exact layout it was written with: these mirrors
//! of `Story`, `NodeKind` and `Transition` are **frozen**, never edit them
//! (a pack must keep loading, docs/IMPLEMENTATION_PLAN.md §1.3). Everything
//! they do not mention (characters, expressions, expressions...) is shared
//! with the current types because it did not change.

use std::collections::BTreeMap;

use orbit_core::{
    AssetId, AssetManifest, Character, CharacterId, ChoiceOption, CustomData, Expr, Node, NodeId,
    NodeKind, Position, PositionDef, Scene, SceneId, Story, StringId, StringTable, Transition,
    TransitionKind, VarDecl,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct TransitionV8 {
    kind: TransitionKind,
    duration: f32,
}

impl From<TransitionV8> for Transition {
    fn from(t: TransitionV8) -> Self {
        Transition {
            kind: t.kind,
            duration: t.duration,
            ease: None,
        }
    }
}

fn tr(t: Option<TransitionV8>) -> Option<Transition> {
    t.map(Into::into)
}

#[derive(Deserialize)]
enum NodeKindV8 {
    Label {
        name: String,
    },
    Say {
        speaker: Option<CharacterId>,
        text_id: StringId,
    },
    Choice {
        options: Vec<ChoiceOption>,
    },
    Jump {
        target: NodeId,
    },
    JumpScene {
        scene: SceneId,
        label: Option<NodeId>,
    },
    SetVar {
        var: String,
        value: Expr,
    },
    If {
        condition: Expr,
        then_target: NodeId,
        else_target: Option<NodeId>,
    },
    Background {
        asset: AssetId,
        transition: Option<TransitionV8>,
    },
    Show {
        character: CharacterId,
        expression: String,
        position: Position,
        transition: Option<TransitionV8>,
        scale: Option<f32>,
        scale_y: Option<f32>,
        flip_x: bool,
        flip_y: bool,
    },
    Hide {
        character: CharacterId,
        transition: Option<TransitionV8>,
    },
    PlayMusic {
        asset: AssetId,
        fade_in: f32,
        looping: bool,
    },
    StopMusic {
        fade_out: f32,
    },
    PlaySound {
        asset: AssetId,
    },
    Wait {
        seconds: f32,
    },
    Call {
        label: NodeId,
    },
    Return,
    End,
    Custom {
        plugin_id: String,
        kind: String,
        version: u32,
        data: CustomData,
    },
}

impl From<NodeKindV8> for NodeKind {
    fn from(k: NodeKindV8) -> Self {
        use NodeKindV8 as V;
        match k {
            V::Label { name } => NodeKind::Label { name },
            V::Say { speaker, text_id } => NodeKind::Say { speaker, text_id, voice: None },
            V::Choice { options } => NodeKind::Choice { options },
            V::Jump { target } => NodeKind::Jump { target },
            V::JumpScene { scene, label } => NodeKind::JumpScene { scene, label },
            V::SetVar { var, value } => NodeKind::SetVar { var, value },
            V::If {
                condition,
                then_target,
                else_target,
            } => NodeKind::If {
                condition,
                then_target,
                else_target,
            },
            V::Background { asset, transition } => NodeKind::Background {
                asset,
                transition: tr(transition),
            },
            V::Show {
                character,
                expression,
                position,
                transition,
                scale,
                scale_y,
                flip_x,
                flip_y,
            } => NodeKind::Show {
                character,
                expression,
                position,
                transition: tr(transition),
                scale,
                scale_y,
                flip_x,
                flip_y,
            },
            V::Hide {
                character,
                transition,
            } => NodeKind::Hide {
                character,
                transition: tr(transition),
            },
            V::PlayMusic {
                asset,
                fade_in,
                looping,
            } => NodeKind::PlayMusic {
                asset,
                fade_in,
                looping,
            },
            V::StopMusic { fade_out } => NodeKind::StopMusic { fade_out },
            V::PlaySound { asset } => NodeKind::PlaySound { asset },
            V::Wait { seconds } => NodeKind::Wait { seconds },
            V::Call { label } => NodeKind::Call { label },
            V::Return => NodeKind::Return,
            V::End => NodeKind::End,
            V::Custom {
                plugin_id,
                kind,
                version,
                data,
            } => NodeKind::Custom {
                plugin_id,
                kind,
                version,
                data,
            },
        }
    }
}

#[derive(Deserialize)]
struct NodeV8 {
    id: NodeId,
    kind: NodeKindV8,
}

#[derive(Deserialize)]
struct SceneV8 {
    id: SceneId,
    name: String,
    nodes: Vec<NodeV8>,
}

/// `Story` as written by schema v8 and before (no `curves`).
#[derive(Deserialize)]
pub(crate) struct StoryV8 {
    schema_version: u32,
    title: String,
    strings: StringTable,
    assets: AssetManifest,
    characters: Vec<Character>,
    positions: Vec<PositionDef>,
    variables: Vec<VarDecl>,
    scenes: Vec<SceneV8>,
}

impl From<StoryV8> for Story {
    fn from(s: StoryV8) -> Self {
        Story {
            // The pack is a v9 story from now on: nothing in it needs a
            // migration (v9 only added optional data).
            schema_version: s.schema_version.max(orbit_core::CURRENT_SCHEMA_VERSION),
            title: s.title,
            strings: s.strings,
            assets: s.assets,
            characters: s.characters,
            positions: s.positions,
            variables: s.variables,
            curves: BTreeMap::new(),
            achievements: Vec::new(),
            scenes: s
                .scenes
                .into_iter()
                .map(|sc| Scene {
                    id: sc.id,
                    name: sc.name,
                    nodes: sc
                        .nodes
                        .into_iter()
                        .map(|n| Node {
                            id: n.id,
                            kind: n.kind.into(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}
