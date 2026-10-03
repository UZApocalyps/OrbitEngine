//! The 3D scene tools (M6-14), through `HeadlessHost` (no GPU, no protocol).

use std::fs;
use std::path::PathBuf;

use orbit_core::AssetKind;
use orbit_mcp::{ErrorKind, HeadlessHost, ProjectHost, ToolError};
use serde_json::{Value, json};

struct Fixture {
    dir: PathBuf,
    host: HeadlessHost,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit_mcp3d_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        fs::create_dir_all(&dir).unwrap();
        let host = HeadlessHost::open(&dir).unwrap();
        Self { dir, host }
    }

    fn ok(&mut self, name: &str, args: Value) -> Value {
        let text = self
            .host
            .call_tool(name, args)
            .unwrap_or_else(|e| panic!("{name} failed: {e}"));
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }

    fn err(&mut self, name: &str, args: Value) -> ToolError {
        match self.host.call_tool(name, args) {
            Ok(text) => panic!("{name} should have failed, got: {text}"),
            Err(e) => e,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn the_tools_are_advertised() {
    let f = Fixture::new("list");
    let tools = f.host.list_tools();
    for name in [
        "list_scenes3d",
        "read_scene3d",
        "validate_scene3d",
        "create_scene3d",
        "edit_scene3d",
        "write_scene3d",
    ] {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(tool.input_schema.get("type"), Some(&json!("object")));
    }
    let read_only = |n: &str| tools.iter().find(|t| t.name == n).unwrap().read_only;
    assert!(read_only("list_scenes3d") && read_only("read_scene3d") && read_only("validate_scene3d"));
    assert!(!read_only("edit_scene3d"));
}

#[test]
fn create_declares_the_asset_and_the_file_reads_back() {
    let mut f = Fixture::new("create");
    let made = f.ok("create_scene3d", json!({"name": "Living room"}));
    assert_eq!(made["path"], "assets/3d/Living_room.scene3d.ron");
    assert!(f.dir.join("assets/3d/Living_room.scene3d.ron").is_file());
    let story_assets = &f.host.project().story().assets.assets;
    assert!(
        story_assets
            .iter()
            .any(|a| a.kind == AssetKind::Scene3d && a.path == "assets/3d/Living_room.scene3d.ron")
    );

    let list = f.ok("list_scenes3d", json!({}));
    assert_eq!(list["scenes"][0]["cameras"], json!(["main"]));
    assert_eq!(list["scenes"][0]["errors"], 0);

    // By id, by path, by name stem.
    for key in [
        made["id"].as_str().unwrap().to_owned(),
        "assets/3d/Living_room.scene3d.ron".to_owned(),
        "Living_room.scene3d".to_owned(),
    ] {
        let read = f.ok("read_scene3d", json!({"scene": key}));
        assert_eq!(read["scene"]["nodes"].as_array().unwrap().len(), 2, "{key}");
    }
    let again = f.err("create_scene3d", json!({"name": "Living room"}));
    assert_eq!(again.kind, ErrorKind::Refused);
    let bad = f.err("create_scene3d", json!({"name": "x", "path": "../x.scene3d.ron"}));
    assert_eq!(bad.kind, ErrorKind::InvalidArguments);
}

#[test]
fn edits_are_written_and_validated() {
    let mut f = Fixture::new("edit");
    f.ok("create_scene3d", json!({"name": "room"}));
    let r = f.ok(
        "edit_scene3d",
        json!({"scene": "room", "ops": [
            {"op": "add_node", "name": "Lamp", "kind": {"type": "point_light", "intensity": 20.0, "range": 6.0},
             "translation": [1.0, 2.0, 0.5]},
            {"op": "add_node", "name": "Close-up", "kind": {"type": "camera", "camera_name": "closeup", "fov_y_deg": 30.0},
             "translation": [0.0, 1.6, 2.0], "rotation_euler_deg": [0.0, 15.0, 0.0]},
            {"op": "set_material", "name": "Wall", "base_color": [0.9, 0.85, 0.8, 1.0], "roughness": 0.8},
            {"op": "set_environment", "background_color": [0.2, 0.3, 0.4], "ambient_intensity": 0.5}
        ]}),
    );
    assert_eq!(r["created_nodes"].as_array().unwrap().len(), 2);
    let read = f.ok("read_scene3d", json!({"scene": "room"}));
    let cams: Vec<&str> = read["scene"]["cameras"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["camera_name"].as_str().unwrap())
        .collect();
    assert_eq!(cams, ["main", "closeup"]);
    let lamp = read["scene"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["name"] == "Lamp")
        .unwrap();
    assert_eq!(lamp["translation"], json!([1.0, 2.0, 0.5]));
    assert_eq!(lamp["kind"]["type"], "point_light");
    assert_eq!(read["scene"]["materials"][0]["name"], "Wall");
    assert_eq!(read["scene"]["environment"]["background"]["color"], json!([0.2, 0.3, 0.4]));

    // A failing edit changes nothing: the first op is valid, the second not.
    let before = fs::read_to_string(f.dir.join("assets/3d/room.scene3d.ron")).unwrap();
    let e = f.err(
        "edit_scene3d",
        json!({"scene": "room", "ops": [
            {"op": "add_node", "name": "Extra", "kind": {"type": "empty"}},
            {"op": "set_visible", "node": "Nope", "visible": false}
        ]}),
    );
    assert_eq!(e.kind, ErrorKind::NotFound);
    assert!(e.message.starts_with("op 1:"), "{}", e.message);
    // The result would have an error (duplicate camera name): refused.
    let e = f.err(
        "edit_scene3d",
        json!({"scene": "room", "ops": [
            {"op": "set_camera", "node": "Close-up", "camera_name": "main"}
        ]}),
    );
    assert!(e.message.contains("duplicate camera"), "{}", e.message);
    assert_eq!(
        fs::read_to_string(f.dir.join("assets/3d/room.scene3d.ron")).unwrap(),
        before
    );
}

#[test]
fn shapes_are_added_and_changed() {
    let mut f = Fixture::new("shapes");
    f.ok("create_scene3d", json!({"name": "room"}));
    f.ok(
        "edit_scene3d",
        json!({"scene": "room", "ops": [
            {"op": "set_material", "name": "Red", "base_color": [1.0, 0.0, 0.0, 1.0]},
            {"op": "add_node", "name": "Box", "kind": {"type": "shape", "shape": "cube", "size": [2.0, 1.0, 1.0], "material": "Red"}},
            {"op": "add_node", "name": "Ball", "kind": {"type": "shape", "shape": "sphere"}, "translation": [0.0, 1.0, 0.0]},
            {"op": "set_shape", "node": "Ball", "radius": 0.25, "segments": 12},
            {"op": "set_shape", "node": "Box", "shape": "torus", "material": ""}
        ]}),
    );
    let read = f.ok("read_scene3d", json!({"scene": "room"}));
    let node = |name: &str| {
        read["scene"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"] == name)
            .unwrap()["kind"]
            .clone()
    };
    assert_eq!(node("Ball")["type"], "shape");
    assert_eq!(node("Ball")["shape"], "sphere");
    assert_eq!(node("Ball")["radius"], json!(0.25));
    assert_eq!(node("Ball")["segments"], json!(12));
    assert_eq!(node("Box")["shape"], "torus");
    assert_eq!(node("Box")["material"], Value::Null);
    let text = fs::read_to_string(f.dir.join("assets/3d/room.scene3d.ron")).unwrap();
    assert!(text.contains("Torus"), "{text}");

    // Unknown kind, a parameter the kind does not have, unknown material.
    for (op, needle) in [
        (json!({"op": "add_node", "name": "X", "kind": {"type": "shape", "shape": "teapot"}}), "unknown shape"),
        (json!({"op": "set_shape", "node": "Ball", "tube": 1.0}), "no `tube`"),
        (json!({"op": "set_shape", "node": "Ball", "material": "Nope"}), "unknown material"),
        (json!({"op": "set_shape", "node": "Ball", "radius": -1.0}), "positive"),
    ] {
        let e = f.err("edit_scene3d", json!({"scene": "room", "ops": [op]}));
        assert!(e.message.contains(needle), "{needle}: {}", e.message);
    }
}

#[test]
fn validation_checks_the_story_cameras() {
    let mut f = Fixture::new("validate");
    let made = f.ok("create_scene3d", json!({"name": "room"}));
    f.ok("create_scene", json!({"name": "intro"}));
    f.ok(
        "add_command",
        json!({"scene": "intro", "command": {"type": "show_scene3d", "scene": made["id"], "camera": "main"}}),
    );
    let ok = f.ok("validate_scene3d", json!({"scene": "room"}));
    assert_eq!(ok["ok"], true);
    assert!(ok["story_issues"].as_array().unwrap().is_empty());

    // A camera renamed in the scene breaks the story: reported.
    f.ok(
        "edit_scene3d",
        json!({"scene": "room", "ops": [{"op": "set_camera", "node": "Camera", "camera_name": "hall"}]}),
    );
    let bad = f.ok("validate_scene3d", json!({"scene": "room"}));
    assert_eq!(bad["ok"], false);
    assert!(bad["story_issues"][0]["message"].as_str().unwrap().contains("`main`"));
}

#[test]
fn write_replaces_the_scene_from_ron_and_refuses_errors() {
    let mut f = Fixture::new("write");
    f.ok("create_scene3d", json!({"name": "room"}));
    let read = f.ok("read_scene3d", json!({"scene": "room"}));
    let ron = read["ron"].as_str().unwrap().to_owned();
    f.ok("write_scene3d", json!({"scene": "room", "ron": ron}));
    let e = f.err("write_scene3d", json!({"scene": "room", "ron": "not ron at all"}));
    assert_eq!(e.kind, ErrorKind::InvalidArguments);
    // A scene with a parent cycle is refused.
    let broken = ron.replacen("parent: None", "parent: Some(\"01ARZ3NDEKTSV4RRFFQ69G5FAV\")", 1);
    assert!(f.host.call_tool("write_scene3d", json!({"scene": "room", "ron": broken})).is_err());
}

#[test]
fn undeclared_scene_files_are_listed() {
    let mut f = Fixture::new("undeclared");
    f.ok("create_scene3d", json!({"name": "a"}));
    fs::copy(
        f.dir.join("assets/3d/a.scene3d.ron"),
        f.dir.join("assets/3d/stray.scene3d.ron"),
    )
    .unwrap();
    let list = f.ok("list_scenes3d", json!({}));
    assert_eq!(list["undeclared_files"], json!(["assets/3d/stray.scene3d.ron"]));
}

#[test]
fn a_scene_that_is_not_an_asset_is_not_found() {
    let mut f = Fixture::new("missing");
    let e = f.err("read_scene3d", json!({"scene": "ghost"}));
    assert_eq!(e.kind, ErrorKind::NotFound);
}
