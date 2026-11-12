//! Live validation, in the editor language: the findings per node (inline display in the
//! scene editor and the inspector) and the full list of problems (the
//! Problems panel, the status bar badges).
//!
//! Validation never runs on the frame: once the edits paused for [DEBOUNCE]
//! (or at once when the user asks) the story is copied and a worker thread runs
//! orbit_core::validate and the file based lints; the findings are published
//! when they arrive and the previous ones stay on screen meanwhile. A burst of
//! edits costs one validation, not one per keystroke; with automatic validation
//! off nothing runs until the user presses Validate.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use orbit_core::{NodeId, NodeKind, Story, ValidationError, ValidationWarning, validate};
use orbit_editor_api::{Diagnostics, Level, Location, Problem, Translator, Waker};

use crate::assets::AssetIssue;

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

/// Quiet time after the last edit before the project is validated.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

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
        E::UnknownAchievement { node, .. } => (Some(*node), t("problem.unknown_achievement")),
        E::InvalidAchievement { message, .. } => (
            None,
            texts.format("problem.invalid_achievement", &[("message", message)]),
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
        W::HotspotsNoExit(n) => (Some(*n), texts.get("problem.hotspots_no_exit")),
        W::AchievementNeverUnlocked(_) => (None, texts.get("problem.achievement_never_unlocked")),
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
    /// Findings that need the registry (not `Send`): computed by the caller.
    extras: Extras,
    texts: Translator,
    root: std::path::PathBuf,
    /// The asset files must be looked at again whatever the manifest says.
    refresh_assets: bool,
}

struct Done {
    key: (u64, u64),
    diagnostics: Diagnostics,
    elapsed: Duration,
    scripts_stamp: Vec<(String, u64, u64)>,
}

/// What the worker keeps between two runs: the asset-file findings only
/// change with the manifest or the files.
#[derive(Default)]
struct WorkerCache {
    asset_key: Option<Vec<(String, String)>>,
    asset_issues: Vec<AssetIssue>,
    /// Rig files read by earlier runs (read again only when changed).
    rigs: orbit_project::rigs::RigFileCache,
}

fn asset_key_of(story: &Story) -> Vec<(String, String)> {
    story
        .assets
        .assets
        .iter()
        .map(|a| (a.id.to_string(), a.path.clone()))
        .collect()
}

/// One validation, entirely on the worker: the file based extras (module
/// scripts, rigs, asset files) and the story rules.
fn run_job(job: Job, cache: &mut WorkerCache) -> Done {
    let started = Instant::now();
    let Job { key, story, mut extras, texts, root, refresh_assets } = job;
    let stamp = scripts_stamp(&root);
    extras.script_problems = if stamp.is_empty() && !orbit_core::scripting::story_uses_scripts(&story) {
        Vec::new()
    } else {
        let modules = orbit_project::scripts::read_modules(&root);
        orbit_project::scripts::check_scripts(&story, &modules)
    };
    // Rigged expressions (rig, clip, skin) change with the story without
    // touching the manifest: looked at on every run.
    extras.rig_problems = orbit_project::rigs::rig_problems_cached(&root, &story, &mut cache.rigs);
    let asset_key = asset_key_of(&story);
    if refresh_assets || cache.asset_key.as_ref() != Some(&asset_key) {
        cache.asset_issues = crate::assets::lint_asset_files(&root, &story);
        cache.asset_key = Some(asset_key);
    }
    extras.asset_issues.clone_from(&cache.asset_issues);
    let diagnostics = compute(&texts, &story, key.1, &extras);
    Done { key, diagnostics, elapsed: started.elapsed(), scripts_stamp: stamp }
}

/// The live validation engine: decides when to validate and hands the
/// findings back. Validation never runs on the frame: the story is copied
/// once the edits paused for [`DEBOUNCE`] (or at once for a request of the
/// user) and a worker thread does the rest. A result for an older revision
/// than the current one is dropped.
pub struct Live {
    jobs: Option<Sender<Job>>,
    results: Receiver<Done>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// "Automatic validation": off, only [`Live::force`] starts a run.
    pub auto: bool,
    /// Validate now, whatever `auto` and the debounce say (Validate button,
    /// before an export, tests). Reset when the run starts.
    pub force: bool,
    /// `(project generation, revision)` the last published findings are for.
    published: Option<(u64, u64)>,
    /// The state the debounce is waiting to validate.
    wanted: Option<(u64, u64)>,
    /// When the debounced validation starts.
    due: Option<Instant>,
    inflight: Option<(u64, u64)>,
    /// Duration of the last validation and the time from the edit to the
    /// publication.
    pub last_run: Option<Duration>,
    pub last_latency: Option<Duration>,
    /// Runs started and results dropped as stale (tests, perf logs).
    pub started: usize,
    pub discarded: usize,
    first_edit: Option<Instant>,
    /// Stamp of the module files and when it was last looked at.
    scripts_stamp: Vec<(String, u64, u64)>,
    scripts_checked: Option<Instant>,
    assets_dirty: bool,
}

impl Live {
    pub fn new(waker: Waker) -> Self {
        let (job_tx, job_rx) = channel::<Job>();
        let (done_tx, done_rx) = channel::<Done>();
        let thread = std::thread::Builder::new()
            .name("orbit-validate".to_owned())
            .spawn(move || {
                let mut cache = WorkerCache::default();
                while let Ok(mut job) = job_rx.recv() {
                    // Only the newest job matters.
                    while let Ok(newer) = job_rx.try_recv() {
                        job = newer;
                    }
                    if done_tx.send(run_job(job, &mut cache)).is_err() {
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
            auto: true,
            force: false,
            published: None,
            wanted: None,
            due: None,
            inflight: None,
            last_run: None,
            last_latency: None,
            started: 0,
            discarded: 0,
            first_edit: None,
            scripts_stamp: Vec::new(),
            scripts_checked: None,
            assets_dirty: true,
        }
    }

    /// Forgets everything (a project was opened or closed).
    pub fn reset(&mut self) {
        self.published = None;
        self.wanted = None;
        self.due = None;
        self.inflight = None;
        self.first_edit = None;
        self.scripts_stamp.clear();
        self.scripts_checked = None;
        self.assets_dirty = true;
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

    /// Whether the published findings are not for `key`.
    pub fn is_stale(&self, key: (u64, u64)) -> bool {
        self.published != Some(key)
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
        // Finished runs: only the one for the current state is published.
        let mut finished = None;
        while let Ok(done) = self.results.try_recv() {
            if Some(done.key) == self.inflight {
                self.inflight = None;
            }
            if done.key == key {
                finished = Some(done);
            } else {
                self.discarded += 1;
            }
        }
        if let Some(done) = finished {
            self.published = Some(done.key);
            self.wanted = None;
            self.due = None;
            self.last_run = Some(done.elapsed);
            self.last_latency = self.first_edit.take().map(|t| now.duration_since(t));
            self.scripts_stamp = done.scripts_stamp;
            self.scripts_checked = Some(now);
            return Some(done.diagnostics);
        }

        if self.assets_dirty {
            // The findings must include the new asset state.
            self.published = None;
        }
        if self.published == Some(key) && !self.force {
            // Module files are edited outside the story: look at them from
            // time to time (automatic validation only).
            let due = self.auto
                && self
                    .scripts_checked
                    .is_none_or(|t| now.duration_since(t) >= SCRIPTS_RECHECK);
            if !due {
                return None;
            }
            self.scripts_checked = Some(now);
            if scripts_stamp(root) == self.scripts_stamp {
                return None;
            }
            self.published = None;
        }
        if !self.auto && !self.force {
            // Results stay as they are (shown as stale) until the user asks.
            self.due = None;
            self.wanted = None;
            return None;
        }
        if self.inflight == Some(key) && !self.force {
            return None;
        }
        if !self.force {
            // Trailing edge: every new revision restarts the wait, so a
            // burst of edits costs one run, after the burst.
            if self.wanted != Some(key) {
                self.wanted = Some(key);
                self.first_edit.get_or_insert(now);
                self.due = Some(now + DEBOUNCE);
            }
            if self.due.is_some_and(|d| d > now) {
                return None;
            }
        }
        self.first_edit.get_or_insert(now);
        let refresh_assets = self.assets_dirty || self.force;
        self.assets_dirty = false;
        self.force = false;
        self.due = None;
        self.wanted = Some(key);
        let job = Job {
            key,
            story: story.clone(),
            extras: registry_extras(story),
            texts: texts.clone(),
            root: root.to_path_buf(),
            refresh_assets,
        };
        self.inflight = Some(key);
        self.started += 1;
        if let Some(tx) = &self.jobs
            && tx.send(job).is_err()
        {
            self.inflight = None;
        }
        None
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

    fn step(live: &mut Live, at: Instant, key: (u64, u64), story: &Story) -> Option<Diagnostics> {
        live.update(at, key, story, &std::env::temp_dir(), &Translator::default(), |_| {
            Extras::default()
        })
    }

    /// Calls `update` until the worker answers (or 5 s).
    fn wait_for(live: &mut Live, at: Instant, key: (u64, u64), story: &Story) -> Option<Diagnostics> {
        for _ in 0..5000 {
            if let Some(d) = step(live, at, key, story) {
                return Some(d);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        None
    }

    #[test]
    fn a_validation_never_runs_inline_and_runs_once_per_revision() {
        let mut live = Live::new(Waker::default());
        let story = story_with_error();
        let t0 = Instant::now();
        // Nothing is published on the frame of the edit.
        assert!(step(&mut live, t0, (1, 1), &story).is_none());
        assert!(live.busy());
        let late = t0 + DEBOUNCE * 2;
        let first = wait_for(&mut live, late, (1, 1), &story);
        assert!(first.is_some_and(|d| d.error_count() > 0));
        assert!(step(&mut live, late, (1, 1), &story).is_none());
        assert_eq!(live.started, 1);
    }

    #[test]
    fn a_hundred_rapid_edits_cost_at_most_one_validation() {
        let mut live = Live::new(Waker::default());
        let story = story_with_error();
        let t0 = Instant::now();
        for i in 0..100u64 {
            let at = t0 + Duration::from_millis(i); // 1 ms apart
            assert!(step(&mut live, at, (1, i + 1), &story).is_none());
        }
        assert_eq!(live.started, 0, "nothing starts during the burst");
        let after = t0 + Duration::from_millis(100) + DEBOUNCE;
        assert!(wait_for(&mut live, after, (1, 100), &story).is_some());
        assert_eq!(live.started, 1);
    }

    #[test]
    fn automatic_validation_off_only_validates_on_request() {
        let mut live = Live::new(Waker::default());
        live.auto = false;
        let story = story_with_error();
        let t0 = Instant::now();
        for i in 0..20u64 {
            let at = t0 + DEBOUNCE * 4 * (i as u32 + 1);
            assert!(step(&mut live, at, (1, i + 1), &story).is_none());
        }
        assert_eq!(live.started, 0);
        assert!(!live.busy());
        assert!(live.is_stale((1, 20)));
        live.force = true;
        let done = wait_for(&mut live, t0, (1, 20), &story);
        assert!(done.is_some_and(|d| d.revision == 20));
        assert_eq!(live.started, 1);
        assert!(!live.is_stale((1, 20)));
        // The next edit makes the results stale again, without a new run.
        assert!(step(&mut live, t0 + DEBOUNCE * 100, (1, 21), &story).is_none());
        assert!(live.is_stale((1, 21)));
        assert_eq!(live.started, 1);
    }

    #[test]
    fn the_result_of_an_older_revision_is_discarded() {
        let mut live = Live::new(Waker::default());
        let story = story_with_error();
        let t0 = Instant::now();
        live.force = true;
        assert!(step(&mut live, t0, (1, 1), &story).is_none());
        // A newer revision arrives before the answer is read.
        let later = t0 + DEBOUNCE * 2;
        assert!(step(&mut live, later, (1, 2), &story).is_none());
        let later = later + DEBOUNCE * 2;
        let mut published = None;
        for _ in 0..5000 {
            if let Some(d) = step(&mut live, later, (1, 2), &story) {
                published = Some(d);
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(published.is_some_and(|d| d.revision == 2));
        assert!(live.discarded >= 1 || live.started == 2);
    }
}
