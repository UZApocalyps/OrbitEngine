//! Tests of copy-on-write for shared pictures.

use std::cell::RefCell;
use std::rc::Rc;

use orbit_core::Story;
use orbit_editor_api::{EditorContext, EditorRegistry, EditorState, ProjectInfo};
use orbit_project::Project;

use super::super::paint::{Brush, Mode, Stroke};
use super::super::pixeltools;
use super::*;
use crate::rig_host::RigLink;

const RIG: &str = r#"(
    rig_version: 1,
    bones: [(id: "root", name: "root")],
    slots: [(id: "s1", name: "eyes", bone: "root", attachment: Some("a1"))],
    skins: [(name: "default", attachments: {"a1": Region((image: "assets/images/face.png", size: (4.0, 4.0), xform: (x: 0.0, y: 0.0, rotation: 0.0, sx: 1.0, sy: 1.0)))})],
    clips: {},
)"#;

const FACE: &str = "assets/images/face.png";

struct Fixture {
    dir: std::path::PathBuf,
    project: Project,
    es: EditorState,
    registry: EditorRegistry,
    st: State,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("orbit_shared_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets/images")).unwrap();
    std::fs::create_dir_all(dir.join("assets/rigs")).unwrap();
    write_png(&dir.join(FACE), 4, 4, &[200u8; 64]).unwrap();
    // The rig being edited, and another rig drawing the same picture.
    std::fs::write(dir.join("assets/rigs/mira.rig.ron"), RIG).unwrap();
    std::fs::write(dir.join("assets/rigs/twin.rig.ron"), RIG).unwrap();
    let mut es = EditorState::default();
    es.project = Some(ProjectInfo {
        name: "t".into(),
        root: dir.clone(),
        story_file: "story.orbit".into(),
        modified: false,
        has_manifest: false,
    });
    let mut st = State::new(Rc::new(RefCell::new(RigLink::default())));
    let rig = orbit_anim::rig::load_rig(RIG).unwrap().rig;
    let mut doc = super::super::model::RigDoc::from_rig(rig, Some(dir.join("assets/rigs/mira.rig.ron")));
    doc.set_canvas(FACE, 4, 4, vec![200u8; 64]);
    st.set_doc(doc);
    st.selected_slot = Some(SlotId::new("s1"));
    Fixture { dir, project: Project::new(Story::new("t")), es, registry: EditorRegistry::new(), st }
}

/// One stroke the way `begin_stroke` does it: a gesture, the picture to
/// paint, a dab, the commit.
fn stroke(ctx: &mut EditorContext<'_>, st: &mut State, path: &str) -> Option<String> {
    st.doc.as_mut().unwrap().begin_gesture();
    let out = pixeltools::editable(ctx, st, path, &SlotId::new("s1"));
    if let Some(p) = &out {
        let doc = st.doc.as_mut().unwrap();
        let before = doc.canvas(p).unwrap().rgba.clone();
        let (w, h) = (doc.canvas(p).unwrap().w, doc.canvas(p).unwrap().h);
        let brush = Brush { size: 2.0, hardness: 1.0, opacity: 1.0, color: [255, 0, 255, 255] };
        let mut s = Stroke::new(w, h, before.clone(), Mode::Paint, brush);
        doc.paint_live(p, |c| s.dab_sized(&mut c.rgba, [1.5, 1.5], 2.0));
        doc.commit_pixels(p, &before);
    }
    st.doc.as_mut().unwrap().end_gesture();
    out
}

fn image_of(st: &State) -> String {
    st.doc.as_ref().unwrap().rig().skins[0].attachments.values().next().unwrap().image().to_owned()
}

#[test]
fn the_first_stroke_on_a_shared_picture_asks_then_copies_for_this_slot() {
    let mut f = fixture("cow");
    let mut ctx = EditorContext::new(Some(&mut f.project), &mut f.es, &f.registry);
    // Shared with the other rig: the stroke is held back, the dialog opens.
    assert_eq!(stroke(&mut ctx, &mut f.st, FACE), None);
    let prompt = f.st.px.shared.prompt.as_ref().expect("a question");
    assert_eq!(prompt.users.len(), 1);
    assert!(prompt.users[0].contains("twin.rig.ron"), "{:?}", prompt.users);
    assert_eq!(image_of(&f.st), FACE, "nothing changed yet");
    // "Make a copy for this rig" (the default button): the rig switches to
    // a copy named after the rig and the slot.
    let rig_before = f.st.doc.as_ref().unwrap().rig().clone();
    f.st.px.shared.prompt = None;
    f.st.px.shared.set_choice(FACE, Choice::Copy);
    let used = stroke(&mut ctx, &mut f.st, FACE).expect("paints the copy");
    assert_eq!(used, "assets/images/mira_eyes.png");
    assert_eq!(image_of(&f.st), used);
    assert!(f.dir.join(&used).is_file(), "the copy is on disk");
    // The original pixels were not touched.
    let doc = f.st.doc.as_ref().unwrap();
    assert_eq!(doc.canvas(FACE).unwrap().rgba, vec![200u8; 64]);
    assert_ne!(doc.canvas(&used).unwrap().rgba, vec![200u8; 64]);
    // The copy and the stroke are ONE undo step: undoing brings the original
    // reference back.
    let doc = f.st.doc.as_mut().unwrap();
    assert!(doc.undo());
    assert_eq!(doc.rig(), &rig_before);
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[test]
fn choosing_the_shared_picture_edits_it_in_place_and_an_unshared_one_just_paints() {
    let mut f = fixture("inplace");
    let mut ctx = EditorContext::new(Some(&mut f.project), &mut f.es, &f.registry);
    f.st.px.shared.set_choice(FACE, Choice::Shared);
    let used = stroke(&mut ctx, &mut f.st, FACE).unwrap();
    assert_eq!(used, FACE);
    assert_eq!(image_of(&f.st), used);
    // Nothing else draws it any more: no question (the file watcher bumps
    // the epoch, which drops the index).
    std::fs::remove_file(f.dir.join("assets/rigs/twin.rig.ron")).unwrap();
    f.st.px.shared = Default::default();
    ctx.state_mut().asset_epoch += 1;
    assert_eq!(decide(&mut ctx, &mut f.st, FACE, &SlotId::new("s1")), Decision::Edit);
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[test]
fn another_slot_of_the_same_rig_counts_as_a_user() {
    let mut f = fixture("slots");
    std::fs::remove_file(f.dir.join("assets/rigs/twin.rig.ron")).unwrap();
    let doc = f.st.doc.as_mut().unwrap();
    let slot = doc.add_slot(&orbit_anim::rig::BoneId::new("root")).unwrap();
    doc.attach_image(&slot, "default", FACE, [4.0, 4.0]).unwrap();
    let mut ctx = EditorContext::new(Some(&mut f.project), &mut f.es, &f.registry);
    assert_eq!(decide(&mut ctx, &mut f.st, FACE, &SlotId::new("s1")), Decision::Ask);
    assert_eq!(f.st.px.shared.prompt.as_ref().unwrap().users.len(), 1);
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[test]
fn the_duplicate_dialog_copies_pictures_by_default() {
    let mut f = fixture("dup");
    let mut ctx = EditorContext::new(Some(&mut f.project), &mut f.es, &f.registry);
    f.st.request_duplicate(&mut ctx);
    assert!(matches!(f.st.dialog, Some(super::super::rigtools::Dialog::Duplicate { copy_pictures: true, .. })));
    let _ = std::fs::remove_dir_all(&f.dir);
}
