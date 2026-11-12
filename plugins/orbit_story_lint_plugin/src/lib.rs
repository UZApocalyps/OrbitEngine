//! "Story Analysis": a read-only panel listing what the editor's validation
//! does not: variables nothing reads, declared variables nothing writes,
//! characters nothing uses; plus an export of the scene graph as a Mermaid
//! `flowchart` (clipboard or a file the user names).
//!
//! * `analysis`: the pure computation (no egui), unit tested;
//! * this file: the panel and the export.
//!
//! Unreachable nodes, scenes that end the game and undefined variables are
//! already reported by the Problems panel (`orbit_core::validate`) and are
//! not repeated here. The plugin never writes to the project.

mod analysis;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use orbit_editor_sdk::native::{NativePlugin, PluginHost};
use orbit_editor_sdk::{
    ActionDef, DockSlot, EditorContext, EditorRequest, Location, PanelDef, egui, icons,
};

use analysis::Analysis;

const PANEL: &str = "orbit.story_lint.analysis";

struct Cache {
    revision: u64,
    root: PathBuf,
    analysis: Analysis,
}

#[derive(Default)]
struct State {
    cache: RefCell<Option<Cache>>,
    /// Destination of the Mermaid export.
    path: RefCell<String>,
}

fn refresh(state: &State, ctx: &EditorContext<'_>) {
    let (Some(project), Some(info)) = (ctx.project(), ctx.project_info()) else {
        *state.cache.borrow_mut() = None;
        return;
    };
    let revision = project.revision();
    let fresh = state
        .cache
        .borrow()
        .as_ref()
        .is_some_and(|c| c.revision == revision && c.root == info.root);
    if !fresh {
        *state.cache.borrow_mut() = Some(Cache {
            revision,
            root: info.root.clone(),
            analysis: analysis::analyze(project.story()),
        });
    }
}

#[derive(Clone, Copy)]
enum Export {
    File,
    Clipboard,
}

fn export(state: &State, ctx: &mut EditorContext<'_>, how: Export, egui_ctx: &egui::Context) {
    let text = {
        let cache = state.cache.borrow();
        let Some(cache) = cache.as_ref() else { return };
        analysis::to_mermaid(&cache.analysis.graph)
    };
    if matches!(how, Export::Clipboard) {
        egui_ctx.copy_text(text);
        let message = ctx.t("storylint.copied");
        ctx.info(message);
        return;
    }
    let path = PathBuf::from(state.path.borrow().trim());
    if path.as_os_str().is_empty() {
        let message = ctx.t("storylint.no_path");
        ctx.warn(message);
        return;
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, text) {
        Ok(()) => {
            let message = ctx.tf("storylint.saved", &[("path", &path.display())]);
            ctx.info(message);
        }
        Err(e) => {
            let message = ctx.tf("storylint.save_failed", &[("path", &path.display()), ("error", &e)]);
            ctx.error(message);
        }
    }
}

/// A titled list; each entry is a button that reveals it in the editor.
fn section(
    ui: &mut egui::Ui,
    title: String,
    help: String,
    empty: String,
    entries: Vec<(String, Location)>,
    reveal: &mut Option<Location>,
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.strong(format!("{title} ({})", entries.len()));
    });
    ui.small(help);
    if entries.is_empty() {
        ui.label(egui::RichText::new(empty).weak());
    }
    for (label, location) in entries {
        if ui.link(label).clicked() {
            *reveal = Some(location);
        }
    }
}

fn panel(ui: &mut egui::Ui, ctx: &mut EditorContext<'_>, state: &State) {
    refresh(state, ctx);
    let mut reveal: Option<Location> = None;
    let mut do_export: Option<Export> = None;
    {
        let cache = state.cache.borrow();
        let Some(cache) = cache.as_ref() else {
            ui.label(ctx.t("storylint.no_project"));
            return;
        };
        let a = &cache.analysis;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.small(ctx.t("storylint.scope"));
            if a.issue_count() == 0 {
                ui.label(format!("{} {}", icons::CHECK_CIRCLE, ctx.t("storylint.all_good")));
            }
            let var_entries = |list: &[analysis::VarIssue]| -> Vec<(String, Location)> {
                list.iter()
                    .map(|v| {
                        let label = if v.declared {
                            v.name.clone()
                        } else {
                            ctx.tf("storylint.undeclared", &[("name", &v.name)])
                        };
                        (label, Location::Variable(v.name.clone()))
                    })
                    .collect()
            };
            section(
                ui,
                ctx.t("storylint.never_read"),
                ctx.t("storylint.never_read_help"),
                ctx.t("storylint.none"),
                var_entries(&a.never_read),
                &mut reveal,
            );
            section(
                ui,
                ctx.t("storylint.never_written"),
                ctx.t("storylint.never_written_help"),
                ctx.t("storylint.none"),
                var_entries(&a.never_written),
                &mut reveal,
            );
            let characters = a
                .unused_characters
                .iter()
                .map(|c| {
                    let name = if c.name.is_empty() { "?".to_owned() } else { c.name.clone() };
                    (name, Location::Character(c.id))
                })
                .collect();
            section(
                ui,
                ctx.t("storylint.unused_characters"),
                ctx.t("storylint.unused_characters_help"),
                ctx.t("storylint.none"),
                characters,
                &mut reveal,
            );

            ui.add_space(8.0);
            ui.separator();
            ui.strong(ctx.t("storylint.graph"));
            let unreachable = a.graph.scenes.iter().filter(|s| !s.reachable).count();
            ui.label(ctx.tf(
                "storylint.graph_summary",
                &[("scenes", &a.graph.scenes.len()), ("links", &a.graph.edges.len()), ("unreachable", &unreachable)],
            ));
            {
                let mut path = state.path.borrow_mut();
                ui.add(
                    egui::TextEdit::singleline(&mut *path)
                        .hint_text(ctx.t("storylint.path_hint"))
                        .desired_width(f32::INFINITY),
                );
            }
            ui.horizontal(|ui| {
                let enabled = !a.graph.scenes.is_empty();
                if ui
                    .add_enabled(enabled, egui::Button::new(format!("{} {}", icons::FLOW_ARROW, ctx.t("storylint.save_mermaid"))))
                    .clicked()
                {
                    do_export = Some(Export::File);
                }
                if ui
                    .add_enabled(enabled, egui::Button::new(format!("{} {}", icons::COPY, ctx.t("storylint.copy_mermaid"))))
                    .clicked()
                {
                    do_export = Some(Export::Clipboard);
                }
            });
        });
    }
    if let Some(location) = reveal {
        ctx.reveal(location);
    }
    if let Some(how) = do_export {
        export(state, ctx, how, &ui.ctx().clone());
    }
}

#[derive(Default)]
pub struct StoryLintPlugin {
    state: Rc<State>,
}

impl NativePlugin for StoryLintPlugin {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        let state = self.state.clone();
        *state.path.borrow_mut() = host.data_dir().join("scenes.mmd").display().to_string();
        let st = state.clone();
        host.panel(PanelDef::new(
            "analysis",
            "storylint.title",
            icons::MAGNIFYING_GLASS,
            DockSlot::Bottom,
            move |ui, ctx| panel(ui, ctx, &st),
        ));
        host.action(
            ActionDef::new("open", "storylint.open", |ctx| {
                ctx.request(EditorRequest::ShowPanel(PANEL.to_owned()));
            })
            .icon(icons::MAGNIFYING_GLASS)
            .category("category.tools")
            .menu("tools", 911),
        );
        Ok(())
    }
}

orbit_editor_sdk::export_plugin!(
    StoryLintPlugin,
    id = "orbit.story_lint",
    name = "Story Analysis",
    description = "Read-only analysis panel: unused or never-written variables, unused characters, and a Mermaid export of the scene graph.",
    author = "OrbitEngine contributors",
);
