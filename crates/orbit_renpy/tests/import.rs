//! A small Ren'Py game imported end to end: the project validates, plays
//! the expected nodes, carries its translation and prints through the DSL.

use std::fs;
use std::path::{Path, PathBuf};

use orbit_core::{NodeKind, Position, Story, TransitionKind, Value};

const SCRIPT: &str = r##"# The script of the game goes in this file.

define e = Character("Eileen", color="#c8ffc8")
define m = Character("Me", color="#ff0000", image="me")
define flash = Fade(0.1, 0.0, 0.4)
define config.name = _("Fixture Quest")
define audio.theme = "audio/theme song.ogg"

default points = 0
default met_eileen = False
default persistent.endings = 0
default inventory = ["map"]
default name = "Player"

image bg street = "street.jpg"

label start:
    scene bg room with fade
    play music theme fadein 1.0
    show eileen happy at left with dissolve
    e "Hello, {b}[name]{/b}! Welcome."
    window hide
    e sad "I lost my {i}keys{/i}.{w} Can you help?"
    $ met_eileen = True
    $ points += 2
    $ inventory.append("key")
    $ renpy.notify("hi")
    menu:
        "What do you do?"
        "Help her" if points >= 2:
            $ points += 1
            jump helped
        "Walk away":
            m "Sorry, I'm busy."
    call side_story
    if points > 2 and met_eileen:
        e "Thanks again!"
    elif points == 2:
        "Not bad."
    else:
        "Hmm."
    jump ending

label helped:
    show me at right
    m "Here are your keys."
    play sound "audio/click.wav"
    hide eileen with dissolve
    with flash
    pause 0.5
    stop music fadeout 2.0
    jump ending

label side_story:
    "A short detour."
    label .inner:
        "Still in the detour."
    return

label ending:
    scene bg street
    with dissolve
    "The end."
    $ persistent.endings += 1
    while points > 10:
        $ points -= 1
    return
"##;

const TL: &str = r##"# TODO: Translation updated at 2026-10-01 10:00

# game/script.rpy:20
translate french start_8f1a2b3c:

    # e "Hello, {b}[name]{/b}! Welcome."
    e "Bonjour, {b}[name]{/b} ! Bienvenue."

# game/script.rpy:38
translate french ending_aa11bb22:

    # "The end."
    "Fin."

translate french strings:

    # game/script.rpy:27
    old "Help her"
    new "L'aider"

    old "Eileen"
    new "Eileen"
"##;

fn temp(name: &str) -> PathBuf {
    // Under the target dir, not the system temp dir.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("orbit_renpy_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn fixture() -> PathBuf {
    let root = temp("game");
    write(&root, "game/script.rpy", SCRIPT.as_bytes());
    write(&root, "game/tl/french/script.rpy", TL.as_bytes());
    write(
        &root,
        "game/tl/None/common.rpy",
        b"translate None strings:\n",
    );
    for img in [
        "images/bg room.png",
        "images/eileen happy.png",
        "images/eileen sad.png",
        "images/sprites/me.png",
        "street.jpg",
        "images/unused.webp",
    ] {
        write(
            &root,
            &format!("game/{img}"),
            b"not decoded by the importer",
        );
    }
    write(&root, "game/audio/theme song.ogg", b"ogg");
    write(&root, "game/audio/click.wav", b"wav");
    write(&root, "game/gui/textbox.png", b"gui");
    root
}

#[test]
fn imports_a_small_game() {
    let src = fixture();
    let out = temp("project");
    let report = orbit_renpy::import(&src, &out, &|_, _| {}).expect("import");
    assert!(
        report.validation_errors.is_empty(),
        "{:#?}",
        report.validation_errors
    );

    // The written project loads and validates.
    let text = fs::read_to_string(out.join("story.orbit")).unwrap();
    let story = Story::from_ron(&text).expect("valid story");
    assert!(out.join("orbit.project.ron").is_file());
    assert!(out.join("import-report.md").is_file());
    assert!(out.join("assets/images/eileen_happy.png").is_file());
    assert!(out.join("assets/audio/theme_song.ogg").is_file());
    assert!(!out.join("assets/gui").exists());
    assert!(
        report
            .skipped_files
            .iter()
            .any(|f| f.contains("unused.webp"))
    );
    assert_eq!(story.title, "Fixture Quest");

    // Scenes: one per top-level label, start first; `.inner` is a label.
    let names: Vec<&str> = story.scenes.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["start", "helped", "side_story", "ending"]);
    let side = &story.scenes[2];
    assert!(
        side.nodes
            .iter()
            .any(|n| matches!(&n.kind, NodeKind::Label { name } if name == "side_story.inner"))
    );
    // A called label returns; `start` ends the game at its `return`.
    assert!(matches!(side.nodes.last().unwrap().kind, NodeKind::Return));
    assert!(matches!(
        story.scenes[3].nodes.last().unwrap().kind,
        NodeKind::End
    ));

    // Characters and expressions from the image names.
    let eileen = story
        .characters
        .iter()
        .find(|c| story.text(c.name_id) == Some("Eileen"))
        .unwrap();
    assert_eq!(eileen.color, orbit_core::Color::rgb(0xc8, 0xff, 0xc8));
    let mut exprs: Vec<&str> = eileen.expressions.iter().map(|e| e.name.as_str()).collect();
    exprs.sort_unstable();
    assert_eq!(exprs, ["happy", "sad"]);

    // Variables, with their types.
    let var = |n: &str| {
        story
            .variables
            .iter()
            .find(|v| v.name == n)
            .map(|v| v.initial.clone())
    };
    assert_eq!(var("points"), Some(Value::Int(0)));
    assert_eq!(var("met_eileen"), Some(Value::Bool(false)));
    assert!(matches!(var("inventory"), Some(Value::List(..))));
    assert!(
        story
            .persistent_variables()
            .any(|v| v.name == "persistent_endings")
    );

    let start = &story.scenes[0].nodes;
    let kinds = |nodes: &[orbit_core::Node], f: fn(&NodeKind) -> bool| {
        nodes.iter().filter(|n| f(&n.kind)).count()
    };
    // scene ... with fade
    assert!(start.iter().any(|n| matches!(&n.kind, NodeKind::Background { transition: Some(t), .. } if matches!(t.kind, TransitionKind::Fade(_)))));
    assert!(start.iter().any(|n| matches!(
        &n.kind,
        NodeKind::Show {
            position: Position::Left,
            transition: Some(_),
            ..
        }
    )));
    assert!(start.iter().any(
        |n| matches!(&n.kind, NodeKind::PlayMusic { looping: true, fade_in, .. } if *fade_in == 1.0)
    ));
    // `e sad "..."` changes the sprite.
    assert!(
        start
            .iter()
            .any(|n| matches!(&n.kind, NodeKind::Show { expression, .. } if expression == "sad"))
    );
    assert_eq!(kinds(start, |k| matches!(k, NodeKind::Choice { .. })), 1);
    assert_eq!(kinds(start, |k| matches!(k, NodeKind::Call { .. })), 1);
    assert!(kinds(start, |k| matches!(k, NodeKind::If { .. })) >= 2);
    assert!(kinds(start, |k| matches!(k, NodeKind::SetVar { .. })) >= 3);
    // `$ renpy.notify(...)` is kept as a commented script and reported.
    assert_eq!(kinds(start, |k| matches!(k, NodeKind::Script { .. })), 1);
    assert!(
        report
            .unsupported
            .iter()
            .any(|i| i.message.contains("renpy.notify") && i.line == 27)
    );
    // The choice condition and the markup.
    let choice = start.iter().find_map(|n| match &n.kind {
        NodeKind::Choice { options } => Some(options),
        _ => None,
    });
    assert!(choice.unwrap()[0].condition.is_some());
    let hello = start.iter().find_map(|n| match &n.kind {
        NodeKind::Say { text_id, .. } => story.text(*text_id).filter(|t| t.starts_with("Hello")),
        _ => None,
    });
    assert_eq!(hello, Some("Hello, {b}[name]{/b}! Welcome."));
    assert!(
        story
            .strings
            .entries
            .values()
            .any(|t| t == "I lost my {i}keys{/i}.{w=0.5} Can you help?")
    );
    assert!(report.warnings.iter().any(|w| w.message.contains("[name]")));

    // helped: sound, hide with dissolve, the `define`d transition, pause.
    let helped = &story.scenes[1].nodes;
    assert_eq!(
        kinds(helped, |k| matches!(k, NodeKind::PlaySound { .. })),
        1
    );
    assert!(helped.iter().any(|n| matches!(
        &n.kind,
        NodeKind::Hide {
            transition: Some(_),
            ..
        }
    )));
    assert!(
        helped
            .iter()
            .any(|n| matches!(&n.kind, NodeKind::Wait { seconds } if *seconds == 0.5))
    );
    assert!(
        helped
            .iter()
            .any(|n| matches!(&n.kind, NodeKind::StopMusic { fade_out } if *fade_out == 2.0))
    );
    // ending: `scene` then a separate `with dissolve`.
    assert!(story.scenes[3].nodes.iter().any(|n| matches!(&n.kind, NodeKind::Background { transition: Some(t), .. } if t.kind == TransitionKind::Dissolve)));

    // The French translation.
    let tl = orbit_i18n::files::load(&out.join("i18n/story.fr.ron"))
        .unwrap()
        .unwrap();
    assert_eq!(tl.lang, "fr");
    assert!(tl.entries.values().any(|t| t == "Fin."));
    assert!(tl.entries.values().any(|t| t == "L'aider"));
    assert!(
        tl.entries
            .values()
            .any(|t| t == "Bonjour, {b}[name]{/b} ! Bienvenue.")
    );
    assert!(
        report
            .translations
            .iter()
            .any(|(l, ok, _)| l == "fr" && *ok >= 3)
    );

    // Round trip through the text DSL: it prints and reads back unchanged.
    let printed = orbit_dsl::print_story(&story);
    assert!(printed.contains("Hello"));
    let back = orbit_dsl::parse(&printed, &story, orbit_dsl::Scope::Story).expect("DSL parses");
    assert_eq!(back.scenes.len(), story.scenes.len());
    assert!(orbit_core::validate(&back).errors.is_empty());

    // A second import into the same folder is refused.
    assert!(orbit_renpy::import(&src, &out, &|_, _| {}).is_err());
    let _ = fs::remove_dir_all(&src);
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn unsupported_statements_are_reported_not_fatal() {
    let src = "init python:\n    def f():\n        pass\n\nscreen hud():\n    text \"x\"\n\nlabel start:\n    nvl clear\n    jump expression target\n    show eileen at weird_transform\n    $ x = renpy.random.randint(1, 6)\n    if x ** 2 > 4:\n        \"big\"\n    return\n";
    let (story, report) = orbit_renpy::convert_scripts("t", &[("script.rpy", src)], &[]);
    assert!(
        report.validation_errors.is_empty(),
        "{:#?}",
        report.validation_errors
    );
    assert!(
        report
            .unsupported
            .iter()
            .any(|i| i.message.contains("init python"))
    );
    assert!(
        report
            .unsupported
            .iter()
            .any(|i| i.message.contains("screen"))
    );
    assert!(
        report
            .unsupported
            .iter()
            .any(|i| i.message.contains("computed jump"))
    );
    assert!(report.warnings.iter().any(|i| i.message.contains("Rhai")));
    assert_eq!(story.scenes.len(), 1);
}
