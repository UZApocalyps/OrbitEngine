//! Live validation, in the editor language: the findings per node (inline display in the
//! scene editor and the inspector) and the full list of problems (the
//! Problems panel, the status bar badges).
//!
//! Small stories are validated inline (well under a millisecond). Big ones
//! (see [`SYNC_NODE_LIMIT`]) are validated **off the frame**: the story is
//! cloned once the edits paused for [`DEBOUNCE`], a worker thread runs
//! `orbit_core::validate` and the findings are published when they arrive
//! (the previous ones stay on screen meanwhile). A burst of edits costs one
//! validation, not one per keystroke.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use orbit_core::{NodeId, NodeKind, Story, ValidationError, ValidationWarning, validate};
use orbit_editor_api::{Diagnostics, Level, Location, Problem, Translator, Waker};

use crate::assets::AssetIssue;

/// Stories with more nodes than this are validated on the worker thread.
pub const SYNC_NODE_LIMIT: usize = 1500;
/// How often the module files are looked at for changes.
const SCRIPTS_RECHECK: Duration = Duration::from_secs(1);

/// `(name, modified, size)` of every module file: changes when one is
/// added, removed or saved.
fn scripts_stamp(root: &std::path::Path) -> Vec<(String, u64, u64)> {
    orbit_project::scripts::list_modules(root)
        .into_iter()
        .map(|name| {
            let path = root.join(orbit_project::scripts::module_path(&name));
            let meta = std::fs::metadata(path).ok();
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos() as u64);
            (name, modified, meta.map_or(0, |m| m.len()))
        })
        .collect()
}

/// Quiet time before a big story is validated again.
pub const DEBOUNCE: Duration = Duration::from_millis(80);

fn label_of(texts: &Translator, story: &Story, id: NodeId) -> String {
    match story.node(id).map(|n| &n.kind) {
        Some(NodeKind::Label { name }) => texts.format("problem.label_named", &[("name", name)]),
        Some(_) => texts.get("problem.not_a_label"),
        None => texts.get("problem.deleted_node"),
    }
}

fn character_of(texts: &Translator, story: &Story, id: orbit_core::CharacterId) -> String {
    story
        .character(id)
        .and_then(|c| story.text(c.name_id))
        .map_or_else(
            || texts.get("problem.deleted_character"),
            |n| texts.format("problem.character_named", &[("name", &n)]),
        )
}

/// Node an error is about, with its message in the editor language. The
/// wording is built here from the error type: `orbit_core` keeps its own
/// (English, technical) `Display` for `orbit validate` and the MCP.
pub fn describe_error(
    texts: &Translator,
    story: &Story,
    error: &ValidationError,
) -> (Option<NodeId>, String) {
    use ValidationError as E;
    let t = |key: &str| texts.get(key);
    match error {
        E::UnknownTarget { from, target } => (
            Some(*from),
            texts.format(
                "problem.unknown_target",
                &[("target", &label_of(texts, story, *target))],
            ),
        ),
        E::TargetNotLabel { from, target } => (
            Some(*from),
            texts.format(
                "problem.target_not_label",
                &[("target", &label_of(texts, story, *target))],
            ),
        ),
        E::UnknownScene { node, .. } => (Some(*node), t("problem.unknown_scene")),
        E::LabelNotInScene { node, scene, .. } => (
            Some(*node),
            texts.format(
                "problem.label_not_in_scene",
                &[(
                    "scene",
                    &story.scene(*scene).map_or(String::new(), |s| {
                        texts.format("problem.scene_suffix", &[("name", &s.name)])
                    }),
                )],
            ),
        ),
        E::EmptyChoice(n) => (Some(*n), t("problem.empty_choice")),
        E::UnknownString { node, .. } => (*node, t("problem.unknown_string")),
        E::UnknownCharacter { node, character } => (
            Some(*node),
            texts.format(
                "problem.unknown_character",
                &[("character", &character_of(texts, story, *character))],
            ),
        ),
        E::UnknownExpression {
            node,
            character,
            expression,
        } => (
            Some(*node),
            texts.format(
                "problem.unknown_expression",
                &[
                    ("character", &character_of(texts, story, *character)),
                    ("expression", expression),
                ],
            ),
        ),
        E::UnknownAsset { node, .. } => (*node, t("problem.unknown_asset")),
        E::AssetKindMismatch {
            node,
            expected,
            found,
            ..
        } => (
            *node,
            texts.format(
                "problem.asset_kind_mismatch",
                &[("found", found), ("expected", expected)],
            ),
        ),
        E::UnknownPosition { node, name } => (
            Some(*node),
            texts.format("problem.unknown_position", &[("name", name)]),
        ),
        E::InvalidDuration { node, seconds } => (
            Some(*node),
            texts.format("problem.invalid_duration", &[("seconds", seconds)]),
        ),
        E::ExprType { node, message } => (
            Some(*node),
            texts.format("problem.expression", &[("message", message)]),
        ),
        E::DuplicateNodeId(n) => (Some(*n), t("problem.duplicate_node")),
        E::InvalidAssetPath { path, .. } => (
            None,
            texts.format("problem.invalid_asset_path", &[("path", path)]),
        ),
        E::DuplicateAssetId(_) => (None, t("problem.duplicate_asset")),
        E::DuplicateCharacterId(c) => (
            None,
            texts.format(
                "problem.duplicate_character",
                &[("character", &character_of(texts, story, *c))],
            ),
        ),
        E::DuplicateExpression { character, name } => (
            None,
            texts.format(
                "problem.duplicate_expression",
                &[
                    ("character", &character_of(texts, story, *character)),
                    ("name", name),
                ],
            ),
        ),
        E::InvalidAnimation {
            character,
            expression,
            issue,
        } => (
            None,
            texts.format(
                "problem.invalid_animation",
                &[
                    ("character", &character_of(texts, story, *character)),
                    ("expression", expression),
                    ("issue", &issue.to_string()),
                ],
            ),
        ),
        E::DuplicateVariable(name) => (
            None,
            texts.format("problem.duplicate_variable", &[("name", name)]),
        ),
        E::DuplicatePosition(name) => (
            None,
            texts.format("problem.duplicate_position", &[("name", name)]),
        ),
        E::InvalidPositionCoords { name } => (
            None,
            texts.format("problem.invalid_position_coords", &[("name", name)]),
        ),
        E::DuplicateSceneId(_) => (None, t("problem.duplicate_scene")),
        E::InvalidScript { node, message } => (
            Some(*node),
            texts.format("problem.invalid_script", &[("message", message)]),
        ),
        E::InvalidScreenNode { node, message } => (
            Some(*node),
            texts.format("problem.invalid_screen_node", &[("message", message)]),
        ),
        E::InvalidStageNode { node, message } => (
            Some(*node),
            texts.format("problem.invalid_stage_node", &[("message", message)]),
        ),
        other => (None, other.to_string()),
    }
}

/// Where an error that is not about a node can be found.
fn error_location(error: &ValidationError) -> Option<Location> {
    use ValidationError as E;
    match error {
        E::InvalidAssetPath { asset, .. }
        | E::DuplicateAssetId(asset)
        | E::MissingAssetFile { asset, .. } => Some(Location::Asset(*asset)),
        E::DuplicateCharacterId(c) => Some(Location::Character(*c)),
        E::DuplicateExpression { character, .. } => Some(Location::Character(*character)),
        E::InvalidAnimation { character, .. } => Some(Location::Character(*character)),
        E::DuplicateVariable(name) => Some(Location::Variable(name.clone())),
        E::DuplicateSceneId(scene) => Some(Location::Scene(*scene)),
        _ => None,
    }
}

/// Where a warning that is not about a node can be found (an empty scene
/// the game ends in).
fn warning_location(warning: &ValidationWarning) -> Option<Location> {
    match warning {
        ValidationWarning::SceneEndsGame {
            scene, node: None, ..
        } => Some(Location::Scene(*scene)),
        _ => None,
    }
}

pub fn describe_warning(
    texts: &Translator,
    warning: &ValidationWarning,
) -> (Option<NodeId>, String) {
    use ValidationWarning as W;
    match warning {
        W::UndefinedVariable { node, var } => (
            Some(*node),
            texts.format("problem.undefined_variable", &[("var", var)]),
        ),
        W::DivisionByZero { node } => (Some(*node), texts.get("problem.division_by_zero")),
        W::UnreachableNode(n) => (Some(*n), texts.get("problem.unreachable_node")),
        W::PossibleInfiniteLoop(n) => (Some(*n), texts.get("problem.infinite_loop")),
        W::SceneEndsGame { node, .. } => (*node, texts.get("problem.scene_ends_game")),
        W::UnknownCurve { owner, name } => (
            match owner {
                orbit_core::CurveOwner::Node(n) => Some(*n),
                orbit_core::CurveOwner::Named(_) => None,
            },
            texts.format("problem.unknown_curve", &[("name", name)]),
        ),
        W::AllOptionsConditional(n) => (Some(*n), texts.get("problem.all_options_conditional")),
        W::TimelineKeyAfterEnd(n) => (Some(*n), texts.get("problem.timeline_key_after_end")),
        W::UnknownEmitter { node, name } => (
            Some(*node),
            texts.format("problem.unknown_emitter", &[("name", name)]),
        ),
    }
}

/// The message of a problem with a rigged expression, in the editor language
/// (the key says what is wrong, the message always ends with the fallback:
/// the still image is used).
pub fn describe_rig_problem(
    texts: &Translator,
    story: &Story,
    p: &orbit_project::rigs::RigProblem,
) -> String {
    use orbit_project::rigs::RigProblemKind as K;
    let character = character_of(texts, story, p.character);
    let (key, extra_name, extra): (&str, &str, &str) = match &p.kind {
        K::Unreadable(e) => ("problem.rig.unreadable", "detail", e),
        K::UnknownClip(c) => ("problem.rig.unknown_clip", "clip", c),
        K::UnknownSkin(s) => ("problem.rig.unknown_skin", "skin", s),
        K::MissingImage(i) => ("problem.rig.missing_image", "image", i),
        K::Budget(m) => ("problem.rig.budget", "detail", m),
    };
    texts.format(
        key,
        &[
            ("character", &character),
            ("expression", &p.expression),
            ("rig", &p.rig),
            (extra_name, &extra),
        ],
    )
}

/// What the story alone cannot tell: findings of plugin validators, nodes of
/// plugins that are not installed, files of the manifest that are missing or
/// have the wrong case. Computed by the caller (the registry is not `Send`).
#[derive(Clone, Debug, Default)]
pub struct Extras {
    pub custom: Vec<(NodeId, String)>,
    pub missing_plugins: Vec<(NodeId, String)>,
    pub asset_issues: Vec<AssetIssue>,
    /// Problems of rigged expressions (rig unreadable, unknown clip / skin...).
    pub rig_problems: Vec<orbit_project::rigs::RigProblem>,
    /// Problems of Rhai sources (script nodes, conditions, shared modules).
    pub script_problems: Vec<orbit_project::scripts::ScriptProblem>,
}

/// Message of a problem of a Rhai source: where in the text, then what.
pub fn describe_script_problem(
    texts: &Translator,
    p: &orbit_project::scripts::ScriptProblem,
) -> String {
    use orbit_project::scripts::ScriptSite;
    let d = &p.diagnostic;
    let place = if d.line > 0 {
        format!("{}:{}  ", d.line, d.column)
    } else {
        String::new()
    };
    match &p.site {
        ScriptSite::Module(name) => texts.format(
            "problem.script.module",
            &[
                ("module", &orbit_project::scripts::module_path(name)),
                ("place", &place),
                ("message", &d.message),
            ],
        ),
        site => {
            let what = texts.get(match site {
                ScriptSite::Condition { .. } => "problem.script.condition",
                _ => "problem.script.node",
            });
            texts.format(
                "problem.script.rhai",
                &[("what", &what), ("place", &place), ("message", &d.message)],
            )
        }
    }
}

/// Runs the story validation and groups the findings by node. `extras` adds
/// the findings that need the registry or the file system.
pub fn compute(texts: &Translator, story: &Story, revision: u64, extras: &Extras) -> Diagnostics {
    orbit_profile::scope!("validate");
    let report = {
        orbit_profile::scope!("validate: rules");
        validate(story)
    };
    let mut out = Diagnostics {
        revision,
        ..Diagnostics::default()
    };
    let push = |out: &mut Diagnostics, level, node: Option<NodeId>, loc, message: String| {
        match node {
            Some(n) => out
                .by_node
                .entry(n)
                .or_default()
                .push((level, message.clone())),
            None => out.general.push((level, message.clone())),
        }
        out.problems.push(Problem {
            level,
            message,
            location: node.map(Location::Node).or(loc),
        });
    };
    for e in &report.errors {
        let (node, message) = describe_error(texts, story, e);
        push(&mut out, Level::Error, node, error_location(e), message);
    }
    for (node, message) in &extras.custom {
        push(
            &mut out,
            Level::Error,
            Some(*node),
            None,
            texts.get(message),
        );
    }
    for issue in &extras.asset_issues {
        let level = if issue.is_error() {
            Level::Error
        } else {
            Level::Warn
        };
        push(
            &mut out,
            level,
            None,
            Some(Location::Asset(issue.asset())),
            issue.message(texts),
        );
    }
    for p in &extras.rig_problems {
        push(
            &mut out,
            Level::Warn,
            None,
            Some(Location::Character(p.character)),
            describe_rig_problem(texts, story, p),
        );
    }
    for p in &extras.script_problems {
        let level = if p.is_error() {
            Level::Error
        } else {
            Level::Warn
        };
        push(
            &mut out,
            level,
            p.site.node(),
            None,
            describe_script_problem(texts, p),
        );
    }
    for w in &report.warnings {
        let (node, message) = describe_warning(texts, w);
        push(&mut out, Level::Warn, node, warning_location(w), message);
    }
    for (node, plugin) in &extras.missing_plugins {
        push(
            &mut out,
            Level::Warn,
            Some(*node),
            None,
            texts.format("problem.missing_plugin", &[("plugin", plugin)]),
        );
    }
    sort_problems(story, &mut out.problems);
    out
}

/// Errors first, then warnings; each group in reading order (definitions
/// without a node first).
fn sort_problems(story: &Story, problems: &mut [Problem]) {
    orbit_profile::scope!("validate: sort");
    // Positions of the nodes that have findings only (a handful), not of
    // the whole story: this map used to be a fifth of a 10k-node run (TL-04).
    let mut order: HashMap<NodeId, usize> = problems
        .iter()
        .filter_map(|p| match &p.location {
            Some(Location::Node(id)) => Some((*id, usize::MAX)),
            _ => None,
        })
        .collect();
    if !order.is_empty() {
        let mut missing = order.len();
        let nodes = story.scenes.iter().flat_map(|s| &s.nodes);
        for (n, node) in nodes.enumerate() {
            if let Some(slot) = order.get_mut(&node.id)
                && *slot == usize::MAX
            {
                *slot = n;
                missing -= 1;
                if missing == 0 {
                    break;
                }
            }
        }
    }
    order.retain(|_, n| *n != usize::MAX);
    problems.sort_by_key(|p| {
        let level = u8::from(p.level != Level::Error);
        let position = match &p.location {
            Some(Location::Node(id)) => order.get(id).map_or(usize::MAX, |i| i + 1),
            _ => 0,
        };
        (level, position)
    });
}

/// Number of nodes of the story.
pub fn node_count(story: &Story) -> usize {
    story.scenes.iter().map(|s| s.nodes.len()).sum()
}

struct Job {
    key: (u64, u64),
    story: Story,
    extras: Extras,
    texts: Translator,
}

struct Done {
    key: (u64, u64),
    diagnostics: Diagnostics,
    elapsed: Duration,
}

/// The live validation engine: decides when to validate (inline, debounced,
/// on the worker) and hands the findings back.
pub struct Live {
    jobs: Option<Sender<Job>>,
    results: Receiver<Done>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// `(project generation, revision)` the last published findings are for.
    published: Option<(u64, u64)>,
    /// The state the findings should describe.
    wanted: Option<(u64, u64)>,
    /// When the debounced validation starts.
    due: Option<Instant>,
    inflight: Option<(u64, u64)>,
    /// When the last worker validation was started.
    last_started: Option<Instant>,
    /// Duration of the last validation (worker or inline) and, for a worker
    /// run, the time from the edit to the publication.
    pub last_run: Option<Duration>,
    pub last_latency: Option<Duration>,
    first_edit: Option<Instant>,
    /// Asset-file findings, refreshed when files or the manifest change.
    asset_issues: Vec<AssetIssue>,
    rig_problems: Vec<orbit_project::rigs::RigProblem>,
    script_problems: Vec<orbit_project::scripts::ScriptProblem>,
    /// Stamp of the module files and when it was last looked at.
    scripts_stamp: Vec<(String, u64, u64)>,
    scripts_checked: Option<Instant>,
    assets_dirty: bool,
    asset_key: Option<Vec<(String, String)>>,
}

impl Live {
    pub fn new(waker: Waker) -> Self {
        let (job_tx, job_rx) = channel::<Job>();
        let (done_tx, done_rx) = channel::<Done>();
        let thread = std::thread::Builder::new()
            .name("orbit-validate".to_owned())
            .spawn(move || {
                while let Ok(mut job) = job_rx.recv() {
                    // Only the newest job matters.
                    while let Ok(newer) = job_rx.try_recv() {
                        job = newer;
                    }
                    let started = Instant::now();
                    let diagnostics = compute(&job.texts, &job.story, job.key.1, &job.extras);
                    let done = Done {
                        key: job.key,
                        diagnostics,
                        elapsed: started.elapsed(),
                    };
                    if done_tx.send(done).is_err() {
                        return;
                    }
                    waker.wake();
                }
            })
            .ok();
        Self {
            jobs: Some(job_tx),
            results: done_rx,
            thread,
            published: None,
            wanted: None,
            due: None,
            inflight: None,
            last_started: None,
            last_run: None,
            last_latency: None,
            first_edit: None,
            asset_issues: Vec::new(),
            rig_problems: Vec::new(),
            script_problems: Vec::new(),
            scripts_stamp: Vec::new(),
            scripts_checked: None,
            assets_dirty: true,
            asset_key: None,
        }
    }

    /// Forgets everything (a project was opened or closed).
    pub fn reset(&mut self) {
        self.published = None;
        self.wanted = None;
        self.due = None;
        self.inflight = None;
        self.last_started = None;
        self.first_edit = None;
        self.asset_issues.clear();
        self.rig_problems.clear();
        self.script_problems.clear();
        self.scripts_stamp.clear();
        self.scripts_checked = None;
        self.assets_dirty = true;
        self.asset_key = None;
    }

    /// Files changed on disk: look at the asset findings again.
    pub fn mark_assets_dirty(&mut self) {
        self.assets_dirty = true;
    }

    /// When the loop must wake up for a pending debounced validation.
    pub fn next_wake(&self) -> Option<Instant> {
        self.due
    }

    /// Whether findings are on their way.
    pub fn busy(&self) -> bool {
        self.due.is_some() || self.inflight.is_some()
    }

    /// One step. `registry_extras` is called only when a validation really
    /// starts. Returns findings to publish, if any.
    pub fn update(
        &mut self,
        now: Instant,
        key: (u64, u64),
        story: &Story,
        root: &std::path::Path,
        texts: &Translator,
        registry_extras: impl FnOnce(&Story) -> Extras,
    ) -> Option<Diagnostics> {
        // A finished worker run.
        let mut finished = None;
        while let Ok(done) = self.results.try_recv() {
            if Some(done.key) == self.wanted {
                finished = Some(done);
            } else if Some(done.key) == self.inflight {
                self.inflight = None;
            }
        }
        if let Some(done) = finished {
            self.inflight = None;
            self.published = Some(done.key);
            self.last_run = Some(done.elapsed);
            self.last_latency = self.first_edit.take().map(|t| now.duration_since(t));
            return Some(done.diagnostics);
        }

        if self.assets_dirty {
            self.assets_dirty = false;
            self.refresh_asset_issues(root, story);
            // The findings must include the new asset state.
            self.published = None;
            self.wanted = None;
        }
        if self.published == Some(key) {
            // Module files are edited outside the story: look at them from
            // time to time.
            let due = self
                .scripts_checked
                .is_none_or(|t| now.duration_since(t) >= SCRIPTS_RECHECK);
            if !due {
                return None;
            }
            self.scripts_checked = Some(now);
            if scripts_stamp(root) == self.scripts_stamp {
                return None;
            }
            self.refresh_scripts(root, story);
            self.published = None;
            self.wanted = None;
        }
        if self.wanted != Some(key) {
            self.wanted = Some(key);
            self.first_edit.get_or_insert(now);
            self.refresh_scripts(root, story);
            // Only a new revision can have changed the manifest.
            if self.asset_key_changed(story) {
                self.refresh_asset_issues(root, story);
            } else {
                // Rigged expressions (rig, clip, skin) change with the story
                // without touching the manifest.
                self.rig_problems = orbit_project::rigs::rig_problems(root, story);
            }
        }
        let big = node_count(story) > SYNC_NODE_LIMIT;
        if !big {
            let started = Instant::now();
            let mut extras = registry_extras(story);
            extras.asset_issues.clone_from(&self.asset_issues);
            extras.rig_problems.clone_from(&self.rig_problems);
            extras.script_problems.clone_from(&self.script_problems);
            let out = compute(texts, story, key.1, &extras);
            self.published = Some(key);
            self.due = None;
            self.first_edit = None;
            self.last_run = Some(started.elapsed());
            self.last_latency = self.last_run;
            return Some(out);
        }
        if self.inflight == Some(key) {
            return None;
        }
        // Leading edge: the first edit after a quiet moment is validated at
        // once; edits that follow a validation closely wait for DEBOUNCE
        // after it (a burst of typing costs a few runs, not one per key).
        let ready_at = self.last_started.map_or(now, |started| started + DEBOUNCE);
        match self.due {
            None if ready_at > now => {
                self.due = Some(ready_at);
                None
            }
            Some(due) if due > now => None,
            _ => {
                self.due = None;
                self.last_started = Some(now);
                let mut extras = registry_extras(story);
                extras.asset_issues.clone_from(&self.asset_issues);
                extras.rig_problems.clone_from(&self.rig_problems);
                extras.script_problems.clone_from(&self.script_problems);
                let job = Job {
                    key,
                    story: story.clone(),
                    extras,
                    texts: texts.clone(),
                };
                self.inflight = Some(key);
                if let Some(tx) = &self.jobs
                    && tx.send(job).is_err()
                {
                    self.inflight = None;
                }
                None
            }
        }
    }

    fn refresh_scripts(&mut self, root: &std::path::Path, story: &Story) {
        self.scripts_stamp = scripts_stamp(root);
        self.scripts_checked = None;
        self.script_problems = if self.scripts_stamp.is_empty()
            && !orbit_core::scripting::story_uses_scripts(story)
        {
            Vec::new()
        } else {
            let modules = orbit_project::scripts::read_modules(root);
            orbit_project::scripts::check_scripts(story, &modules)
        };
    }

    fn asset_key_changed(&mut self, story: &Story) -> bool {
        let key: Vec<(String, String)> = story
            .assets
            .assets
            .iter()
            .map(|a| (a.id.to_string(), a.path.clone()))
            .collect();
        if self.asset_key.as_ref() == Some(&key) {
            return false;
        }
        self.asset_key = Some(key);
        true
    }

    fn refresh_asset_issues(&mut self, root: &std::path::Path, story: &Story) {
        self.asset_issues = crate::assets::lint_asset_files(root, story);
        self.rig_problems = orbit_project::rigs::rig_problems(root, story);
        self.asset_key = None;
        self.asset_key_changed(story);
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.jobs = None; // ends the worker's loop
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use orbit_core::{Node, Scene, SceneId};

    use super::*;

    fn story_with_error() -> Story {
        let mut story = Story::new("t");
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes: vec![Node {
                id: NodeId::new(),
                kind: NodeKind::Jump {
                    target: NodeId::new(),
                },
            }],
        });
        story
    }

    #[test]
    fn rig_problems_are_warnings_located_on_the_character() {
        use orbit_project::rigs::{RigProblem, RigProblemKind};
        let mut story = Story::new("t");
        let name_id = story.strings.add("Hero");
        let id = orbit_core::CharacterId::new();
        story.characters.push(orbit_core::Character {
            id,
            name_id,
            color: orbit_core::Color::rgb(1, 2, 3),
            expressions: Vec::new(),
            default_position: None,
            default_scale: 1.0,
        });
        let kinds = [
            RigProblemKind::Unreadable("boom".into()),
            RigProblemKind::UnknownClip("run".into()),
            RigProblemKind::UnknownSkin("winter".into()),
            RigProblemKind::MissingImage("a.png".into()),
            RigProblemKind::Budget("too many bones".into()),
        ];
        let extras = Extras {
            rig_problems: kinds
                .iter()
                .map(|kind| RigProblem {
                    character: id,
                    expression: "idle".into(),
                    rig: "assets/rigs/h.rig.ron".into(),
                    kind: kind.clone(),
                })
                .collect(),
            ..Extras::default()
        };
        for lang in [orbit_editor_api::Lang::En, orbit_editor_api::Lang::Fr] {
            let texts = crate::i18n::translator(lang);
            let diag = compute(&texts, &story, 1, &extras);
            let rig: Vec<_> = diag
                .problems
                .iter()
                .filter(|p| p.location == Some(Location::Character(id)))
                .collect();
            assert_eq!(rig.len(), 5);
            assert!(rig.iter().all(|p| p.level == Level::Warn));
            // Every message names the expression and has no unfilled placeholder.
            assert!(rig.iter().all(|p| p.message.contains("idle") && !p.message.contains('{')), "{rig:?}");
            assert!(rig[1].message.contains("run") && rig[2].message.contains("winter"));
        }
    }

    #[test]
    fn problems_carry_a_location_and_errors_come_first() {
        let story = story_with_error();
        let diag = compute(
            &crate::i18n::translator(orbit_editor_api::Lang::Fr),
            &story,
            1,
            &Extras::default(),
        );
        assert!(diag.error_count() >= 1);
        let first = &diag.problems[0];
        assert_eq!(first.level, Level::Error);
        assert!(matches!(first.location, Some(Location::Node(_))));
        assert!(
            diag.problems
                .iter()
                .skip_while(|p| p.level == Level::Error)
                .all(|p| p.level == Level::Warn)
        );
    }

    #[test]
    fn a_small_story_is_validated_inline_and_only_once_per_revision() {
        let mut live = Live::new(Waker::default());
        let story = story_with_error();
        let root = std::env::temp_dir();
        let now = Instant::now();
        let first = live.update(now, (1, 1), &story, &root, &Translator::default(), |_| {
            Extras::default()
        });
        assert!(first.is_some_and(|d| d.error_count() > 0));
        assert!(
            live.update(now, (1, 1), &story, &root, &Translator::default(), |_| {
                Extras::default()
            })
            .is_none()
        );
        assert!(
            live.update(now, (1, 2), &story, &root, &Translator::default(), |_| {
                Extras::default()
            })
            .is_some()
        );
        assert!(!live.busy());
    }
}
