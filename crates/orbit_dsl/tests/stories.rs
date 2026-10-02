//! Round trips on the shipped stories, and hand-written edits.

use orbit_core::{NodeKind, Story};
use orbit_dsl::{ApplyError, Scope, apply, diff, parse, print_scene, print_story};
use orbit_project::Project;

fn load(name: &str) -> Story {
    let path = format!(
        "{}/../../examples/stories/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Story::from_ron_unchecked(&src).unwrap()
}

fn round_trip(name: &str) {
    let story = load(name);
    let text = print_story(&story);
    let back = parse(&text, &story, Scope::Story).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    assert_eq!(back, story, "{name}");
    assert_eq!(print_story(&back), text, "{name}: canonical text is stable");
    assert!(diff(&story, &back).unwrap().is_empty());
    for scene in &story.scenes {
        let text = print_scene(&story, scene.id).unwrap();
        assert_eq!(
            parse(&text, &story, Scope::Scene(scene.id)).unwrap(),
            story,
            "{name}"
        );
    }
}

#[test]
fn test_minimal_round_trips() {
    round_trip("test_minimal.ron");
}

#[test]
fn test_features_round_trips() {
    round_trip("test_features.ron");
}

#[test]
fn demo_round_trips() {
    round_trip("demo.ron");
}

#[test]
fn printed_minimal_story_reads_like_a_script() {
    let text = print_story(&load("test_minimal.ron"));
    assert!(text.starts_with("scene "), "{text}");
    assert!(text.contains("menu #@"));
    assert!(text.contains(" -> "));
    // Every node and scene line carries its id (menu options are not nodes).
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with("        "))
    {
        assert!(
            line.trim_end().rsplit_once(" #@").is_some(),
            "unannotated line: {line}"
        );
    }
}

/// Removes every `#@…` annotation of the lines whose trimmed text starts
/// with `prefix`.
fn without_ids(text: &str) -> String {
    text.lines()
        .map(|l| match l.rsplit_once(" #@") {
            Some((body, _)) if !l.trim_start().starts_with("scene") => body.to_owned(),
            _ => l.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn hand_edits_merge_into_the_story() {
    let story = load("test_minimal.ron");
    let scene = &story.scenes[0];
    let text = print_scene(&story, scene.id).unwrap();

    // Change the first narrator line, add a new line and a variable change.
    let (first_say, first_text) = scene
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Say {
                speaker: None,
                text_id,
            } => Some((n.id, story.text(*text_id).unwrap())),
            _ => None,
        })
        .unwrap();
    let mut edited = String::new();
    for line in text.lines() {
        if line.contains(&format!("#@{first_say}")) {
            edited.push_str(&format!("    \"Edited!\" #@{first_say}   # a comment\n"));
            edited.push_str("    \"Brand new line.\"\n");
            edited.push_str("    $ edited = true\n");
        } else {
            edited.push_str(line);
            edited.push('\n');
        }
    }
    assert!(!edited.contains(first_text));

    let mut project = Project::new(story.clone());
    // `edited` is not declared: the parser does not validate, the editor's
    // validator will report it like for any other edit.
    let n = apply(&mut project, &edited, Scope::Scene(scene.id)).unwrap();
    assert!(n >= 4, "{n} commands");
    let after = project.story();
    let node = after.node(first_say).unwrap();
    let NodeKind::Say { text_id, .. } = node.kind else {
        panic!()
    };
    assert_eq!(after.text(text_id), Some("Edited!"));
    // The string id of the edited line is kept.
    assert!(story.strings.entries.contains_key(&text_id));
    assert_eq!(after.scenes[0].nodes.len(), scene.nodes.len() + 2);
    let (_, idx) = after.locate(first_say).unwrap();
    assert!(
        matches!(&after.scenes[0].nodes[idx + 2].kind, NodeKind::SetVar { var, .. } if var == "edited")
    );
    // Other scenes are untouched.
    assert_eq!(after.scenes[1..], story.scenes[1..]);

    // Undo is one step.
    project.undo().unwrap();
    assert_eq!(project.story(), &story);
}

#[test]
fn lines_without_ids_are_new_nodes() {
    let story = load("test_minimal.ron");
    let scene = &story.scenes[0];
    let text = without_ids(&print_scene(&story, scene.id).unwrap());
    let back = parse(&text, &story, Scope::Scene(scene.id)).unwrap();
    let new_scene = back.scene(scene.id).unwrap();
    assert_eq!(new_scene.nodes.len(), scene.nodes.len());
    for (a, b) in new_scene.nodes.iter().zip(&scene.nodes) {
        assert_ne!(a.id, b.id);
        assert_eq!(
            std::mem::discriminant(&a.kind),
            std::mem::discriminant(&b.kind)
        );
    }
}

#[test]
fn errors_leave_the_project_untouched() {
    let story = load("test_minimal.ron");
    let scene = &story.scenes[0];
    let mut text = print_scene(&story, scene.id).unwrap();
    text.push_str("    show nobody happy at left\n    wait\n");
    let mut project = Project::new(story.clone());
    let Err(ApplyError::Syntax(diags)) = apply(&mut project, &text, Scope::Scene(scene.id)) else {
        panic!("expected a syntax error");
    };
    let last = text.lines().count();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!((diags[0].line, diags[0].column), (last, 9));
    assert_eq!(project.story(), &story);
    assert!(!project.can_undo());
}

#[test]
fn deleting_a_targeted_label_is_refused_by_the_commands() {
    let story = load("test_minimal.ron");
    // Find a label targeted from another node, and delete only the label.
    let (scene, label) = story
        .scenes
        .iter()
        .find_map(|s| {
            s.nodes.iter().find_map(|n| {
                let targeted = story
                    .scenes
                    .iter()
                    .flat_map(|s| &s.nodes)
                    .any(|m| m.kind.targets().contains(&n.id));
                (matches!(n.kind, NodeKind::Label { .. }) && targeted).then_some((s.id, n.id))
            })
        })
        .expect("a targeted label");
    let text: String = print_scene(&story, scene)
        .unwrap()
        .lines()
        .filter(|l| !l.contains(&format!("#@{label}")))
        .map(|l| format!("{l}\n"))
        .collect();
    let mut project = Project::new(story.clone());
    let err = apply(&mut project, &text, Scope::Scene(scene)).unwrap_err();
    // Either the name no longer resolves (same-scene reference) or the
    // command layer refuses to remove a referenced node.
    assert!(
        matches!(err, ApplyError::Syntax(_) | ApplyError::Command(_)),
        "{err}"
    );
    assert_eq!(project.story(), &story);
}

#[test]
fn moves_between_scenes_keep_ids() {
    let story = load("test_minimal.ron");
    assert!(story.scenes.len() >= 2);
    let text = print_story(&story);
    // Move the last node line of the first scene to the top of the second.
    let lines: Vec<&str> = text.lines().collect();
    let second = lines.iter().rposition(|l| l.starts_with("scene ")).unwrap();
    let first_scene_last = (0..second)
        .rev()
        .find(|&i| {
            lines[i].starts_with("    ")
                && !lines[i].starts_with("        ")
                && !lines[i].trim_start().starts_with("menu")
        })
        .unwrap();
    let moved = lines[first_scene_last];
    let mut out: Vec<&str> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if i == first_scene_last {
            continue;
        }
        out.push(l);
        if i == second {
            out.push(moved);
        }
    }
    let text = out.join("\n");
    let mut project = Project::new(story.clone());
    match apply(&mut project, &text, Scope::Story) {
        Ok(_) => {
            let id = moved.rsplit_once("#@").unwrap().1.parse().unwrap();
            let (si, ni) = project.story().locate(id).unwrap();
            assert_eq!((si, ni), (story.scenes.len() - 1, 0));
        }
        // A moved line naming a label of its old scene no longer resolves.
        Err(ApplyError::Syntax(d)) => assert!(d[0].message.contains("label"), "{d:?}"),
        Err(e) => panic!("{e}"),
    }
}
