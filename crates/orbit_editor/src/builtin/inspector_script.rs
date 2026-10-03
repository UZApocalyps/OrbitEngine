//! Rhai scripts in the inspector (ADR 0015): the `Script` node (code,
//! branches, error output, "Run in sandbox") and the Rhai mode of a
//! condition (`If`, choice option). Also the pieces the "Scripts" panel
//! shares: the highlighting code editor, the diagnostics list and the cache
//! of the project checks.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Id, RichText, TextEdit, Ui};
use orbit_core::{Expr, NodeId, NodeKind, ScriptBranch, Story};
use orbit_editor_api::fields::{self, label_picker};
use orbit_editor_api::{EditorContext, UiText};
use orbit_project::scripts::{ScriptProblem, ScriptSite};

use super::inspector::{commit, new_label_button, tfield, with_new_label};
use super::script_highlight::{char_index_of, layout_job};
use super::ui_util::{danger, faint, strong, success, warning, weak};

/// How long the module files are trusted before being read again.
const RECHECK: Duration = Duration::from_secs(1);

// ----------------------------------------------------------------- checks

/// The checks of every Rhai source of the project, computed once per story
/// revision (and when a module file changes), not once per frame.
#[derive(Clone)]
pub(super) struct Checked {
    revision: u64,
    project: PathBuf,
    at: Instant,
    /// Where the module files are read and written.
    pub root: PathBuf,
    pub modules: Arc<BTreeMap<String, String>>,
    pub problems: Arc<Vec<ScriptProblem>>,
}

fn checked_id() -> Id {
    Id::new("orbit_script_checked")
}

/// Forgets the cached checks (a module was saved, created or deleted).
pub(super) fn invalidate(ui: &Ui) {
    ui.data_mut(|d| d.remove::<Checked>(checked_id()));
}

/// The cached checks; `None` without a project.
pub(super) fn checked(ui: &Ui, ctx: &EditorContext<'_>) -> Option<Checked> {
    let story = ctx.story()?;
    let revision = ctx.project()?.revision();
    let info = ctx.project_info()?;
    let now = Instant::now();
    let prev: Option<Checked> = ui.data(|d| d.get_temp(checked_id()));
    if let Some(p) = &prev
        && p.revision == revision
        && p.project == info.root
        && now.duration_since(p.at) < RECHECK
    {
        return prev;
    }
    let root = orbit_project::find_assets_root(&info.root, story);
    let modules = orbit_project::scripts::read_modules(&root);
    let reuse = prev
        .as_ref()
        .filter(|p| p.revision == revision && p.root == root && *p.modules == modules);
    let (modules, problems) = match reuse {
        Some(p) => (p.modules.clone(), p.problems.clone()),
        None => {
            let problems = orbit_project::scripts::check_scripts(story, &modules);
            (Arc::new(modules), Arc::new(problems))
        }
    };
    let out = Checked {
        revision,
        project: info.root.clone(),
        at: now,
        root,
        modules,
        problems,
    };
    ui.data_mut(|d| d.insert_temp(checked_id(), out.clone()));
    Some(out)
}

/// A finding of a script, reduced to what the list shows.
#[derive(Clone, Debug)]
pub(super) struct Finding {
    pub error: bool,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl Finding {
    pub fn of(p: &ScriptProblem) -> Self {
        Self {
            error: p.is_error(),
            line: p.diagnostic.line,
            column: p.diagnostic.column,
            message: p.diagnostic.message.clone(),
        }
    }
}

/// The findings of one site.
pub(super) fn findings(checked: Option<&Checked>, site: &ScriptSite) -> Vec<Finding> {
    checked
        .map(|c| {
            c.problems
                .iter()
                .filter(|p| p.site == *site)
                .map(Finding::of)
                .collect()
        })
        .unwrap_or_default()
}

// ------------------------------------------------------------ code editor

fn jump_id(ui: &Ui, salt: impl fields::Salt) -> Id {
    ui.make_persistent_id(("script_jump", salt))
}

/// A code field with Rhai highlighting. Returns the new text when it was
/// edited. The lines of `findings` that are errors get a red background; a
/// click in the findings list (see [`findings_list`] with the same `salt`,
/// in the same parent) moves the cursor here.
pub(super) fn code_editor(
    ui: &mut Ui,
    salt: impl fields::Salt,
    text: &str,
    findings: &[Finding],
    rows: usize,
) -> Option<String> {
    let id = ui.make_persistent_id(("script_code", salt));
    let error_lines: Vec<usize> = findings
        .iter()
        .filter(|f| f.error && f.line > 0)
        .map(|f| f.line)
        .collect();
    let jump: Option<(usize, usize)> = ui.data_mut(|d| d.remove_temp(jump_id(ui, salt)));
    if let Some((line, column)) = jump {
        let index = char_index_of(text, line, column);
        let ctx = ui.ctx().clone();
        if let Some(mut state) = TextEdit::load_state(&ctx, id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(index),
                )));
            state.store(&ctx, id);
        }
        ui.memory_mut(|m| m.request_focus(id));
    }
    let mut buffer = text.to_owned();
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = layout_job(ui, buf.as_str(), &error_lines);
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let response = ui.add(
        TextEdit::multiline(&mut buffer)
            .id(id)
            .code_editor()
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .lock_focus(true)
            .layouter(&mut layouter),
    );
    response.changed().then_some(buffer)
}

/// The findings under a code field: `line:column  message`; a click puts the
/// cursor of the editor with the same `salt` there. Without findings and
/// with `show_ok`, a green "no problem" line.
pub(super) fn findings_list(
    ui: &mut Ui,
    salt: impl fields::Salt,
    findings: &[Finding],
    show_ok: bool,
) {
    if findings.is_empty() {
        if show_ok {
            ui.label(
                RichText::new(format!(
                    "{}  {}",
                    orbit_editor_api::icons::CHECK_CIRCLE,
                    ui.t("script.no_problem")
                ))
                .small()
                .color(success()),
            );
        }
        return;
    }
    for f in findings {
        let (glyph, color) = if f.error {
            (orbit_editor_api::icons::WARNING_CIRCLE, danger(ui))
        } else {
            (orbit_editor_api::icons::WARNING, warning(ui))
        };
        let place = if f.line > 0 {
            format!("{}:{}  ", f.line, f.column)
        } else {
            String::new()
        };
        let response = ui
            .add(
                egui::Label::new(
                    RichText::new(format!("{glyph}  {place}{}", f.message))
                        .small()
                        .color(color),
                )
                .sense(egui::Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if f.line > 0 && response.clicked() {
            let key = jump_id(ui, salt);
            ui.data_mut(|d| d.insert_temp(key, (f.line, f.column)));
        }
    }
}

// ------------------------------------------------------------ script node

/// What the sandbox run of a node produced, kept while the node is selected.
#[derive(Clone)]
struct Run {
    seed: u64,
    run: orbit_script::TestRun,
}

/// Inspector of a `Script` node.
pub(super) fn script_node(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    source: &str,
    branches: &[ScriptBranch],
    on_error: Option<NodeId>,
) {
    let checked = checked(ui, ctx);
    let found = findings(checked.as_ref(), &ScriptSite::Node(id));
    let rebuild = |source: String, branches: Vec<ScriptBranch>, on_error: Option<NodeId>| {
        NodeKind::Script {
            source,
            branches,
            on_error,
        }
    };

    ui.label(strong(ui.t("script.code"), 13.0));
    if let Some(new) = code_editor(ui, ("node", id), source, &found, 10) {
        commit(ctx, id, rebuild(new, branches.to_vec(), on_error), "source");
    }
    findings_list(ui, ("node", id), &found, true);
    ui.label(
        RichText::new(ui.t("script.api_hint"))
            .small()
            .color(faint(ui)),
    );

    // ---- sandbox
    ui.add_space(6.0);
    let run_id = ui.make_persistent_id(("script_run", id));
    if ui
        .button(format!(
            "{}  {}",
            orbit_editor_api::icons::PLAY,
            ui.t("script.run")
        ))
        .on_hover_text(ui.t("script.run.tip"))
        .clicked()
        && let Some(story) = ctx.story()
    {
        let previous: Option<Run> = ui.data(|d| d.get_temp(run_id));
        let seed = previous.map_or(1, |r| r.seed + 1);
        let modules = checked.as_ref().map(|c| c.modules.clone()).unwrap_or_default();
        let run = orbit_project::scripts::test_script(story, &modules, source, seed);
        ui.data_mut(|d| d.insert_temp(run_id, Run { seed, run }));
    }
    if let Some(run) = ui.data(|d| d.get_temp::<Run>(run_id)) {
        show_run(ui, &run.run);
    }

    // ---- branches
    ui.add_space(8.0);
    ui.label(strong(ui.t("script.branches"), 13.0));
    ui.label(
        RichText::new(ui.t("script.branches.hint"))
            .small()
            .color(weak(ui)),
    );
    enum Do {
        Rename(usize, String),
        Target(usize, NodeId),
        Remove(usize),
        Add,
        OnError(Option<NodeId>),
        NewErrorLabel,
    }
    let mut action = None;
    if let Some(story) = ctx.story() {
        for (i, branch) in branches.iter().enumerate() {
            ui.horizontal(|ui| {
                let mut done = false;
                let hint = ui.t("script.branch.name_hint");
                if let Some(name) = fields::text_field(
                    ui,
                    ("script_branch_name", id, i),
                    &branch.name,
                    &hint,
                    1,
                    &mut done,
                ) {
                    action = Some(Do::Rename(i, name));
                }
                if ui
                    .small_button(orbit_editor_api::icons::TRASH)
                    .on_hover_text(ui.t("script.branch.remove"))
                    .clicked()
                {
                    action = Some(Do::Remove(i));
                }
            });
            tfield(ui, "script.branch.leads_to", |ui| {
                if let Some(Some(t)) = label_picker(
                    ui,
                    ("script_branch_target", id, i),
                    story,
                    Some(branch.target),
                    None,
                ) {
                    action = Some(Do::Target(i, t));
                }
            });
            ui.add_space(4.0);
        }
        if ui
            .button(format!(
                "{}  {}",
                orbit_editor_api::icons::PLUS,
                ui.t("script.branch.add")
            ))
            .clicked()
        {
            action = Some(Do::Add);
        }

        // ---- error output
        ui.add_space(8.0);
        tfield(ui, "script.on_error", |ui| {
            let none = ui.t("script.on_error.none");
            if let Some(chosen) =
                label_picker(ui, ("script_on_error", id), story, on_error, Some(&none))
            {
                action = Some(Do::OnError(chosen));
            }
        });
        ui.label(
            RichText::new(ui.t("script.on_error.hint"))
                .small()
                .color(weak(ui)),
        );
        if new_label_button(ui) {
            action = Some(Do::NewErrorLabel);
        }
    }
    let Some(action) = action else { return };
    let mut next = branches.to_vec();
    match action {
        Do::Rename(i, name) => {
            next[i].name = name;
            commit(ctx, id, rebuild(source.to_owned(), next, on_error), "branch-name");
        }
        Do::Target(i, t) => {
            next[i].target = t;
            commit(ctx, id, rebuild(source.to_owned(), next, on_error), "branch-target");
        }
        Do::Remove(i) => {
            next.remove(i);
            commit(ctx, id, rebuild(source.to_owned(), next, on_error), "branch-remove");
        }
        Do::Add => {
            let taken: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
            let name = orbit_project::ops::unique_name("branch", taken);
            let source = source.to_owned();
            with_new_label(ctx, id, move |label| {
                next.push(ScriptBranch {
                    name,
                    target: label,
                });
                rebuild(source, next, on_error)
            });
        }
        Do::OnError(target) => {
            commit(ctx, id, rebuild(source.to_owned(), next, target), "on-error");
        }
        Do::NewErrorLabel => {
            let source = source.to_owned();
            with_new_label(ctx, id, move |label| rebuild(source, next, Some(label)));
        }
    }
}

/// The result of a sandbox run: chosen branch or failure, variable changes,
/// logs, random draws.
fn show_run(ui: &mut Ui, run: &orbit_script::TestRun) {
    ui.add_space(4.0);
    match &run.result {
        Ok(Some(branch)) => {
            ui.label(
                RichText::new(format!(
                    "{}  {}",
                    orbit_editor_api::icons::CHECK_CIRCLE,
                    ui.tf("script.run.branch", &[("name", branch)])
                ))
                .color(success()),
            );
        }
        Ok(None) => {
            ui.label(
                RichText::new(format!(
                    "{}  {}",
                    orbit_editor_api::icons::CHECK_CIRCLE,
                    ui.t("script.run.no_branch")
                ))
                .color(success()),
            );
        }
        Err(failure) => {
            let text = if let (Some(line), Some(column)) = (failure.line, failure.column) {
                ui.tf(
                    "script.run.failed_at",
                    &[
                        ("line", &line),
                        ("column", &column),
                        ("message", &failure.message),
                    ],
                )
            } else {
                ui.tf("script.run.failed", &[("message", &failure.message)])
            };
            ui.label(
                RichText::new(format!("{}  {text}", orbit_editor_api::icons::WARNING_CIRCLE))
                    .color(danger(ui)),
            );
        }
    }
    if run.changes.is_empty() {
        ui.label(RichText::new(ui.t("script.run.no_changes")).small().color(weak(ui)));
    } else {
        ui.label(RichText::new(ui.t("script.run.changes")).small().color(weak(ui)));
        for (name, before, after) in &run.changes {
            ui.label(RichText::new(format!("{name}:  {before}  →  {after}")).monospace());
        }
    }
    if !run.logs.is_empty() {
        ui.label(RichText::new(ui.t("script.run.logs")).small().color(weak(ui)));
        for line in &run.logs {
            ui.label(RichText::new(line).monospace().color(weak(ui)));
        }
    }
    ui.label(
        RichText::new(ui.tn("script.run.draws", run.draws as usize, &[]))
            .small()
            .color(faint(ui)),
    );
}

// -------------------------------------------------------------- conditions

/// A condition field with a "Rhai" switch. Without it, the usual editor
/// (no-code builder or expression text); with it, a code field holding an
/// `Expr::Rhai`. `option` is the choice option the field belongs to.
pub(super) fn condition_field(
    ui: &mut Ui,
    ctx: &EditorContext<'_>,
    node: NodeId,
    option: Option<usize>,
    salt: impl fields::Salt,
    cond: &Expr,
) -> Option<Expr> {
    let story: &Story = ctx.story()?;
    let rhai = matches!(cond, Expr::Rhai(_));
    let mut out = None;
    let label = ui.t("script.rhai_mode");
    if let Some(on) = fields::checkbox(ui, rhai, &label) {
        out = Some(if on {
            Expr::Rhai("true".to_owned())
        } else {
            Expr::lit(true)
        });
    }
    match cond {
        Expr::Rhai(code) => {
            let checked = checked(ui, ctx);
            let found = findings(checked.as_ref(), &ScriptSite::Condition { node, option });
            if let Some(new) = code_editor(ui, ("cond", node, option), code, &found, 3) {
                out = Some(Expr::Rhai(new));
            }
            findings_list(ui, ("cond", node, option), &found, false);
            ui.label(
                RichText::new(ui.t("script.rhai_mode.hint"))
                    .small()
                    .color(faint(ui)),
            );
        }
        _ => {
            if let Some(c) = super::inspector::condition_editor(ui, salt, story, cond) {
                out = Some(c);
            }
        }
    }
    out
}
