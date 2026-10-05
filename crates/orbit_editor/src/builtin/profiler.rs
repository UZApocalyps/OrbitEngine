//! Profiler panel (TL-04): frame times of the editor (CPU, and GPU when the
//! device has timestamp queries), the most expensive spans and the GPU
//! passes, from `orbit_profile`.

use egui::{Color32, Grid, RichText, Sense, Stroke, Ui, pos2, vec2};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, PanelDef, Plugin, PluginManifest, icons,
};

use super::ids;
use super::ui_util::{faint, icon_button, weak};

pub struct ProfilerPlugin;

/// Frames shown in the graph and summarized in the tables.
const WINDOW: usize = 240;
/// Spans listed.
const TOP: usize = 20;

impl Plugin for ProfilerPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::PROFILER, "Profiler", "0.1.0")
    }
}

impl EditorPlugin for ProfilerPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "profiler",
            "profiler.title",
            icons::GAUGE,
            DockSlot::Hidden,
            panel,
        )
        .opens_in(DockSlot::Bottom));
    }
}

fn dim(ui: &mut Ui, text: String, fainter: bool) {
    let color = if fainter { faint(ui) } else { weak(ui) };
    ui.label(RichText::new(text).color(color));
}

/// Mean, 95th percentile and worst of `values`.
pub fn summary(values: &[f64]) -> Option<(f64, f64, f64)> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    let p95 = sorted[((sorted.len() - 1) as f64 * 0.95).round() as usize];
    Some((mean, p95, sorted[sorted.len() - 1]))
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    if !orbit_profile::COMPILED {
        dim(ui, ctx.t("profiler.compiled_out"), false);
        return;
    }
    let mut recording = orbit_profile::is_enabled();
    ui.horizontal(|ui| {
        if ui
            .toggle_value(
                &mut recording,
                format!("{}  {}", icons::RECORD, ctx.t("profiler.record")),
            )
            .changed()
        {
            orbit_profile::set_enabled(recording);
        }
        let mut gpu = orbit_gfx::gpu_timing();
        if ui
            .checkbox(&mut gpu, ctx.t("profiler.gpu_timing"))
            .on_hover_text(ctx.t("profiler.gpu_timing_hint"))
            .changed()
        {
            orbit_gfx::set_gpu_timing(gpu);
        }
        if icon_button(ui, icons::BROOM, &ctx.t("profiler.clear")).clicked() {
            orbit_profile::clear();
        }
    });
    // The editor only draws on input: keep a slow pulse while the panel is
    // open so that the numbers stay current.
    if recording {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
    }

    let frames = orbit_profile::frames(WINDOW);
    // The first frame after a pause or a clear has no meaningful duration.
    let times: Vec<(f64, Option<f64>)> = frames
        .iter()
        .skip(1)
        .map(|f| (f.cpu_ms, f.gpu_ms()))
        .collect();
    if times.is_empty() {
        dim(ui, ctx.t("profiler.empty"), true);
        return;
    }
    let cpu: Vec<f64> = times.iter().map(|t| t.0).collect();
    let gpu: Vec<f64> = times.iter().filter_map(|t| t.1).collect();
    if let Some((mean, p95, max)) = summary(&cpu) {
        ui.label(ctx.tf(
            "profiler.cpu_summary",
            &[
                ("mean", &format!("{mean:.1}")),
                ("p95", &format!("{p95:.1}")),
                ("max", &format!("{max:.1}")),
            ],
        ));
    }
    match summary(&gpu) {
        Some((mean, p95, max)) => {
            ui.label(ctx.tf(
                "profiler.gpu_summary",
                &[
                    ("mean", &format!("{mean:.2}")),
                    ("p95", &format!("{p95:.2}")),
                    ("max", &format!("{max:.2}")),
                ],
            ));
        }
        None => {
            dim(ui, ctx.t("profiler.gpu_unavailable"), true);
        }
    }
    graph(ui, &times);

    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.label(RichText::new(ctx.t("profiler.scopes")).strong());
            let stats = orbit_profile::aggregate(&frames);
            Grid::new("orbit_profiler_scopes")
                .striped(true)
                .num_columns(4)
                .show(ui, |ui| {
                    dim(ui, ctx.t("profiler.scope"), false);
                    dim(ui, ctx.t("profiler.mean"), false);
                    dim(ui, ctx.t("profiler.max"), false);
                    dim(ui, ctx.t("profiler.calls"), false);
                    ui.end_row();
                    for s in stats.iter().take(TOP) {
                        ui.label(s.name);
                        ui.label(format!("{:.2}", s.mean_ms));
                        ui.label(format!("{:.2}", s.max_ms));
                        ui.label(format!("{:.1}", s.calls));
                        ui.end_row();
                    }
                });
            let passes = orbit_profile::aggregate_gpu(&frames);
            if !passes.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(ctx.t("profiler.gpu_passes")).strong());
                Grid::new("orbit_profiler_gpu")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (name, ms) in passes {
                            ui.label(name);
                            ui.label(format!("{ms:.3}"));
                            ui.end_row();
                        }
                    });
            }
        });
}

/// Bars of CPU time per frame, GPU time as a line, guides at 60 and 30 fps.
fn graph(ui: &mut Ui, times: &[(f64, Option<f64>)]) {
    let height = 90.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let worst = times.iter().map(|t| t.0).fold(0.0, f64::max);
    let scale = worst.max(34.0) as f32;
    let y_of = |ms: f64| rect.bottom() - (ms as f32 / scale).min(1.0) * (height - 4.0);
    for (ms, color) in [
        (1000.0 / 60.0, Color32::from_rgb(90, 170, 110)),
        (1000.0 / 30.0, Color32::from_rgb(220, 150, 60)),
    ] {
        let y = y_of(ms);
        painter.hline(
            rect.x_range(),
            y,
            Stroke::new(1.0, color.gamma_multiply(0.6)),
        );
    }
    let n = WINDOW.max(times.len()) as f32;
    let w = rect.width() / n;
    let x0 = rect.right() - times.len() as f32 * w;
    let bar = visuals.selection.bg_fill;
    let mut gpu_line = Vec::new();
    for (i, (cpu, gpu)) in times.iter().enumerate() {
        let x = x0 + i as f32 * w;
        painter.rect_filled(
            egui::Rect::from_min_max(pos2(x, y_of(*cpu)), pos2(x + w.max(1.0), rect.bottom())),
            0.0,
            bar,
        );
        if let Some(g) = gpu {
            gpu_line.push(pos2(x + w * 0.5, y_of(*g)));
        }
    }
    if gpu_line.len() > 1 {
        painter.add(egui::Shape::line(
            gpu_line,
            Stroke::new(1.5, Color32::from_rgb(230, 90, 160)),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn summary_gives_mean_p95_and_max() {
        let values: Vec<f64> = (1..=100).map(f64::from).collect();
        let (mean, p95, max) = summary(&values).unwrap();
        assert_eq!(mean, 50.5);
        assert_eq!(p95, 95.0);
        assert_eq!(max, 100.0);
        assert!(summary(&[]).is_none());
    }
}
