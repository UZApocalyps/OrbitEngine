//! The live MCP server inside the editor, end to end and without a window:
//! a real HTTP server on a free port, a hand-written HTTP client, and an
//! `EditorCore` (no GPU) whose frame loop is played by the test thread.
//!
//! What it proves: the server is off until started; requests without the
//! token (or with a wrong one, or a foreign `Host`) are refused; an authorised
//! client runs `initialize`, `tools/list` and edits the **open project** (the
//! edit is live, unsaved, one undo entry, journaled); a user edit made in
//! between turns a stale `expected_version` into an explicit conflict; the
//! preview tools say clearly that there is no GPU; the lock file works.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::Context;
use orbit_core::NodeKind;
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor::live::{self, NoPreview};
use orbit_editor_api::Selection;
use orbit_editor_api::ai::{AiCommand, AiServerStatus};
use orbit_project::MANIFEST_FILE;
use serde_json::{Value, json};

// ------------------------------------------------------------------ helpers

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-live-mcp-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn demo_project(root: &Path) -> PathBuf {
    let project = root.join("demo-project");
    std::fs::create_dir_all(&project).unwrap();
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stories/demo.ron");
    std::fs::copy(demo, project.join("story.ron")).unwrap();
    std::fs::write(
        project.join(MANIFEST_FILE),
        "(format: 1, name: \"Demo\", story: \"story.ron\", plugins: [])",
    )
    .unwrap();
    project
}

fn editor_with_demo(dir: &TempDir) -> (EditorCore, Context) {
    let ctx = Context::default();
    let mut core = EditorCore::new(CoreOptions::with_language(orbit_editor_api::Lang::Fr), &ctx);
    core.open_project(&demo_project(dir.path()));
    assert!(core.has_project());
    (core, ctx)
}

/// Runs `client` on its own thread while this one plays the editor's frame
/// loop (draining the live queue), as the window app does.
fn with_editor<R: Send>(core: &mut EditorCore, client: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|scope| {
        let handle = scope.spawn(client);
        let started = std::time::Instant::now();
        while !handle.is_finished() {
            core.pump_live(&mut NoPreview);
            std::thread::sleep(Duration::from_millis(2));
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "the client never finished"
            );
        }
        handle.join().expect("client thread panicked")
    })
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The JSON-RPC message: the body itself, or the last `data:` event of
    /// an SSE-framed response.
    fn json(&self) -> Value {
        if let Ok(value) = serde_json::from_str(self.body.trim()) {
            return value;
        }
        self.body
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
            .next_back()
            .unwrap_or_else(|| panic!("no JSON message in: {}", self.body))
    }
}

fn dechunk(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = raw;
    while let Some(pos) = rest.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(std::str::from_utf8(&rest[..pos]).unwrap().trim(), 16)
            .unwrap_or(0);
        rest = &rest[pos + 2..];
        if size == 0 || rest.len() < size {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size..];
        rest = rest.strip_prefix(b"\r\n").unwrap_or(rest);
    }
    out
}

/// One HTTP/1.1 request, `Connection: close`.
fn http(port: u16, method: &str, headers: &[(&str, String)], body: Option<&Value>) -> Reply {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let payload = body.map(Value::to_string).unwrap_or_default();
    let mut head = format!("{method} /mcp HTTP/1.1\r\nConnection: close\r\n");
    let has_host = headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host"));
    if !has_host {
        head.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if body.is_some() {
        head.push_str(&format!("Content-Length: {}\r\n", payload.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(payload.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no HTTP response: {:?}", String::from_utf8_lossy(&raw)));
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let mut body = raw[split + 4..].to_vec();
    if headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"))
    {
        body = dechunk(&body);
    }
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

/// An MCP client over the streamable HTTP transport.
struct Client {
    port: u16,
    token: String,
    session: Option<String>,
    next_id: u64,
}

impl Client {
    fn new(port: u16, token: &str) -> Self {
        Self {
            port,
            token: token.to_owned(),
            session: None,
            next_id: 1,
        }
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        let mut h = vec![
            ("Content-Type", "application/json".to_owned()),
            ("Accept", "application/json, text/event-stream".to_owned()),
            ("Authorization", format!("Bearer {}", self.token)),
        ];
        if let Some(session) = &self.session {
            h.push(("Mcp-Session-Id", session.clone()));
            h.push(("MCP-Protocol-Version", "2025-06-18".to_owned()));
        }
        h
    }

    fn initialize(&mut self) -> Value {
        let reply = http(
            self.port,
            "POST",
            &self.headers(),
            Some(&json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "live-test", "version": "0"}
                }
            })),
        );
        assert_eq!(reply.status, 200, "initialize: {}", reply.body);
        self.session = reply.header("mcp-session-id").map(str::to_owned);
        assert!(self.session.is_some(), "no session id: {:?}", reply.headers);
        let result = reply.json();
        let ack = http(
            self.port,
            "POST",
            &self.headers(),
            Some(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})),
        );
        assert!(
            ack.status == 202 || ack.status == 200,
            "initialized: {}",
            ack.status
        );
        result
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let reply = http(
            self.port,
            "POST",
            &self.headers(),
            Some(
                &json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}),
            ),
        );
        assert_eq!(reply.status, 200, "{method}: {}", reply.body);
        reply.json()
    }

    /// `tools/call`: `(is_error, text, image count)`.
    fn call(&mut self, name: &str, args: Value) -> (bool, String, usize) {
        let reply = self.request("tools/call", json!({"name": name, "arguments": args}));
        let result = &reply["result"];
        assert!(result.is_object(), "{name}: {reply}");
        let content = result["content"].as_array().unwrap();
        let text = content
            .iter()
            .filter(|c| c["type"] == "text")
            .map(|c| c["text"].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        let images = content.iter().filter(|c| c["type"] == "image").count();
        (result["isError"].as_bool().unwrap_or(false), text, images)
    }

    fn ok(&mut self, name: &str, args: Value) -> String {
        let (is_error, text, _) = self.call(name, args);
        assert!(!is_error, "{name} failed: {text}");
        text
    }
}

fn dialogue_texts(core: &EditorCore) -> Vec<String> {
    let story = core.story().unwrap();
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| match &n.kind {
            NodeKind::Say { text_id, .. } => story.text(*text_id).map(str::to_owned),
            _ => None,
        })
        .collect()
}

// -------------------------------------------------------------------- tests

#[test]
fn the_server_is_off_until_the_user_starts_it() {
    let dir = TempDir::new("off");
    let (core, _ctx) = editor_with_demo(&dir);
    assert!(!core.mcp.is_running());
    assert_eq!(core.state.ai.status, AiServerStatus::Stopped);
    assert!(core.state.ai.url.is_none());
    // A token exists from the start (generated once, kept in the preferences).
    assert!(core.state.ai.token.starts_with("orbit_"));
}

#[test]
fn requests_without_a_valid_token_or_from_a_foreign_host_are_refused() {
    let dir = TempDir::new("auth");
    let (mut core, _ctx) = editor_with_demo(&dir);
    core.state.ai.port = 0;
    live::start(&mut core).unwrap();
    let port = core.mcp.port().unwrap();
    let token = core.state.ai.token.clone();
    let init = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "t", "version": "0"}}
    });
    let base = |auth: Option<String>| {
        let mut h = vec![
            ("Content-Type", "application/json".to_owned()),
            ("Accept", "application/json, text/event-stream".to_owned()),
        ];
        if let Some(auth) = auth {
            h.push(("Authorization", auth));
        }
        h
    };

    // No header, wrong token, wrong scheme, a prefix of the right token.
    for auth in [
        None,
        Some("Bearer orbit_wrong".to_owned()),
        Some(format!("Basic {token}")),
        Some(format!("Bearer {}", &token[..token.len() - 1])),
        Some(token.clone()),
    ] {
        let reply = http(port, "POST", &base(auth.clone()), Some(&init));
        assert_eq!(reply.status, 401, "{auth:?} must be refused");
        assert!(reply.header("www-authenticate").is_some());
    }
    assert!(core.mcp.is_running());

    // The right token but a foreign Host (DNS rebinding): refused as well.
    let mut headers = base(Some(format!("Bearer {token}")));
    headers.push(("Host", "evil.example.com".to_owned()));
    let reply = http(port, "POST", &headers, Some(&init));
    assert_eq!(reply.status, 403, "{}", reply.body);

    // The right token on the right host works.
    let reply = http(
        port,
        "POST",
        &base(Some(format!("Bearer {token}"))),
        Some(&init),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);

    core.pump_live(&mut NoPreview);
    assert!(core.state.ai.rejected >= 5, "refusals are counted");
    live::stop(&mut core);
}

#[test]
fn an_ai_edit_over_http_is_live_unsaved_undoable_and_journaled() {
    let dir = TempDir::new("edit");
    let (mut core, _ctx) = editor_with_demo(&dir);
    core.state.ai.port = 0;
    live::start(&mut core).unwrap();
    let (port, token) = (core.mcp.port().unwrap(), core.state.ai.token.clone());
    let before = dialogue_texts(&core);
    let revision = core.session.as_ref().unwrap().project.revision();

    // initialize + tools/list (the 3D tools are listed only with the switch on)
    orbit_core::experimental::set_preference_3d(true);
    let tools = with_editor(&mut core, || {
        let mut client = Client::new(port, &token);
        let init = client.initialize();
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("live mode")
        );
        client.request("tools/list", json!({}))
    });
    let tools = tools["result"]["tools"].as_array().unwrap();
    let find = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap();
    for name in [
        "add_dialogue",
        "read_scene",
        "preview_play",
        "preview_step",
        "preview_choose",
        "preview_state",
        "preview_screenshot",
        "capture_scene3d",
        "list_scenes3d",
        "edit_scene3d",
        "select_node",
        "build_project",
        "export_game",
    ] {
        find(name);
    }
    assert_eq!(find("export_game")["annotations"]["destructiveHint"], true);
    assert_eq!(
        find("build_project")["annotations"]["destructiveHint"],
        true
    );
    assert_eq!(find("preview_state")["annotations"]["readOnlyHint"], true);

    // An edit: one call, one undo entry, in the open project.
    let scene_name = core.story().unwrap().scenes[0].name.clone();
    let text = with_editor(&mut core, || {
        let mut client = Client::new(port, &token);
        client.initialize();
        client.ok(
            "add_dialogue",
            json!({"scene": scene_name, "text": "Bonjour depuis l'IA"}),
        )
    });
    assert!(text.contains("Bonjour"), "{text}");
    let mut after = dialogue_texts(&core);
    assert_eq!(after.len(), before.len() + 1);
    assert!(after.contains(&"Bonjour depuis l'IA".to_owned()));
    after.retain(|t| t != "Bonjour depuis l'IA");
    assert_eq!(after, before);
    // Visible, and not written to disk: it is the user's to save.
    assert!(core.is_modified());
    let on_disk = std::fs::read_to_string(dir.path().join("demo-project/story.ron")).unwrap();
    assert!(!on_disk.contains("Bonjour depuis l'IA"));
    assert_ne!(core.session.as_ref().unwrap().project.revision(), revision);

    // The journal says what happened.
    core.pump_live(&mut NoPreview);
    let entry = core.state.ai.log.back().unwrap();
    assert_eq!(entry.tool, "add_dialogue");
    assert!(entry.ok && entry.undoable);
    assert!(!entry.touched.is_empty());
    assert!(core.state.ai.can_undo_last);
    assert!(core.state.ai.edited_within(5.0));
    assert!(matches!(
        core.state.ai.status,
        AiServerStatus::Listening | AiServerStatus::Connected { .. }
    ));

    // One undo entry: the panel's button reverts the whole call.
    let label = core
        .session
        .as_ref()
        .unwrap()
        .project
        .undo_label()
        .unwrap()
        .to_owned();
    assert_eq!(label, "MCP : add_dialogue");
    core.state.ai.command(AiCommand::UndoLast);
    core.pump_live(&mut NoPreview);
    assert_eq!(dialogue_texts(&core), before);
    assert!(!core.state.ai.can_undo_last);
    live::stop(&mut core);
}

#[test]
fn the_undo_button_never_touches_the_users_own_edits() {
    let dir = TempDir::new("undo-guard");
    let (mut core, _ctx) = editor_with_demo(&dir);
    // The user's own edit is on top of the history.
    core.edit_nth_say(0, "ma réplique").unwrap();
    let before = dialogue_texts(&core);
    core.state.ai.command(AiCommand::UndoLast);
    core.pump_live(&mut NoPreview);
    assert_eq!(dialogue_texts(&core), before, "the user's edit stays");
    assert!(
        core.state
            .toasts
            .iter()
            .any(|t| t.message.contains("la vôtre"))
    );
}

#[test]
fn a_user_edit_in_between_makes_a_stale_version_an_explicit_conflict() {
    let dir = TempDir::new("conflict");
    let (mut core, _ctx) = editor_with_demo(&dir);
    core.state.ai.port = 0;
    live::start(&mut core).unwrap();
    let (port, token) = (core.mcp.port().unwrap(), core.state.ai.token.clone());
    let scene_name = core.story().unwrap().scenes[0].name.clone();

    // The AI reads the scene and notes the version of the first line.
    let listing = with_editor(&mut core, || {
        let mut client = Client::new(port, &token);
        client.initialize();
        client.ok("read_scene", json!({"scene": scene_name}))
    });
    let (node, version) = {
        let story = core.story().unwrap();
        let node = story.scenes[0]
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Say { .. }))
            .unwrap()
            .id;
        let version = core
            .session
            .as_ref()
            .unwrap()
            .project
            .node_version(node)
            .unwrap();
        (node, version)
    };
    assert!(listing.contains(&node.to_string()), "{listing}");

    // The user edits that very line in the editor.
    core.edit_nth_say(0, "texte de l'utilisateur").unwrap();

    // The AI's edit with the old version is refused, naming the cause.
    let (is_error, text, _) = with_editor(&mut core, || {
        let mut client = Client::new(port, &token);
        client.initialize();
        client.call(
            "edit_dialogue",
            json!({"node": node.to_string(), "text": "texte de l'IA", "expected_version": version}),
        )
    });
    assert!(is_error);
    assert!(text.starts_with("version_conflict"), "{text}");
    assert!(text.contains("user"), "{text}");
    assert!(dialogue_texts(&core).contains(&"texte de l'utilisateur".to_owned()));
    assert!(!dialogue_texts(&core).contains(&"texte de l'IA".to_owned()));

    // Re-reading gives the current version and the retry works.
    let version = core
        .session
        .as_ref()
        .unwrap()
        .project
        .node_version(node)
        .unwrap();
    let text = with_editor(&mut core, || {
        let mut client = Client::new(port, &token);
        client.initialize();
        client.ok(
            "edit_dialogue",
            json!({"node": node.to_string(), "text": "texte de l'IA", "expected_version": version}),
        )
    });
    assert!(text.contains("IA"), "{text}");
    assert!(dialogue_texts(&core).contains(&"texte de l'IA".to_owned()));
    // The failed call left a journal entry flagged as an error.
    core.pump_live(&mut NoPreview);
    assert!(
        core.state
            .ai
            .log
            .iter()
            .any(|e| !e.ok && e.tool == "edit_dialogue")
    );
    live::stop(&mut core);
}

#[test]
fn preview_screenshot_export_and_select_without_a_gpu() {
    let dir = TempDir::new("nogpu");
    let (mut core, _ctx) = editor_with_demo(&dir);
    core.state.ai.port = 0;
    live::start(&mut core).unwrap();
    let (port, token) = (core.mcp.port().unwrap(), core.state.ai.token.clone());
    let node = core.story().unwrap().scenes[0].nodes[0].id;

    let results = with_editor(&mut core, || {
        let mut client = Client::new(port, &token);
        client.initialize();
        let mut out = Vec::new();
        for (tool, args) in [
            ("preview_play", json!({})),
            ("preview_state", json!({})),
            ("preview_step", json!({"steps": 2})),
            ("preview_screenshot", json!({})),
            ("capture_scene3d", json!({"scene": "room"})),
            ("export_game", json!({"target": "windows"})),
            ("build_project", json!({})),
            ("select_node", json!({"node": node.to_string()})),
            // The rest keeps working.
            ("list_scenes", json!({})),
            ("validate", json!({})),
        ] {
            out.push((tool, client.call(tool, args)));
        }
        out
    });
    for (tool, (is_error, text, images)) in &results {
        match *tool {
            "preview_play" | "preview_state" | "preview_step" | "preview_screenshot"
            | "capture_scene3d" => {
                assert!(*is_error, "{tool}: {text}");
                assert!(text.starts_with("unavailable"), "{tool}: {text}");
                assert!(text.contains("GPU"), "{tool}: {text}");
                assert_eq!(*images, 0);
            }
            // Exporting never needs the GPU. This project has no asset files
            // and no player template: the errors say so, they are not
            // `unavailable`.
            "build_project" => {
                assert!(*is_error, "{tool}: {text}");
                assert!(text.contains("asset file"), "{tool}: {text}");
            }
            "export_game" => {
                assert!(*is_error, "{tool}: {text}");
                assert!(text.contains("template"), "{tool}: {text}");
            }
            _ => assert!(!is_error, "{tool}: {text}"),
        }
    }
    assert_eq!(core.state.selection, Some(Selection::Node(node)));
    live::stop(&mut core);
}

#[test]
fn export_tools_run_the_real_export() {
    let dir = TempDir::new("export_tools");
    let (mut core, _ctx) = editor_with_demo(&dir);
    // The project gets its assets, and a (fake) Linux player template.
    let project = dir.path().join("demo-project");
    let assets = project.join("assets/demo");
    std::fs::create_dir_all(&assets).unwrap();
    let repo_assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/demo");
    for entry in std::fs::read_dir(repo_assets).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), assets.join(entry.file_name())).unwrap();
    }
    let templates = dir.path().join("templates");
    std::fs::create_dir_all(templates.join("linux")).unwrap();
    std::fs::write(templates.join("linux/orbit-player"), b"fake player").unwrap();
    core.state
        .plugin_prefs
        .entry("orbit.export".to_owned())
        .or_default()
        .insert(
            "templates_dir".to_owned(),
            templates.to_string_lossy().into_owned(),
        );
    core.state.ai.port = 0;
    live::start(&mut core).unwrap();
    let (port, token) = (core.mcp.port().unwrap(), core.state.ai.token.clone());

    let results = with_editor(&mut core, || {
        let mut client = Client::new(port, &token);
        client.initialize();
        vec![
            client.call("build_project", json!({})),
            client.call("export_game", json!({"target": "linux", "out_dir": "dist"})),
            client.call(
                "export_game",
                json!({"target": "linux", "out_dir": "../outside"}),
            ),
            client.call("export_game", json!({"target": "mac"})),
        ]
    });
    let (is_error, text, _) = &results[0];
    assert!(!is_error && text.starts_with("Built "), "{text}");
    assert!(project.join("build/demo.pak").is_file());
    let (is_error, text, _) = &results[1];
    assert!(!is_error && text.contains("Exported linux"), "{text}");
    assert!(project.join("dist/demo-1.0.0-linux.tar.gz").is_file());
    assert!(
        results[2].0 && results[2].1.contains("inside the project"),
        "{:?}",
        results[2]
    );
    assert!(
        results[3].0 && results[3].1.contains("unknown target"),
        "{:?}",
        results[3]
    );
    live::stop(&mut core);
}

#[test]
fn stopping_the_server_closes_the_port_and_a_busy_port_is_reported() {
    let dir = TempDir::new("stop");
    let (mut core, _ctx) = editor_with_demo(&dir);
    core.state.ai.port = 0;
    live::start(&mut core).unwrap();
    let port = core.mcp.port().unwrap();
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());

    // A second editor on the same port fails with a message, not a panic.
    let ctx2 = Context::default();
    let mut other = EditorCore::new(
        CoreOptions::with_language(orbit_editor_api::Lang::Fr),
        &ctx2,
    );
    other.state.ai.port = port;
    let err = live::start(&mut other).unwrap_err();
    assert!(err.contains("cannot listen"), "{err}");
    assert!(matches!(other.state.ai.status, AiServerStatus::Failed(_)));

    live::stop(&mut core);
    assert_eq!(core.state.ai.status, AiServerStatus::Stopped);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}

#[test]
fn the_panel_commands_start_stop_and_regenerate_the_token() {
    let dir = TempDir::new("panel");
    let (mut core, _ctx) = editor_with_demo(&dir);
    core.state.ai.port = 0;
    let old = core.state.ai.token.clone();
    core.state.ai.command(AiCommand::Start);
    core.pump_live(&mut NoPreview);
    assert!(core.state.ai.status.is_running());
    let port = core.mcp.port().unwrap();

    core.state.ai.command(AiCommand::RegenerateToken);
    core.pump_live(&mut NoPreview);
    assert_ne!(core.state.ai.token, old);
    let new_port = core.mcp.port().unwrap();
    let _ = port;
    // The old token no longer opens the door; the new one does.
    let init = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "t", "version": "0"}}
    });
    let headers = |token: &str| {
        vec![
            ("Content-Type", "application/json".to_owned()),
            ("Accept", "application/json, text/event-stream".to_owned()),
            ("Authorization", format!("Bearer {token}")),
        ]
    };
    assert_eq!(
        http(new_port, "POST", &headers(&old), Some(&init)).status,
        401
    );
    assert_eq!(
        http(
            new_port,
            "POST",
            &headers(&core.state.ai.token),
            Some(&init)
        )
        .status,
        200
    );

    core.state.ai.command(AiCommand::Stop);
    core.pump_live(&mut NoPreview);
    assert_eq!(core.state.ai.status, AiServerStatus::Stopped);
}

// -------------------------------------------------------------------- lock

#[test]
fn opening_a_project_takes_the_lock_and_closing_releases_it() {
    let dir = TempDir::new("lock");
    let (mut core, _ctx) = editor_with_demo(&dir);
    let lock = dir.path().join("demo-project/.orbit/lock");
    assert!(lock.is_file());
    assert!(core.mcp.holds_lock());
    assert!(core.state.ai.lock_warning.is_none());
    core.close_project();
    assert!(!lock.exists());
    assert!(!core.mcp.holds_lock());
}

#[test]
fn a_project_held_by_headless_mcp_is_reported_when_the_editor_opens_it() {
    let dir = TempDir::new("lock-other");
    let project = demo_project(dir.path());
    // What `orbit mcp` writes while it runs (another process id).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    std::fs::create_dir_all(project.join(".orbit")).unwrap();
    std::fs::write(
        project.join(".orbit/lock"),
        format!(
            "owner=headless-mcp\npid={}\nstarted_ms={now}\nheartbeat_ms={now}\n",
            std::process::id() + 1
        ),
    )
    .unwrap();
    let ctx = Context::default();
    let mut core = EditorCore::new(CoreOptions::with_language(orbit_editor_api::Lang::Fr), &ctx);
    core.open_project(&project);
    let warning = core.state.ai.lock_warning.clone().expect("a warning");
    assert!(warning.contains("MCP headless"), "{warning}");
    assert!(warning.contains("écrase"), "{warning}");
    assert!(!core.mcp.holds_lock(), "the other side keeps the lock");
    assert!(
        core.state
            .console
            .entries()
            .iter()
            .any(|e| e.message.contains("déjà ouvert"))
    );
    // Closing does not remove somebody else's lock.
    core.close_project();
    assert!(project.join(".orbit/lock").is_file());
}

#[test]
fn headless_mcp_refuses_a_project_the_editor_holds() {
    let dir = TempDir::new("lock-headless");
    let (core, _ctx) = editor_with_demo(&dir);
    assert!(core.mcp.holds_lock());
    // Same process id => `inspect` ignores our own lock; fake another holder.
    let project = dir.path().join("demo-project");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    std::fs::write(
        project.join(".orbit/lock"),
        format!(
            "owner=editor\npid={}\nstarted_ms={now}\nheartbeat_ms={now}\n",
            std::process::id() + 1
        ),
    )
    .unwrap();
    let err = orbit_mcp::serve_stdio_with(&project, false).unwrap_err();
    assert!(err.contains("l'éditeur"), "{err}");
    assert!(err.contains("--force"), "{err}");
}
