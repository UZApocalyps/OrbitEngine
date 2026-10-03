//! Inspectors of the `OpenScreen` and `CloseScreen` nodes (ADR 0016): the
//! screen is picked from the project's UI screens (with a free text
//! fallback), `OpenScreen` also has `wait`, an optional result variable and
//! the list of parameters handed to the screen.

use egui::{ComboBox, RichText, Ui};
use orbit_core::{NodeId, NodeKind, Operand, ScreenParam, Value};
use orbit_editor_api::fields::{self, checkbox};
use orbit_editor_api::varwidgets::{story_vars, untyped_operand_editor};
use orbit_editor_api::{EditorContext, UiText};
use orbit_ui::UiBundle;

use super::inspector::{commit, info, tfield};
use super::ui_util::weak;

/// Ids of the screens a story can open: the project's `ui/` screens, or the
/// stock ones when the project has none. Cached for a couple of seconds (the
/// folder is read from disk).
pub(super) fn screen_ids(ui: &Ui, ctx: &EditorContext<'_>) -> Vec<String> {
    let root = ctx.project_info().map(|p| p.root.clone());
    let now = ui.input(|i| i.time);
    let key = egui::Id::new("orbit_screen_ids_cache");
    type Cache = (f64, Option<std::path::PathBuf>, Vec<String>);
    if let Some((at, cached_root, ids)) = ui.data(|d| d.get_temp::<Cache>(key))
        && cached_root == root
        && now - at < 2.0
    {
        return ids;
    }
    let ids = read_screen_ids(root.as_deref());
    ui.data_mut(|d| d.insert_temp::<Cache>(key, (now, root, ids.clone())));
    ids
}

/// The screen ids of the project at `root` (stock screens when none).
pub(super) fn read_screen_ids(root: Option<&std::path::Path>) -> Vec<String> {
    let project = root
        .map(|r| r.join("ui"))
        .filter(|d| d.is_dir())
        .and_then(|d| UiBundle::load_dir(&d).ok())
        .filter(|b| !b.screens.is_empty());
    let bundle = project.unwrap_or_else(UiBundle::builtin);
    bundle.screens.iter().map(|s| s.id.clone()).collect()
}

/// A combo of the project's screens plus a free text field.
fn screen_picker(
    ui: &mut Ui,
    ctx: &EditorContext<'_>,
    id: NodeId,
    current: &str,
) -> Option<String> {
    let screens = screen_ids(ui, ctx);
    let mut out = None;
    let mut chosen = current.to_owned();
    let choose = ui.t("inspector.screen.choose");
    ComboBox::from_id_salt(("screen_pick", id))
        .selected_text(if current.is_empty() { choose } else { current.to_owned() })
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for s in &screens {
                ui.selectable_value(&mut chosen, s.clone(), s);
            }
        });
    if chosen != current {
        out = Some(chosen);
    }
    let mut done = false;
    let hint = ui.t("inspector.screen.name_hint");
    if let Some(t) = fields::text_field(ui, ("screen_name", id), current, &hint, 1, &mut done) {
        out = Some(t);
    }
    if !current.is_empty() && !screens.iter().any(|s| s == current) {
        ui.label(
            RichText::new(ui.t("inspector.screen.unknown"))
                .small()
                .color(ui.visuals().warn_fg_color),
        );
    }
    out
}

pub(super) fn close_screen(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, screen: &str) {
    tfield(ui, "inspector.screen", |ui| {
        if let Some(s) = screen_picker(ui, ctx, id, screen) {
            commit(ctx, id, NodeKind::CloseScreen { screen: s }, "screen");
        }
    });
    info(ui, "inspector.close_screen_info");
}

pub(super) fn open_screen(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    screen: &str,
    wait: bool,
    result: &Option<String>,
    params: &[ScreenParam],
) {
    let rebuild = |screen: &str, wait: bool, result: Option<String>, params: Vec<ScreenParam>| {
        NodeKind::OpenScreen {
            screen: screen.to_owned(),
            wait,
            result,
            params,
        }
    };
    tfield(ui, "inspector.screen", |ui| {
        if let Some(s) = screen_picker(ui, ctx, id, screen) {
            commit(ctx, id, rebuild(&s, wait, result.clone(), params.to_vec()), "screen");
        }
    });
    ui.add_space(6.0);
    let wait_label = ui.t("inspector.screen.wait");
    if let Some(w) = checkbox(ui, wait, &wait_label) {
        // Without waiting there is no result to store.
        let result = if w { result.clone() } else { None };
        commit(ctx, id, rebuild(screen, w, result, params.to_vec()), "wait");
    }
    info(ui, "inspector.screen.wait_tip");

    // The result variable: only when the story waits for the screen.
    let vars: Vec<String> = ctx
        .story()
        .map(|s| s.variables.iter().map(|v| v.name.clone()).collect())
        .unwrap_or_default();
    ui.add_enabled_ui(wait, |ui| {
        let has = result.is_some();
        let label = ui.t("inspector.screen.store_result");
        if let Some(on) = checkbox(ui, has, &label) {
            let new = on.then(|| vars.first().cloned().unwrap_or_default());
            commit(ctx, id, rebuild(screen, wait, new, params.to_vec()), "result");
        }
        if let Some(current) = result {
            let mut chosen = current.clone();
            ComboBox::from_id_salt(("screen_result", id))
                .selected_text(if current.is_empty() {
                    ui.t("varop.choose_var")
                } else {
                    current.clone()
                })
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for v in &vars {
                        ui.selectable_value(&mut chosen, v.clone(), v);
                    }
                });
            if chosen != *current {
                commit(
                    ctx,
                    id,
                    rebuild(screen, wait, Some(chosen), params.to_vec()),
                    "result",
                );
            }
        }
    });

    // The parameters.
    ui.add_space(8.0);
    let title = ui.t("inspector.screen.params");
    ui.label(RichText::new(title).small().color(weak(ui)));
    let story_vars = ctx.story().map(story_vars).unwrap_or_default();
    let mut changed: Option<Vec<ScreenParam>> = None;
    let mut commit_key = "param";
    for (i, p) in params.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut done = false;
            let hint = ui.t("inspector.screen.param_name");
            if let Some(n) =
                fields::text_field(ui, ("screen_param_name", id, i), &p.name, &hint, 1, &mut done)
            {
                let mut v = params.to_vec();
                v[i].name = n;
                changed = Some(v);
            }
        });
        ui.horizontal(|ui| {
            if let Some(o) = untyped_operand_editor(ui, ("screen_param", id, i), &story_vars, &p.value)
            {
                let mut v = params.to_vec();
                v[i].value = o;
                changed = Some(v);
            }
            if ui
                .small_button(egui_phosphor::regular::TRASH)
                .on_hover_text(ui.t("inspector.screen.remove_param"))
                .clicked()
            {
                let mut v = params.to_vec();
                v.remove(i);
                changed = Some(v);
                commit_key = "param_remove";
            }
        });
        ui.separator();
    }
    if ui
        .small_button(format!(
            "{}  {}",
            egui_phosphor::regular::PLUS,
            ui.t("inspector.screen.add_param")
        ))
        .clicked()
    {
        let mut v = params.to_vec();
        v.push(ScreenParam {
            name: format!("param{}", v.len() + 1),
            value: Operand::Lit(Value::Int(0)),
        });
        changed = Some(v);
        commit_key = "param_add";
    }
    if let Some(v) = changed {
        commit(ctx, id, rebuild(screen, wait, result.clone(), v), commit_key);
    }
}
