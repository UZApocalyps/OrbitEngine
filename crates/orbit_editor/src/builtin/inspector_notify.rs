//! The `Notify` node in the inspector: text (markup, `[variable]`), named
//! style, optional icon, sound and duration.

use egui::{ComboBox, RichText, Ui};
use orbit_core::{AssetKind, NodeId, NodeKind, Notify};
use orbit_editor_api::fields::{self, asset_picker};
use orbit_editor_api::{EditorContext, UiText};
use orbit_project::Command;

use super::inspector::{commit, tfield};
use super::ui_util::faint;

/// Style names every theme knows (the theme may define more).
const BUILTIN_STYLES: [&str; 3] = ["default", "achievement", "system"];

pub(super) fn notify_node(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, n: &Notify) {
    let Some(story) = ctx.story() else { return };
    let text = story.text(n.text_id).unwrap_or_default().to_owned();
    let mut new_text = None;
    let mut done = false;
    tfield(ui, "inspector.text", |ui| {
        let hint = ui.t("inspector.notify.hint");
        new_text = fields::text_field(ui, ("notify_text", id), &text, &hint, 3, &mut done);
    });

    let mut next: Option<(Notify, &'static str)> = None;
    tfield(ui, "inspector.notify.style", |ui| {
        let current = n.style.clone().unwrap_or_else(|| "default".to_owned());
        ui.horizontal(|ui| {
            ComboBox::from_id_salt(("notify_style", id))
                .selected_text(&current)
                .width(140.0)
                .show_ui(ui, |ui| {
                    for name in BUILTIN_STYLES {
                        if ui.selectable_label(current == name, name).clicked() {
                            let style = (name != "default").then(|| name.to_owned());
                            next = Some((Notify { style, ..n.clone() }, "style"));
                        }
                    }
                });
            let mut custom = n.style.clone().unwrap_or_default();
            let edit = ui.add(
                egui::TextEdit::singleline(&mut custom)
                    .desired_width(ui.available_width())
                    .hint_text(ui.t("inspector.notify.custom_style")),
            );
            if edit.changed() {
                let style = (!custom.trim().is_empty() && custom.trim() != "default")
                    .then(|| custom.trim().to_owned());
                next = Some((Notify { style, ..n.clone() }, "style"));
            }
        });
    });

    for (key, current, kind) in [
        ("inspector.notify.icon", n.icon, AssetKind::Image),
        ("inspector.notify.sound", n.sound, AssetKind::Sound),
    ] {
        let label = ui.t(key);
        let first = story.assets.assets.iter().find(|a| a.kind == kind).map(|a| a.id);
        let set = |v| {
            let mut m = n.clone();
            if kind == AssetKind::Image {
                m.icon = v;
            } else {
                m.sound = v;
            }
            m
        };
        if let Some(on) = fields::checkbox(ui, current.is_some(), &label)
            && (!on || first.is_some())
        {
            next = Some((set(if on { first } else { None }), "asset"));
        }
        if let Some(a) = current
            && let Some(new) = asset_picker(ui, ("notify_asset", id, key), story, kind, a)
        {
            next = Some((set(Some(new)), "asset"));
        }
    }

    let own = ui.t("inspector.notify.own_duration");
    if let Some(on) = fields::checkbox(ui, n.duration.is_some(), &own) {
        next = Some((Notify { duration: on.then_some(3.0), ..n.clone() }, "duration"));
    }
    if let Some(d) = n.duration
        && let Some(new) = fields::seconds_field(ui, d)
    {
        next = Some((Notify { duration: Some(new.max(0.1)), ..n.clone() }, "duration"));
    }
    ui.label(RichText::new(ui.t("inspector.notify.info")).small().color(faint(ui)));

    if let Some(t) = new_text {
        let command = Command::SetString { id: n.text_id, text: Some(t) };
        if let Err(e) = ctx.exec_merged(command, &format!("notify-text:{id}")) {
            ctx.error(e.to_string());
        }
    }
    if done {
        ctx.end_merge();
    }
    if let Some((kind, field)) = next {
        commit(ctx, id, NodeKind::Notify(kind), field);
    }
}
