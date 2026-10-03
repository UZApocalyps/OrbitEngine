//! How a node looks in lists: kind key, label, icon and one-line summary.

use orbit_core::{Node, NodeKind, Story};
use orbit_editor_api::{Translator, icons};

/// Key used to find an inspector: the variant name, or `plugin.kind` for a
/// plugin node.
pub fn kind_key(kind: &NodeKind) -> String {
    match kind {
        NodeKind::Label { .. } => "Label".into(),
        NodeKind::Say { .. } => "Say".into(),
        NodeKind::Choice { .. } => "Choice".into(),
        NodeKind::Jump { .. } => "Jump".into(),
        NodeKind::JumpScene { .. } => "JumpScene".into(),
        NodeKind::SetVar { .. } => "SetVar".into(),
        NodeKind::If { .. } => "If".into(),
        NodeKind::Background { .. } => "Background".into(),
        NodeKind::Show { .. } => "Show".into(),
        NodeKind::Hide { .. } => "Hide".into(),
        NodeKind::PlayMusic { .. } => "PlayMusic".into(),
        NodeKind::StopMusic { .. } => "StopMusic".into(),
        NodeKind::PlaySound { .. } => "PlaySound".into(),
        NodeKind::Wait { .. } => "Wait".into(),
        NodeKind::Effect(_) => "Effect".into(),
        NodeKind::Timeline { .. } => "Timeline".into(),
        NodeKind::Call { .. } => "Call".into(),
        NodeKind::Return => "Return".into(),
        NodeKind::End => "End".into(),
        NodeKind::Custom {
            plugin_id, kind, ..
        } => format!("{plugin_id}.{kind}"),
    }
}

/// Human name of a node kind, in the editor language.
pub fn kind_label(texts: &Translator, kind: &NodeKind) -> String {
    texts.get(match kind {
        NodeKind::Label { .. } => "node.kind.label",
        NodeKind::Say { .. } => "node.kind.say",
        NodeKind::Choice { .. } => "node.kind.choice",
        NodeKind::Jump { .. } => "node.kind.jump",
        NodeKind::JumpScene { .. } => "node.kind.jump_scene",
        NodeKind::SetVar { .. } => "node.kind.set_var",
        NodeKind::If { .. } => "node.kind.if",
        NodeKind::Background { .. } => "node.kind.background",
        NodeKind::Show { .. } => "node.kind.show",
        NodeKind::Hide { .. } => "node.kind.hide",
        NodeKind::PlayMusic { .. } => "node.kind.play_music",
        NodeKind::StopMusic { .. } => "node.kind.stop_music",
        NodeKind::PlaySound { .. } => "node.kind.play_sound",
        NodeKind::Wait { .. } => "node.kind.wait",
        NodeKind::Effect(_) => "node.kind.effect",
        NodeKind::Timeline { .. } => "node.kind.timeline",
        NodeKind::Call { .. } => "node.kind.call",
        NodeKind::Return => "node.kind.return",
        NodeKind::End => "node.kind.end",
        NodeKind::Custom { .. } => "node.kind.custom",
    })
}

pub fn kind_icon(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Label { .. } => icons::FLAG,
        NodeKind::Say { .. } => icons::CHAT_TEXT,
        NodeKind::Choice { .. } => icons::GIT_BRANCH,
        NodeKind::Jump { .. } => icons::ARROW_BEND_UP_RIGHT,
        NodeKind::JumpScene { .. } => icons::ARROW_SQUARE_OUT,
        NodeKind::SetVar { .. } => icons::FUNCTION,
        NodeKind::If { .. } => icons::GIT_BRANCH,
        NodeKind::Background { .. } => icons::IMAGE,
        NodeKind::Show { .. } => icons::USER_PLUS,
        NodeKind::Hide { .. } => icons::USER_MINUS,
        NodeKind::PlayMusic { .. } => icons::MUSIC_NOTE,
        NodeKind::StopMusic { .. } => icons::STOP,
        NodeKind::PlaySound { .. } => icons::SPEAKER_HIGH,
        NodeKind::Wait { .. } => icons::TIMER,
        NodeKind::Effect(_) => icons::SPARKLE,
        NodeKind::Timeline { .. } => icons::FILM_STRIP,
        NodeKind::Call { .. } => icons::PHONE_CALL,
        NodeKind::Return => icons::ARROW_U_UP_LEFT,
        NodeKind::End => icons::STOP,
        NodeKind::Custom { .. } => icons::PUZZLE_PIECE,
    }
}

/// Accent color of a node kind (by family: visual, sound, logic, flow).
pub fn kind_color(kind: &NodeKind) -> egui::Color32 {
    use egui::Color32;
    match kind {
        NodeKind::Say { .. } => Color32::from_rgb(130, 140, 160),
        NodeKind::Background { .. } | NodeKind::Show { .. } | NodeKind::Hide { .. } => {
            Color32::from_rgb(86, 156, 240)
        }
        NodeKind::PlayMusic { .. } | NodeKind::StopMusic { .. } | NodeKind::PlaySound { .. } => {
            Color32::from_rgb(176, 124, 236)
        }
        NodeKind::Choice { .. } | NodeKind::SetVar { .. } | NodeKind::If { .. } => {
            Color32::from_rgb(236, 168, 70)
        }
        NodeKind::Label { .. }
        | NodeKind::Jump { .. }
        | NodeKind::JumpScene { .. }
        | NodeKind::Call { .. }
        | NodeKind::Return
        | NodeKind::End => Color32::from_rgb(98, 190, 168),
        NodeKind::Wait { .. } => Color32::from_rgb(150, 156, 170),
        NodeKind::Effect(_) | NodeKind::Timeline { .. } => Color32::from_rgb(236, 120, 96),
        NodeKind::Custom { .. } => Color32::from_rgb(232, 110, 160),
    }
}

fn character_name(story: &Story, id: orbit_core::CharacterId) -> String {
    story
        .character(id)
        .and_then(|c| story.text(c.name_id))
        .unwrap_or("?")
        .to_owned()
}

fn label_name(story: &Story, id: orbit_core::NodeId) -> String {
    match story.node(id).map(|n| &n.kind) {
        Some(NodeKind::Label { name }) => name.clone(),
        _ => "?".to_owned(),
    }
}

fn asset_name(story: &Story, id: orbit_core::AssetId) -> String {
    story.asset_path(id).unwrap_or("?").to_owned()
}

/// A stage position in words ("left", "free (0.40, 0.95)"...).
pub fn position_name(texts: &Translator, position: &orbit_core::Position) -> String {
    use orbit_core::Position;
    match position {
        Position::Left => texts.get("node.position.left"),
        Position::Center => texts.get("node.position.center"),
        Position::Right => texts.get("node.position.right"),
        Position::Custom(n) => n.clone(),
        Position::Free { x, y } => texts.format(
            "node.position.free",
            &[("x", &format!("{x:.2}")), ("y", &format!("{y:.2}"))],
        ),
    }
}

/// What a `Show` changes about the sprite besides its position, for a list
/// row: `"  ·  ×0.8  ·  mirror X"`, empty for the standard look.
pub fn look_text(
    texts: &Translator,
    scale: Option<f32>,
    scale_y: Option<f32>,
    flip_x: bool,
    flip_y: bool,
) -> String {
    let mut out = String::new();
    match (scale, scale_y) {
        (Some(s), Some(sy)) if sy != s => out.push_str(&format!("  ·  ×{s:.2} / ×{sy:.2}")),
        (Some(s), _) => out.push_str(&format!("  ·  ×{s:.2}")),
        (None, Some(sy)) => out.push_str(&format!(
            "  ·  ×{sy:.2} {}",
            texts.get("node.look.vertical")
        )),
        (None, None) => {}
    }
    if flip_x {
        out.push_str(&format!("  ·  {}", texts.get("node.look.mirror_x")));
    }
    if flip_y {
        out.push_str(&format!("  ·  {}", texts.get("node.look.mirror_y")));
    }
    out
}

/// Cuts a long text for a list row.
pub fn truncate(text: &str, max_chars: usize) -> String {
    let single_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= max_chars {
        single_line
    } else {
        let cut: String = single_line.chars().take(max_chars).collect();
        format!("{}…", cut.trim_end())
    }
}

/// One line describing a node, for the explorer.
pub fn summary(texts: &Translator, story: &Story, node: &Node) -> String {
    match &node.kind {
        NodeKind::Label { name } => texts.format("node.summary.label", &[("name", name)]),
        NodeKind::Say { speaker, text_id } => {
            let who =
                speaker.map_or_else(|| texts.get("node.narrator"), |c| character_name(story, c));
            texts.format(
                "node.summary.say",
                &[
                    ("who", &who),
                    ("text", &truncate(story.text(*text_id).unwrap_or("?"), 70)),
                ],
            )
        }
        NodeKind::Choice { options } => texts.plural("node.summary.options", options.len(), &[]),
        NodeKind::Jump { target } => format!("→ {}", label_name(story, *target)),
        NodeKind::JumpScene { scene, label } => {
            let name = story.scene(*scene).map_or("?", |s| s.name.as_str());
            match label {
                Some(l) => texts.format(
                    "node.summary.jump_scene_label",
                    &[("scene", &name), ("label", &label_name(story, *l))],
                ),
                None => texts.format("node.summary.jump_scene", &[("scene", &name)]),
            }
        }
        NodeKind::SetVar { var, value } => format!("{var} = {value}"),
        NodeKind::If {
            condition,
            then_target,
            ..
        } => texts.format(
            "node.summary.if",
            &[
                ("condition", condition),
                ("label", &label_name(story, *then_target)),
            ],
        ),
        NodeKind::Background { asset, .. } => asset_name(story, *asset),
        NodeKind::Show {
            character,
            expression,
            scale,
            scale_y,
            flip_x,
            flip_y,
            ..
        } => format!(
            "{} ({expression}){}",
            character_name(story, *character),
            look_text(texts, *scale, *scale_y, *flip_x, *flip_y)
        ),
        NodeKind::Hide { character, .. } => character_name(story, *character),
        NodeKind::PlayMusic { asset, .. } | NodeKind::PlaySound { asset } => {
            asset_name(story, *asset)
        }
        NodeKind::StopMusic { .. } | NodeKind::Return | NodeKind::End => String::new(),
        NodeKind::Wait { seconds } => texts.format("node.summary.wait", &[("seconds", seconds)]),
        NodeKind::Call { label } => format!("→ {}", label_name(story, *label)),
        NodeKind::Effect(e) => {
            let (kind, detail) = super::fx_text::effect_row(texts, story, e);
            format!("{kind}  ·  {detail}")
        }
        NodeKind::Timeline {
            duration,
            looping,
            wait,
            tracks,
        } => super::fx_text::timeline_detail(texts, *duration, *looping, *wait, tracks),
        NodeKind::Custom {
            plugin_id, kind, ..
        } => format!("{plugin_id}.{kind}"),
    }
}

#[cfg(test)]
mod tests {
    use orbit_core::{NodeId, Scene, SceneId};

    use super::*;

    #[test]
    fn summaries_name_speakers_and_targets() {
        let mut story = Story::new("t");
        let text_id = story.strings.add("Bonjour tout le monde");
        let label = Node {
            id: NodeId::new(),
            kind: NodeKind::Label { name: "fin".into() },
        };
        let say = Node {
            id: NodeId::new(),
            kind: NodeKind::Say {
                speaker: None,
                text_id,
            },
        };
        let jump = Node {
            id: NodeId::new(),
            kind: NodeKind::Jump { target: label.id },
        };
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes: vec![say.clone(), jump.clone(), label],
        });
        let fr = crate::i18n::translator(orbit_editor_api::Lang::Fr);
        assert_eq!(
            summary(&fr, &story, &say),
            "Narrateur : Bonjour tout le monde"
        );
        assert_eq!(summary(&fr, &story, &jump), "→ fin");
        let en = crate::i18n::translator(orbit_editor_api::Lang::En);
        assert_eq!(
            summary(&en, &story, &say),
            "Narrator: Bonjour tout le monde"
        );
        assert_eq!(kind_label(&en, &say.kind), "Dialogue");
        assert_eq!(kind_key(&say.kind), "Say");
    }

    #[test]
    fn long_text_is_cut_on_one_line() {
        assert_eq!(truncate("a\nb   c", 10), "a b c");
        assert_eq!(truncate("abcdefghij", 4), "abcd…");
    }
}
