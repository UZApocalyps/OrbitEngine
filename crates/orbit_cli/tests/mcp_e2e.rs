//! End to end: the real `orbit mcp` binary, driven over stdio like Claude
//! Code does (newline-delimited JSON-RPC).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    next_id: u64,
}

impl Client {
    fn start(project: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_orbit"))
            .args(["mcp", "--project"])
            .arg(project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("cannot start `orbit mcp`");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
            next_id: 1,
        }
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Sends a request and returns its `result`, panicking on a JSON-RPC error.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or_else(|_| panic!("no response to {method}"));
            let message: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("stdout must only carry JSON-RPC, got {line:?}: {e}"));
            if message["id"] == json!(id) {
                assert!(message.get("error").is_none(), "{method}: {message}");
                return message["result"].clone();
            }
            // Notifications and server requests are ignored.
        }
    }

    fn handshake(&mut self) -> Value {
        let init = self.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "e2e-test", "version": "0"}}),
        );
        self.notify("notifications/initialized", json!({}));
        init
    }

    /// Calls a tool; returns (is_error, text).
    fn tool(&mut self, name: &str, arguments: Value) -> (bool, String) {
        let result = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        (result["isError"].as_bool().unwrap_or(false), text)
    }

    /// Calls a tool that must succeed; parses its output as JSON when it is.
    fn tool_ok(&mut self, name: &str, arguments: Value) -> Value {
        let (is_error, text) = self.tool(name, arguments);
        assert!(!is_error, "{name} failed: {text}");
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }

    /// Closes stdin (the client went away) and checks a clean exit.
    fn finish(mut self) {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        assert!(status.success(), "orbit mcp exited with {status}");
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orbit_mcp_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn orbit_validate(target: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_orbit"))
        .arg("validate")
        .arg(target)
        .output()
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn node_id(result: &Value, pointer: &str) -> String {
    result
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no {pointer} in {result}"))
        .to_owned()
}

#[test]
fn an_ai_client_builds_a_scene_with_a_choice_over_stdio() {
    let dir = temp_dir("flow");
    let mut c = Client::start(&dir);

    // Handshake.
    let init = c.handshake();
    assert_eq!(init["serverInfo"]["name"], "orbit-mcp");
    assert!(init["capabilities"]["tools"].is_object());
    assert!(init["capabilities"]["resources"].is_object());

    // Tools, with schemas and annotations.
    let listed = c.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().unwrap();
    let find = |name: &str| {
        tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("no tool {name}"))
    };
    for name in [
        "list_scenes",
        "read_scene",
        "create_scene",
        "add_dialogue",
        "add_choice",
        "add_command",
        "create_character",
        "validate",
        "simulate",
        "undo",
        "redo",
    ] {
        assert_eq!(find(name)["inputSchema"]["type"], "object");
    }
    assert_eq!(find("delete_scene")["annotations"]["destructiveHint"], true);
    assert_eq!(find("read_scene")["annotations"]["readOnlyHint"], true);
    assert_eq!(
        find("add_dialogue")["annotations"]["destructiveHint"],
        false
    );

    // Resources.
    let resources = c.request("resources/list", json!({}));
    assert!(
        resources["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["uri"] == "orbit://schema/ir")
    );
    let schema = c.request("resources/read", json!({"uri": "orbit://schema/ir"}));
    let schema_text = schema["contents"][0]["text"].as_str().unwrap();
    assert!(
        serde_json::from_str::<Value>(schema_text)
            .unwrap()
            .pointer("/$defs/NodeKind")
            .is_some()
    );

    // A refused call is a tool error (the model can read it), not a protocol error.
    let (is_error, text) = c.tool("read_scene", json!({"scene": "nope"}));
    assert!(is_error && text.starts_with("not_found"), "{text}");

    // Two characters, one scene, lines, and a two-branch choice.
    c.tool_ok(
        "create_character",
        json!({"name": "Alice", "color": "#e07070"}),
    );
    c.tool_ok("create_character", json!({"name": "Bob"}));
    c.tool_ok(
        "declare_variable",
        json!({"name": "brave", "initial": false}),
    );
    c.tool_ok("create_scene", json!({"name": "crossroads"}));
    let say = |c: &mut Client, who: &str, text: &str| -> String {
        let r = c.tool_ok(
            "add_dialogue",
            json!({"scene": "crossroads", "speaker": who, "text": text}),
        );
        node_id(&r, "/dialogue/id")
    };
    let command = |c: &mut Client, command: Value| -> String {
        let r = c.tool_ok(
            "add_command",
            json!({"scene": "crossroads", "command": command}),
        );
        node_id(&r, "/node/id")
    };
    say(&mut c, "Alice", "Which way?");
    let anchor = say(&mut c, "Bob", "Your call.");
    command(&mut c, json!({"type": "label", "name": "left"}));
    say(&mut c, "Bob", "The left path is dark.");
    let set_brave = command(
        &mut c,
        json!({"type": "set_var", "var": "brave", "value": "true"}),
    );
    command(&mut c, json!({"type": "label", "name": "right"}));
    say(&mut c, "Alice", "The right path is sunny.");
    command(&mut c, json!({"type": "label", "name": "end"}));
    say(&mut c, "Alice", "Anyway.");
    command(&mut c, json!({"type": "end"}));
    // The left branch skips the right one.
    c.tool_ok(
        "add_command",
        json!({"scene": "crossroads", "after": set_brave,
               "command": {"type": "jump", "target": "end"}}),
    );
    // The choice goes right after the question.
    c.tool_ok(
        "add_choice",
        json!({"scene": "crossroads", "after": anchor, "options": [
            {"text": "Go left", "target": "left"},
            {"text": "Go right", "target": "right"}]}),
    );

    // Validation and both branches.
    let report = c.tool_ok("validate", json!({}));
    assert_eq!(report["ok"], true, "{report}");
    let left = c.tool_ok("simulate", json!({"choices": [0]}));
    let left = left.as_str().unwrap();
    assert!(left.contains("Bob: The left path is dark."), "{left}");
    assert!(!left.contains("sunny"), "{left}");
    assert!(left.contains("brave=true"), "{left}");
    assert!(left.contains("Alice: Anyway."), "{left}");
    let right = c.tool_ok("simulate", json!({"choices": ["Go right"]}));
    let right = right.as_str().unwrap();
    assert!(right.contains("Alice: The right path is sunny."), "{right}");
    assert!(!right.contains("dark"), "{right}");
    assert!(right.contains("brave=false"), "{right}");

    // undo / redo travel through the protocol too.
    c.tool_ok("undo", json!({}));
    c.tool_ok("redo", json!({}));

    // The scene as an AI reads it.
    let (_, listing) = c.tool("read_scene", json!({"scene": "crossroads"}));
    assert!(
        listing.contains("choice [0] \"Go left\" -> label \"left\""),
        "{listing}"
    );

    c.finish();

    // The file on disk is a valid project for the regular tools.
    let (ok, output) = orbit_validate(&dir);
    assert!(ok, "orbit validate failed: {output}");
    assert!(output.contains("0 error(s)"), "{output}");
    let story =
        orbit_core::Story::from_ron(&std::fs::read_to_string(dir.join("story.ron")).unwrap())
            .unwrap();
    assert_eq!(story.characters.len(), 2);
    assert_eq!(story.scenes.len(), 1);
    assert!(dir.join("orbit.project.ron").is_file());
    let log = std::fs::read_to_string(dir.join(".orbit/mcp.log")).unwrap();
    assert!(log.contains("tool=add_choice ok"), "{log}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_calls_and_bad_projects_fail_cleanly() {
    // A missing project is reported on stderr with a non-zero exit code.
    let out = Command::new(env!("CARGO_BIN_EXE_orbit"))
        .args(["mcp", "--project", "/definitely/not/a/project"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "stdout is reserved for the protocol");

    // Bad arguments do not hurt a running server.
    let dir = temp_dir("garbage");
    let mut c = Client::start(&dir);
    c.handshake();
    let (is_error, text) = c.tool("create_scene", json!({"name": 42}));
    assert!(is_error && text.starts_with("invalid_arguments"), "{text}");
    let (is_error, text) = c.tool("not_a_tool", json!({}));
    assert!(is_error && text.starts_with("not_found"), "{text}");
    let (is_error, text) = c.tool(
        "import_asset",
        json!({"source": "../../etc/passwd", "kind": "image"}),
    );
    assert!(is_error && text.starts_with("outside_project"), "{text}");
    // Still alive and consistent.
    let scenes = c.tool_ok("list_scenes", json!({}));
    assert_eq!(scenes, json!([]));
    c.finish();
    let _ = std::fs::remove_dir_all(&dir);
}
