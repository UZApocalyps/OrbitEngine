//! The 2D rig tools (M5B-07), through `HeadlessHost` (no GPU, no protocol).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use orbit_anim::{Angle, Rgba};
use orbit_anim::rig::{
    Attachment, AttachmentId, Bone, BoneId, RegionAttachment, Rig, RigClip, RigTrack, Skin, Slot, SlotId,
    Xform, save_rig,
};
use orbit_anim::{Key, Track};
use orbit_anim::{Curve, Loop};
use orbit_mcp::{ErrorKind, HeadlessHost, ProjectHost, ToolError};
use serde_json::{Value, json};

struct Fixture {
    dir: PathBuf,
    host: HeadlessHost,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit_mcprig_{tag}_{}_{}",
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

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    /// A round opaque picture on a transparent background.
    fn write_disc(&self, rel: &str) {
        let mut img = image::RgbaImage::new(32, 32);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            if dx * dx + dy * dy < 14.0 * 14.0 {
                *p = image::Rgba([200, 120, 40, 255]);
            }
        }
        let path = self.dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        img.save(path).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn bone(id: &str, parent: Option<&str>, x: f32, y: f32, length: f32) -> Bone {
    Bone {
        id: BoneId::new(id),
        name: id.to_string(),
        parent: parent.map(BoneId::new),
        rest: Xform::translate(x, y),
        length,
    }
}

/// root + arm (40 long, 50 up), a body region on the root, a `wave` clip
/// turning the arm by 90 degrees in one second.
fn hero_rig() -> Rig {
    let mut attachments = BTreeMap::new();
    attachments.insert(
        AttachmentId::new("body"),
        Attachment::Region(RegionAttachment {
            image: "assets/images/body.png".into(),
            size: [32.0, 32.0],
            xform: Xform::translate(0.0, -16.0),
        }),
    );
    let mut clips = BTreeMap::new();
    clips.insert(
        "wave".to_string(),
        RigClip {
            duration: 1.0,
            looping: Loop::Forever,
            tracks: vec![RigTrack::BoneRotate {
                bone: BoneId::new("arm"),
                track: Track::new(vec![
                    Key { t: 0.0, value: Angle(0.0), ease: Curve::default() },
                    Key { t: 1.0, value: Angle(90.0), ease: Curve::default() },
                ]),
            }],
        },
    );
    Rig {
        bones: vec![bone("root", None, 0.0, 0.0, 10.0), bone("arm", Some("root"), 0.0, -50.0, 40.0)],
        slots: vec![Slot {
            id: SlotId::new("s_body"),
            name: "body".into(),
            bone: BoneId::new("root"),
            attachment: Some(AttachmentId::new("body")),
            color: Rgba::new(1.0, 1.0, 1.0, 1.0),
        }],
        skins: vec![Skin { name: "default".into(), attachments }, Skin { name: "winter".into(), attachments: BTreeMap::new() }],
        clips,
        ..Rig::default()
    }
}

/// A project with the rig file (not declared), its picture and a character.
fn project(tag: &str) -> Fixture {
    let mut f = Fixture::new(tag);
    f.write("assets/rigs/hero.rig.ron", save_rig(&hero_rig()).unwrap().as_bytes());
    f.write_disc("assets/images/body.png");
    f.write("incoming/poster.png", b"not really a png");
    let poster = f.ok("import_asset", json!({"source": "incoming/poster.png"}));
    f.ok(
        "create_character",
        json!({"name": "Hero", "expressions": [{"name": "idle", "asset": poster["id"]}]}),
    );
    f
}

#[test]
fn the_tools_are_advertised() {
    let f = Fixture::new("adv");
    let tools = f.host.list_tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    for n in ["rig_list", "rig_read", "rig_sample", "rig_mesh_auto", "set_expression_rig"] {
        assert!(names.contains(&n), "missing {n}");
    }
    for n in ["rig_list", "rig_read", "rig_sample"] {
        assert!(tools.iter().find(|t| t.name == n).unwrap().read_only, "{n}");
    }
}

#[test]
fn list_and_read() {
    let mut f = project("list");
    let list = f.ok("rig_list", json!({}));
    assert_eq!(list["count"], 1);
    let r = &list["rigs"][0];
    assert_eq!(r["path"], "assets/rigs/hero.rig.ron");
    assert_eq!(r["declared"], false);
    assert_eq!(r["skins"], json!(["default", "winter"]));
    assert_eq!(r["clips"][0]["name"], "wave");
    assert_eq!(r["images"], json!(["assets/images/body.png"]));
    assert_eq!(r["missing_images"], json!([]));

    for key in ["hero", "hero.rig.ron", "assets/rigs/hero.rig.ron"] {
        let read = f.ok("rig_read", json!({"rig": key}));
        assert_eq!(read["rig"]["bones"].as_array().unwrap().len(), 2, "{key}");
        assert_eq!(read["rig"]["bones"][1]["parent"], "root");
        assert!(read["ron"].as_str().unwrap().contains("wave"));
    }
    assert_eq!(f.err("rig_read", json!({"rig": "nobody"})).kind, ErrorKind::NotFound);
}

#[test]
fn sample_wraps_time_like_the_runtime() {
    let mut f = project("sample");
    let s = f.ok("rig_sample", json!({"rig": "hero", "clip": "wave", "time": 0.5}));
    let arm = &s["bones"][1];
    assert_eq!(arm["name"], "arm");
    assert!((arm["local"]["rotation"].as_f64().unwrap() - 45.0).abs() < 0.01);
    // 1.25 s, ping-pong: back at 0.75 of the clip.
    let s = f.ok(
        "rig_sample",
        json!({"rig": "hero", "clip": "wave", "time": 1.25, "loop_mode": "ping_pong"}),
    );
    assert!((s["clip_time"].as_f64().unwrap() - 0.75).abs() < 0.001);
    assert!((s["bones"][1]["local"]["rotation"].as_f64().unwrap() - 67.5).abs() < 0.01);
    // Once: held on the last pose.
    let s = f.ok("rig_sample", json!({"rig": "hero", "clip": "wave", "time": 9.0, "loop_mode": "once"}));
    assert!((s["bones"][1]["local"]["rotation"].as_f64().unwrap() - 90.0).abs() < 0.01);
    assert_eq!(f.err("rig_sample", json!({"rig": "hero", "clip": "nope"})).kind, ErrorKind::NotFound);
}

#[test]
fn mesh_auto_writes_a_weighted_mesh() {
    let mut f = project("mesh");
    let r = f.ok("rig_mesh_auto", json!({"rig": "hero", "attachment": "body", "spacing": 8.0}));
    assert!(r["vertices"].as_u64().unwrap() >= 4);
    assert!(r["triangles"].as_u64().unwrap() >= 2);
    assert_eq!(r["weighted"], true);
    let read = f.ok("rig_read", json!({"rig": "hero"}));
    let att = &read["rig"]["skins"][0]["attachments"][0];
    assert_eq!(att["type"], "mesh");
    assert_eq!(att["weighted"], true);
    assert!(read["issues"].as_array().unwrap().iter().all(|i| i["severity"] != "error"));

    // Refusals leave the file alone.
    let before = fs::read_to_string(f.dir.join("assets/rigs/hero.rig.ron")).unwrap();
    assert_eq!(
        f.err("rig_mesh_auto", json!({"rig": "hero", "attachment": "ghost"})).kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        f.err("rig_mesh_auto", json!({"rig": "hero", "attachment": "body", "image": "assets/images/none.png"})).kind,
        ErrorKind::NotFound
    );
    assert_eq!(fs::read_to_string(f.dir.join("assets/rigs/hero.rig.ron")).unwrap(), before);
}

#[test]
fn set_expression_rig_declares_the_rig_and_undoes() {
    let mut f = project("set");
    let c = f.ok(
        "set_expression_rig",
        json!({"character": "Hero", "expression": "idle", "rig": "hero", "clip": "wave",
               "skin": "winter", "loop_mode": "ping_pong"}),
    );
    assert_eq!(c["clip"], "wave");
    assert_eq!(c["loop_mode"], "ping_pong");
    let rig_id = c["rig"].as_str().unwrap().to_owned();

    let chars = f.ok("list_characters", json!({}));
    let rig = &chars[0]["expressions"][0]["rig"];
    assert_eq!(rig["rig"], rig_id.as_str());
    assert_eq!(rig["skin"], "winter");
    assert_eq!(rig["loop_mode"], "ping_pong");
    let list = f.ok("rig_list", json!({}));
    assert_eq!(list["rigs"][0]["declared"], true);
    assert_eq!(list["rigs"][0]["used_by"].as_array().unwrap().len(), 1);

    // Back to the still image, by asset id this time.
    f.ok("set_expression_rig", json!({"character": "Hero", "expression": "idle"}));
    let chars = f.ok("list_characters", json!({}));
    assert!(chars[0]["expressions"][0]["rig"].is_null());
    f.ok("undo", json!({}));
    let chars = f.ok("list_characters", json!({}));
    assert_eq!(chars[0]["expressions"][0]["rig"]["clip"], "wave");
    f.ok(
        "set_expression_rig",
        json!({"character": "Hero", "expression": "idle", "rig": rig_id, "clip": "wave"}),
    );
}

#[test]
fn set_expression_rig_refusals() {
    let mut f = project("refuse");
    let wave = |extra: Value| {
        let mut a = json!({"character": "Hero", "expression": "idle", "rig": "hero", "clip": "wave"});
        for (k, v) in extra.as_object().unwrap() {
            a[k] = v.clone();
        }
        a
    };
    assert_eq!(f.err("set_expression_rig", wave(json!({"clip": "dance"}))).kind, ErrorKind::Refused);
    assert_eq!(f.err("set_expression_rig", wave(json!({"skin": "summer"}))).kind, ErrorKind::Refused);
    assert_eq!(f.err("set_expression_rig", wave(json!({"expression": "angry"}))).kind, ErrorKind::NotFound);
    let poster = f.ok("list_assets", json!({}));
    let image_id = poster
        .to_string()
        .split("\"id\": \"")
        .nth(1)
        .map(|s| s.split('"').next().unwrap().to_owned());
    if let Some(id) = image_id {
        // An image asset is not a rig.
        assert!(f.host.call_tool("set_expression_rig", wave(json!({"rig": id}))).is_err());
    }
    // Nothing was declared by the refusals.
    let list = f.ok("rig_list", json!({}));
    assert_eq!(list["rigs"][0]["declared"], false);
    let no_clip = f.err(
        "set_expression_rig",
        json!({"character": "Hero", "expression": "idle", "rig": "hero"}),
    );
    assert_eq!(no_clip.kind, ErrorKind::InvalidArguments);
}

#[test]
fn validate_reports_rig_problems() {
    let mut f = project("validate");
    f.ok(
        "set_expression_rig",
        json!({"character": "Hero", "expression": "idle", "rig": "hero", "clip": "wave"}),
    );
    let v = f.ok("validate", json!({}));
    assert_eq!(v["rig_problem_count"], 0, "{v}");

    // The rig file loses its clip behind the story's back.
    let mut rig = hero_rig();
    rig.clips.clear();
    f.write("assets/rigs/hero.rig.ron", save_rig(&rig).unwrap().as_bytes());
    let v = f.ok("validate", json!({}));
    assert_eq!(v["rig_problem_count"], 1, "{v}");
    assert!(v["rig_problems"][0].as_str().unwrap().contains("no clip `wave`"));
}
