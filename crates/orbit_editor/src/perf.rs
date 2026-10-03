//! Headless reference benchmarks (TL-04): `orbit-editor --bench <out.json>`,
//! driven by `cargo xtask bench`, which compares the results with a
//! baseline. Each scenario is timed over a few runs after a warm-up; the
//! median is the number to watch. The editor scenarios also record the
//! profiler's most expensive spans, so a regression points at its cause.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_core::{AssetId, NodeKind, Story};
use orbit_editor_api::{EditorContext, Lang, Level};
use orbit_project::Command;
use orbit_render2d::Sprite2d;
use orbit_render2d::bench::{CpuBatcher, texture};
use serde_json::{Value, json};

use crate::core::{CoreOptions, EditorCore};

/// Nodes of the large story.
pub const NODES: usize = 10_000;

/// One measured scenario.
#[derive(Clone, Debug)]
pub struct Measure {
    pub name: &'static str,
    pub runs: Vec<Duration>,
    /// What one run does (shown in the report).
    pub detail: String,
    /// The profiler's top spans during the runs (editor scenarios).
    pub top: Vec<orbit_profile::ScopeStat>,
}

impl Measure {
    fn ms(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self.runs.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        v.sort_by(f64::total_cmp);
        v
    }

    pub fn median_ms(&self) -> f64 {
        let v = self.ms();
        if v.is_empty() { 0.0 } else { v[v.len() / 2] }
    }

    pub fn min_ms(&self) -> f64 {
        self.ms().first().copied().unwrap_or(0.0)
    }

    pub fn max_ms(&self) -> f64 {
        self.ms().last().copied().unwrap_or(0.0)
    }

    fn to_json(&self) -> Value {
        json!({
            "median_ms": round(self.median_ms()),
            "min_ms": round(self.min_ms()),
            "max_ms": round(self.max_ms()),
            "runs": self.runs.len(),
            "detail": self.detail,
            "top_scopes": self.top.iter().take(8).map(|s| json!({
                "name": s.name,
                "mean_ms": round(s.mean_ms),
                "calls": round(s.calls),
            })).collect::<Vec<_>>(),
        })
    }
}

fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Runs `f` once to warm up, then `runs` times (one profiler frame each).
fn time(runs: usize, mut f: impl FnMut()) -> Vec<Duration> {
    f();
    orbit_profile::clear();
    (0..runs)
        .map(|_| {
            orbit_profile::begin_frame();
            let t = Instant::now();
            f();
            let elapsed = t.elapsed();
            orbit_profile::finish_frame();
            elapsed
        })
        .collect()
}

/// A scenario timed with [`time`], with the spans of its runs.
fn measure(name: &'static str, detail: impl Into<String>, runs: Vec<Duration>) -> Measure {
    Measure {
        name,
        runs,
        detail: detail.into(),
        top: top_since(0),
    }
}

/// Plays `story` from its entry to the end, always taking the first option.
/// Returns the number of steps.
pub fn play_through(story: &Arc<Story>) -> usize {
    use orbit_vm::{StepResult, Vm};
    let Some(entry) = story.entry() else {
        return 0;
    };
    let Ok(mut vm) = Vm::new(story.clone(), entry) else {
        return 0;
    };
    let mut steps = 0;
    loop {
        steps += 1;
        match vm.step() {
            StepResult::Choice { .. } => {
                if vm.choose(0).is_err() {
                    break;
                }
            }
            StepResult::End | StepResult::Error(_) => break,
            _ => {}
        }
        if steps > 1_000_000 {
            break;
        }
    }
    steps
}

/// 10 000 sprites over 64 textures, half of them in runs of equal textures
/// (what text and UI produce), on 4 layers.
fn sprites() -> Vec<Sprite2d> {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    (0..10_000u32)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let tex = if i % 2 == 0 {
                (i / 50) % 64
            } else {
                (seed % 64) as u32
            };
            Sprite2d::new(
                texture(tex),
                [(i % 1920) as f32, (i / 8) as f32],
                [16.0, 16.0],
            )
            .layer(orbit_render2d::Layer((i % 4) as u8))
            .z((i / 500) as f32)
            .rotation((i % 7) as f32 * 0.1)
        })
        .collect()
}

fn frame(core: &mut EditorCore, ctx: &Context, time: f64) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1360.0, 820.0))),
        time: Some(time),
        ..RawInput::default()
    };
    ctx.run_ui(input, |ui| core.ui(ui))
        .drop_without_applying_deltas();
}

/// Frames until `done` (at most `max`).
fn frames_until(
    core: &mut EditorCore,
    ctx: &Context,
    t: &mut f64,
    max: Duration,
    mut done: impl FnMut(&EditorCore) -> bool,
) -> bool {
    let started = Instant::now();
    while started.elapsed() < max {
        orbit_profile::begin_frame();
        frame(core, ctx, *t);
        orbit_profile::finish_frame();
        *t += 0.016;
        if done(core) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

fn top_since(start: usize) -> Vec<orbit_profile::ScopeStat> {
    let frames = orbit_profile::frames(orbit_profile::HISTORY);
    let n = frames.len().saturating_sub(start.min(frames.len()));
    orbit_profile::aggregate(&frames[frames.len() - n..])
}

/// The editor on a 10 000-node project: opening, an idle frame, and an edit
/// that breaks a node until its error is shown (the latency test).
fn editor(dir: &Path, quick: bool) -> Result<Vec<Measure>, String> {
    let texts = crate::i18n::translator(Lang::En);
    let root = crate::storage::create_project(dir, "Bench", &texts).map_err(|e| e.to_string())?;
    let story = crate::bench::generate_story(NODES);
    std::fs::write(
        root.join("story.orbit"),
        story.to_ron().map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    let ctx = Context::default();
    let mut core = EditorCore::new(CoreOptions::with_language(Lang::En), &ctx);
    let mut t = 0.0;
    let opened = Instant::now();
    core.open_project(&root);
    let ready = frames_until(&mut core, &ctx, &mut t, Duration::from_secs(60), |c| {
        c.state.diagnostics.revision == 0 && !c.state.diagnostics.problems.is_empty()
    });
    if !ready {
        return Err("the first validation of the 10k story never arrived".into());
    }
    let open = vec![opened.elapsed()];
    let mut out = vec![measure(
        "editor_open_10k",
        "open a 10k-node project until its first findings are shown",
        open,
    )];

    // Idle frames (the scene panel shows the 10k scene).
    orbit_profile::clear();
    let n = if quick { 20 } else { 60 };
    let idle = time(n, || {
        orbit_profile::begin_frame();
        frame(&mut core, &ctx, t);
        orbit_profile::finish_frame();
        t += 0.016;
    });
    let mut m = measure(
        "editor_frame_10k",
        "one headless editor frame, 10k-node project",
        idle,
    );
    m.top = top_since(0);
    out.push(m);

    // Edit -> error visible, after a pause (as in the latency test).
    orbit_profile::clear();
    let runs = if quick { 3 } else { 8 };
    // Dialogue lines spread over the scene (nothing targets them).
    let lines: Vec<_> = core
        .story()
        .ok_or("no story")?
        .scenes[0]
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Say { .. }))
        .map(|n| n.id)
        .collect();
    let mut latencies = Vec::new();
    for i in 0..runs {
        std::thread::sleep(Duration::from_millis(200));
        let target = lines[(i + 1) * lines.len() / (runs + 1)];
        let Some(session) = core.session.as_mut() else {
            return Err("no session".into());
        };
        let edit = Instant::now();
        {
            let mut ectx =
                EditorContext::new(Some(&mut session.project), &mut core.state, &core.registry);
            ectx.exec(Command::SetNodeKind {
                id: target,
                kind: NodeKind::Background {
                    asset: AssetId::new(),
                    transition: None,
                },
                expected_version: None,
            })
            .map_err(|e| e.to_string())?;
        }
        let shown = frames_until(&mut core, &ctx, &mut t, Duration::from_secs(10), |c| {
            c.state
                .diagnostics
                .of(target)
                .iter()
                .any(|(l, _)| *l == Level::Error)
        });
        if !shown {
            return Err("the error never appeared".into());
        }
        latencies.push(edit.elapsed());
        if std::env::var_os("ORBIT_BENCH_DEBUG").is_some() {
            eprintln!("edit {i}: {:?}", edit.elapsed());
            for f in orbit_profile::frames(40) {
                if f.cpu_ms > 10.0 {
                    let spans: Vec<String> = f
                        .scopes
                        .iter()
                        .filter(|s| s.duration_ms > 2.0)
                        .map(|s| format!("{}={:.1}", s.name, s.duration_ms))
                        .collect();
                    eprintln!(
                        "  frame {} {:.1} ms: {}",
                        f.index,
                        f.cpu_ms,
                        spans.join(" ")
                    );
                }
            }
        }
    }
    let mut m = measure(
        "editor_edit_to_error_10k",
        "edit a node of a 10k-node story until its error is shown",
        latencies,
    );
    m.top = top_since(0);
    out.push(m);
    Ok(out)
}

/// Runs every scenario. `quick` uses fewer runs (CI).
pub fn run(quick: bool) -> Result<Vec<Measure>, String> {
    let runs = if quick { 3 } else { 10 };
    let mut out = Vec::new();

    let story = crate::bench::generate_story(NODES);
    let text = story.to_ron().map_err(|e| e.to_string())?;
    let nodes = crate::diagnostics::node_count(&story);
    out.push(measure(
        "story_load_10k",
        format!(
            "parse + validate a {nodes}-node story ({} KB of RON)",
            text.len() / 1024
        ),
        time(runs, || {
            let s = Story::from_ron(&text).expect("the generated story loads");
            std::hint::black_box(s);
        }),
    ));
    out.push(measure(
        "story_save_10k",
        format!("serialize a {nodes}-node story"),
        time(runs, || {
            std::hint::black_box(story.to_ron().ok());
        }),
    ));

    let shared = Arc::new(story.clone());
    let steps = play_through(&shared);
    out.push(measure(
        "vm_play_10k",
        format!("play a {nodes}-node story to the end ({steps} steps)"),
        time(runs, || {
            std::hint::black_box(play_through(&shared));
        }),
    ));

    let texts = crate::i18n::translator(Lang::En);
    out.push(measure(
        "validate_10k",
        format!("editor validation (findings and messages) of a {nodes}-node story"),
        time(runs, || {
            std::hint::black_box(crate::diagnostics::compute(
                &texts,
                &story,
                1,
                &crate::diagnostics::Extras::default(),
            ));
        }),
    ));

    let mut index = crate::search::SearchIndex::default();
    let mut key = 0;
    out.push(measure(
        "search_index_10k",
        format!("build the search index of a {nodes}-node story"),
        time(runs, || {
            key += 1;
            index.refresh(&story, (1, key), &texts);
        }),
    ));

    let bundle = orbit_ui::asset::UiBundle::builtin();
    let theme = bundle.theme.clone().unwrap_or_default();
    let ui_texts = orbit_ui::Texts::builtin("en");
    let ctx = orbit_ui::asset::DataContext {
        show_quit: true,
        ..Default::default()
    };
    out.push(measure(
        "ui_screens_layout",
        format!(
            "build and lay out the {} stock data screens",
            bundle.screens.len()
        ),
        time(runs * 3, || {
            for s in &bundle.screens {
                let mut screen =
                    orbit_ui::DataScreen::new(s.clone(), &theme, &ui_texts, ctx.clone());
                screen.ui().layout();
                std::hint::black_box(screen.ui().focused());
            }
        }),
    ));

    let sprites = sprites();
    let mut batcher = CpuBatcher::default();
    let draws = batcher.build(&sprites);
    out.push(measure(
        "batch_2d_10k",
        format!("sort and batch 10 000 sprites ({draws} draw calls)"),
        time(runs * 3, || {
            std::hint::black_box(batcher.build(&sprites));
        }),
    ));

    let dir = std::env::temp_dir().join(format!("orbit-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let editor = editor(&dir, quick);
    let _ = std::fs::remove_dir_all(&dir);
    out.extend(editor?);
    Ok(out)
}

/// The JSON report written by `--bench`.
pub fn report(measures: &[Measure]) -> Value {
    let scenarios: serde_json::Map<String, Value> = measures
        .iter()
        .map(|m| (m.name.to_owned(), m.to_json()))
        .collect();
    json!({
        "format": 1,
        "build": if cfg!(debug_assertions) { "debug" } else { "release" },
        "os": std::env::consts::OS,
        "scenarios": scenarios,
    })
}

/// `--bench <out>`: runs, prints a table, writes the report.
pub fn run_to_file(out: &Path) -> Result<(), String> {
    let quick = std::env::var_os("ORBIT_BENCH_QUICK").is_some();
    let measures = run(quick)?;
    for m in &measures {
        println!(
            "{:<26} median {:>9.3} ms   min {:>9.3}   max {:>9.3}   {}",
            m.name,
            m.median_ms(),
            m.min_ms(),
            m.max_ms(),
            m.detail
        );
        for s in m.top.iter().take(5) {
            println!(
                "    {:<40} {:>8.3} ms/frame  x{:.1}",
                s.name, s.mean_ms, s.calls
            );
        }
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&report(&measures)).map_err(|e| e.to_string())?;
    std::fs::write(out, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_large_story_plays_to_the_end() {
        let story = Arc::new(crate::bench::generate_story(500));
        assert!(play_through(&story) > 100);
    }

    #[test]
    fn sprites_batch_into_fewer_draws_than_sprites() {
        let s = sprites();
        let draws = CpuBatcher::default().build(&s);
        assert!(draws > 10 && draws < s.len(), "{draws}");
    }

    #[test]
    fn the_report_has_one_entry_per_scenario() {
        let m = measure(
            "x",
            "d",
            vec![Duration::from_millis(2), Duration::from_millis(4)],
        );
        let r = report(&[m]);
        assert_eq!(r["scenarios"]["x"]["median_ms"], 4.0);
        assert_eq!(r["format"], 1);
    }
}
