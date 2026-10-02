//! Live validation, in French: the findings per node (inline display in the
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
use orbit_editor_api::{Diagnostics, Level, Location, Problem, Waker};

use crate::assets::AssetIssue;

/// Stories with more nodes than this are validated on the worker thread.
pub const SYNC_NODE_LIMIT: usize = 1500;
/// Quiet time before a big story is validated again.
pub const DEBOUNCE: Duration = Duration::from_millis(80);

fn label_of(story: &Story, id: NodeId) -> String {
    match story.node(id).map(|n| &n.kind) {
        Some(NodeKind::Label { name }) => format!("« {name} »"),
        Some(_) => "un nœud qui n'est pas une étiquette".to_owned(),
        None => "un nœud supprimé".to_owned(),
    }
}

fn character_of(story: &Story, id: orbit_core::CharacterId) -> String {
    story
        .character(id)
        .and_then(|c| story.text(c.name_id))
        .map_or_else(
            || "un personnage supprimé".to_owned(),
            |n| format!("« {n} »"),
        )
}

/// Node an error is about, with its French message.
pub fn describe_error(story: &Story, error: &ValidationError) -> (Option<NodeId>, String) {
    use ValidationError as E;
    match error {
        E::UnknownTarget { from, target } => (
            Some(*from),
            format!("Cible introuvable ({})", label_of(story, *target)),
        ),
        E::TargetNotLabel { from, target } => (
            Some(*from),
            format!(
                "La cible doit être une étiquette, pas {}",
                label_of(story, *target)
            ),
        ),
        E::EmptyChoice(n) => (Some(*n), "Ce choix n'a aucune option".to_owned()),
        E::UnknownString { node, .. } => (
            *node,
            "Texte introuvable dans la table des textes".to_owned(),
        ),
        E::UnknownCharacter { node, character } => (
            Some(*node),
            format!("Personnage inconnu ({})", character_of(story, *character)),
        ),
        E::UnknownExpression {
            node,
            character,
            expression,
        } => (
            Some(*node),
            format!(
                "{} n'a pas d'expression « {expression} »",
                character_of(story, *character)
            ),
        ),
        E::UnknownAsset { node, .. } => {
            (*node, "Ressource introuvable dans le manifeste".to_owned())
        }
        E::AssetKindMismatch {
            node,
            expected,
            found,
            ..
        } => (
            *node,
            format!("La ressource est de type « {found} », « {expected} » attendu"),
        ),
        E::UnknownPosition { node, name } => (Some(*node), format!("Position inconnue « {name} »")),
        E::InvalidDuration { node, seconds } => (
            Some(*node),
            format!("Durée invalide ({seconds}) : un nombre positif est attendu"),
        ),
        E::ExprType { node, message } => (Some(*node), format!("Expression : {message}")),
        E::DuplicateNodeId(n) => (Some(*n), "Identifiant de nœud en double".to_owned()),
        E::InvalidAssetPath { path, .. } => (
            None,
            format!("Chemin de ressource invalide : « {path} » (relatif, sans « .. » ni « \\ »)"),
        ),
        E::DuplicateAssetId(_) => (None, "Identifiant de ressource en double".to_owned()),
        E::DuplicateCharacterId(c) => (
            None,
            format!(
                "Identifiant de personnage en double ({})",
                character_of(story, *c)
            ),
        ),
        E::DuplicateExpression { character, name } => (
            None,
            format!(
                "{} définit deux fois l'expression « {name} »",
                character_of(story, *character)
            ),
        ),
        E::DuplicateVariable(name) => (None, format!("Variable « {name} » déclarée deux fois")),
        E::DuplicatePosition(name) => (
            None,
            format!("Nom de position en double ou réservé : « {name} »"),
        ),
        E::InvalidPositionCoords { name } => (
            None,
            format!("La position « {name} » sort de l'écran (0 à 1 attendu)"),
        ),
        E::DuplicateSceneId(_) => (None, "Identifiant de scène en double".to_owned()),
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
        E::DuplicateVariable(name) => Some(Location::Variable(name.clone())),
        E::DuplicateSceneId(scene) => Some(Location::Scene(*scene)),
        _ => None,
    }
}

fn describe_warning(warning: &ValidationWarning) -> (Option<NodeId>, String) {
    use ValidationWarning as W;
    match warning {
        W::UndefinedVariable { node, var } => (
            Some(*node),
            format!("La variable « {var} » n'est ni déclarée ni assignée"),
        ),
        W::UnreachableNode(n) => (Some(*n), "Ce nœud n'est jamais atteint".to_owned()),
        W::PossibleInfiniteLoop(n) => (
            Some(*n),
            "Boucle possible sans réplique, choix ni attente".to_owned(),
        ),
        W::AllOptionsConditional(n) => (
            Some(*n),
            "Toutes les options ont une condition : le joueur peut être bloqué".to_owned(),
        ),
    }
}

/// What the story alone cannot tell: findings of plugin validators, nodes of
/// plugins that are not installed, files of the manifest that are missing or
/// have the wrong case. Computed by the caller (the registry is not `Send`).
#[derive(Clone, Debug, Default)]
pub struct Extras {
    pub custom: Vec<(NodeId, String)>,
    pub missing_plugins: Vec<(NodeId, String)>,
    pub asset_issues: Vec<AssetIssue>,
}

/// Runs the story validation and groups the findings by node. `extras` adds
/// the findings that need the registry or the file system.
pub fn compute(story: &Story, revision: u64, extras: &Extras) -> Diagnostics {
    let report = validate(story);
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
        let (node, message) = describe_error(story, e);
        push(&mut out, Level::Error, node, error_location(e), message);
    }
    for (node, message) in &extras.custom {
        push(&mut out, Level::Error, Some(*node), None, message.clone());
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
            issue.message(),
        );
    }
    for w in &report.warnings {
        let (node, message) = describe_warning(w);
        push(&mut out, Level::Warn, node, None, message);
    }
    for (node, plugin) in &extras.missing_plugins {
        push(
            &mut out,
            Level::Warn,
            Some(*node),
            None,
            format!("Plugin absent : « {plugin} » ; ce nœud est conservé en lecture seule"),
        );
    }
    sort_problems(story, &mut out.problems);
    out
}

/// Errors first, then warnings; each group in reading order (definitions
/// without a node first).
fn sort_problems(story: &Story, problems: &mut [Problem]) {
    let mut order: HashMap<NodeId, usize> = HashMap::new();
    let mut n = 0;
    for scene in &story.scenes {
        for node in &scene.nodes {
            order.entry(node.id).or_insert(n);
            n += 1;
        }
    }
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
                    let diagnostics = compute(&job.story, job.key.1, &job.extras);
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
            return None;
        }
        if self.wanted != Some(key) {
            self.wanted = Some(key);
            self.first_edit.get_or_insert(now);
            // Only a new revision can have changed the manifest.
            if self.asset_key_changed(story) {
                self.refresh_asset_issues(root, story);
            }
        }
        let big = node_count(story) > SYNC_NODE_LIMIT;
        if !big {
            let started = Instant::now();
            let mut extras = registry_extras(story);
            extras.asset_issues.clone_from(&self.asset_issues);
            let out = compute(story, key.1, &extras);
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
                let job = Job {
                    key,
                    story: story.clone(),
                    extras,
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
    fn problems_carry_a_location_and_errors_come_first() {
        let story = story_with_error();
        let diag = compute(&story, 1, &Extras::default());
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
        let first = live.update(now, (1, 1), &story, &root, |_| Extras::default());
        assert!(first.is_some_and(|d| d.error_count() > 0));
        assert!(
            live.update(now, (1, 1), &story, &root, |_| Extras::default())
                .is_none()
        );
        assert!(
            live.update(now, (1, 2), &story, &root, |_| Extras::default())
                .is_some()
        );
        assert!(!live.busy());
    }
}
