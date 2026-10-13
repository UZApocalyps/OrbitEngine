//! In-game notifications through the shell: a `Notify` node shows a toast
//! without stopping the story, a rollback or a load removes it, a click
//! dismisses it, a button can ask for one.

use orbit_core::{Node, NodeId, NodeKind, Notify, Scene, SceneId, VarDecl};

use super::*;

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

/// "one" -> notify "Gold: [gold]" -> "two" -> End.
fn notify_story() -> Arc<Story> {
    let mut story = Story::new("toasts");
    story.variables = vec![VarDecl::game("gold", Value::Int(5))];
    let one = story.strings.add("one");
    let two = story.strings.add("two");
    let text_id = story.strings.add("Gold: [gold]");
    let say = |t| node(NodeKind::Say { speaker: None, text_id: t, voice: None });
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "main".into(),
        nodes: vec![
            say(one),
            node(NodeKind::Notify(Notify {
                text_id,
                icon: None,
                sound: None,
                duration: Some(3.0),
                style: None,
            })),
            say(two),
            node(NodeKind::End),
        ],
    });
    assert!(orbit_core::validate(&story).errors.is_empty());
    Arc::new(story)
}

fn notify_shell() -> Shell {
    let story = notify_story();
    let mut sh = Shell::new(
        story.clone(),
        Box::new(MemoryStorage::new()),
        Localizer::from_story(&story),
        Theme::default(),
        ShellConfig::default(),
    );
    sh.start_new_game();
    sh.update(0.02);
    sh
}

fn next_line(sh: &mut Shell) {
    sh.input(ui(UiInput::Confirm));
    if kind(sh) == FlowKind::Say {
        sh.input(ui(UiInput::Confirm));
    }
    sh.update(0.02);
}

#[test]
fn a_notify_node_shows_a_toast_and_the_story_goes_on() {
    let mut sh = notify_shell();
    assert_eq!(current_text(&sh), "one");
    assert!(sh.toasts.is_empty());
    next_line(&mut sh);
    assert_eq!(current_text(&sh), "two", "non-blocking");
    assert_eq!(sh.toasts.texts(), ["Gold: 5"]);
    // Announced to screen readers.
    assert_eq!(sh.last_toast.as_ref().map(|(_, t)| t.as_str()), Some("Gold: 5"));
    sh.update(3.5);
    assert!(sh.toasts.is_empty(), "gone after its duration");
}

#[test]
fn a_rollback_removes_the_story_toast_and_keeps_the_others() {
    let mut sh = notify_shell();
    next_line(&mut sh);
    sh.apply(UiEvent::Toast("Saved".into()));
    assert_eq!(sh.toasts.texts(), ["Gold: 5", "Saved"]);
    sh.input(ui(UiInput::Left));
    assert_eq!(current_text(&sh), "one");
    assert_eq!(sh.toasts.texts(), ["Saved"]);
}

#[test]
fn a_click_on_a_toast_dismisses_it_without_advancing() {
    let mut sh = notify_shell();
    next_line(&mut sh);
    sh.update(0.5);
    let r = sh.toasts.rects()[0];
    let p = [r[0] + 4.0, r[1] + 4.0];
    sh.input(ui(UiInput::PointerDown(p)));
    sh.input(ui(UiInput::PointerUp(p)));
    assert_eq!(current_text(&sh), "two", "the click did not reach the game");
    sh.update(0.3);
    assert!(sh.toasts.is_empty());
}

#[test]
fn a_button_can_show_a_notification() {
    let mut sh = notify_shell();
    sh.apply(UiEvent::Notify(orbit_ui::Notification {
        text: "{b}Hi{/b}".into(),
        markup: true,
        sound: Some("assets/ding.ogg".into()),
        ..Default::default()
    }));
    assert_eq!(sh.toasts.texts(), ["Hi"]);
    sh.take_effects();
    sh.update(0.02);
    let effects = sh.take_effects();
    assert!(effects.iter().any(|e| matches!(e, Effect::PlayAsset(p) if p == "assets/ding.ogg")), "{effects:?}");
}
