//! The "Scripts" panel (ADR 0015): the shared Rhai modules of the project
//! (`scripts/<name>.rhai`), listed, created, edited, saved and deleted
//! through `orbit_project::scripts`, with the highlighting code editor, the
//! diagnostics of the text being edited and the nodes that import the
//! module. Modules are project files, not story data: saving is explicit
//! (no undo), unsaved text is kept per module until saved or reverted.

use std::collections::BTreeMap;

use egui::{RichText, Ui};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, Location, PanelDef, Plugin,
    PluginManifest, UiText, icons,
};
use orbit_project::scripts as project_scripts;

use super::ids;
use super::inspector_script::{
    Finding, checked, code_editor, findings_list, invalidate,
};
use super::nodeinfo::{kind_label, summary, truncate};
use super::reveal::reveal;
use super::ui_util::{danger, faint, strong, weak};

pub struct ScriptsPlugin;

impl Plugin for ScriptsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::SCRIPTS, "Scripts", "0.1.0")
    }
}

impl EditorPlugin for ScriptsPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "scripts",
            "script.panel.title",
            icons::CODE,
            DockSlot::Right,
            panel,
        ));
    }
}

/// Template of a new module.
const NEW_MODULE: &str = "// Shared functions: `import \"name\" as m;` then `m::f()`.\n";

#[derive(Clone, Default)]
struct State {
    selected: Option<String>,
    /// Unsaved text per module.
    drafts: BTreeMap<String, String>,
    new_name: String,
    error: Option<String>,
    confirm_delete: bool,
    /// Findings of `(module, text)`, recomputed when either changes.
    findings: Option<(String, String, Vec<Finding>)>,
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    if !ctx.has_project() {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ctx.t("script.panel.no_project")).color(weak(ui)));
        });
        return;
    }
    let state_id = egui::Id::new("orbit_scripts_panel");
    let mut st: State = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let Some(info) = checked(ui, ctx) else { return };
    let names: Vec<String> = info.modules.keys().cloned().collect();
    if st.selected.as_ref().is_some_and(|s| !names.contains(s)) {
        st.selected = None;
    }
    if st.selected.is_none() {
        st.selected = names.first().cloned();
    }
    let mut reveal_node = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(ui.t("script.panel.hint")).small().color(weak(ui)));
            ui.add_space(4.0);

            // ---- list and creation
            ui.horizontal_wrapped(|ui| {
                for name in &names {
                    let dirty = st.drafts.get(name).is_some_and(|d| d != &info.modules[name]);
                    let label = if dirty { format!("{name} •") } else { name.clone() };
                    let on = st.selected.as_deref() == Some(name.as_str());
                    if ui.selectable_label(on, label).clicked() {
                        st.selected = Some(name.clone());
                        st.confirm_delete = false;
                        st.error = None;
                    }
                }
            });
            if names.is_empty() {
                ui.label(RichText::new(ui.t("script.panel.empty")).color(faint(ui)));
            }
            ui.horizontal(|ui| {
                let hint = ui.t("script.panel.name_hint");
                ui.add(
                    egui::TextEdit::singleline(&mut st.new_name)
                        .hint_text(hint)
                        .desired_width(140.0),
                );
                let name = st.new_name.trim().to_owned();
                let valid = orbit_core::scripting::is_valid_module_name(&name)
                    && !names.contains(&name);
                if ui
                    .add_enabled(
                        valid,
                        egui::Button::new(format!("{}  {}", icons::FILE_PLUS, ui.t("script.panel.create"))),
                    )
                    .on_hover_text(ui.t("script.panel.create.tip"))
                    .clicked()
                {
                    match project_scripts::write_module(&info.root, &name, NEW_MODULE) {
                        Ok(()) => {
                            invalidate(ui);
                            st.selected = Some(name);
                            st.new_name.clear();
                            st.error = None;
                        }
                        Err(e) => st.error = Some(e),
                    }
                }
            });
            if let Some(e) = &st.error {
                ui.label(RichText::new(e).small().color(danger(ui)));
            }

            let Some(name) = st.selected.clone() else { return };
            let Some(saved) = info.modules.get(&name) else { return };
            ui.separator();
            ui.label(strong(project_scripts::module_path(&name), 13.0));

            // ---- text, findings
            let text = st.drafts.get(&name).cloned().unwrap_or_else(|| saved.clone());
            let stale = st
                .findings
                .as_ref()
                .is_none_or(|(n, t, _)| *n != name || *t != text);
            if stale {
                let mut modules = (*info.modules).clone();
                modules.insert(name.clone(), String::new());
                let engine = orbit_script::ScriptEngine::new(modules);
                let found = engine
                    .check(&text, false)
                    .into_iter()
                    .map(|d| Finding {
                        error: d.severity == orbit_script::Severity::Error,
                        line: d.line,
                        column: d.column,
                        message: d.message,
                    })
                    .collect();
                st.findings = Some((name.clone(), text.clone(), found));
            }
            let found = st.findings.as_ref().map(|f| f.2.clone()).unwrap_or_default();
            if let Some(new) = code_editor(ui, ("module", &name), &text, &found, 14) {
                st.drafts.insert(name.clone(), new);
            }
            findings_list(ui, ("module", &name), &found, true);

            // ---- save, revert, delete
            let dirty = st.drafts.get(&name).is_some_and(|d| d != saved);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        dirty,
                        egui::Button::new(format!("{}  {}", icons::FLOPPY_DISK, ui.t("script.panel.save"))),
                    )
                    .clicked()
                {
                    let draft = st.drafts.get(&name).cloned().unwrap_or_default();
                    match project_scripts::write_module(&info.root, &name, &draft) {
                        Ok(()) => {
                            st.drafts.remove(&name);
                            st.error = None;
                            invalidate(ui);
                        }
                        Err(e) => st.error = Some(e),
                    }
                }
                if ui
                    .add_enabled(
                        dirty,
                        egui::Button::new(format!("{}  {}", icons::ARROW_COUNTER_CLOCKWISE, ui.t("script.panel.revert"))),
                    )
                    .clicked()
                {
                    st.drafts.remove(&name);
                }
                let delete = if st.confirm_delete {
                    ui.t("script.panel.delete_confirm")
                } else {
                    ui.t("script.panel.delete")
                };
                let button = egui::Button::new(
                    RichText::new(format!("{}  {delete}", icons::TRASH)).color(danger(ui)),
                );
                if ui.add(button).clicked() {
                    if st.confirm_delete {
                        match project_scripts::delete_module(&info.root, &name) {
                            Ok(()) => {
                                st.drafts.remove(&name);
                                st.selected = None;
                                st.error = None;
                                invalidate(ui);
                            }
                            Err(e) => st.error = Some(e),
                        }
                        st.confirm_delete = false;
                    } else {
                        st.confirm_delete = true;
                    }
                }
            });

            // ---- users
            if let Some(story) = ctx.story() {
                let users = project_scripts::module_users(story, &name);
                ui.add_space(6.0);
                ui.label(strong(ui.tn("script.panel.users", users.len(), &[]), 12.0));
                if users.is_empty() && st.confirm_delete {
                    // Nothing to warn about.
                } else if st.confirm_delete {
                    ui.label(
                        RichText::new(ui.t("script.panel.delete_warning"))
                            .small()
                            .color(danger(ui)),
                    );
                }
                let texts = ctx.translator();
                for id in users {
                    let Some(node) = story.node(id) else { continue };
                    let line = format!(
                        "{}  ·  {}",
                        kind_label(&texts, &node.kind),
                        truncate(&summary(&texts, story, node), 50)
                    );
                    if ui.link(line).clicked() {
                        reveal_node = Some(id);
                    }
                }
            }
        });
    ui.data_mut(|d| d.insert_temp(state_id, st));
    if let Some(id) = reveal_node {
        reveal(ctx, Location::Node(id));
    }
}
