//! The rig building tools (`rig_create`, `rig_bone_*`, `rig_bind`, clips and
//! keys...), through `HeadlessHost`: a 2-bone rig is built from a generated
//! picture, bound, animated and sampled; refusals leave the file untouched.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use orbit_anim::rig::{Rig, load_rig};
use orbit_mcp::rig_tools::{LiveRig, set_live_rig};
use orbit_mcp::{ErrorKind, HeadlessHost, ProjectHost, ToolError};
use serde_json::{Value, json};

struct Fixture {
    dir: PathBuf,
    host: HeadlessHost,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit_mcprigedit_{tag}_{}_{}",
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
        let text = self.host.call_tool(name, args).unwrap_or_else(|e| panic!("{name} failed: {e}"));
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }

    fn err(&mut self, name: &str, args: Value) -> ToolError {
        match self.host.call_tool(name, args) {
            Ok(text) => panic!("{name} should have failed, got: {text}"),
            Err(e) => e,
        }
    }

    /// A 32x64 opaque bar on a transparent canvas (a "character").
    fn write_bar(&self, rel: &str) {
        let mut img = image::RgbaImage::new(32, 64);
        for (x, y, p) in img.enumerate_pixels_mut() {
            if (4..28).contains(&x) {
                *p = image::Rgba([200, if y < 32 { 40 } else { 160 }, 90, 255]);
            }
        }
        let path = self.dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        img.save(path).unwrap();
    }

    fn rig_path(&self, stem: &str) -> PathBuf {
        self.dir.join(format!("assets/rigs/{stem}.rig.ron"))
    }

    fn rig_file(&self, stem: &str) -> String {
        fs::read_to_string(self.rig_path(stem)).unwrap()
    }

    fn rig(&self, stem: &str) -> Rig {
        load_rig(&self.rig_file(stem)).unwrap().rig
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn no_errors(v: &Value) {
    let issues = v["issues"].as_array().cloned().unwrap_or_default();
    assert!(issues.iter().all(|i| i["severity"] != "error"), "{v}");
}

/// A hero rig: root at the feet, `spine` and `head` going up, bound, with a
/// `nod` clip (the head turns by 90 degrees in one second, in-out easing).
fn hero(f: &mut Fixture) {
    f.write_bar("incoming/hero.png");
    f.ok("rig_create", json!({"image": "incoming/hero.png"}));
    f.ok("rig_bone_add", json!({"rig": "hero", "name": "spine", "parent": "root", "head": [0, 0], "tip": [0, -32]}));
    f.ok("rig_bone_add", json!({"rig": "hero", "name": "head", "parent": "spine", "head": [0, -32], "tip": [0, -64]}));
    f.ok("rig_bind", json!({"rig": "hero"}));
    f.ok("rig_clip_create", json!({"rig": "hero", "name": "nod", "duration": 1.0, "looping": "forever"}));
    f.ok(
        "rig_clip_key_set",
        json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "head", "time": 0.0, "value": 0.0, "ease": "InOutQuad"}),
    );
    f.ok(
        "rig_clip_key_set",
        json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "head", "time": 1.0, "value": 90.0}),
    );
}

#[test]
fn the_edit_tools_are_advertised() {
    let f = Fixture::new("adv");
    let tools = f.host.list_tools();
    for n in [
        "rig_create", "rig_add_image_part", "rig_delete", "rig_bone_add", "rig_bone_update", "rig_bone_delete",
        "rig_bone_reparent", "rig_bind", "rig_slot_update", "rig_skin_add", "rig_skin_duplicate", "rig_skin_delete",
        "rig_ik_set", "rig_ik_remove", "rig_clip_create", "rig_clip_update", "rig_clip_delete", "rig_clip_key_set",
        "rig_clip_key_delete", "rig_add_to_project", "rig_cut",
    ] {
        let t = tools.iter().find(|t| t.name == n).unwrap_or_else(|| panic!("missing {n}"));
        assert!(!t.read_only, "{n}");
    }
}

#[test]
fn a_two_bone_rig_is_built_bound_animated_and_sampled() {
    let mut f = Fixture::new("e2e");
    f.write_bar("incoming/hero.png");

    // The picture is imported as an asset; the rig has a root bone and a mesh.
    let c = f.ok("rig_create", json!({"image": "incoming/hero.png"}));
    assert_eq!(c["rig"], "assets/rigs/hero.rig.ron");
    assert_eq!(c["image"], "assets/images/hero.png");
    assert_eq!(c["summary"]["bones"][0]["name"], "root");
    assert!(f.dir.join("assets/images/hero.png").is_file());
    let assets = f.ok("list_assets", json!({})).to_string();
    assert!(assets.contains("assets/images/hero.png"), "{assets}");

    let spine = f.ok("rig_bone_add", json!({"rig": "hero", "name": "spine", "parent": "root", "head": [0, 0], "tip": [0, -32]}));
    assert!(spine["bone"].as_str().is_some());
    // Pointing up like its parent: no extra rotation relative to it.
    let spine_rot = spine["summary"]["bones"][1]["rotation"].as_f64().unwrap();
    assert!(spine_rot.abs() < 0.01, "{spine_rot}");
    assert!((spine["summary"]["bones"][1]["length"].as_f64().unwrap() - 32.0).abs() < 0.01);
    f.ok("rig_bone_add", json!({"rig": "hero", "name": "head", "parent": "spine", "head": [0, -32], "tip": [0, -64]}));

    let b = f.ok("rig_bind", json!({"rig": "hero"}));
    assert_eq!(b["meshes_bound"], 1);
    let read = f.ok("rig_read", json!({"rig": "hero"}));
    assert_eq!(read["rig"]["skins"][0]["attachments"][0]["weighted"], true);

    f.ok("rig_clip_create", json!({"rig": "hero", "name": "nod", "duration": 1.0, "looping": "forever"}));
    let k = f.ok(
        "rig_clip_key_set",
        json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "head", "time": 0.0, "value": 0.0, "ease": "InOutQuad"}),
    );
    assert_eq!(k["keys_on_track"], 1);
    let k = f.ok(
        "rig_clip_key_set",
        json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "head", "time": 1.0, "value": 90.0}),
    );
    assert_eq!(k["keys_on_track"], 2);
    // A key at the same time replaces.
    let k = f.ok(
        "rig_clip_key_set",
        json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "head", "time": 1.0, "value": 90.0}),
    );
    assert_eq!(k["keys_on_track"], 2);
    assert_eq!(k["replaced_existing_key"], true);

    let at0 = f.ok("rig_sample", json!({"rig": "hero", "clip": "nod", "time": 0.0}));
    assert!(at0["meshes"][0]["max_displacement"].as_f64().unwrap() < 0.01, "{at0}");
    let mid = f.ok("rig_sample", json!({"rig": "hero", "clip": "nod", "time": 0.5, "vertices": true}));
    let head = mid["bones"].as_array().unwrap().iter().find(|b| b["name"] == "head").unwrap();
    assert!((head["local"]["rotation"].as_f64().unwrap() - 45.0).abs() < 0.01, "{head}");
    // The mesh really deforms: the vertices around the head moved.
    let m = &mid["meshes"][0];
    assert!(m["max_displacement"].as_f64().unwrap() > 3.0, "{m}");
    assert_eq!(m["vertices"].as_array().unwrap().len(), m["vertex_count"].as_u64().unwrap() as usize);

    // Clean, declarable, no errors.
    let read = f.ok("rig_read", json!({"rig": "hero"}));
    no_errors(&read);
    let d = f.ok("rig_add_to_project", json!({"rig": "hero"}));
    assert_eq!(d["already_declared"], false);
    assert!(f.ok("rig_add_to_project", json!({"rig": "hero"}))["already_declared"].as_bool().unwrap());
    let v = f.ok("validate", json!({}));
    assert_eq!(v["rig_problem_count"], 0, "{v}");

    // The file on disk is what the tools said.
    let rig = f.rig("hero");
    assert_eq!(rig.bones.len(), 3);
    assert_eq!(rig.clips["nod"].tracks.len(), 1);
}

#[test]
fn refusals_write_nothing() {
    let mut f = Fixture::new("refuse");
    hero(&mut f);
    let before = f.rig_file("hero");

    let cases: Vec<(&str, Value, ErrorKind)> = vec![
        ("rig_bone_add", json!({"rig": "hero", "name": "head", "parent": "root"}), ErrorKind::Refused),
        ("rig_bone_add", json!({"rig": "hero", "parent": "nobody"}), ErrorKind::NotFound),
        ("rig_bone_add", json!({"rig": "hero", "length": -4.0}), ErrorKind::InvalidArguments),
        ("rig_bone_add", json!({"rig": "hero", "tip": [1, 2]}), ErrorKind::InvalidArguments),
        ("rig_bone_reparent", json!({"rig": "hero", "bone": "spine", "parent": "head"}), ErrorKind::Refused),
        ("rig_bone_reparent", json!({"rig": "hero", "bone": "spine", "parent": "spine"}), ErrorKind::Refused),
        ("rig_bone_delete", json!({"rig": "hero", "bone": "nobody"}), ErrorKind::NotFound),
        ("rig_slot_update", json!({"rig": "hero", "slot": "hero", "attachment": "ghost"}), ErrorKind::NotFound),
        ("rig_slot_update", json!({"rig": "hero", "slot": "hero", "color": [2, 0, 0, 1]}), ErrorKind::InvalidArguments),
        ("rig_slot_update", json!({"rig": "hero", "slot": "hero", "draw_order": 5}), ErrorKind::InvalidArguments),
        ("rig_skin_delete", json!({"rig": "hero", "skin": "default"}), ErrorKind::Refused),
        ("rig_skin_add", json!({"rig": "hero", "skin": "default"}), ErrorKind::Refused),
        ("rig_ik_set", json!({"rig": "hero", "bones": ["root", "head"], "target": "spine"}), ErrorKind::Refused),
        ("rig_ik_set", json!({"rig": "hero", "bones": ["head"], "target": "head"}), ErrorKind::Refused),
        ("rig_ik_remove", json!({"rig": "hero", "index": 3}), ErrorKind::NotFound),
        ("rig_clip_create", json!({"rig": "hero", "name": "nod", "duration": 1.0}), ErrorKind::Refused),
        ("rig_clip_create", json!({"rig": "hero", "name": "x", "duration": 0.0}), ErrorKind::InvalidArguments),
        ("rig_clip_update", json!({"rig": "hero", "name": "nod", "duration": 0.5}), ErrorKind::Refused),
        ("rig_clip_update", json!({"rig": "hero", "name": "nope", "duration": 2.0}), ErrorKind::NotFound),
        ("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "head", "time": 5.0, "value": 1.0}), ErrorKind::Refused),
        ("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "head", "time": 0.5, "value": [1, 2]}), ErrorKind::InvalidArguments),
        ("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "head", "time": 0.5, "value": 3.0, "ease": "Wobbly"}), ErrorKind::NotFound),
        ("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "tail", "time": 0.5, "value": 3.0}), ErrorKind::NotFound),
        ("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "time": 0.5, "value": 3.0}), ErrorKind::InvalidArguments),
        ("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "deform", "target": "hero", "time": 0.5, "value": [[0, 0]]}), ErrorKind::Refused),
        ("rig_clip_key_set", json!({"rig": "hero", "clip": "ghost", "track": "bone_rotate", "target": "head", "time": 0.5, "value": 3.0}), ErrorKind::NotFound),
        ("rig_clip_key_delete", json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "head", "time": 0.4}), ErrorKind::NotFound),
        ("rig_create", json!({"name": "hero"}), ErrorKind::Refused),
        ("rig_create", json!({"image": "incoming/missing.png"}), ErrorKind::NotFound),
        ("rig_bind", json!({"rig": "nobody"}), ErrorKind::NotFound),
    ];
    for (tool, args, kind) in cases {
        let e = f.err(tool, args.clone());
        assert_eq!(e.kind, kind, "{tool} {args}: {e}");
    }
    assert_eq!(f.rig_file("hero"), before);
}

#[test]
fn bones_slots_skins_and_ik() {
    let mut f = Fixture::new("misc");
    hero(&mut f);

    // Bones: rename, move in the world, keep children coherent.
    let u = f.ok("rig_bone_update", json!({"rig": "hero", "bone": "head", "name": "skull", "x": 3.0}));
    assert!(u["summary"]["bones"].as_array().unwrap().iter().any(|b| b["name"] == "skull"));
    f.ok("rig_bone_update", json!({"rig": "hero", "bone": "skull", "name": "head"}));
    let moved = f.ok("rig_bone_update", json!({"rig": "hero", "bone": "head", "head": [10.0, -40.0]}));
    let bones = moved["summary"]["bones"].as_array().unwrap();
    assert_eq!(bones.len(), 3);

    // Reparent keeps the place in the world: the world origin is unchanged.
    let hand = f.ok("rig_bone_add", json!({"rig": "hero", "name": "hand", "parent": "head", "head": [20.0, -50.0], "tip": [20.0, -70.0]}));
    let s = f.ok("rig_sample", json!({"rig": "hero", "clip": "nod", "time": 0.0}));
    let before = s["bones"].as_array().unwrap().iter().find(|b| b["name"] == "hand").unwrap()["world"].clone();
    f.ok("rig_bone_reparent", json!({"rig": "hero", "bone": "hand", "parent": "root"}));
    let s = f.ok("rig_sample", json!({"rig": "hero", "clip": "nod", "time": 0.0}));
    let after = s["bones"].as_array().unwrap().iter().find(|b| b["name"] == "hand").unwrap()["world"].clone();
    assert!((before["x"].as_f64().unwrap() - after["x"].as_f64().unwrap()).abs() < 0.01, "{before} {after}");
    assert!((before["y"].as_f64().unwrap() - after["y"].as_f64().unwrap()).abs() < 0.01);
    assert!(hand["bone"].is_string());

    // Slot: color, draw order, hide.
    f.write_bar("incoming/hat.png");
    let part = f.ok("rig_add_image_part", json!({"rig": "hero", "image": "incoming/hat.png", "bone": "head", "name": "hat"}));
    assert_eq!(part["summary"]["slots"].as_array().unwrap().len(), 2);
    f.ok("rig_slot_update", json!({"rig": "hero", "slot": "hat", "color": "#ff8800", "draw_order": 0}));
    let rig = f.rig("hero");
    assert_eq!(rig.slots[0].name, "hat");
    assert!((rig.slots[0].color.r - 1.0).abs() < 1e-3 && (rig.slots[0].color.g - 136.0 / 255.0).abs() < 1e-3);
    f.ok("rig_slot_update", json!({"rig": "hero", "slot": "hat", "attachment": ""}));
    assert!(f.rig("hero").slots[0].attachment.is_none());

    // Skins.
    let d = f.ok("rig_skin_duplicate", json!({"rig": "hero", "skin": "default", "name": "winter"}));
    assert_eq!(d["skin"], "winter");
    f.ok("rig_skin_add", json!({"rig": "hero", "skin": "empty"}));
    f.ok("rig_skin_delete", json!({"rig": "hero", "skin": "empty"}));
    f.ok("rig_skin_delete", json!({"rig": "hero", "skin": "winter"}));

    // IK: a 2-bone chain reaching a target, then updated, then removed.
    f.ok("rig_bone_add", json!({"rig": "hero", "name": "goal", "head": [0.0, -100.0]}));
    let ik = f.ok("rig_ik_set", json!({"rig": "hero", "bones": ["spine", "head"], "target": "goal", "mix": 0.5}));
    assert_eq!(ik["ik_index"], 0);
    let ik = f.ok("rig_ik_set", json!({"rig": "hero", "bones": ["spine", "head"], "target": "goal", "bend_positive": true}));
    assert_eq!(ik["ik_index"], 0);
    assert_eq!(f.rig("hero").ik.len(), 1);
    assert!((f.rig("hero").ik[0].mix - 0.5).abs() < 1e-6);
    f.ok("rig_ik_remove", json!({"rig": "hero", "bones": ["spine", "head"]}));
    assert!(f.rig("hero").ik.is_empty());

    // Deleting a bone takes its tracks along and leaves a valid rig.
    let del = f.ok("rig_bone_delete", json!({"rig": "hero", "bone": "head"}));
    assert!(del["deleted_bones"].as_array().unwrap().iter().any(|b| b == "head"));
    assert_eq!(del["deleted_tracks"], 1);
    no_errors(&f.ok("rig_read", json!({"rig": "hero"})));
}

#[test]
fn clip_keys_of_every_kind_and_delete() {
    let mut f = Fixture::new("keys");
    hero(&mut f);
    // Named curve from the story, bezier, and the other tracks.
    f.ok("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "bone_translate", "target": "spine", "time": 0.0, "value": [0, 0], "ease": {"bezier": [0.3, 0.0, 0.7, 1.0]}}));
    f.ok("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "bone_translate", "target": "spine", "time": 1.0, "value": [4, -2], "ease": "out_back"}));
    f.ok("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "bone_scale", "target": "spine", "time": 0.5, "value": [1.1, 0.9]}));
    f.ok("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "slot_color", "target": "hero", "time": 0.5, "value": [1, 0.5, 0.5, 1]}));
    f.ok("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "slot_attachment", "target": "hero", "time": 0.25, "value": null}));
    let n = f.rig("hero").skins[0].attachments.iter().find_map(|(_, a)| match a {
        orbit_anim::rig::Attachment::Mesh(m) => Some(m.vertices.len()),
        _ => None,
    });
    let n = n.unwrap();
    f.ok("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "deform", "target": "hero", "time": 0.5, "value": vec![[1.0, 1.0]; n]}));
    let e = f.err("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "deform", "target": "hero", "time": 0.5, "value": [[1.0, 1.0]]}));
    assert_eq!(e.kind, ErrorKind::Refused);
    f.ok("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "draw_order", "time": 0.0, "value": ["hero"]}));
    let rig = f.rig("hero");
    assert_eq!(rig.clips["nod"].tracks.len(), 7);

    // A story curve is a valid ease; an unknown one is not.
    f.ok("set_curve", json!({"name": "snappy", "bezier": [0.1, 0.0, 0.2, 1.0]}));
    f.ok("rig_clip_key_set", json!({"rig": "hero", "clip": "nod", "track": "bone_rotate", "target": "head", "time": 0.5, "value": 10.0, "ease": "snappy"}));

    // Delete keys: the emptied track goes away.
    let d = f.ok("rig_clip_key_delete", json!({"rig": "hero", "clip": "nod", "track": "slot_attachment", "target": "hero", "time": 0.25}));
    assert_eq!(d["track_removed"], true);
    assert_eq!(f.rig("hero").clips["nod"].tracks.len(), 6);

    // Lengthening then shortening around the keys.
    f.ok("rig_clip_update", json!({"rig": "hero", "name": "nod", "duration": 2.0, "looping": "ping_pong"}));
    assert_eq!(f.err("rig_clip_update", json!({"rig": "hero", "name": "nod", "duration": 0.9})).kind, ErrorKind::Refused);
    f.ok("rig_clip_delete", json!({"rig": "hero", "name": "nod"}));
    assert!(f.rig("hero").clips.is_empty());
}

#[test]
fn deleting_a_rig_that_is_played_is_refused() {
    let mut f = Fixture::new("delete");
    hero(&mut f);
    f.write_bar("incoming/poster.png");
    let poster = f.ok("import_asset", json!({"source": "incoming/poster.png"}));
    f.ok("create_character", json!({"name": "Hero", "expressions": [{"name": "idle", "asset": poster["id"]}]}));
    f.ok("set_expression_rig", json!({"character": "Hero", "expression": "idle", "rig": "hero", "clip": "nod"}));
    assert_eq!(f.err("rig_delete", json!({"rig": "hero"})).kind, ErrorKind::Refused);
    assert_eq!(f.err("rig_clip_delete", json!({"rig": "hero", "name": "nod"})).kind, ErrorKind::Refused);
    assert!(f.rig_path("hero").is_file());
    f.ok("set_expression_rig", json!({"character": "Hero", "expression": "idle"}));
    f.ok("rig_delete", json!({"rig": "hero"}));
    assert!(!f.rig_path("hero").exists());
    let list = f.ok("rig_list", json!({}));
    assert_eq!(list["count"], 0);
}

#[test]
fn an_empty_rig_and_the_default_name() {
    let mut f = Fixture::new("empty");
    let c = f.ok("rig_create", json!({"name": "blank"}));
    assert_eq!(c["rig"], "assets/rigs/blank.rig.ron");
    assert_eq!(c["summary"]["bones"].as_array().unwrap().len(), 0);
    assert_eq!(f.err("rig_bind", json!({"rig": "blank"})).kind, ErrorKind::Refused);
    assert_eq!(f.err("rig_add_image_part", json!({"rig": "blank", "image": "incoming/none.png"})).kind, ErrorKind::NotFound);
    f.ok("rig_bone_add", json!({"rig": "blank", "name": "root"}));
    let before = f.rig_file("blank");
    // No mesh to bind: refused, file untouched.
    assert_eq!(f.err("rig_bind", json!({"rig": "blank"})).kind, ErrorKind::Refused);
    assert_eq!(f.rig_file("blank"), before);
}

/// A stand-in for the rig editor holding the rig open.
struct Open {
    path: PathBuf,
    rig: RefCell<Rig>,
    applied: RefCell<usize>,
}

impl LiveRig for Open {
    fn current(&self, abs: &Path) -> Option<Rig> {
        same(abs, &self.path).then(|| self.rig.borrow().clone())
    }

    fn apply(&self, abs: &Path, rig: &Rig, _label: &str) -> Result<bool, String> {
        if !same(abs, &self.path) {
            return Ok(false);
        }
        *self.rig.borrow_mut() = rig.clone();
        *self.applied.borrow_mut() += 1;
        Ok(true)
    }
}

#[test]
fn an_open_rig_editor_gets_the_edits_instead_of_the_file() {
    let mut f = Fixture::new("live");
    hero(&mut f);
    let path = f.rig_path("hero");
    let on_disk = f.rig_file("hero");
    let open = Rc::new(Open { path: path.clone(), rig: RefCell::new(f.rig("hero")), applied: RefCell::new(0) });
    set_live_rig(Some(open.clone()));

    let r = f.ok("rig_bone_add", json!({"rig": "hero", "name": "tail", "parent": "root"}));
    assert!(r["note"].as_str().unwrap().contains("open rig editor"), "{r}");
    assert_eq!(*open.applied.borrow(), 1);
    assert_eq!(open.rig.borrow().bones.len(), 4);
    // The tool wrote nothing itself (the editor's document saves).
    assert_eq!(f.rig_file("hero"), on_disk);
    // Reads see the editor's state.
    let read = f.ok("rig_read", json!({"rig": "hero"}));
    assert_eq!(read["rig"]["bones"].as_array().unwrap().len(), 4);
    // A refused edit does not reach the editor.
    f.err("rig_bone_add", json!({"rig": "hero", "name": "tail"}));
    assert_eq!(*open.applied.borrow(), 1);

    set_live_rig(None);
    let r = f.ok("rig_bone_add", json!({"rig": "hero", "name": "ear", "parent": "root"}));
    assert!(r["note"].as_str().unwrap().contains("rewritten"));
    assert_eq!(f.rig("hero").bones.len(), 4);
}

#[test]
fn rig_cut_splits_a_picture_into_a_new_part() {
    let mut f = Fixture::new("cut");
    hero(&mut f);
    let original = fs::read(f.dir.join("assets/images/hero.png")).unwrap();
    // The top-left quarter of the bar (x 4..16, y 0..20) becomes "fringe".
    let c = f.ok(
        "rig_cut",
        json!({"rig": "hero", "slot": "hero", "polygon": [[4, 0], [16, 0], [16, 20], [4, 20]], "name": "fringe", "bone": "head"}),
    );
    no_errors(&c);
    assert_eq!(c["image"], "assets/images/fringe.png");
    assert_eq!(c["size"], json!([12, 20]));
    assert_eq!(c["offset"], json!([4, 0]));
    assert_eq!(c["source_image"], "assets/images/hero_filled.png");
    let part = image::open(f.dir.join("assets/images/fringe.png")).unwrap().to_rgba8();
    assert_eq!(part.dimensions(), (12, 20));
    assert_eq!(part.get_pixel(0, 0).0, [200, 40, 90, 255]);
    // The original file is kept; the rig shows the filled copy.
    assert_eq!(fs::read(f.dir.join("assets/images/hero.png")).unwrap(), original);
    let rig = f.rig("hero");
    let names: Vec<&str> = rig.slots.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["hero", "fringe"]);
    let images: Vec<String> = rig.skins[0].attachments.values().map(|a| a.image().to_owned()).collect();
    assert!(images.contains(&"assets/images/hero_filled.png".to_owned()), "{images:?}");
    assert!(!images.contains(&"assets/images/hero.png".to_owned()));
    let assets = f.ok("list_assets", json!({})).to_string();
    assert!(assets.contains("assets/images/fringe.png") && assets.contains("assets/images/hero_filled.png"), "{assets}");

    // Behind, the original underneath: no new source picture.
    let k = f.ok("rig_cut", json!({"rig": "hero", "slot": "hero", "polygon": [[4, 40], [28, 40], [28, 64], [4, 64]], "below": true, "fill_hole": false}));
    assert_eq!(k["source_image"], "assets/images/hero_filled.png");
    assert_eq!(f.rig("hero").slots[0].name, "hero_cut");

    // Refusals write nothing.
    let before = f.rig_file("hero");
    assert_eq!(f.err("rig_cut", json!({"rig": "hero", "slot": "hero", "polygon": [[0, 0], [1, 1]]})).kind, ErrorKind::InvalidArguments);
    assert_eq!(
        f.err("rig_cut", json!({"rig": "hero", "slot": "hero", "polygon": [[0, 0], [3, 0], [3, 3], [0, 3]]})).kind,
        ErrorKind::Refused
    );
    assert_eq!(f.rig_file("hero"), before);
    assert!(!f.dir.join("assets/images/hero_cut_2.png").exists());
}

fn same(a: &Path, b: &Path) -> bool {
    fs::canonicalize(a).ok() == fs::canonicalize(b).ok()
}
