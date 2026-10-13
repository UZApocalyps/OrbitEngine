//! `renpy.notify` and the `Notify` screen action become `Notify` nodes.

use std::fs;
use std::path::PathBuf;

use orbit_core::{NodeKind, Story};

fn import(tag: &str, script: &str) -> (Story, orbit_renpy::ImportReport) {
    let root: PathBuf = std::env::temp_dir().join(format!("orbit_renpy_notify_{tag}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("game")).unwrap();
    fs::write(root.join("game/script.rpy"), script).unwrap();
    let out = root.join("out");
    let report = orbit_renpy::import(&root, &out, &|_, _| {}).expect("import");
    let story = Story::from_ron(&fs::read_to_string(out.join("story.orbit")).unwrap()).unwrap();
    (story, report)
}

fn notifies(story: &Story) -> Vec<String> {
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| match &n.kind {
            NodeKind::Notify(x) => {
                assert!(x.icon.is_none() && x.sound.is_none() && x.duration.is_none());
                assert!(x.style.is_none());
                story.text(x.text_id).map(str::to_owned)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn one_line_python() {
    let (story, report) = import(
        "line",
        "label start:\n    $ renpy.notify(\"Saved\")\n    \"End\"\n    return\n",
    );
    assert_eq!(notifies(&story), ["Saved"]);
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
}

#[test]
fn python_block() {
    let (story, _) = import(
        "block",
        "label start:\n    python:\n        renpy.notify(\"A\")\n        renpy.notify(\"B\")\n    \"End\"\n    return\n",
    );
    assert_eq!(notifies(&story), ["A", "B"]);
}

#[test]
fn screen_action() {
    let (story, report) = import(
        "screen",
        "label start:\n    show screen s\n    \"End\"\n    return\n\nscreen s():\n    vbox:\n        textbutton \"Go\" action [Notify(\"Hi\"), Jump(\"start\")]\n        textbutton \"Other\" action Notify(\"Yo\")\n",
    );
    assert_eq!(notifies(&story), ["Hi", "Yo"]);
    assert!(report.unsupported.is_empty(), "{:?}", report.unsupported);
}
