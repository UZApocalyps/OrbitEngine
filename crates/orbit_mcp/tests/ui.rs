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

/// `ui_version` 9: screen music and editable save slots through MCP.
#[test]
fn screen_music_and_save_slot_cards_through_mcp() {
    let (dir, mut h) = host("music");
    h.call_tool(
        "write_ui_screen",
        json!({"ron": "(id: \"load\", root: (kind: Stack(dir: Vertical), children: [\
                 (id: \"slots\", kind: SaveSlots(mode: Load)),\
                 (id: \"jingle\", kind: Button(text: Literal(\"J\"), action: [PlayMusic(\"assets/music/j.ogg\"), StopMusic]))]))"}),
    )
    .unwrap();
    // The music: a missing file is a warning, not an error.
    let r = json_of(
        h.call_tool(
            "set_ui_screen_music",
            json!({"screen": "load", "music": "(asset: \"assets/music/menu.ogg\", volume: 0.5, over_game: Duck)"}),
        )
        .unwrap(),
    );
    assert!(r["missing"].as_array().unwrap().len() >= 2, "{r}");
    let list = json_of(h.call_tool("list_ui", json!({})).unwrap());
    assert_eq!(list["screens"][0]["music"], "assets/music/menu.ogg");
    let text = fs::read_to_string(dir.join("ui/load.screen.ron")).unwrap();
    assert!(text.contains("music:") && text.contains("Duck"), "{text}");
    assert!(
        h.call_tool("set_ui_screen_music", json!({"screen": "load", "music": "(volume: 3.0)"}))
            .is_err(),
        "a volume above 1 is an error"
    );
    assert!(h.call_tool("set_ui_screen_music", json!({"screen": "nope"})).is_err());
    h.call_tool("set_ui_screen_music", json!({"screen": "load"})).unwrap();
    assert!(!fs::read_to_string(dir.join("ui/load.screen.ron")).unwrap().contains("music:"));

    // The save slots card.
    let r = json_of(
        h.call_tool(
            "set_ui_save_slots",
            json!({"screen": "load", "template": "default",
                   "look": "(card_style: Some(\"card\"), card_height: Some(200.0))"}),
        )
        .unwrap(),
    );
    assert_eq!(r["templated"], true);
    let text = fs::read_to_string(dir.join("ui/load.screen.ron")).unwrap();
    assert!(text.contains("SlotField(Thumbnail)") && text.contains("SlotEmpty"), "{text}");
    assert!(text.contains("card_height"), "{text}");
    let r = json_of(h.call_tool("set_ui_save_slots", json!({"screen": "load", "template": "stock"})).unwrap());
    assert_eq!(r["templated"], false);
    assert!(h.call_tool("set_ui_save_slots", json!({"screen": "load", "template": "wat"})).is_err());
    assert!(h.call_tool("set_ui_save_slots", json!({"screen": "load", "widget": "jingle"})).is_err());
}

/// `ui_version` 8: image-state styles, the dialogue box section and
/// applying a style to a widget, through the MCP tools.
#[test]
fn image_button_styles_and_the_dialogue_box_through_mcp() {
    let (dir, mut h) = host("dlg");
    let style = "(fills: (normal: Some(Image(image: \"assets/ui/b.png\", fit: Stretch)), \
                 hover: Some(NineSlice(image: \"assets/ui/b_hover.png\", insets: (8.0, 8.0, 8.0, 8.0)))), \
                 show_text: Some(false), shadow: Some((color: (0, 0, 0, 90), offset: (4.0, 4.0))))";
    h.call_tool("write_ui_style", json!({"name": "img_button", "ron": style})).unwrap();
    let r = json_of(h.call_tool("read_ui_style", json!({"name": "img_button"})).unwrap());
    let ron = r["ron"].as_str().unwrap();
    assert!(ron.contains("b_hover.png") && ron.contains("show_text"), "{ron}");

    h.call_tool(
        "write_ui_screen",
        json!({"ron": "(id: \"menu\", root: (kind: Stack(dir: Vertical), children: [\
                 (id: \"go\", kind: Button(text: Literal(\"Go\"))),\
                 (id: \"logo\", kind: Image(asset: Some(\"assets/ui/logo.png\"), action: [Open(\"menu\")]))]))"}),
    )
    .unwrap();
    h.call_tool("set_ui_widget_style", json!({"screen": "menu", "widget": "go", "style": "img_button"}))
        .unwrap();
    let screen = fs::read_to_string(dir.join("ui/menu.screen.ron")).unwrap();
    assert!(screen.contains("img_button") && screen.contains("Open(\"menu\")"), "{screen}");
    assert!(
        h.call_tool("set_ui_widget_style", json!({"screen": "menu", "widget": "nope", "style": "x"}))
            .is_err()
    );

    // The dialogue box: a section written alone leaves the others.
    let r = json_of(
        h.call_tool(
            "write_ui_dialogue",
            json!({"ron": "(textbox: (style: Some(\"img_button\"), name_plate: Some((style: Some(\"ghost\"))), \
                   anchor: Some((canvas: (0.5, 1.0), pivot: (0.5, 1.0), offset: (0.0, -20.0)))))"}),
        )
        .unwrap(),
    );
    let issues = r["issues"].as_array().unwrap();
    assert!(
        issues.iter().any(|i| i["message"].as_str().unwrap().contains("unknown style `ghost`")),
        "{issues:?}"
    );
    let r = json_of(h.call_tool("read_ui_dialogue", json!({})).unwrap());
    let ron = r["ron"].as_str().unwrap();
    assert!(ron.contains("img_button") && ron.contains("show_numbers"), "{ron}");
    assert!(h.call_tool("write_ui_dialogue", json!({"ron": "(textbox: 3)"})).is_err());
}
