//! Live MCP in the editor (M4): the editor side of `orbit_mcp::live`.
//!
//! The server (a thread of its own) posts [`LiveJob`]s to the shared
//! [`CommandQueue`]; [`pump`] drains it on the editor thread, between two
//! frames, and answers each job:
//!
//! * the story tools run through `orbit_mcp::ToolRunner` against the **open
//!   session's `Project`**, one undo entry per call (`Project::group`): the
//!   user sees the edit happen, can undo it, and node versions guard against
//!   a concurrent edit by the user (`version_conflict`);
//! * the preview tools go to a [`PreviewBackend`] (the window app implements
//!   it on top of the GPU runtime; [`NoPreview`] answers `unavailable`, which
//!   is what an editor without GPU does: everything else keeps working);
//! * every call is journaled for the "Assistant IA" panel and `.orbit/mcp.log`.
//!
//! Nothing in this module needs a window or a GPU: the whole path is tested
//! with a bare [`crate::core::EditorCore`].

use std::time::Instant;

use orbit_core::{NodeId, Story, Value};
use orbit_editor_api::ai::{AiCommand, AiLogEntry, AiServerStatus};
use orbit_editor_api::{EditorContext, Level, Location};
use orbit_mcp::host::{ResourceCx, append_log, read_resource, resource_list};
use orbit_mcp::live::{CommandQueue, LiveConfig, LiveJob, LiveReply, LiveRequest, LiveServer};
use orbit_mcp::live_tools::{self, LiveCall, live_tool_infos};
use orbit_mcp::sandbox::Sandbox;
use orbit_mcp::{
    ErrorKind, ExportRequest, ToolError, ToolInfo, ToolOutput, ToolRunner, UNDO_PREFIX,
};
use orbit_project::ProjectManifest;
use orbit_project::lock::{self, ProjectLock};
use orbit_runtime::{Flow, PreviewSnapshot};
use serde_json::{Value as Json, json};

use crate::builtin::reveal;
use crate::core::EditorCore;

/// Plugin id under which the settings are saved in the user config.
pub const PREFS_ID: &str = "orbit.ai";

/// A client silent for this long is shown as disconnected.
const CLIENT_IDLE_MS: u64 = 90_000;

/// What a preview tool produced.
pub struct PreviewOutput {
    pub text: String,
    /// A PNG picture (the screenshot tool).
    pub png: Option<Vec<u8>>,
}

/// Where the preview tools run: the GPU runtime of the window app.
pub trait PreviewBackend {
    fn run(&mut self, core: &mut EditorCore, call: &LiveCall) -> Result<PreviewOutput, ToolError>;
}

/// The backend of an editor without a preview runtime (no GPU, tests).
pub struct NoPreview;

impl PreviewBackend for NoPreview {
    fn run(
        &mut self,
        _core: &mut EditorCore,
        _call: &LiveCall,
    ) -> Result<PreviewOutput, ToolError> {
        Err(ToolError::unavailable(
            "the preview is not available: this editor has no GPU (no runtime to draw the game). \
             Story tools (read_scene, add_dialogue, validate, simulate...) still work; use \
             `simulate` to play the story without a picture.",
        ))
    }
}

/// State of the live service, owned by the editor core.
pub struct LiveService {
    queue: CommandQueue,
    server: Option<LiveServer>,
    runner: ToolRunner,
    next_log_id: u64,
    lock: Option<ProjectLock>,
    /// The takeover of our lock was already reported.
    takeover_reported: bool,
}

impl Default for LiveService {
    fn default() -> Self {
        Self {
            queue: CommandQueue::new(),
            server: None,
            runner: ToolRunner::new(),
            next_log_id: 1,
            lock: None,
            takeover_reported: false,
        }
    }
}

impl LiveService {
    pub fn is_running(&self) -> bool {
        self.server.is_some()
    }

    /// Port the server really listens on (it may differ from the setting
    /// when the setting is 0).
    pub fn port(&self) -> Option<u16> {
        self.server.as_ref().map(LiveServer::port)
    }

    /// Whether another process holds the project (see `orbit_project::lock`).
    pub fn holds_lock(&self) -> bool {
        self.lock.is_some()
    }
}

// ------------------------------------------------------------ settings

/// Loads the port and the token from the user config; makes a token when
/// there is none yet.
pub fn load_settings(core: &mut EditorCore) {
    let prefs = core.state.plugin_prefs.get(PREFS_ID).cloned();
    if let Some(prefs) = prefs {
        if let Some(port) = prefs.get("port").and_then(|p| p.parse().ok()) {
            core.state.ai.port = port;
        }
        if let Some(token) = prefs.get("token").filter(|t| !t.is_empty()) {
            core.state.ai.token.clone_from(token);
        }
    }
    if core.state.ai.token.is_empty() {
        core.state.ai.token = orbit_mcp::live::generate_token();
        save_settings(core);
    }
}

fn save_settings(core: &mut EditorCore) {
    let prefs = core
        .state
        .plugin_prefs
        .entry(PREFS_ID.to_owned())
        .or_default();
    prefs.insert("port".to_owned(), core.state.ai.port.to_string());
    prefs.insert("token".to_owned(), core.state.ai.token.clone());
    core.persist();
}

// ----------------------------------------------------------- start / stop

/// Starts the server on `ai.port` with `ai.token`. Off by default: only an
/// explicit request (the panel's button, `--mcp-port`) gets here.
pub fn start(core: &mut EditorCore) -> Result<(), String> {
    if core.mcp.server.is_some() {
        return Err(core.state.t("live.already_running"));
    }
    if core.state.ai.token.is_empty() {
        core.state.ai.token = orbit_mcp::live::generate_token();
    }
    let waker = core.state.waker.clone();
    core.mcp.queue.set_waker(move || waker.wake());
    let config = LiveConfig {
        port: core.state.ai.port,
        token: core.state.ai.token.clone(),
    };
    match LiveServer::start(core.mcp.queue.clone(), config) {
        Ok(server) => {
            core.state.ai.url = Some(server.url());
            core.state.ai.status = AiServerStatus::Listening;
            let message = core.state.tf("live.listening", &[("url", &server.url())]);
            core.state.console.push(Level::Info, message);
            core.mcp.server = Some(server);
            save_settings(core);
            Ok(())
        }
        Err(e) => {
            core.state.ai.status = AiServerStatus::Failed(e.clone());
            core.state.ai.url = None;
            let message = core.state.tf("live.failed", &[("error", &e)]);
            core.state.console.push(Level::Error, message);
            Err(e)
        }
    }
}

/// Stops the server (clients are disconnected).
pub fn stop(core: &mut EditorCore) {
    if let Some(server) = core.mcp.server.take() {
        server.stop();
        let message = core.state.t("live.stopped");
        core.state.console.push(Level::Info, message);
    }
    core.state.ai.status = AiServerStatus::Stopped;
    core.state.ai.url = None;
}

/// Ends the service with the editor.
pub fn shutdown(core: &mut EditorCore) {
    stop(core);
    core.mcp.lock = None;
}

// ---------------------------------------------------------------- the lock

/// A project was opened: takes its lock, warning when another process holds it.
pub fn on_project_opened(core: &mut EditorCore) {
    core.mcp.lock = None;
    core.mcp.takeover_reported = false;
    core.state.ai.lock_warning = None;
    let Some(session) = &core.session else { return };
    let root = session.location.root.clone();
    if let Some(other) = lock::inspect(&root) {
        let message = core
            .state
            .tf("live.lock_other", &[("other", &other.describe())]);
        core.state.console.push(Level::Warn, message.clone());
        let toast = core.state.t("live.lock_other_toast");
        core.state.push_toast(Level::Warn, toast);
        core.state.ai.lock_warning = Some(message);
        return; // the other side keeps the lock
    }
    // A bare story file has no project folder of its own: no `.orbit` folder
    // is created next to it just to hold a lock.
    if session.location.manifest.is_none() {
        return;
    }
    match ProjectLock::acquire(&root, "editor") {
        Ok(lock) => core.mcp.lock = Some(lock),
        Err(e) => log::debug!("cannot write the project lock: {e}"),
    }
}

/// The project was closed: releases the lock.
pub fn on_project_closed(core: &mut EditorCore) {
    core.mcp.lock = None;
    core.mcp.takeover_reported = false;
    core.state.ai.lock_warning = None;
}

fn check_lock(core: &mut EditorCore) {
    if core.mcp.takeover_reported {
        return;
    }
    let Some(other) = core.mcp.lock.as_ref().and_then(ProjectLock::taken_over_by) else {
        return;
    };
    core.mcp.takeover_reported = true;
    let message = core
        .state
        .tf("live.lock_taken", &[("other", &other.describe())]);
    core.state.console.push(Level::Warn, message.clone());
    let toast = core.state.t("live.lock_taken_toast");
    core.state.push_toast(Level::Warn, toast);
    core.state.ai.lock_warning = Some(message);
}

// -------------------------------------------------------------------- pump

/// Runs on the editor thread between two frames: carries out the panel's
/// requests, publishes the server status and answers the queued calls.
pub fn pump(core: &mut EditorCore, backend: &mut dyn PreviewBackend) {
    process_commands(core);
    check_lock(core);
    let jobs = core.mcp.queue.drain();
    for job in jobs {
        let LiveJob { request, reply } = job;
        let answer = execute(core, request, backend);
        // The client may have gone away while waiting: nothing to do.
        let _ = reply.send(answer);
    }
    publish_status(core);
}

fn publish_status(core: &mut EditorCore) {
    if let Some(server) = &core.mcp.server {
        use std::sync::atomic::Ordering;
        let status = server.status();
        // A session outlives its client (HTTP has no hang-up): one that has
        // been silent for a while no longer counts as a connected client.
        let idle_ms = orbit_platform::unix_time_ms()
            .saturating_sub(status.last_request_ms.load(Ordering::Relaxed));
        let clients = if idle_ms < CLIENT_IDLE_MS {
            status.sessions.load(Ordering::Relaxed)
        } else {
            0
        };
        core.state.ai.rejected = status.rejected.load(Ordering::Relaxed);
        core.state.ai.status = if clients > 0 {
            AiServerStatus::Connected { clients }
        } else {
            AiServerStatus::Listening
        };
    }
    core.state.ai.can_undo_last = core
        .session
        .as_ref()
        .and_then(|s| s.project.undo_label())
        .is_some_and(|label| label.starts_with(UNDO_PREFIX));
}

fn process_commands(core: &mut EditorCore) {
    for command in std::mem::take(&mut core.state.ai.commands) {
        match command {
            AiCommand::Start => {
                let _ = start(core);
            }
            AiCommand::Stop => stop(core),
            AiCommand::RegenerateToken => {
                core.state.ai.token = orbit_mcp::live::generate_token();
                save_settings(core);
                // The running server keeps the old token until restarted:
                // restart it so the old one stops working at once.
                if core.mcp.server.is_some() {
                    stop(core);
                    let _ = start(core);
                }
                let toast = core.state.t("live.token_regenerated");
                core.state.push_toast(Level::Info, toast);
            }
            AiCommand::UndoLast => undo_last_ai_edit(core),
            AiCommand::ClearLog => core.state.ai.log.clear(),
        }
    }
}

/// "Annuler la dernière action de l'IA": only while that action is still the
/// top of the history (never silently undoes the user's own edits).
fn undo_last_ai_edit(core: &mut EditorCore) {
    let Some(session) = core.session.as_mut() else {
        return;
    };
    let Some(label) = session.project.undo_label().map(str::to_owned) else {
        let toast = core.state.t("live.nothing_to_undo");
        core.state.push_toast(Level::Info, toast);
        return;
    };
    if !label.starts_with(UNDO_PREFIX) {
        let toast = core.state.t("live.not_ai_edit");
        core.state.push_toast(Level::Warn, toast);
        return;
    }
    match session.project.undo() {
        Ok(true) => {
            let toast = core.state.tf("live.undone", &[("label", &label)]);
            core.state.push_toast(Level::Info, toast);
            let id = core.mcp.next_log_id;
            core.mcp.next_log_id += 1;
            // These two texts are stored already translated (the journal
            // keeps what the AI sent as is, so it has no key + args form).
            core.state.ai.push_log(AiLogEntry {
                id,
                time_ms: orbit_platform::unix_time_ms(),
                tool: core.state.t("live.undo_log_tool"),
                args: String::new(),
                result: core.state.tf("live.undo_log_result", &[("label", &label)]),
                ok: true,
                duration_ms: 0,
                touched: Vec::new(),
                undoable: false,
            });
        }
        Ok(false) => {}
        Err(e) => {
            let toast = core.state.tf("live.undo_failed", &[("error", &e)]);
            core.state.push_toast(Level::Error, toast);
        }
    }
}

// ------------------------------------------------------------ the requests

fn execute(
    core: &mut EditorCore,
    request: LiveRequest,
    backend: &mut dyn PreviewBackend,
) -> LiveReply {
    match request {
        LiveRequest::ListTools => LiveReply::Tools(list_tools(core)),
        LiveRequest::ListResources => LiveReply::Resources(
            core.session
                .as_ref()
                .map(|s| resource_list(s.project.story()))
                .unwrap_or_default(),
        ),
        LiveRequest::ReadResource { uri } => LiveReply::Resource(read_one(core, &uri)),
        LiveRequest::CallTool { name, args } => {
            LiveReply::Tool(logged(core, &name, args, |core, name, args| {
                run_tool(core, name, args, backend)
            }))
        }
        LiveRequest::Export(request) => {
            let name = if request.build_only {
                live_tools::BUILD_PROJECT
            } else {
                live_tools::EXPORT_GAME
            };
            let args = json!({
                "target": request.target, "profile": request.profile, "out_dir": request.out_dir,
            });
            LiveReply::Export(
                logged(core, name, args, |core, _, _| {
                    export(core, &request).map(ToolOutput::text)
                })
                .map(|out| out.text),
            )
        }
    }
}

fn list_tools(core: &EditorCore) -> Vec<ToolInfo> {
    let mut tools = core.mcp.runner.list(&core.registry.project);
    for tool in &mut tools {
        if tool.name == "save" {
            tool.read_only = false;
            tool.description = "Write the story to disk now. In live mode the AI's edits stay in \
                the editor, unsaved like the user's own, until the user saves (Ctrl+S) or this \
                tool is called."
                .to_owned();
        }
    }
    tools.extend(live_tool_infos());
    if !orbit_core::experimental::enabled_3d() {
        tools.retain(|t| !orbit_mcp::host::is_3d_tool(&t.name));
    }
    tools
}

fn sandbox_of(core: &EditorCore) -> Result<Sandbox, ToolError> {
    let session = core
        .session
        .as_ref()
        .ok_or_else(|| ToolError::unavailable("no project is open in the editor"))?;
    Sandbox::new(&session.location.root).map_err(Into::into)
}

fn manifest_of(core: &EditorCore) -> ProjectManifest {
    core.session
        .as_ref()
        .map(|s| {
            s.location
                .manifest
                .clone()
                .unwrap_or_else(|| ProjectManifest::new(s.name.clone()))
        })
        .unwrap_or_else(|| ProjectManifest::new("Untitled"))
}

fn read_one(core: &EditorCore, uri: &str) -> Result<(&'static str, String), ToolError> {
    let session = core
        .session
        .as_ref()
        .ok_or_else(|| ToolError::unavailable("no project is open in the editor"))?;
    let sandbox = sandbox_of(core)?;
    let manifest = manifest_of(core);
    read_resource(
        &ResourceCx {
            project: &session.project,
            sandbox: &sandbox,
            manifest: &manifest,
            story_file: &session.location.story_file,
            registry: &core.registry.project,
        },
        uri,
    )
}

/// Runs a call and journals it (panel + `.orbit/mcp.log`).
fn logged(
    core: &mut EditorCore,
    name: &str,
    args: Json,
    run: impl FnOnce(&mut EditorCore, &str, Json) -> Result<ToolOutput, ToolError>,
) -> Result<ToolOutput, ToolError> {
    let started = Instant::now();
    let summary = shorten(&args.to_string(), 300);
    let (revision, changes) = core
        .session
        .as_ref()
        .map_or((0, 0), |s| (s.project.revision(), s.pending_changes()));
    let result = run(core, name, args);
    let (changed, touched) = core.session.as_ref().map_or((false, Vec::new()), |s| {
        (s.project.revision() != revision, s.touched_since(changes))
    });
    let (ok, text) = match &result {
        Ok(out) => (true, shorten(&out.text, 240)),
        Err(e) => (false, shorten(&e.to_string(), 240)),
    };
    let history = matches!(name, "undo" | "redo" | "save");
    if changed {
        core.state.ai.last_edit = Some(Instant::now());
    }
    let id = core.mcp.next_log_id;
    core.mcp.next_log_id += 1;
    core.state.ai.push_log(AiLogEntry {
        id,
        time_ms: orbit_platform::unix_time_ms(),
        tool: name.to_owned(),
        args: summary.clone(),
        result: text.clone(),
        ok,
        duration_ms: started.elapsed().as_millis() as u64,
        touched,
        undoable: changed && !history && ok,
    });
    if let Ok(sandbox) = sandbox_of(core) {
        let status = if ok { "ok" } else { "ERROR" };
        append_log(
            &sandbox,
            &format!("live tool={name} {status} args={summary} result={text}"),
        );
    }
    result
}

fn shorten(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let cut: String = flat.chars().take(max).collect();
        format!("{cut}...")
    }
}

fn run_tool(
    core: &mut EditorCore,
    name: &str,
    args: Json,
    backend: &mut dyn PreviewBackend,
) -> Result<ToolOutput, ToolError> {
    if core.session.is_none() {
        return Err(ToolError::unavailable("no project is open in the editor"));
    }
    if let Some(parsed) = live_tools::parse_live_call(name, args.clone()) {
        return match parsed? {
            LiveCall::SelectNode { node } => select_node(core, &node).map(ToolOutput::text),
            LiveCall::Export(request) => export(core, &request).map(ToolOutput::text),
            call => {
                let out = backend.run(core, &call)?;
                let mut output = ToolOutput::text(out.text);
                if let Some(png) = out.png {
                    output = output.with_png(&png);
                }
                Ok(output)
            }
        };
    }

    let sandbox = sandbox_of(core)?;
    let manifest = manifest_of(core);
    let args = if args.is_null() { json!({}) } else { args };
    let core_ref = &mut *core;
    let Some(session) = core_ref.session.as_mut() else {
        return Err(ToolError::unavailable("no project is open in the editor"));
    };
    let (result, save) = core_ref.mcp.runner.dispatch(
        &mut session.project,
        &sandbox,
        &manifest,
        &core_ref.registry.project,
        name,
        args,
    );
    let text = result?;
    if save && !core.save() {
        return Err(ToolError::new(
            ErrorKind::Io,
            "the story could not be saved (see the editor's toast)",
        ));
    }
    Ok(ToolOutput::text(text))
}

fn select_node(core: &mut EditorCore, node: &str) -> Result<String, ToolError> {
    let id: NodeId = orbit_mcp::convert::parse_id(node, "node")?;
    let session = core
        .session
        .as_mut()
        .ok_or_else(|| ToolError::unavailable("no project is open in the editor"))?;
    if session.project.story().node(id).is_none() {
        return Err(ToolError::not_found(format!("no node {id}")));
    }
    let mut ctx = EditorContext::new(Some(&mut session.project), &mut core.state, &core.registry);
    reveal::reveal(&mut ctx, Location::Node(id));
    let place = reveal::describe(&ctx, &Location::Node(id));
    Ok(json!({"selected": id.to_string(), "where": place}).to_string())
}

/// Build / export through `orbit_export`, the same code as the Export panel
/// and `orbit build` / `orbit export`. The project is exported from disk:
/// unsaved edits (the AI's own, for instance) are saved first. Runs on the
/// editor thread, so the window is busy until it ends.
fn export(core: &mut EditorCore, request: &ExportRequest) -> Result<String, ToolError> {
    let (root, story_file, has_manifest) = {
        let session = core
            .session
            .as_ref()
            .ok_or_else(|| ToolError::unavailable("no project is open in the editor"))?;
        (
            session.location.root.clone(),
            session.location.story_file.clone(),
            session.location.manifest.is_some(),
        )
    };
    if core.is_modified() && !core.save() {
        return Err(ToolError::new(
            ErrorKind::Io,
            "the project could not be saved before the export",
        ));
    }
    let mut templates = orbit_export::Templates::standard();
    let dir = core
        .state
        .plugin_prefs
        .get("orbit.export")
        .and_then(|prefs| prefs.get("templates_dir"))
        .filter(|d| !d.trim().is_empty());
    if let Some(dir) = dir {
        templates = templates.with_dir_first(dir.trim());
    }
    let options = orbit_export::ExportOptions {
        templates,
        cancel: None,
        android: Default::default(),
    };
    let project = if has_manifest {
        root
    } else {
        root.join(story_file)
    };
    orbit_export::run_request(
        &project,
        request.build_only,
        request.target.as_deref(),
        request.profile.as_deref(),
        request.out_dir.as_deref(),
        &options,
    )
    .map_err(|failure| match failure {
        orbit_export::ToolFailure::BadRequest(message) => ToolError::invalid(message),
        orbit_export::ToolFailure::Failed(message) => ToolError::new(ErrorKind::Io, message),
    })
}

// ------------------------------------------------------- preview state JSON

fn value_json(value: &Value) -> Json {
    orbit_mcp::convert::value_to_json(value)
}

fn position_name(position: &orbit_core::Position) -> String {
    match position {
        orbit_core::Position::Left => "left".to_owned(),
        orbit_core::Position::Center => "center".to_owned(),
        orbit_core::Position::Right => "right".to_owned(),
        orbit_core::Position::Custom(name) => name.clone(),
        orbit_core::Position::Free { x, y } => format!("free({x:.3}, {y:.3})"),
    }
}

/// The preview as JSON for the AI: status, current node, the line or choice
/// on screen, variables and the declarative scene.
pub fn preview_state_json(story: &Story, status: &str, snapshot: Option<&PreviewSnapshot>) -> Json {
    let Some(snap) = snapshot else {
        return json!({"status": status, "running": false});
    };
    let flow = match &snap.flow {
        Flow::Say(line) => json!({
            "kind": "say",
            "speaker": line.speaker.as_ref().map(|(name, _)| name),
            "text": line.text,
            "already_read": line.already_read,
        }),
        Flow::Choice(options) => json!({
            "kind": "choice",
            "options": options.iter().enumerate()
                .map(|(i, text)| json!({"index": i, "text": text})).collect::<Vec<_>>(),
        }),
        Flow::Waiting { remaining } => json!({"kind": "waiting", "remaining_seconds": remaining}),
        Flow::Transition { elapsed, duration } => {
            json!({"kind": "transition", "elapsed": elapsed, "duration": duration})
        }
        Flow::Ended => json!({"kind": "ended"}),
        Flow::Screen(screen) => json!({"kind": "screen", "screen": screen}),
        Flow::Failed(message) => json!({"kind": "failed", "message": message}),
    };
    let node = snap.node.map(|id| {
        let place = story
            .locate(id)
            .map(|(scene, index)| json!({"scene": story.scenes[scene].name, "index": index}));
        json!({"id": id.to_string(), "at": place})
    });
    let asset_path = |id: orbit_core::AssetId| {
        story
            .assets
            .get(id)
            .map_or_else(|| id.to_string(), |a| a.path.clone())
    };
    let characters: Vec<Json> = snap
        .scene
        .characters
        .iter()
        .map(|c| {
            let name = story
                .character(c.character)
                .and_then(|ch| story.text(ch.name_id))
                .unwrap_or("?");
            json!({
                "character": name,
                "id": c.character.to_string(),
                "expression": c.expression,
                "position": position_name(&c.position),
                "scale": c.scale,
                "flip_x": c.flip_x,
                "flip_y": c.flip_y,
            })
        })
        .collect();
    let variables: serde_json::Map<String, Json> = snap
        .variables
        .iter()
        .map(|(k, v)| (k.clone(), value_json(v)))
        .collect();
    json!({
        "status": status,
        "running": true,
        "node": node,
        "flow": flow,
        "variables": variables,
        "scene": {
            "background": snap.scene.background.map(asset_path),
            "characters": characters,
            "music": snap.scene.music.as_ref().map(|m| json!({
                "asset": asset_path(m.asset), "looping": m.looping,
            })),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use orbit_runtime::Line;
    use orbit_vm::SceneState;

    use super::*;

    #[test]
    fn the_preview_state_is_readable_json() {
        let story = Story::new("T");
        let snapshot = PreviewSnapshot {
            flow: Flow::Choice(vec!["Left".into(), "Right".into()]),
            node: None,
            variables: BTreeMap::from([("trust".to_owned(), Value::Int(2))]),
            scene: SceneState::default(),
        };
        let json = preview_state_json(&story, "playing", Some(&snapshot));
        assert_eq!(json["status"], "playing");
        assert_eq!(json["running"], true);
        assert_eq!(json["flow"]["kind"], "choice");
        assert_eq!(json["flow"]["options"][1]["text"], "Right");
        assert_eq!(json["variables"]["trust"], 2);
        assert!(json["scene"]["background"].is_null());

        let say = PreviewSnapshot {
            flow: Flow::Say(Line {
                speaker: Some(("Alice".into(), [1, 2, 3])),
                text: "Hi".into(),
                already_read: false,
                voice: None,
            }),
            ..snapshot
        };
        let json = preview_state_json(&story, "playing", Some(&say));
        assert_eq!(json["flow"]["speaker"], "Alice");
        assert_eq!(json["flow"]["text"], "Hi");

        let stopped = preview_state_json(&story, "stopped", None);
        assert_eq!(stopped["running"], false);
    }

    #[test]
    fn journal_text_is_one_short_line() {
        assert_eq!(shorten("a\n  b\tc", 10), "a b c");
        let long = "x".repeat(50);
        let cut = shorten(&long, 10);
        assert_eq!(cut, format!("{}...", "x".repeat(10)));
    }
}
