//! How a node looks in lists: kind key, label, icon and one-line summary.

use orbit_core::{Node, NodeKind, Story};
use orbit_editor_api::icons;

/// Key used to find an inspector: the variant name, or `plugin.kind` for a
/// plugin node.
pub fn kind_key(kind: &NodeKind) -> String {
    match kind {
        NodeKind::Label { .. } => "Label".into(),
        NodeKind::Say { .. } => "Say".into(),
        NodeKind::Choice { .. } => "Choice".into(),
        NodeKind::Jump { .. } => "Jump".into(),
        NodeKind::SetVar { .. } => "SetVar".into(),
        NodeKind::If { .. } => "If".into(),
        NodeKind::Background { .. } => "Background".into(),
        NodeKind::Show { .. } => "Show".into(),
        NodeKind::Hide { .. } => "Hide".into(),
        NodeKind::PlayMusic { .. } => "PlayMusic".into(),
        NodeKind::StopMusic { .. } => "StopMusic".into(),
        NodeKind::PlaySound { .. } => "PlaySound".into(),
        NodeKind::Wait { .. } => "Wait".into(),
        NodeKind::Call { .. } => "Call".into(),
        NodeKind::Return => "Return".into(),
        NodeKind::End => "End".into(),
        NodeKind::Custom {
            plugin_id, kind, ..
        } => format!("{plugin_id}.{kind}"),
    }
}

/// Human name of a node kind.
pub fn kind_label(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Label { .. } => "Étiquette",
        NodeKind::Say { .. } => "Réplique",
        NodeKind::Choice { .. } => "Choix",
        NodeKind::Jump { .. } => "Saut",
        NodeKind::SetVar { .. } => "Variable",
        NodeKind::If { .. } => "Condition",
        NodeKind::Background { .. } => "Décor",
        NodeKind::Show { .. } => "Afficher un personnage",
        NodeKind::Hide { .. } => "Masquer un personnage",
        NodeKind::PlayMusic { .. } => "Musique",
        NodeKind::StopMusic { .. } => "Arrêt de la musique",
        NodeKind::PlaySound { .. } => "Son",
        NodeKind::Wait { .. } => "Attente",
        NodeKind::Call { .. } => "Appel",
        NodeKind::Return => "Retour",
        NodeKind::End => "Fin",
        NodeKind::Custom { .. } => "Nœud de plugin",
    }
}

pub fn kind_icon(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Label { .. } => icons::FLAG,
        NodeKind::Say { .. } => icons::CHAT_TEXT,
        NodeKind::Choice { .. } => icons::GIT_BRANCH,
        NodeKind::Jump { .. } => icons::ARROW_BEND_UP_RIGHT,
        NodeKind::SetVar { .. } => icons::FUNCTION,
        NodeKind::If { .. } => icons::GIT_BRANCH,
        NodeKind::Background { .. } => icons::IMAGE,
        NodeKind::Show { .. } => icons::USER_PLUS,
        NodeKind::Hide { .. } => icons::USER_MINUS,
        NodeKind::PlayMusic { .. } => icons::MUSIC_NOTE,
        NodeKind::StopMusic { .. } => icons::STOP,
        NodeKind::PlaySound { .. } => icons::SPEAKER_HIGH,
        NodeKind::Wait { .. } => icons::TIMER,
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
        | NodeKind::Call { .. }
        | NodeKind::Return
        | NodeKind::End => Color32::from_rgb(98, 190, 168),
        NodeKind::Wait { .. } => Color32::from_rgb(150, 156, 170),
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
pub fn summary(story: &Story, node: &Node) -> String {
    match &node.kind {
        NodeKind::Label { name } => format!("« {name} »"),
        NodeKind::Say { speaker, text_id } => {
            let who = speaker.map_or_else(|| "Narrateur".to_owned(), |c| character_name(story, c));
            format!(
                "{who} : {}",
                truncate(story.text(*text_id).unwrap_or("?"), 70)
            )
        }
        NodeKind::Choice { options } => format!("{} option(s)", options.len()),
        NodeKind::Jump { target } => format!("→ {}", label_name(story, *target)),
        NodeKind::SetVar { var, value } => format!("{var} = {value}"),
        NodeKind::If {
            condition,
            then_target,
            ..
        } => format!("si {condition} → {}", label_name(story, *then_target)),
        NodeKind::Background { asset, .. } => asset_name(story, *asset),
        NodeKind::Show {
            character,
            expression,
            ..
        } => format!("{} ({expression})", character_name(story, *character)),
        NodeKind::Hide { character, .. } => character_name(story, *character),
        NodeKind::PlayMusic { asset, .. } | NodeKind::PlaySound { asset } => {
            asset_name(story, *asset)
        }
        NodeKind::StopMusic { .. } | NodeKind::Return | NodeKind::End => String::new(),
        NodeKind::Wait { seconds } => format!("{seconds} s"),
        NodeKind::Call { label } => format!("→ {}", label_name(story, *label)),
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
        assert_eq!(summary(&story, &say), "Narrateur : Bonjour tout le monde");
        assert_eq!(summary(&story, &jump), "→ fin");
        assert_eq!(kind_key(&say.kind), "Say");
    }

    #[test]
    fn long_text_is_cut_on_one_line() {
        assert_eq!(truncate("a\nb   c", 10), "a b c");
        assert_eq!(truncate("abcdefghij", 4), "abcd…");
    }
}
