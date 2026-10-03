//! Screens opened by the story (UI-14 / UI-15 / UI-16): HUDs that do not
//! pause, waiting screens that hand a value back, rollback and saves with a
//! screen open, generated lists, transitions.

use orbit_core::{Node, NodeId, NodeKind, Operand, ScalarType, Scene, SceneId, ScreenParam, VarDecl};
use orbit_save::{CaptureInfo, SaveData};

use super::*;

const HUD_SCREEN: &str = r#"(id: "hud", hud: true, pauses_game: true,
    root: (kind: Stack(dir: Vertical), layout: (width: Fixed(200.0), height: Fixed(80.0)), children: [
        (kind: Button(text: Bind(Var("gold")), action: [VarSet((var: "gold", op: Add, value: Lit(Int(1))))]),
         layout: (width: Fixed(200.0), height: Fixed(80.0))),
    ]))"#;

const BAG_SCREEN: &str = r#"(id: "bag", pauses_game: true, on_cancel: [Close],
    transition_in: Some((kind: SlideUp, duration: 0.5)),
    transition_out: Some((kind: Fade, duration: 0.5)),
    root: (kind: Stack(dir: Vertical), children: [
        (kind: Label(text: Bind(Param("who")))),
        (kind: Repeat(source: Var("bag")), children: [
            (kind: Button(text: Bind(Item), action: [CloseWith(Item)])),
        ]),
    ]))"#;

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

/// HUD -> "one" -> bag (waits, result `picked`) -> "two" -> close HUD -> "three".
fn screens_story() -> Arc<Story> {
    let mut story = Story::new("screens");
    story.variables = vec![
        VarDecl::game("gold", Value::Int(0)),
        VarDecl::game("picked", Value::from("")),
        VarDecl::game(
            "bag",
            Value::list(ScalarType::Str, [Value::from("key"), Value::from("map")]),
        ),
    ];
    let one = story.strings.add("one");
    let two = story.strings.add("two");
    let three = story.strings.add("three");
    let say = |t| {
        node(NodeKind::Say {
            speaker: None,
            text_id: t,
        })
    };
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "main".into(),
        nodes: vec![
            node(NodeKind::OpenScreen {
                screen: "hud".into(),
                wait: false,
                result: None,
                params: vec![],
            }),
            say(one),
            node(NodeKind::OpenScreen {
                screen: "bag".into(),
                wait: true,
                result: Some("picked".into()),
                params: vec![ScreenParam {
                    name: "who".into(),
                    value: Operand::Lit(Value::from("Aiko")),
                }],
            }),
            say(two),
            node(NodeKind::CloseScreen {
                screen: "hud".into(),
            }),
            say(three),
            node(NodeKind::End),
        ],
    });
    assert!(orbit_core::validate(&story).errors.is_empty());
    Arc::new(story)
}

fn screens_shell() -> Shell {
    let story = screens_story();
    let mut sh = Shell::new(
        story.clone(),
        Box::new(MemoryStorage::new()),
        Localizer::from_story(&story),
        Theme::default(),
        ShellConfig::default(),
    );
    sh.set_ui_bundle(&bundle_of(&[HUD_SCREEN, BAG_SCREEN]));
    sh.start_new_game();
    sh.update(0.02);
    sh
}

fn ids(sh: &Shell) -> Vec<&str> {
    sh.story_screens().iter().map(|s| s.state.screen.as_str()).collect()
}

fn var(sh: &Shell, name: &str) -> Value {
    sh.game().unwrap().director.vm().variable(name).cloned().unwrap()
}

/// Advances the line on screen (typewriter first).
fn next_line(sh: &mut Shell) {
    sh.input(ui(UiInput::Confirm));
    if kind(sh) == FlowKind::Say {
        sh.input(ui(UiInput::Confirm));
    }
    sh.update(0.02);
}

#[test]
fn a_hud_does_not_pause_and_lets_input_through() {
    let mut sh = screens_shell();
    assert_eq!(ids(&sh), vec!["hud"]);
    assert_eq!(current_text(&sh), "one");
    assert!(!sh.game_paused(), "a HUD never pauses, even with pauses_game");
    let t = sh.play_time;
    sh.update(0.5);
    assert!(sh.play_time > t);
    // A click on the HUD button runs its action and does not advance.
    let r = sh.story_screens[0].screen.ui().rect_of(1).unwrap();
    let p = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
    sh.input(ui(UiInput::PointerMove(p)));
    sh.input(ui(UiInput::PointerDown(p)));
    sh.input(ui(UiInput::PointerUp(p)));
    assert_eq!(var(&sh, "gold"), Value::Int(1));
    assert_eq!(current_text(&sh), "one");
    // Elsewhere (and the keyboard) the game gets the input.
    next_line(&mut sh);
    assert_eq!(kind(&sh), FlowKind::Screen);
}

#[test]
fn a_waiting_screen_returns_its_value_to_the_story() {
    let mut sh = screens_shell();
    next_line(&mut sh);
    assert_eq!(kind(&sh), FlowKind::Screen);
    assert_eq!(ids(&sh), vec!["hud", "bag"]);
    assert!(sh.game_paused(), "the bag asks to pause the game");
    // Its parameter and the rows of the list are built.
    assert_eq!(sh.story_screens[1].screen.params()[0].1, Value::from("Aiko"));
    assert!(sh.story_screens[1].screen.ui().find(2).is_some(), "two rows");
    // The keyboard goes to the screen: the first row is focused.
    sh.input(ui(UiInput::Confirm));
    assert_eq!(var(&sh, "picked"), Value::from("key"));
    assert_eq!(current_text(&sh), "two");
    assert_eq!(ids(&sh), vec!["hud"]);
    // It closes through its closing transition.
    assert_eq!(sh.ghosts.len(), 1);
    assert!(sh.is_animating());
    sh.update(1.0);
    assert!(sh.ghosts.is_empty());
    next_line(&mut sh);
    assert_eq!(current_text(&sh), "three");
    assert!(ids(&sh).is_empty(), "CloseScreen closed the HUD");
}

#[test]
fn escape_closes_without_a_value_and_skip_stops_at_the_screen() {
    let mut sh = screens_shell();
    sh.apply(UiEvent::ToggleSkip);
    for _ in 0..20 {
        sh.update(0.1);
    }
    // Skip all is off by default: an unread line stops it anyway; force it.
    next_line(&mut sh);
    assert_eq!(kind(&sh), FlowKind::Screen);
    assert!(!sh.game().unwrap().skipping(), "a waiting screen stops skip");
    sh.input(ui(UiInput::Cancel));
    assert_eq!(var(&sh, "picked"), Value::from(""));
    assert_eq!(current_text(&sh), "two");
}

#[test]
fn rollback_reopens_the_screen_and_undoes_its_value() {
    let mut sh = screens_shell();
    next_line(&mut sh);
    sh.input(ui(UiInput::Confirm));
    assert_eq!(current_text(&sh), "two");
    sh.apply(UiEvent::Rollback);
    sh.update(0.02);
    assert_eq!(kind(&sh), FlowKind::Screen);
    assert_eq!(ids(&sh), vec!["hud", "bag"]);
    assert_eq!(var(&sh, "picked"), Value::from(""));
    assert!(sh.ghosts.is_empty(), "rollback cuts transitions");
    assert!(sh.story_screens[1].elapsed.is_infinite(), "reopened without its opening transition");
    // Another pick this time.
    sh.input(ui(UiInput::Down));
    sh.input(ui(UiInput::Confirm));
    assert_eq!(var(&sh, "picked"), Value::from("map"));
    // Back to the first line: before the bag, only the HUD.
    sh.apply(UiEvent::Rollback);
    sh.apply(UiEvent::Rollback);
    sh.update(0.02);
    assert_eq!(current_text(&sh), "one");
    assert_eq!(ids(&sh), vec!["hud"]);
}

#[test]
fn a_save_made_with_the_screen_open_loads_with_it() {
    let mut sh = screens_shell();
    next_line(&mut sh);
    let vm = sh.game().unwrap().director.vm();
    let save = SaveData::capture(vm, CaptureInfo::new(1, 1));
    let bytes = save.encode().unwrap();

    let mut other = screens_shell();
    let mut vm = Vm::from_start(screens_story()).unwrap();
    SaveData::decode(&bytes).unwrap().apply_to(&mut vm).unwrap();
    other.install_vm(vm, true);
    other.update(0.02);
    assert_eq!(kind(&other), FlowKind::Screen);
    assert_eq!(ids(&other), vec!["hud", "bag"]);
    other.input(ui(UiInput::Confirm));
    assert_eq!(var(&other, "picked"), Value::from("key"));
    assert_eq!(current_text(&other), "two");
}

#[test]
fn a_screen_the_project_lacks_is_closed_instead_of_blocking() {
    let mut story = (*screens_story()).clone();
    if let NodeKind::OpenScreen { screen, .. } = &mut story.scenes[0].nodes[2].kind {
        *screen = "missing".into();
    }
    let story = Arc::new(story);
    let mut sh = Shell::new(
        story.clone(),
        Box::new(MemoryStorage::new()),
        Localizer::from_story(&story),
        Theme::default(),
        ShellConfig::default(),
    );
    sh.set_ui_bundle(&bundle_of(&[HUD_SCREEN]));
    sh.start_new_game();
    sh.update(0.02);
    next_line(&mut sh);
    assert_eq!(current_text(&sh), "two");
}

#[test]
fn reduced_motion_turns_screen_transitions_off() {
    let mut sh = screens_shell();
    sh.apply(UiEvent::SetReducedMotion(true));
    next_line(&mut sh);
    assert!(!sh.is_animating());
    sh.input(ui(UiInput::Confirm));
    assert!(sh.ghosts.is_empty());
}
