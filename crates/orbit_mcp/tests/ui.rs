//! UI tools (UI-17), through `HeadlessHost`.

use std::fs;

use orbit_mcp::{HeadlessHost, ProjectHost};
use serde_json::{Value, json};

fn host(tag: &str) -> (std::path::PathBuf, HeadlessHost) {
    let dir = std::env::temp_dir().join(format!("orbit_mcp_ui_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let h = HeadlessHost::open(&dir).unwrap();
    (dir, h)
}

const MENU: &str = "(id: \"hello\", root: (kind: Stack(dir: Vertical)))";

fn json_of(text: String) -> Value {
    serde_json::from_str(&text).unwrap()
}

#[test]
fn list_ui_shows_hud_and_warns_about_missing_screens() {
    let (_dir, mut h) = host("hud");
    h.call_tool("write_ui_screen", json!({"ron": "(id: \"hud\", hud: true, transition_in: Some((kind: SlideUp, duration: 0.3)), root: (kind: Stack(dir: Vertical)))"})).unwrap();
    h.call_tool("create_scene", json!({"name": "s"})).unwrap();
    for screen in ["hud", "ghost"] {
        h.call_tool(
            "add_command",
            json!({"scene": "s", "command": {"type": "open_screen", "screen": screen}}),
        )
        .unwrap();
    }
    let r = json_of(h.call_tool("list_ui", json!({})).unwrap());
    let s = &r["screens"][0];
    assert_eq!(s["hud"], true);
    assert_eq!(s["transition_in"]["kind"], "SlideUp");
    let issues = r["issues"].as_array().unwrap();
    assert!(
        issues.iter().any(|i| i["message"].as_str().unwrap().contains("`ghost`")),
        "{issues:?}"
    );
    assert!(!issues.iter().any(|i| i["message"].as_str().unwrap().contains("screen `hud`")));
}

#[test]
fn screens_styles_and_splashes_round_trip() {
    let (dir, mut h) = host("rt");
    let r = json_of(h.call_tool("list_ui", json!({})).unwrap());
    assert_eq!(r["screens"].as_array().unwrap().len(), 0);

    h.call_tool("write_ui_screen", json!({"ron": MENU})).unwrap();
    assert!(dir.join("ui/hello.screen.ron").is_file());
    assert!(
        h.call_tool("write_ui_screen", json!({"ron": MENU, "create_only": true}))
            .is_err()
    );
    let r = json_of(h.call_tool("read_ui_screen", json!({"id": "hello"})).unwrap());
    assert!(r["ron"].as_str().unwrap().contains("hello"));

    h.call_tool("write_ui_style", json!({"name": "big", "ron": "()"}))
        .unwrap();
    assert!(h.call_tool("read_ui_style", json!({"name": "big"})).is_ok());
    h.call_tool("delete_ui_style", json!({"name": "big"})).unwrap();
    assert!(h.call_tool("read_ui_style", json!({"name": "big"})).is_err());

    h.call_tool(
        "set_ui_system_screen",
        json!({"system": "pause", "screen": "hello"}),
    )
    .unwrap();
    assert!(
        h.call_tool("set_ui_system_screen", json!({"system": "nope"}))
            .is_err()
    );
    h.call_tool(
        "add_ui_splash",
        json!({"kind": "text", "value": "Studio", "duration": 2.0}),
    )
    .unwrap();
    // A non-positive duration is an error: refused, nothing written.
    assert!(
        h.call_tool(
            "add_ui_splash",
            json!({"kind": "text", "value": "x", "duration": 0.0})
        )
        .is_err()
    );
    let r = json_of(h.call_tool("list_ui", json!({})).unwrap());
    assert_eq!(r["splashes"].as_array().unwrap().len(), 1);
    assert_eq!(r["screens"][0]["system_role"], "pause");

    h.call_tool("delete_ui_splash", json!({"index": 0})).unwrap();
    // Deleting a screen also drops its system link.
    h.call_tool("delete_ui_screen", json!({"id": "hello"})).unwrap();
    assert!(!dir.join("ui/hello.screen.ron").exists());
    assert!(!dir.join("ui/ui.ron").exists());
    assert!(h.call_tool("delete_ui_screen", json!({"id": "hello"})).is_err());
}

#[test]
fn bad_input_is_refused_without_writing() {
    let (dir, mut h) = host("bad");
    assert!(h.call_tool("write_ui_screen", json!({"ron": "not ron"})).is_err());
    assert!(
        h.call_tool(
            "write_ui_screen",
            json!({"ron": "(id: \"../x\", root: (kind: Stack(dir: Vertical)))"})
        )
        .is_err()
    );
    assert!(!dir.join("ui").exists() || fs::read_dir(dir.join("ui")).unwrap().next().is_none());
}
