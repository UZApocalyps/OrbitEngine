//! In-process tests of every tool, through `HeadlessHost` (no protocol).

use std::fs;
use std::path::PathBuf;

use orbit_core::{Story, TransitionKind};
use orbit_mcp::{ErrorKind, HeadlessHost, ProjectHost, ToolError};
use orbit_plugin_api::{McpToolDef, Plugin, PluginManifest, ProjectRegistry};
use orbit_project::Command;
use serde_json::{Value, json};

fn temp_project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "orbit_mcp_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

struct Fixture {
    dir: PathBuf,
    host: HeadlessHost,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = temp_project(tag);
        let host = HeadlessHost::open(&dir).unwrap();
        Self { dir, host }
    }

    fn call(&mut self, name: &str, args: Value) -> Result<String, ToolError> {
        self.host.call_tool(name, args)
    }

    /// Calls a tool that must succeed and returns its JSON output.
    fn ok(&mut self, name: &str, args: Value) -> Value {
        let text = self
            .call(name, args)
            .unwrap_or_else(|e| panic!("{name} failed: {e}"));
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }

    fn err(&mut self, name: &str, args: Value) -> ToolError {
        match self.call(name, args) {
            Ok(text) => panic!("{name} should have failed, got: {text}"),
            Err(e) => e,
        }
    }

    fn story(&self) -> &Story {
        self.host.project().story()
    }

    fn on_disk(&self) -> Story {
        let text = fs::read_to_string(self.dir.join("story.ron")).unwrap();
        Story::from_ron_unchecked(&text).unwrap()
    }

    fn write_asset_source(&self, rel: &str) {
        let path = self.dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"not really a png").unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn id_of(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no string at {pointer} in {value}"))
        .to_owned()
}

#[test]
fn an_empty_folder_becomes_a_project() {
    let f = Fixture::new("new");
    assert!(f.dir.join("orbit.project.ron").is_file());
    assert!(f.dir.join("story.ron").is_file());
    assert!(f.story().scenes.is_empty());
}

#[test]
fn tools_advertise_schemas_and_hints() {
    let f = Fixture::new("list");
    let tools = f.host.list_tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    for expected in [
        "list_scenes",
        "read_scene",
        "create_scene",
        "rename_scene",
        "delete_scene",
        "add_dialogue",
        "edit_dialogue",
        "add_choice",
        "add_command",
        "delete_node",
        "move_node",
        "create_character",
        "edit_character",
        "declare_variable",
        "list_assets",
        "import_asset",
        "validate",
        "simulate",
        "undo",
        "redo",
        "save",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
    for tool in &tools {
        assert_eq!(
            tool.input_schema.get("type").and_then(Value::as_str),
            Some("object"),
            "{}",
            tool.name
        );
        assert!(!tool.description.is_empty());
        assert!(
            !serde_json::to_string(&tool.input_schema)
                .unwrap()
                .contains("$ref"),
            "{} schema must be inlined",
            tool.name
        );
    }
    let destructive: Vec<&str> = tools
        .iter()
        .filter(|t| t.destructive)
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(destructive, ["delete_scene", "delete_node"]);
    let read_only: Vec<&str> = tools
        .iter()
        .filter(|t| t.read_only)
        .map(|t| t.name.as_str())
        .collect();
    assert!(read_only.contains(&"read_scene") && read_only.contains(&"simulate"));
    assert!(!read_only.contains(&"add_dialogue") && !read_only.contains(&"save"));
}

/// Builds: image assets, two characters, one scene with labels and a choice.
fn build_story(f: &mut Fixture) -> (String, Value) {
    f.write_asset_source("incoming/alice.png");
    f.write_asset_source("incoming/bob.png");
    f.write_asset_source("incoming/room.png");
    f.write_asset_source("incoming/theme.ogg");
    let alice_img = f.ok("import_asset", json!({"source": "incoming/alice.png"}));
    let bob_img = f.ok("import_asset", json!({"source": "incoming/bob.png"}));
    f.ok("import_asset", json!({"source": "incoming/room.png"}));
    f.ok(
        "import_asset",
        json!({"source": "incoming/theme.ogg", "kind": "music"}),
    );
    f.ok(
        "create_character",
        json!({"name": "Alice", "color": "#ff8080",
               "expressions": [{"name": "happy", "asset": id_of(&alice_img, "/id")}]}),
    );
    f.ok(
        "create_character",
        json!({"name": "Bob",
               "expressions": [{"name": "neutral", "asset": id_of(&bob_img, "/id")}]}),
    );
    f.ok("declare_variable", json!({"name": "trust", "initial": 0}));
    let scene = f.ok("create_scene", json!({"name": "intro"}));
    let scene_id = id_of(&scene, "/scene");
    f.ok(
        "add_command",
        json!({"scene": "intro", "command": {"type": "background", "asset": "room",
               "transition": {"kind": "dissolve", "duration": 1.0}}}),
    );
    f.ok(
        "add_command",
        json!({"scene": "intro", "command": {"type": "play_music", "asset": "theme"}}),
    );
    f.ok(
        "add_dialogue",
        json!({"scene": "intro", "speaker": "Alice", "text": "Hello Bob!", "expression": "happy", "position": "left"}),
    );
    f.ok(
        "add_dialogue",
        json!({"scene": "intro", "speaker": "Bob", "text": "Hi Alice.", "expression": "neutral", "position": "right"}),
    );
    let silence = f.ok(
        "add_dialogue",
        json!({"scene": "intro", "text": "A silence falls."}),
    );
    // Labels first, then the choice that targets them.
    f.ok(
        "add_command",
        json!({"scene": "intro", "command": {"type": "label", "name": "kind"}}),
    );
    f.ok(
        "add_command",
        json!({"scene": "intro", "command": {"type": "set_var", "var": "trust", "value": "trust + 1"}}),
    );
    f.ok(
        "add_dialogue",
        json!({"scene": "intro", "speaker": "Alice", "text": "You are kind."}),
    );
    f.ok(
        "add_command",
        json!({"scene": "intro", "command": {"type": "label", "name": "done"}}),
    );
    f.ok(
        "add_dialogue",
        json!({"scene": "intro", "speaker": "Bob", "text": "The end."}),
    );
    f.ok(
        "add_command",
        json!({"scene": "intro", "command": {"type": "end"}}),
    );
    // The choice goes right after the narration line, before the `kind` label.
    let choice = f.ok(
        "add_choice",
        json!({"scene": "intro", "after": id_of(&silence, "/dialogue/id"),
        "options": [
          {"text": "Be kind", "target": "kind"},
          {"text": "Walk away", "target": "done", "condition": "trust >= 0"}
        ]}),
    );
    (scene_id, choice)
}

#[test]
fn building_a_story_keeps_it_valid_and_saved() {
    let mut f = Fixture::new("flow");
    let (_scene, _choice) = build_story(&mut f);

    let report = f.ok("validate", json!({}));
    assert_eq!(report["ok"], true, "{report}");
    // Saved after every call: the file on disk is the in-memory story.
    assert_eq!(&f.on_disk(), f.story());
    Story::from_ron(&fs::read_to_string(f.dir.join("story.ron")).unwrap()).unwrap();

    // The scene reads back with ids and versions.
    let text = f.call("read_scene", json!({"scene": "intro"})).unwrap();
    assert!(text.contains("say [Alice]: \"Hello Bob!\""), "{text}");
    assert!(text.contains("show Alice (happy) at left"), "{text}");
    assert!(
        text.contains("choice [0] \"Be kind\" -> label \"kind\""),
        "{text}"
    );
    assert!(text.contains("background assets/images/room.png"), "{text}");
    assert!(text.contains(" v"), "{text}");

    // Both branches play. The choice is reached after the narration line.
    let kind = f.call("simulate", json!({"choices": [0]})).unwrap();
    assert!(kind.contains("Alice: You are kind."), "{kind}");
    assert!(kind.contains("Bob: The end."), "{kind}");
    assert!(kind.contains("trust=1"), "{kind}");
    assert!(kind.contains("story ended"), "{kind}");
    let away = f
        .call("simulate", json!({"choices": ["Walk away"]}))
        .unwrap();
    assert!(!away.contains("You are kind."), "{away}");
    assert!(away.contains("Bob: The end."), "{away}");
    // Out of choices: stops and shows the options.
    let stuck = f.call("simulate", json!({})).unwrap();
    assert!(stuck.contains("awaiting a choice"), "{stuck}");
    // Bad choice index is a clear error.
    assert_eq!(
        f.err("simulate", json!({"choices": [7]})).kind,
        ErrorKind::InvalidArguments
    );

    // The action log recorded the calls.
    let log = fs::read_to_string(f.dir.join(".orbit/mcp.log")).unwrap();
    assert!(log.contains("tool=add_dialogue ok"), "{log}");
    assert!(log.contains("tool=simulate ERROR"), "{log}");
}

#[test]
fn every_tool_call_is_one_undo_entry() {
    let mut f = Fixture::new("undo");
    f.ok("create_character", json!({"name": "Alice"}));
    f.ok("create_scene", json!({"name": "s"}));
    let before = f.story().clone();
    // add_dialogue with an expression touches a string, a show node and a say node.
    f.write_asset_source("incoming/a.png");
    let img = f.ok("import_asset", json!({"source": "incoming/a.png"}));
    f.ok(
        "edit_character",
        json!({"character": "Alice", "set_expressions": [{"name": "happy", "asset": id_of(&img, "/id")}]}),
    );
    let before_dialogue = f.story().clone();
    f.ok(
        "add_dialogue",
        json!({"scene": "s", "speaker": "Alice", "text": "Hi", "expression": "happy"}),
    );
    assert_eq!(f.story().scenes[0].nodes.len(), 2);
    f.ok("undo", json!({}));
    assert_eq!(
        f.story(),
        &before_dialogue,
        "one undo reverts the whole call"
    );
    assert_eq!(&f.on_disk(), f.story(), "undo is saved too");
    f.ok("redo", json!({}));
    assert_eq!(f.story().scenes[0].nodes.len(), 2);
    assert_eq!(&f.on_disk(), f.story());
    f.ok("undo", json!({}));
    f.ok("undo", json!({})); // edit_character
    f.ok("undo", json!({})); // import_asset
    assert_eq!(f.story(), &before);
    let nothing = f.ok("undo", json!({}));
    assert!(nothing["undone"].is_null() || nothing["undone"].is_string());
}

#[test]
fn scenes_can_be_created_renamed_and_deleted() {
    let mut f = Fixture::new("scenes");
    f.ok("create_scene", json!({"name": "one"}));
    f.ok("create_scene", json!({"name": "two", "index": 0}));
    assert_eq!(f.story().scenes[0].name, "two");
    assert_eq!(
        f.err("create_scene", json!({"name": "ONE"})).kind,
        ErrorKind::Refused
    );
    assert_eq!(
        f.err("create_scene", json!({"name": "  "})).kind,
        ErrorKind::InvalidArguments
    );
    assert_eq!(
        f.err("create_scene", json!({"name": "x", "index": 9})).kind,
        ErrorKind::InvalidArguments
    );
    f.ok("rename_scene", json!({"scene": "one", "new_name": "uno"}));
    assert_eq!(f.story().scenes[1].name, "uno");
    assert_eq!(
        f.err("rename_scene", json!({"scene": "nope", "new_name": "z"}))
            .kind,
        ErrorKind::NotFound
    );
    f.ok("add_dialogue", json!({"scene": "uno", "text": "bye"}));
    let strings_before = f.story().strings.entries.len();
    assert_eq!(strings_before, 1);
    let deleted = f.ok("delete_scene", json!({"scene": "uno"}));
    assert_eq!(deleted["nodes_removed"], 1);
    assert_eq!(f.story().scenes.len(), 1);
    assert!(
        f.story().strings.entries.is_empty(),
        "orphan strings are released"
    );
    f.ok("undo", json!({}));
    assert_eq!(f.story().scenes.len(), 2);
    assert_eq!(f.story().strings.entries.len(), 1);
}

#[test]
fn deleting_a_referenced_node_or_scene_is_refused() {
    let mut f = Fixture::new("refs");
    f.ok("create_scene", json!({"name": "a"}));
    f.ok("create_scene", json!({"name": "b"}));
    let label = f.ok(
        "add_command",
        json!({"scene": "b", "command": {"type": "label", "name": "target"}}),
    );
    f.ok(
        "add_command",
        json!({"scene": "a", "command": {"type": "jump", "target": "target"}}),
    );
    let label_id = id_of(&label, "/node/id");
    let e = f.err("delete_node", json!({"node": label_id}));
    assert_eq!(e.kind, ErrorKind::Refused);
    assert!(e.message.contains("still targeted"), "{e}");
    let e = f.err("delete_scene", json!({"scene": "b"}));
    assert_eq!(e.kind, ErrorKind::Refused);
    assert_eq!(f.story().scenes.len(), 2, "nothing was removed");
    assert_eq!(&f.on_disk(), f.story());
}

#[test]
fn edits_conflict_on_stale_versions() {
    let mut f = Fixture::new("conflict");
    f.ok("create_scene", json!({"name": "s"}));
    let added = f.ok("add_dialogue", json!({"scene": "s", "text": "v1"}));
    let node = id_of(&added, "/dialogue/id");
    let version = added["dialogue"]["version"].as_u64().unwrap();
    f.ok(
        "edit_dialogue",
        json!({"node": node, "text": "v2", "expected_version": version}),
    );
    let before = f.story().clone();
    let e = f.err(
        "edit_dialogue",
        json!({"node": node, "text": "v3", "expected_version": version}),
    );
    assert_eq!(e.kind, ErrorKind::VersionConflict);
    assert!(e.message.contains("expected"), "{e}");
    assert_eq!(f.story(), &before, "no partial change");
    let e = f.err(
        "delete_node",
        json!({"node": node, "expected_version": version}),
    );
    assert_eq!(e.kind, ErrorKind::VersionConflict);
    let e = f.err(
        "move_node",
        json!({"node": node, "index": 0, "expected_version": version}),
    );
    assert_eq!(e.kind, ErrorKind::VersionConflict);
    // Without a version the edit goes through.
    f.ok("edit_dialogue", json!({"node": node, "text": "v3"}));
    assert_eq!(
        f.story().text(match &f.story().scenes[0].nodes[0].kind {
            orbit_core::NodeKind::Say { text_id, .. } => *text_id,
            _ => unreachable!(),
        }),
        Some("v3")
    );
}

#[test]
fn nodes_can_be_moved_and_deleted() {
    let mut f = Fixture::new("move");
    f.ok("create_scene", json!({"name": "s"}));
    f.ok("create_scene", json!({"name": "t"}));
    let a = id_of(
        &f.ok("add_dialogue", json!({"scene": "s", "text": "a"})),
        "/dialogue/id",
    );
    let b = id_of(
        &f.ok("add_dialogue", json!({"scene": "s", "text": "b"})),
        "/dialogue/id",
    );
    let c = id_of(
        &f.ok("add_dialogue", json!({"scene": "s", "text": "c"})),
        "/dialogue/id",
    );
    f.ok("move_node", json!({"node": c, "after": "start"}));
    let order = |f: &Fixture, scene: usize| -> Vec<String> {
        f.story().scenes[scene]
            .nodes
            .iter()
            .map(|n| n.id.to_string())
            .collect()
    };
    assert_eq!(order(&f, 0), [c.clone(), a.clone(), b.clone()]);
    f.ok("move_node", json!({"node": c, "after": b}));
    assert_eq!(order(&f, 0), [a.clone(), b.clone(), c.clone()]);
    f.ok("move_node", json!({"node": a, "scene": "t"}));
    assert_eq!(order(&f, 1), std::slice::from_ref(&a));
    assert_eq!(order(&f, 0), [b.clone(), c.clone()]);
    assert_eq!(
        f.err("move_node", json!({"node": b, "after": b})).kind,
        ErrorKind::InvalidArguments
    );
    assert_eq!(
        f.err("move_node", json!({"node": b, "after": a})).kind,
        ErrorKind::NotFound
    );
    f.ok("delete_node", json!({"node": b}));
    assert_eq!(order(&f, 0), std::slice::from_ref(&c));
    assert_eq!(
        f.story().strings.entries.len(),
        2,
        "deleted node's text released"
    );
    f.ok("undo", json!({}));
    assert_eq!(order(&f, 0), [b, c]);
    assert_eq!(f.story().strings.entries.len(), 3);
}

#[test]
fn characters_variables_and_assets() {
    let mut f = Fixture::new("defs");
    f.write_asset_source("incoming/a.png");
    f.write_asset_source("incoming/b.png");
    let a = f.ok("import_asset", json!({"source": "incoming/a.png"}));
    let b = f.ok("import_asset", json!({"source": "incoming/b.png"}));
    assert_eq!(a["copied"], true);
    assert_eq!(a["path"], "assets/images/a.png");
    assert!(f.dir.join("assets/images/a.png").is_file());
    // Importing again registers nothing new.
    let again = f.ok("import_asset", json!({"source": "incoming/a.png"}));
    assert_eq!(again["already_registered"], true);
    assert_eq!(f.story().assets.assets.len(), 2);
    // An asset already under assets/ is registered without copying.
    f.write_asset_source("assets/music/m.ogg");
    let m = f.ok(
        "import_asset",
        json!({"source": "assets/music/m.ogg", "kind": "music"}),
    );
    assert_eq!(m["copied"], false);
    // Audio needs an explicit kind.
    f.write_asset_source("incoming/x.ogg");
    assert_eq!(
        f.err("import_asset", json!({"source": "incoming/x.ogg"}))
            .kind,
        ErrorKind::InvalidArguments
    );
    let listed = f.ok("list_assets", json!({"kind": "image"}));
    assert_eq!(listed.as_array().unwrap().len(), 2);

    let created = f.ok(
        "create_character",
        json!({"name": "Alice", "expressions": [{"name": "happy", "asset": "a.png"}]}),
    );
    let id = id_of(&created, "/id");
    assert_eq!(created["expressions"][0]["path"], "assets/images/a.png");
    assert_eq!(
        f.err("create_character", json!({"name": "alice"})).kind,
        ErrorKind::Refused
    );
    // Wrong kind of asset for an expression.
    assert_eq!(
        f.err(
            "create_character",
            json!({"name": "Z", "expressions": [{"name": "x", "asset": "m.ogg"}]})
        )
        .kind,
        ErrorKind::NotFound
    );
    let edited = f.ok(
        "edit_character",
        json!({"character": &id, "name": "Alicia", "color": "#102030",
               "set_expressions": [{"name": "sad", "asset": id_of(&b, "/id")}]}),
    );
    assert_eq!(edited["name"], "Alicia");
    assert_eq!(edited["color"], "#102030");
    assert_eq!(edited["expressions"].as_array().unwrap().len(), 2);
    f.ok("create_scene", json!({"name": "s"}));
    f.ok(
        "add_command",
        json!({"scene": "s", "command": {"type": "show", "character": "Alicia", "expression": "sad"}}),
    );
    let e = f.err(
        "edit_character",
        json!({"character": id, "remove_expressions": ["sad"]}),
    );
    assert_eq!(e.kind, ErrorKind::Refused);
    f.ok(
        "edit_character",
        json!({"character": id, "remove_expressions": ["happy"]}),
    );
    assert_eq!(
        f.err("edit_character", json!({"character": id})).kind,
        ErrorKind::InvalidArguments
    );

    let v = f.ok(
        "declare_variable",
        json!({"name": "score", "initial": 1.5, "scope": "persistent"}),
    );
    assert_eq!(v["type"], "float");
    assert_eq!(v["scope"], "persistent");
    f.ok(
        "declare_variable",
        json!({"name": "route", "initial": "forest"}),
    );
    f.ok("declare_variable", json!({"name": "met", "initial": false}));
    for bad in [
        json!({"name": "1x", "initial": 1}),
        json!({"name": "ok", "initial": null}),
        json!({"name": "ok", "initial": [1]}),
    ] {
        assert_eq!(
            f.err("declare_variable", bad).kind,
            ErrorKind::InvalidArguments
        );
    }
    assert_eq!(
        f.ok("list_variables", json!({})).as_array().unwrap().len(),
        3
    );
    assert_eq!(
        f.ok("list_characters", json!({})).as_array().unwrap().len(),
        1
    );
    assert_eq!(&f.on_disk(), f.story());
}

#[test]
fn commands_cover_the_ir_and_check_their_targets() {
    let mut f = Fixture::new("commands");
    f.write_asset_source("incoming/room.png");
    f.write_asset_source("incoming/ding.wav");
    f.ok("import_asset", json!({"source": "incoming/room.png"}));
    f.ok(
        "import_asset",
        json!({"source": "incoming/ding.wav", "kind": "sound"}),
    );
    f.ok("create_scene", json!({"name": "s"}));
    f.ok("declare_variable", json!({"name": "x", "initial": 0}));
    f.ok(
        "add_command",
        json!({"scene": "s", "command": {"type": "label", "name": "top"}}),
    );
    for command in [
        json!({"type": "background", "asset": "room", "transition": {"kind": "fade", "duration": 2.0, "color": "white"}}),
        json!({"type": "play_sound", "asset": "ding.wav"}),
        json!({"type": "stop_music", "fade_out": 1.5}),
        json!({"type": "wait", "seconds": 0.25}),
        json!({"type": "set_var", "var": "x", "value": "x + 1"}),
        json!({"type": "if", "condition": "x < 3 && !false", "then_target": "top"}),
        json!({"type": "call", "label": "top"}),
        json!({"type": "return"}),
        json!({"type": "custom", "plugin_id": "acme.fx", "kind": "shake", "version": 2, "data": {"power": 3}}),
        json!({"type": "jump", "target": "top"}),
    ] {
        f.ok("add_command", json!({"scene": "s", "command": command}));
    }
    let nodes = &f.story().scenes[0].nodes;
    assert_eq!(nodes.len(), 11);
    match &nodes[1].kind {
        orbit_core::NodeKind::Background {
            transition: Some(t),
            ..
        } => {
            assert_eq!(t.duration, 2.0);
            assert!(matches!(t.kind, TransitionKind::Fade(c) if c == orbit_core::Color::WHITE));
        }
        other => panic!("{other:?}"),
    }
    // Rejections.
    for (command, kind) in [
        (json!({"type": "label", "name": "top"}), ErrorKind::Refused),
        (
            json!({"type": "jump", "target": "nowhere"}),
            ErrorKind::NotFound,
        ),
        (
            json!({"type": "background", "asset": "ding.wav"}),
            ErrorKind::NotFound,
        ),
        (
            json!({"type": "set_var", "var": "x", "value": "1 +"}),
            ErrorKind::InvalidArguments,
        ),
        (
            json!({"type": "set_var", "var": "bad name", "value": "1"}),
            ErrorKind::InvalidArguments,
        ),
        (
            json!({"type": "wait", "seconds": -1}),
            ErrorKind::InvalidArguments,
        ),
        (
            json!({"type": "show", "character": "ghost", "expression": "x"}),
            ErrorKind::NotFound,
        ),
        (json!({"type": "teleport"}), ErrorKind::InvalidArguments),
    ] {
        let before = f.story().clone();
        assert_eq!(
            f.err(
                "add_command",
                json!({"scene": "s", "command": command.clone()})
            )
            .kind,
            kind,
            "{command}"
        );
        assert_eq!(f.story(), &before);
    }
    // Jumping to a non-label node is refused with a hint.
    let first = f.story().scenes[0].nodes[1].id.to_string();
    let e = f.err(
        "add_command",
        json!({"scene": "s", "command": {"type": "jump", "target": first}}),
    );
    assert!(e.message.contains("not a label"), "{e}");
    // An undeclared variable is only a note.
    let note = f.ok(
        "add_command",
        json!({"scene": "s", "command": {"type": "set_var", "var": "undeclared", "value": "1"}}),
    );
    assert!(note["note"].as_str().unwrap().contains("declare_variable"));
}

#[test]
fn bad_input_never_panics_and_never_changes_the_project() {
    let mut f = Fixture::new("fuzz");
    f.ok("create_scene", json!({"name": "s"}));
    let before = f.story().clone();
    let junk = [
        Value::Null,
        json!([]),
        json!("text"),
        json!(42),
        json!({"scene": 1}),
        json!({"scene": "s", "unknown_field": true}),
        json!({"scene": "s", "after": "not-an-id", "text": "x"}),
        json!({"scene": "s", "index": 18446744073709551615u64, "text": "x"}),
        json!({"scene": "\u{0}", "text": "x"}),
        json!({"node": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "text": "x"}),
    ];
    for tool in [
        "read_scene",
        "add_dialogue",
        "edit_dialogue",
        "add_choice",
        "add_command",
        "delete_node",
        "move_node",
        "create_character",
        "edit_character",
        "declare_variable",
        "import_asset",
        "simulate",
        "rename_scene",
        "delete_scene",
    ] {
        for args in &junk {
            if let Err(e) = f.call(tool, args.clone()) {
                // A caught panic would surface as an internal error.
                assert_ne!(e.kind, ErrorKind::Internal, "{tool} {args}: {e}");
            }
        }
    }
    assert_eq!(f.err("no_such_tool", json!({})).kind, ErrorKind::NotFound);
    assert_eq!(f.story(), &before);
    assert_eq!(&f.on_disk(), f.story());
}

#[test]
fn files_outside_the_project_are_never_touched() {
    let mut f = Fixture::new("sec");
    let outside = temp_project("sec_outside");
    fs::write(outside.join("secret.png"), b"secret").unwrap();
    let abs_outside = outside.join("secret.png").to_string_lossy().into_owned();
    for source in [
        "../secret.png".to_owned(),
        "incoming/../../secret.png".to_owned(),
        abs_outside.clone(),
        "C:\\Windows\\win.ini".to_owned(),
        "/etc/passwd".to_owned(),
        "\\\\server\\share\\x.png".to_owned(),
    ] {
        let e = f.err("import_asset", json!({"source": source, "kind": "image"}));
        assert!(
            matches!(
                e.kind,
                ErrorKind::OutsideProject
                    | ErrorKind::NotFound
                    | ErrorKind::InvalidArguments
                    | ErrorKind::Io
            ),
            "{source}: {e}"
        );
        assert!(f.story().assets.assets.is_empty(), "{source}");
    }
    assert_eq!(
        f.err("import_asset", json!({"source": "../secret.png"}))
            .kind,
        ErrorKind::OutsideProject
    );
    assert_eq!(
        f.err("import_asset", json!({"source": abs_outside})).kind,
        ErrorKind::OutsideProject
    );
    // A destination name cannot escape either.
    f.write_asset_source("incoming/a.png");
    let ok = f.ok(
        "import_asset",
        json!({"source": "incoming/a.png", "name": "../../evil.png"}),
    );
    assert!(id_of(&ok, "/path").starts_with("assets/images/"), "{ok}");
    assert!(!f.dir.parent().unwrap().join("evil.png").exists());
    // A directory is not an asset.
    assert_eq!(
        f.err(
            "import_asset",
            json!({"source": "incoming", "kind": "image"})
        )
        .kind,
        ErrorKind::InvalidArguments
    );
    // A symbolic link pointing out of the project is refused.
    let link = f.dir.join("incoming").join("link.png");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(outside.join("secret.png"), &link).is_ok();
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(outside.join("secret.png"), &link).is_ok();
    if made {
        let e = f.err(
            "import_asset",
            json!({"source": "incoming/link.png", "kind": "image"}),
        );
        assert_eq!(e.kind, ErrorKind::OutsideProject, "{e}");
    }
    assert!(!f.dir.join("assets/images/secret.png").exists());
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn a_story_file_can_be_opened_directly() {
    let dir = temp_project("file");
    let mut story = Story::new("T");
    story.strings.add("x");
    fs::write(dir.join("mygame.ron"), story.to_ron().unwrap()).unwrap();
    let mut host = HeadlessHost::open(&dir.join("mygame.ron")).unwrap();
    host.call_tool("create_scene", json!({"name": "a"}))
        .unwrap();
    assert!(
        Story::from_ron_unchecked(&fs::read_to_string(dir.join("mygame.ron")).unwrap())
            .unwrap()
            .scenes
            .iter()
            .any(|s| s.name == "a")
    );
    assert!(HeadlessHost::open(&dir.join("missing.ron")).is_err());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn resources_describe_the_project() {
    let mut f = Fixture::new("res");
    build_story(&mut f);
    let resources = f.host.list_resources();
    let uris: Vec<&str> = resources.iter().map(|r| r.uri.as_str()).collect();
    for uri in [
        "orbit://project/manifest",
        "orbit://project/scenes",
        "orbit://project/characters",
        "orbit://project/assets",
        "orbit://project/variables",
        "orbit://project/validation",
        "orbit://schema/ir",
    ] {
        assert!(uris.contains(&uri), "{uri}");
        let (_, text) = f.host.read_resource(uri).unwrap();
        assert!(!text.is_empty());
    }
    assert!(uris.iter().any(|u| u.starts_with("orbit://scene/")));
    let (_, text) = f.host.read_resource("orbit://project/validation").unwrap();
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["ok"], true);
    let scene_uri = uris
        .iter()
        .find(|u| u.starts_with("orbit://scene/"))
        .unwrap();
    let (_, scene) = f.host.read_resource(scene_uri).unwrap();
    assert!(scene.contains("Hello Bob!"));
    assert!(f.host.read_resource("orbit://nope").is_err());
    assert!(f.host.read_resource("orbit://scene/garbage").is_err());
}

#[test]
fn validate_reports_problems_and_simulate_refuses_broken_stories() {
    let mut f = Fixture::new("broken");
    f.write_asset_source("incoming/a.png");
    f.ok("import_asset", json!({"source": "incoming/a.png"}));
    f.ok("create_scene", json!({"name": "s"}));
    f.ok(
        "add_command",
        json!({"scene": "s", "command": {"type": "background", "asset": "a"}}),
    );
    // The file disappears behind our back.
    fs::remove_file(f.dir.join("assets/images/a.png")).unwrap();
    let report = f.ok("validate", json!({}));
    assert_eq!(report["ok"], false);
    assert!(
        report["errors"][0]
            .as_str()
            .unwrap()
            .contains("does not exist"),
        "{report}"
    );
    let relaxed = f.ok("validate", json!({"check_files": false}));
    assert_eq!(relaxed["ok"], true);
}

// ------------------------------------------------------------ plugin tools

struct TestPlugin;

impl Plugin for TestPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("test.tools", "Test tools", "1.0.0")
    }

    fn register(&self, registry: &mut ProjectRegistry) {
        registry
            .register_mcp_tool(McpToolDef {
                name: "make_two_scenes".into(),
                description: "Creates two scenes, optionally failing after the first.".into(),
                input_schema: r#"{"type":"object","properties":{"fail":{"type":"boolean"}}}"#
                    .into(),
                destructive: false,
                call: Box::new(|project, args| {
                    let fail = serde_json::from_str::<Value>(args)
                        .ok()
                        .and_then(|v| v["fail"].as_bool())
                        .unwrap_or(false);
                    for (i, name) in ["p1", "p2"].iter().enumerate() {
                        project
                            .execute(Command::InsertScene {
                                index: project.story().scenes.len(),
                                scene: orbit_core::Scene {
                                    id: orbit_core::SceneId::new(),
                                    name: (*name).into(),
                                    nodes: vec![],
                                },
                            })
                            .map_err(|e| e.to_string())?;
                        if fail && i == 0 {
                            return Err("plugin gave up".into());
                        }
                    }
                    Ok("made".into())
                }),
            })
            .unwrap();
    }
}

#[test]
fn plugin_tools_are_exposed_grouped_and_rolled_back() {
    let mut f = Fixture::new("plugin");
    f.host.install_plugin(&TestPlugin).unwrap();
    let tools = f.host.list_tools();
    let tool = tools
        .iter()
        .find(|t| t.name == "test.tools.make_two_scenes")
        .expect("plugin tool listed");
    assert!(tool.input_schema.contains_key("properties"));
    assert!(!tool.destructive);

    let e = f.err("test.tools.make_two_scenes", json!({"fail": true}));
    assert_eq!(e.kind, ErrorKind::Plugin);
    assert!(
        f.story().scenes.is_empty(),
        "a failing plugin call leaves no trace"
    );

    assert_eq!(
        f.call("test.tools.make_two_scenes", json!({})).unwrap(),
        "made"
    );
    assert_eq!(f.story().scenes.len(), 2);
    assert_eq!(&f.on_disk(), f.story());
    f.ok("undo", json!({}));
    assert!(f.story().scenes.is_empty(), "two commands, one undo entry");
}

#[test]
fn story_survives_a_restart() {
    let mut f = Fixture::new("restart");
    build_story(&mut f);
    let saved = f.story().clone();
    let reopened = HeadlessHost::open(&f.dir).unwrap();
    assert_eq!(reopened.project().story(), &saved);
}

#[test]
fn scene_jumps_are_added_checked_and_protect_their_target() {
    use orbit_core::NodeKind;

    let mut f = Fixture::new("jump_scene");
    f.ok("create_scene", json!({"name": "a"}));
    f.ok("create_scene", json!({"name": "b"}));
    let mid = f.ok(
        "add_command",
        json!({"scene": "b", "command": {"type": "label", "name": "mid"}}),
    );
    let mid_id = id_of(&mid, "/node/id");
    f.ok(
        "add_command",
        json!({"scene": "a", "command": {"type": "label", "name": "own"}}),
    );

    // To the start of the scene, then to a label of that scene.
    let start = f.ok(
        "add_command",
        json!({"scene": "a", "command": {"type": "jump_scene", "scene": "b"}}),
    );
    assert!(
        start["node"]["content"]
            .as_str()
            .is_some_and(|d| d.contains("jump_scene")),
        "{start}"
    );
    f.ok(
        "add_command",
        json!({"scene": "a", "command": {"type": "jump_scene", "scene": "b", "label": "mid"}}),
    );
    let jumps: Vec<_> = f.story().scenes[0]
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::JumpScene { scene, label } => Some((*scene, *label)),
            _ => None,
        })
        .collect();
    let b = f.story().scenes[1].id;
    assert_eq!(jumps.len(), 2);
    assert_eq!(jumps[0], (b, None));
    assert_eq!(jumps[1].0, b);
    assert_eq!(jumps[1].1.map(|l| l.to_string()), Some(mid_id));

    // A label of another scene, or an unknown scene, is refused.
    let e = f.err(
        "add_command",
        json!({"scene": "a", "command": {"type": "jump_scene", "scene": "b", "label": "own"}}),
    );
    assert_eq!(e.kind, ErrorKind::InvalidArguments);
    assert!(e.message.contains("not in scene"), "{e}");
    let e = f.err(
        "add_command",
        json!({"scene": "a", "command": {"type": "jump_scene", "scene": "nope"}}),
    );
    assert_eq!(e.kind, ErrorKind::NotFound);

    // The target scene cannot be deleted while a jump aims at it.
    let e = f.err("delete_scene", json!({"scene": "b"}));
    assert_eq!(e.kind, ErrorKind::Refused);
    assert!(e.message.contains("scene jump"), "{e}");
    assert_eq!(f.story().scenes.len(), 2);

    // And the story validates (errors none).
    let report = f.ok("validate", json!({}));
    assert_eq!(
        report["errors"].as_array().map(Vec::len),
        Some(0),
        "{report}"
    );
}

#[test]
fn show_accepts_scale_mirror_and_free_positions_and_characters_a_default_scale() {
    let mut f = Fixture::new("look");
    f.write_asset_source("incoming/alice.png");
    let img = f.ok("import_asset", json!({"source": "incoming/alice.png"}));
    let created = f.ok(
        "create_character",
        json!({"name": "Alice", "default_scale": 0.8,
               "expressions": [{"name": "happy", "asset": id_of(&img, "/id")}]}),
    );
    assert_eq!(created["default_scale"], 0.8);
    f.ok("create_scene", json!({"name": "intro"}));

    // Free position, uniform scale and mirror through add_command.
    f.ok(
        "add_command",
        json!({"scene": "intro", "command": {"type": "show", "character": "Alice",
               "expression": "happy", "position": {"x": 0.4, "y": 0.95},
               "scale": 1.25, "flip_x": true}}),
    );
    // Non uniform stretch with a named position.
    f.ok(
        "add_command",
        json!({"scene": "intro", "command": {"type": "show", "character": "Alice",
               "expression": "happy", "position": "left", "scale": 1.0,
               "scale_y": 0.5, "flip_y": true}}),
    );
    // add_dialogue inserts a show with the same options.
    f.ok(
        "add_dialogue",
        json!({"scene": "intro", "speaker": "Alice", "text": "Hi", "expression": "happy",
               "position": {"x": 0.7, "y": 1.0}, "scale": 0.5, "flip_x": true}),
    );
    let shows: Vec<_> = f.story().scenes[0]
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            orbit_core::NodeKind::Show {
                position,
                scale,
                scale_y,
                flip_x,
                flip_y,
                ..
            } => Some((position.clone(), *scale, *scale_y, *flip_x, *flip_y)),
            _ => None,
        })
        .collect();
    assert_eq!(
        shows,
        [
            (
                orbit_core::Position::Free { x: 0.4, y: 0.95 },
                Some(1.25),
                None,
                true,
                false
            ),
            (
                orbit_core::Position::Left,
                Some(1.0),
                Some(0.5),
                false,
                true
            ),
            (
                orbit_core::Position::Free { x: 0.7, y: 1.0 },
                Some(0.5),
                None,
                true,
                false
            ),
        ]
    );
    assert_eq!(f.story().characters[0].default_scale, 0.8);
    assert_eq!(f.ok("validate", json!({}))["ok"], true);
    // The scene description carries the look.
    let text = f.call("read_scene", json!({"scene": "intro"})).unwrap();
    assert!(
        text.contains("free(0.400, 0.950) scale 1.25 mirrored-x"),
        "{text}"
    );
    // The file on disk is the story, and edit_character changes the default.
    f.ok(
        "edit_character",
        json!({"character": "Alice", "default_scale": 1.5}),
    );
    assert_eq!(f.story().characters[0].default_scale, 1.5);
    assert_eq!(&f.on_disk(), f.story());

    // Out-of-range values are refused and change nothing.
    let before = f.story().clone();
    for bad in [
        json!({"type": "show", "character": "Alice", "expression": "happy", "scale": 0.0}),
        json!({"type": "show", "character": "Alice", "expression": "happy", "scale": 50.0}),
        json!({"type": "show", "character": "Alice", "expression": "happy", "scale_y": -1.0}),
        json!({"type": "show", "character": "Alice", "expression": "happy",
               "position": {"x": 9.0, "y": 0.5}}),
    ] {
        let e = f.err("add_command", json!({"scene": "intro", "command": bad}));
        assert_eq!(e.kind, ErrorKind::InvalidArguments, "{e}");
    }
    f.err(
        "edit_character",
        json!({"character": "Alice", "default_scale": 0.0}),
    );
    f.err(
        "create_character",
        json!({"name": "Bob", "default_scale": 99.0}),
    );
    assert_eq!(*f.story(), before);
}
