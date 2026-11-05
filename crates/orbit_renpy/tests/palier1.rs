//! "Palier 1" of the Ren'Py import: transitions and transforms, script
//! defined images, character styles, navigation screens, NVL / extend /
//! voice / window, achievements, persistent data and the `renpy.*` audio
//! calls. `fixtures/sample_game` is a small game using all of them; its
//! import is checked feature by feature and against a golden summary
//! (`fixtures/sample_game/expected.txt`, rewritten with
//! `ORBIT_RENPY_BLESS=1`).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_core::{
    AnimProp, Color, HotspotShape, Loop, Motion, NodeKind, Position, Story, TrackData,
    TransitionKind, Value, VarScope,
};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample_game");

/// A PNG header of `w` x `h` (the importer only reads the header).
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0]);
    b
}

const MEDIA: [&str; 29] = [
    "images/bg room.png",
    "images/bg town.png",
    "images/bg kitchen.png",
    "images/bg garden.png",
    "images/bg/night.png",
    "images/eileen happy.png",
    "images/eileen sad.png",
    "images/lucy.png",
    "images/hat.png",
    "images/lamp on.png",
    "images/lamp off.png",
    "images/sea.jpg",
    "images/side eileen.png",
    "images/fire 1.png",
    "images/fire 2.png",
    "images/augustina_base.png",
    "images/augustina_outfit_casual.png",
    "images/augustina_outfit_dress.png",
    "images/augustina_happy.png",
    "images/map ground.png",
    "images/map hover.png",
    "images/door idle.png",
    "images/door hover.png",
    "images/door_idle.png",
    "images/door_hover.png",
    "images/shutter.png",
    "audio/theme.ogg",
    "audio/click.wav",
    "voice/e1.ogg",
];

const TL: &str = "translate french start_aaaa1111:\n\n    # l \"Hello there.{w} Nice {size=+10}big{/size} day.\"\n    l \"Bonjour.{w} Belle journée.\"\n";

/// Copies the fixture into a fresh folder of the target dir (never the
/// system temp dir) with placeholder media.
fn materialize(name: &str) -> PathBuf {
    let root =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("renpy_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let game = root.join("game");
    fs::create_dir_all(&game).unwrap();
    for entry in fs::read_dir(Path::new(FIXTURE).join("game")).unwrap() {
        let p = entry.unwrap().path();
        fs::copy(&p, game.join(p.file_name().unwrap())).unwrap();
    }
    for m in MEDIA {
        let path = game.join(m);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, png(200, 300)).unwrap();
    }
    fs::create_dir_all(game.join("tl/french")).unwrap();
    fs::write(game.join("tl/french/script.rpy"), TL).unwrap();
    root
}

/// A deterministic text summary of an import: scenes with their node
/// kinds, and the report remarks.
fn summary(story: &Story, report: &orbit_renpy::ImportReport) -> String {
    let mut s = String::new();
    for scene in &story.scenes {
        let _ = writeln!(s, "scene {}", scene.name);
        for n in &scene.nodes {
            let kind = format!("{:?}", n.kind);
            let head: String = kind.chars().take_while(|c| c.is_alphanumeric()).collect();
            let detail = match &n.kind {
                NodeKind::Label { name } => format!(" {name}"),
                NodeKind::Say { text_id, .. } => {
                    format!(" {:?}", story.text(*text_id).unwrap_or(""))
                }
                NodeKind::Show {
                    expression,
                    position,
                    scale,
                    ..
                } => format!(" {expression} {position:?} {scale:?}"),
                _ => String::new(),
            };
            let _ = writeln!(s, "  {head}{detail}");
        }
    }
    let _ = writeln!(s, "unsupported:");
    for i in &report.unsupported {
        let _ = writeln!(s, "  {i}");
    }
    let _ = writeln!(s, "warnings:");
    for i in &report.warnings {
        let _ = writeln!(s, "  {i}");
    }
    s.replace("\r\n", "\n")
}

#[test]
fn sample_game_imports_every_palier1_feature() {
    let src = materialize("src");
    let out = src.with_file_name(format!(
        "{}_out",
        src.file_name().unwrap().to_string_lossy()
    ));
    let _ = fs::remove_dir_all(&out);
    let report = orbit_renpy::import(&src, &out, &|_, _| {}).expect("import");
    assert!(
        report.validation_errors.is_empty(),
        "{:#?}",
        report.validation_errors
    );
    let story = Story::from_ron(&fs::read_to_string(out.join("story.orbit")).unwrap()).unwrap();
    assert!(orbit_core::validate(&story).errors.is_empty());

    let all: Vec<&NodeKind> = story
        .scenes
        .iter()
        .flat_map(|s| s.nodes.iter().map(|n| &n.kind))
        .collect();
    let count = |f: &dyn Fn(&NodeKind) -> bool| all.iter().filter(|k| f(k)).count();
    let char_named = |n: &str| {
        story
            .characters
            .iter()
            .find(|c| story.text(c.name_id) == Some(n))
            .unwrap_or_else(|| panic!("character {n}"))
            .id
    };
    let (eileen, lucy) = (char_named("Eileen"), char_named("Lucy"));

    // 1. Transforms: a named position, inline transforms, timelines.
    assert!(
        story
            .positions
            .iter()
            .any(|p| p.name == "midleft" && p.x == 0.375 && p.y == 1.0)
    );
    assert!(all.iter().any(
        |k| matches!(k, NodeKind::Show { position: Position::Custom(n), .. } if n == "midleft")
    ));
    assert!(all.iter().any(|k| matches!(k, NodeKind::Show { character, position: Position::Free { x, .. }, scale: Some(s), .. } if *character == eileen && (*x - 0.55).abs() < 1e-5 && *s == 0.8)));
    assert!(all.iter().any(|k| matches!(k, NodeKind::Show { position: Position::Free { x, y }, .. } if *x == 0.5 && *y == 0.75)));
    let timelines: Vec<(&Loop, &Vec<orbit_core::StoryTrack>)> = all
        .iter()
        .filter_map(|k| match k {
            NodeKind::Timeline {
                looping,
                tracks,
                wait: false,
                ..
            } => Some((looping, tracks)),
            _ => None,
        })
        .collect();
    assert_eq!(timelines.len(), 3);
    // `bob` loops forever on the y offset of Lucy.
    let bob = timelines
        .iter()
        .find(|(l, _)| **l == Loop::Forever)
        .expect("bob");
    assert!(
        bob.1
            .iter()
            .all(|t| t.target == orbit_core::AnimTarget::Character(lucy)
                && t.prop == AnimProp::Offset)
    );
    // The inline ATL of Eileen: offset, opacity, scale and rotation.
    let inline = timelines
        .iter()
        .find(|(_, t)| t.iter().any(|t| t.prop == AnimProp::Rotation))
        .expect("inline ATL");
    let props: Vec<AnimProp> = inline.1.iter().map(|t| t.prop).collect();
    for p in [
        AnimProp::Offset,
        AnimProp::Opacity,
        AnimProp::Scale,
        AnimProp::Rotation,
    ] {
        assert!(props.contains(&p), "{p:?} in {props:?}");
    }
    // The animation ends on the staging: the offset track ends at zero.
    let offset = inline
        .1
        .iter()
        .find(|t| t.prop == AnimProp::Offset)
        .unwrap();
    let TrackData::Vec2(track) = &offset.data else {
        panic!()
    };
    assert_eq!(track.keys.last().unwrap().value.x, 0.0);

    // Transitions: Fade with a color, defined dissolves, approximations.
    assert!(all.iter().any(|k| matches!(k, NodeKind::Background { transition: Some(t), .. } if t.kind == TransitionKind::Fade(Color::WHITE) && (t.duration - 0.5).abs() < 1e-5)));
    assert!(all.iter().any(|k| matches!(k, NodeKind::Show { transition: Some(t), .. } if t.kind == TransitionKind::Dissolve && t.duration == 2.0)));
    assert_eq!(count(&|k| matches!(k, NodeKind::Effect(_))), 1, "vpunch");

    // 2. Script-defined images.
    assert!(out.join("assets/renpy_solid_000000ff.png").is_file());
    assert!(out.join("assets/renpy_solid_ff000080.png").is_file());
    let fire = story
        .characters
        .iter()
        .find(|c| story.text(c.name_id) == Some("Fire"))
        .unwrap();
    let Some(Motion::Animation(anim)) = &fire.expressions[0].motion else {
        panic!("fire frames")
    };
    assert_eq!(anim.frames.len(), 2);
    assert!((anim.fps - 10.0).abs() < 1e-3);
    let aug = story
        .characters
        .iter()
        .find(|c| story.text(c.name_id) == Some("Augustina"))
        .unwrap();
    assert_eq!(aug.expressions.len(), 1);

    // 3. Character styles and text tags.
    let texts: Vec<&str> = story.strings.entries.values().map(String::as_str).collect();
    assert!(
        texts.contains(&"\"{i}Hello there.{w=0.5} Nice big day.{/i}\""),
        "{texts:#?}"
    );
    assert!(texts.contains(&"{color=#ffeedd}Welcome to the {b}sample{/b} game!{/color}"));
    // `{nw}` + `extend`: one line.
    assert!(texts.contains(&"{color=#ffeedd}Let me show you around the place.{/color}"));
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("{nw}") || *t == " the place.")
    );
    assert!(texts.contains(&"{color=#ffeedd}Wait for it...\nThere!{/color}"));
    // The translation keeps the style.
    let tl = orbit_i18n::files::load(&out.join("i18n/story.fr.ron"))
        .unwrap()
        .unwrap();
    assert!(
        tl.entries
            .values()
            .any(|t| t == "\"{i}Bonjour.{w=0.5} Belle journée.{/i}\"")
    );

    // 4. Navigation screens.
    let maps: Vec<&orbit_core::HotspotMap> = all
        .iter()
        .filter_map(|k| match k {
            NodeKind::Hotspots(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(maps.len(), 2);
    let town = maps
        .iter()
        .find(|m| m.background.is_some())
        .expect("imagemap");
    assert_eq!(town.hotspots.len(), 4);
    assert_eq!(
        town.hotspots[0].shape,
        HotspotShape::Rect {
            x: 10.0 / 1280.0,
            y: 20.0 / 720.0,
            w: 200.0 / 1280.0,
            h: 100.0 / 720.0
        }
    );
    assert_eq!(story.text(town.hotspots[0].text_id), Some("Kitchen"));
    assert!(town.hotspots[2].condition.is_some());
    assert!(town.hotspots.iter().all(|h| h.hover_image.is_some()));
    let doors = maps
        .iter()
        .find(|m| m.background.is_none())
        .expect("image buttons");
    // Image buttons: their idle picture is drawn and hit-tested (a mask),
    // the hover picture shows on hover.
    let HotspotShape::Mask { image, x, y, w, h, draw, .. } = doors.hotspots[0].shape else {
        panic!("a mask zone: {:?}", doors.hotspots[0].shape);
    };
    assert_eq!((x, y, w, h), (100.0 / 1280.0, 200.0 / 720.0, 200.0 / 1280.0, 300.0 / 720.0));
    assert!(draw);
    let idle_path = story.asset_path(image).unwrap();
    assert!(idle_path.contains("door") && idle_path.contains("idle"), "{idle_path}");
    assert!(doors.hotspots.iter().all(|h| h.hover_image.is_some()));
    // `Jump("garden")` (another scene) alone: one zone action, no generated block;
    // `[SetVariable("points", 3), Jump("ending")]`: zone actions.
    assert!(matches!(doors.hotspots[0].actions.as_slice(), [orbit_core::HotspotAction::JumpScene { .. }]));
    assert!(matches!(
        doors.hotspots[1].actions.first(),
        Some(orbit_core::HotspotAction::SetVar { var, value: orbit_core::Operand::Lit(orbit_core::Value::Int(3)), .. }) if var == "points"
    ));
    assert!(doors.hotspots[1].actions.iter().any(|a| matches!(a, orbit_core::HotspotAction::JumpScene { .. })));
    // `show screen` of text buttons: a choice.
    let ending = story.scenes.iter().find(|s| s.name == "ending").unwrap();
    assert!(
        ending
            .nodes
            .iter()
            .any(|n| matches!(&n.kind, NodeKind::Choice { options } if options.len() == 2))
    );
    // `Call("garden")` makes `garden` return.
    let garden = story.scenes.iter().find(|s| s.name == "garden").unwrap();
    assert!(matches!(
        garden.nodes.last().unwrap().kind,
        NodeKind::Return
    ));

    // 5. Voice, achievements, persistent data, renpy.* calls.
    assert!(
        all.iter()
            .any(|k| matches!(k, NodeKind::Say { voice: Some(_), .. }))
    );
    assert_eq!(story.achievements.len(), 2);
    assert_eq!(count(&|k| matches!(k, NodeKind::Unlock { .. })), 2);
    let var = |n: &str| {
        story
            .variables
            .iter()
            .find(|v| v.name == n)
            .unwrap_or_else(|| panic!("{n}"))
    };
    assert_eq!(var("persistent_seen_end").scope, VarScope::Persistent);
    assert_eq!(var("persistent_visits").initial, Value::Int(0));
    assert_eq!(var("persistent_visits").scope, VarScope::Persistent);
    assert_eq!(var("persistent_met_lucy").initial, Value::Bool(false));
    assert!(
        all.iter()
            .any(|k| matches!(k, NodeKind::PlayMusic { fade_in, .. } if *fade_in == 2.0))
    );
    assert!(
        all.iter()
            .any(|k| matches!(k, NodeKind::StopMusic { fade_out } if *fade_out == 1.0))
    );
    assert_eq!(count(&|k| matches!(k, NodeKind::PlaySound { .. })), 1);

    // The report: what is left is listed with its place.
    let unsupported: Vec<String> = report.unsupported.iter().map(ToString::to_string).collect();
    assert!(!unsupported.iter().any(|i| i.contains("renpy.notify")));
    assert_eq!(count(&|k| matches!(k, NodeKind::Notify(_))), 2);
    assert!(
        unsupported
            .iter()
            .any(|i| i.starts_with("definitions.rpy:57:") && i.contains("parameters"))
    );
    assert!(
        !unsupported
            .iter()
            .any(|i| i.contains("town_map") || i.contains("call screen"))
    );
    assert!(report.warnings.iter().any(|w| w.message.contains("NVL")));
    assert!(report.warnings.iter().any(|w| w.message.contains("crop")));

    // The printed story reads back.
    let printed = orbit_dsl::print_story(&story);
    let back = orbit_dsl::parse(&printed, &story, orbit_dsl::Scope::Story).expect("DSL parses");
    assert_eq!(back.scenes.len(), story.scenes.len());

    // Golden summary (node kinds and report).
    let got = summary(&story, &report);
    let golden = Path::new(FIXTURE).join("expected.txt");
    if std::env::var_os("ORBIT_RENPY_BLESS").is_some() {
        fs::write(&golden, &got).unwrap();
    }
    let want = fs::read_to_string(&golden)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert_eq!(
        got, want,
        "golden summary changed (ORBIT_RENPY_BLESS=1 to update)"
    );

    // Deterministic: a second import gives the same summary.
    let out2 = out.with_extension("2");
    let _ = fs::remove_dir_all(&out2);
    let report2 = orbit_renpy::import(&src, &out2, &|_, _| {}).unwrap();
    let story2 = Story::from_ron(&fs::read_to_string(out2.join("story.orbit")).unwrap()).unwrap();
    assert_eq!(summary(&story2, &report2), got);

    for d in [&src, &out, &out2] {
        let _ = fs::remove_dir_all(d);
    }
}

fn convert(src: &str) -> (Story, orbit_renpy::ImportReport) {
    let media = [
        ("images/eileen happy.png", orbit_renpy::MediaKind::Image),
        ("images/bg room.png", orbit_renpy::MediaKind::Image),
    ];
    let (story, report) = orbit_renpy::convert_scripts("t", &[("script.rpy", src)], &media);
    assert!(
        report.validation_errors.is_empty(),
        "{:#?}",
        report.validation_errors
    );
    (story, report)
}

fn nodes(story: &Story) -> Vec<&NodeKind> {
    story
        .scenes
        .iter()
        .flat_map(|s| s.nodes.iter().map(|n| &n.kind))
        .collect()
}

#[test]
fn atl_repeat_and_warpers_become_a_looping_timeline() {
    let (story, _) = convert(
        "transform bob:\n    yoffset 0\n    easein 0.5 yoffset -108\n    easeout 0.5 yoffset 0\n    repeat 3\nlabel start:\n    show eileen happy at left, bob\n    \"x\"\n",
    );
    let n = nodes(&story);
    let show = n
        .iter()
        .position(|k| {
            matches!(
                k,
                NodeKind::Show {
                    position: Position::Left,
                    ..
                }
            )
        })
        .unwrap();
    let NodeKind::Timeline {
        duration,
        looping,
        wait,
        tracks,
    } = n[show + 1]
    else {
        panic!("timeline after the show")
    };
    assert_eq!((*duration, *looping, *wait), (1.0, Loop::Loop(3), false));
    let TrackData::Vec2(t) = &tracks[0].data else {
        panic!()
    };
    assert_eq!(t.keys.len(), 3);
    assert!((t.keys[1].value.y + 0.1).abs() < 1e-6, "-108 px of 1080");
    assert_eq!(
        t.keys[0].ease,
        orbit_core::Curve::Preset(orbit_core::Ease::OutSine)
    );
}

#[test]
fn transitions_with_color_warper_and_pause() {
    let (story, report) = convert(
        "define flash = Fade(0.1, 0.0, 0.4, color=\"#fff\")\ndefine soft = Dissolve(1.0, time_warp=_warper.ease)\nlabel start:\n    scene bg room with flash\n    show eileen happy with soft\n    with Pause(0.25)\n    with wiperight\n    \"x\"\n",
    );
    let n = nodes(&story);
    assert!(n.iter().any(|k| matches!(k, NodeKind::Background { transition: Some(t), .. } if t.kind == TransitionKind::Fade(Color::WHITE))));
    assert!(
        n.iter()
            .any(|k| matches!(k, NodeKind::Show { transition: Some(t), .. } if t.ease.is_some()))
    );
    assert!(
        n.iter()
            .any(|k| matches!(k, NodeKind::Wait { seconds } if *seconds == 0.25))
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.line == 7 && w.message.contains("wiperight"))
    );
}

#[test]
fn unconverted_transforms_and_extend_are_reported() {
    let (story, report) = convert(
        "transform shake(d):\n    xoffset d\ndefine e = Character(\"E\")\nlabel start:\n    show eileen happy at shake(5)\n    show eileen at mystery\n    e \"One.\"\n    extend \" Two.\"\n",
    );
    let at = |l: usize| {
        report
            .unsupported
            .iter()
            .any(|i| i.file == "script.rpy" && i.line == l)
    };
    assert!(at(1) && at(5) && at(6), "{:#?}", report.unsupported);
    // Without `{nw}`: a second line repeating the first.
    let texts: Vec<&str> = nodes(&story)
        .iter()
        .filter_map(|k| match k {
            NodeKind::Say { text_id, .. } => story.text(*text_id),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["One.", "One. Two."]);
}
