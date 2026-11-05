//! The action list of a hotspot zone in the inspector: what a click does
//! before (or instead of) going to the zone's label. Variables are changed
//! with the no-code operation editor; the first flow action (jump, scene,
//! call, return, stay) decides where the story goes.

use egui::{ComboBox, RichText, Ui};
use orbit_core::{AssetKind, HotspotAction, Notify, SceneId, Story, StringId, VarOp};
use orbit_editor_api::fields::{self, asset_picker, label_picker};
use orbit_editor_api::varwidgets::{self, VarInfo};
use orbit_editor_api::{UiText, icons};

use super::ui_util::{danger, weak};

/// The kinds offered by the "add an action" menu, with their text key.
pub const KINDS: [(&str, &str); 8] = [
    ("set", "inspector.hotspots.act.set"),
    ("notify", "inspector.hotspots.act.notify"),
    ("sound", "inspector.hotspots.act.sound"),
    ("stay", "inspector.hotspots.act.stay"),
    ("jump", "inspector.hotspots.act.jump"),
    ("scene", "inspector.hotspots.act.scene"),
    ("call", "inspector.hotspots.act.call"),
    ("return", "inspector.hotspots.act.return"),
];

fn key_of(a: &HotspotAction) -> &'static str {
    match a {
        HotspotAction::SetVar { .. } => "set",
        HotspotAction::Notify(_) => "notify",
        HotspotAction::PlaySound { .. } => "sound",
        HotspotAction::Stay => "stay",
        HotspotAction::Jump { .. } => "jump",
        HotspotAction::JumpScene { .. } => "scene",
        HotspotAction::Call { .. } => "call",
        HotspotAction::Return => "return",
    }
}

/// A new action of kind `key` with sensible defaults (`None`: impossible,
/// e.g. a sound without any sound asset). A notification also returns its
/// new text, to store with the change.
pub fn new_action(
    story: &Story,
    vars: &[VarInfo],
    key: &str,
    target: orbit_core::NodeId,
    scene: SceneId,
    text: &str,
) -> Option<(HotspotAction, Option<(StringId, String)>)> {
    Some(match key {
        "set" => {
            let op = varwidgets::blank_op(vars);
            if op.var.is_empty() {
                return None;
            }
            (HotspotAction::SetVar { var: op.var, op: op.op, value: op.value }, None)
        }
        "notify" => {
            let text_id = StringId::new();
            (
                HotspotAction::Notify(Notify { text_id, icon: None, sound: None, duration: None, style: None }),
                Some((text_id, text.to_owned())),
            )
        }
        "sound" => {
            let asset = story.assets.assets.iter().find(|a| a.kind == AssetKind::Sound)?.id;
            (HotspotAction::PlaySound { asset }, None)
        }
        "stay" => (HotspotAction::Stay, None),
        "jump" => (HotspotAction::Jump { label: target }, None),
        "call" => (HotspotAction::Call { label: target }, None),
        "return" => (HotspotAction::Return, None),
        "scene" => {
            let to = story.scenes.iter().find(|s| s.id != scene).or(story.scenes.first())?.id;
            (HotspotAction::JumpScene { scene: to, label: None }, None)
        }
        _ => return None,
    })
}

/// What the list editor asks for.
pub enum ActionEdit {
    /// The new list.
    List(Vec<HotspotAction>),
    /// The new list, and a new notification text to store.
    WithText(Vec<HotspotAction>, StringId, String),
    /// A notification's text changed.
    Text(StringId, String),
}

/// The editor of a zone's actions (`salt` must be unique per zone).
pub fn actions_editor(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    story: &Story,
    vars: &[VarInfo],
    actions: &[HotspotAction],
    target: orbit_core::NodeId,
    scene: SceneId,
) -> Option<ActionEdit> {
    let mut out = None;
    let first_flow = actions.iter().position(HotspotAction::is_flow);
    for (k, a) in actions.iter().enumerate() {
        let ignored = first_flow.is_some_and(|f| k > f);
        ui.horizontal_wrapped(|ui| {
            let label = KINDS.iter().find(|(key, _)| *key == key_of(a)).map_or("", |(_, l)| l);
            let mut text = RichText::new(format!("{}. {}", k + 1, ui.t(label))).small().strong();
            if ignored {
                text = text.color(weak(ui)).strikethrough();
            }
            ui.label(text);
            let replace = |new: HotspotAction| {
                let mut list = actions.to_vec();
                list[k] = new;
                ActionEdit::List(list)
            };
            match a {
                HotspotAction::SetVar { var, op, value } => {
                    let current = VarOp::new(var.clone(), *op, value.clone());
                    if let Some(new) = varwidgets::varop_editor(ui, (salt, "op", k), vars, &current, true)
                        && !new.op.uses_key()
                    {
                        out = Some(replace(HotspotAction::SetVar { var: new.var, op: new.op, value: new.value }));
                    }
                }
                HotspotAction::Jump { label } | HotspotAction::Call { label } => {
                    if let Some(Some(l)) = label_picker(ui, (salt, "label", k), story, Some(*label), None) {
                        let new = match a {
                            HotspotAction::Jump { .. } => HotspotAction::Jump { label: l },
                            _ => HotspotAction::Call { label: l },
                        };
                        out = Some(replace(new));
                    }
                }
                HotspotAction::JumpScene { scene: to, .. } => {
                    let name = story.scene(*to).map_or("?", |s| s.name.as_str());
                    ComboBox::from_id_salt((salt, "scene", k)).selected_text(name).show_ui(ui, |ui| {
                        for s in &story.scenes {
                            if ui.selectable_label(s.id == *to, &s.name).clicked() && s.id != *to {
                                out = Some(replace(HotspotAction::JumpScene { scene: s.id, label: None }));
                            }
                        }
                    });
                }
                HotspotAction::PlaySound { asset } => {
                    if let Some(new) = asset_picker(ui, (salt, "sound", k), story, AssetKind::Sound, *asset) {
                        out = Some(replace(HotspotAction::PlaySound { asset: new }));
                    }
                }
                HotspotAction::Notify(n) => {
                    let mut done = false;
                    let text = story.text(n.text_id).unwrap_or_default();
                    let hint = ui.t("inspector.hotspots.act.notify_hint");
                    if let Some(t) = fields::text_field(ui, (salt, "notify", k), text, &hint, 1, &mut done) {
                        out = Some(ActionEdit::Text(n.text_id, t));
                    }
                }
                HotspotAction::Return | HotspotAction::Stay => {}
            }
            let mut list = actions.to_vec();
            if k > 0 && ui.small_button(icons::ARROW_UP).on_hover_text(ui.t("inspector.hotspots.act.up")).clicked() {
                list.swap(k, k - 1);
                out = Some(ActionEdit::List(list.clone()));
            }
            if k + 1 < actions.len()
                && ui.small_button(icons::ARROW_DOWN).on_hover_text(ui.t("inspector.hotspots.act.down")).clicked()
            {
                list.swap(k, k + 1);
                out = Some(ActionEdit::List(list.clone()));
            }
            if ui
                .add(egui::Button::new(RichText::new(icons::TRASH).color(danger(ui))).frame(false))
                .on_hover_text(ui.t("inspector.hotspots.act.remove"))
                .clicked()
            {
                let mut list = actions.to_vec();
                list.remove(k);
                out = Some(ActionEdit::List(list));
            }
        });
    }
    if first_flow.is_none() && !actions.is_empty() {
        ui.label(RichText::new(ui.t("inspector.hotspots.act.then_target")).small().color(weak(ui)));
    }
    ComboBox::from_id_salt((salt, "add"))
        .selected_text(format!("{}  {}", icons::PLUS, ui.t("inspector.hotspots.act.add")))
        .show_ui(ui, |ui| {
            for (key, label) in KINDS {
                if ui.selectable_label(false, ui.t(label)).clicked() {
                    let text = ui.t("inspector.hotspots.act.notify_default");
                    match new_action(story, vars, key, target, scene, &text) {
                        Some((a, None)) => {
                            let mut list = actions.to_vec();
                            list.push(a);
                            out = Some(ActionEdit::List(list));
                        }
                        Some((a, Some((id, text)))) => {
                            let mut list = actions.to_vec();
                            list.push(a);
                            out = Some(ActionEdit::WithText(list, id, text));
                        }
                        None => {}
                    }
                }
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::{NodeId, Value, VarDecl};

    #[test]
    fn new_actions_have_defaults_or_are_refused() {
        let mut story = Story::new("t");
        let scene = SceneId::new();
        let target = NodeId::new();
        // No variable, no sound: those kinds cannot be added yet.
        assert!(new_action(&story, &[], "set", target, scene, "").is_none());
        assert!(new_action(&story, &[], "sound", target, scene, "").is_none());
        story.variables.push(VarDecl::game("gold", Value::Int(0)));
        let vars = varwidgets::story_vars(&story);
        let (set, _) = new_action(&story, &vars, "set", target, scene, "").unwrap();
        assert!(matches!(set, HotspotAction::SetVar { ref var, .. } if var == "gold"));
        let (jump, _) = new_action(&story, &vars, "jump", target, scene, "").unwrap();
        assert_eq!(jump, HotspotAction::Jump { label: target });
        let (notify, text) = new_action(&story, &vars, "notify", target, scene, "Found it").unwrap();
        let (id, t) = text.unwrap();
        assert!(matches!(notify, HotspotAction::Notify(ref n) if n.text_id == id));
        assert_eq!(t, "Found it");
        for (key, _) in KINDS {
            if key != "sound" && key != "scene" {
                assert!(new_action(&story, &vars, key, target, scene, "x").is_some(), "{key}");
            }
        }
    }
}
