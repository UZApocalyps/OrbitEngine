//! "Story Statistics": a read-only docked panel with the numbers of the
//! story (scenes, steps, words and lines per scene and per character,
//! choices, estimated reading time, voice coverage) and a CSV export.
//!
//! * `stats`: the pure computation (no egui), unit tested;
//! * this file: the panel, the reading speed setting (stored in the editor's
//!   per-plugin preferences) and the export.
//!
//! The plugin never writes to the project: the CSV goes to the clipboard or
//! to a file the user names (by default in the plugin's own data folder).

mod stats;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use orbit_editor_sdk::native::{NativePlugin, PluginHost};
use orbit_editor_sdk::{
    ActionDef, DockSlot, EditorContext, EditorRequest, PanelDef, SettingsDef, egui, icons,
};

use stats::{DEFAULT_WPM, MAX_WPM, MIN_WPM, Speaker, StoryStats};

/// The plugin id (also in `manifest.ron`).
pub const ID: &str = "orbit.story_stats";
const PANEL: &str = "orbit.story_stats.stats";
const WPM_KEY: &str = "wpm";

/// Statistics computed for one revision of one project.
struct Cache {
    revision: u64,
    root: PathBuf,
    stats: StoryStats,
}

#[derive(Default)]
struct State {
    cache: RefCell<Option<Cache>>,
    /// Destination of the CSV export.
    path: RefCell<String>,
}

fn wpm(ctx: &EditorContext<'_>) -> u32 {
    ctx.pref(ID, WPM_KEY)
        .and_then(|v| v.parse::<u32>().ok())
        .map_or(DEFAULT_WPM, stats::clamp_wpm)
}

/// Statistics of the open project, recomputed only when its revision (or the
/// project) changed.
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
            stats: stats::compute(project.story()),
        });
    }
}

fn row(ui: &mut egui::Ui, label: String, value: String) {
    ui.label(label);
    ui.label(egui::RichText::new(value).strong());
    ui.end_row();
}

fn speaker_name(ctx: &EditorContext<'_>, c: &stats::CharacterStats) -> String {
    match c.speaker {
        Speaker::Narrator => ctx.t("storystats.narrator"),
        Speaker::Character(_) if c.name.is_empty() => "?".to_owned(),
        Speaker::Character(_) => c.name.clone(),
    }
}

fn export(state: &State, ctx: &mut EditorContext<'_>, to_clipboard: bool, egui_ctx: &egui::Context) {
    let csv = {
        let cache = state.cache.borrow();
        let Some(cache) = cache.as_ref() else { return };
        stats::to_csv(&cache.stats, wpm(ctx))
    };
    if to_clipboard {
        egui_ctx.copy_text(csv);
        let message = ctx.t("storystats.copied");
        ctx.info(message);
        return;
    }
    let path = PathBuf::from(state.path.borrow().trim());
    if path.as_os_str().is_empty() {
        let message = ctx.t("storystats.no_path");
        ctx.warn(message);
        return;
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, csv) {
        Ok(()) => {
            let message = ctx.tf("storystats.saved", &[("path", &path.display())]);
            ctx.info(message);
        }
        Err(e) => {
            let message = ctx.tf("storystats.save_failed", &[("path", &path.display()), ("error", &e)]);
            ctx.error(message);
        }
    }
}

fn panel(ui: &mut egui::Ui, ctx: &mut EditorContext<'_>, state: &State) {
    refresh(state, ctx);
    let wpm = wpm(ctx);
    let mut action: Option<bool> = None; // Some(to_clipboard)
    {
        let cache = state.cache.borrow();
        let Some(cache) = cache.as_ref() else {
            ui.label(ctx.t("storystats.no_project"));
            return;
        };
        let s = &cache.stats;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.strong(ctx.t("storystats.overview"));
            egui::Grid::new("storystats.overview").num_columns(2).show(ui, |ui| {
                row(ui, ctx.t("storystats.scenes"), s.scenes.len().to_string());
                row(ui, ctx.t("storystats.steps"), s.steps.to_string());
                row(ui, ctx.t("storystats.lines"), s.lines.to_string());
                row(ui, ctx.t("storystats.words"), s.words.to_string());
                row(ui, ctx.t("storystats.choices"), s.choices.to_string());
                row(ui, ctx.t("storystats.options"), s.options.to_string());
                row(
                    ui,
                    ctx.tf("storystats.reading_time", &[("wpm", &wpm)]),
                    stats::format_duration(stats::reading_seconds(s.words, wpm)),
                );
            });

            ui.add_space(6.0);
            ui.strong(ctx.t("storystats.voice"));
            if s.lines == 0 {
                ui.label(ctx.t("storystats.no_lines"));
            } else {
                ui.add(
                    egui::ProgressBar::new(s.voice_coverage())
                        .text(ctx.tf(
                            "storystats.voice_summary",
                            &[("voiced", &s.voiced), ("total", &s.lines), ("missing", &s.unvoiced())],
                        ))
                        .desired_width(ui.available_width().min(360.0)),
                );
            }

            ui.add_space(6.0);
            ui.collapsing(ctx.t("storystats.by_scene"), |ui| {
                egui::Grid::new("storystats.scenes").striped(true).show(ui, |ui| {
                    for h in ["scene", "steps", "lines", "words", "choices", "voiced"] {
                        ui.strong(ctx.t(&format!("storystats.col.{h}")));
                    }
                    ui.end_row();
                    for sc in &s.scenes {
                        ui.label(&sc.name);
                        ui.label(sc.steps.to_string());
                        ui.label(sc.lines.to_string());
                        ui.label(sc.words.to_string());
                        ui.label(format!("{} ({})", sc.choices, sc.options));
                        ui.label(format!("{}/{}", sc.voiced, sc.lines));
                        ui.end_row();
                    }
                });
            });
            ui.collapsing(ctx.t("storystats.by_character"), |ui| {
                egui::Grid::new("storystats.characters").striped(true).show(ui, |ui| {
                    for h in ["character", "lines", "words", "voiced"] {
                        ui.strong(ctx.t(&format!("storystats.col.{h}")));
                    }
                    ui.end_row();
                    for c in &s.characters {
                        ui.label(speaker_name(ctx, c));
                        ui.label(c.lines.to_string());
                        ui.label(c.words.to_string());
                        ui.label(format!("{}/{}", c.voiced, c.lines));
                        ui.end_row();
                    }
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.strong(ctx.t("storystats.export"));
            {
                let mut path = state.path.borrow_mut();
                ui.add(
                    egui::TextEdit::singleline(&mut *path)
                        .hint_text(ctx.t("storystats.path_hint"))
                        .desired_width(f32::INFINITY),
                );
            }
            ui.horizontal(|ui| {
                if ui.button(format!("{} {}", icons::FILE_CSV, ctx.t("storystats.save_csv"))).clicked() {
                    action = Some(false);
                }
                if ui.button(format!("{} {}", icons::COPY, ctx.t("storystats.copy_csv"))).clicked() {
                    action = Some(true);
                }
            });
        });
    }
    if let Some(to_clipboard) = action {
        export(state, ctx, to_clipboard, &ui.ctx().clone());
    }
}

#[derive(Default)]
pub struct StoryStatsPlugin {
    state: Rc<State>,
}

impl NativePlugin for StoryStatsPlugin {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        let state = self.state.clone();
        *state.path.borrow_mut() = host.data_dir().join("story_stats.csv").display().to_string();

        let st = state.clone();
        host.panel(PanelDef::new(
            "stats",
            "storystats.title",
            icons::CHART_BAR,
            DockSlot::Right,
            move |ui, ctx| panel(ui, ctx, &st),
        ));
        host.action(
            ActionDef::new("open", "storystats.open", |ctx| {
                ctx.request(EditorRequest::ShowPanel(PANEL.to_owned()));
            })
            .icon(icons::CHART_BAR)
            .category("category.tools")
            .menu("tools", 910),
        );
        host.settings_page(SettingsDef::new("storystats.settings_title", |ui, ctx| {
            ui.label(ctx.t("storystats.wpm_label"));
            let mut value = wpm(ctx);
            if ui
                .add(egui::DragValue::new(&mut value).range(MIN_WPM..=MAX_WPM).suffix(" wpm"))
                .changed()
            {
                ctx.set_pref(ID, WPM_KEY, value.to_string());
            }
            ui.small(ctx.t("storystats.wpm_help"));
        }));
        Ok(())
    }
}

orbit_editor_sdk::export_plugin!(
    StoryStatsPlugin,
    id = "orbit.story_stats",
    name = "Story Statistics",
    description = "Read-only statistics panel: scenes, steps, words and lines per scene and per character, choices, estimated reading time, voice coverage, CSV export.",
    author = "OrbitEngine contributors",
);
