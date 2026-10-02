//! Screen reader (M7-09): the accessibility tree of the shell and the
//! actions coming back from it. No OS, no GPU.

use super::*;
use orbit_ui::a11y::{A11yAction, A11yRole, A11yTree, CHOICE_BASE, CHOICES_ID, LINE_ID, ROOT_ID};

fn tree(sh: &mut Shell) -> A11yTree {
    sh.a11y_tree("Game")
}

fn by_name(tree: &A11yTree, role: A11yRole, name: &str) -> u64 {
    tree.nodes
        .iter()
        .find(|n| n.role() == role && n.name == name)
        .unwrap_or_else(|| panic!("no {role:?} named {name:?} in {tree:#?}"))
        .id
}

fn assert_consistent(tree: &A11yTree) {
    assert_eq!(tree.nodes[0].id, ROOT_ID);
    for n in &tree.nodes {
        for c in &n.children {
            assert!(tree.node(*c).is_some(), "dangling child {c}");
        }
    }
    assert!(tree.node(tree.focus).is_some(), "focus on a missing node");
}

#[test]
fn the_main_menu_is_exposed_and_clickable() {
    let mut sh = shell();
    let t = tree(&mut sh);
    assert_consistent(&t);
    let new_game = by_name(&t, A11yRole::Button, &sh.texts().get("ui.menu.new_game"));
    // The first button has the focus.
    assert_eq!(t.focus, new_game);
    sh.a11y_action(new_game, A11yAction::Click);
    assert!(sh.has_game(), "the click started a game");
}

#[test]
fn dialogue_lines_are_announced_with_the_speaker() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    let t = tree(&mut sh);
    assert_consistent(&t);
    let line = t.node(LINE_ID).expect("a live region with the line");
    assert_eq!(line.role(), A11yRole::Live);
    let spoken = &sh.game().unwrap().spoken.as_ref().unwrap().1;
    assert_eq!(&line.name, spoken);
    assert!(line.name.contains(&orbit_ui::a11y::plain(&current_text(&sh))));
    if let Flow::Say(l) = sh.game().unwrap().director.flow()
        && let Some((name, _)) = &l.speaker
    {
        assert!(line.name.starts_with(&format!("{name}: ")));
    }
    assert_eq!(t.announcements(), vec![line.name.as_str()]);

    // The tree only changes with the next line.
    assert_eq!(tree(&mut sh), t);
    play_lines(&mut sh, 1);
    let next = tree(&mut sh);
    assert_ne!(next.node(LINE_ID).unwrap().name, t.node(LINE_ID).unwrap().name);
}

#[test]
fn choices_are_buttons_and_a_click_picks_one() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    for _ in 0..2000 {
        if kind(&sh) == FlowKind::Choice {
            break;
        }
        step(&mut sh);
    }
    assert_eq!(kind(&sh), FlowKind::Choice);
    let t = tree(&mut sh);
    assert_consistent(&t);
    let list = t.node(CHOICES_ID).expect("the choice list");
    assert_eq!(list.children.len(), sh.game().unwrap().choice_labels.len());
    assert!(list.children.len() >= 2);
    assert_eq!(t.focus, CHOICE_BASE);
    assert!(t.node(LINE_ID).is_none(), "no line while choosing");
    sh.a11y_action(CHOICE_BASE + 1, A11yAction::Click);
    assert_ne!(kind(&sh), FlowKind::Choice);
}

#[test]
fn screens_on_top_hide_the_game_and_take_the_focus() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    sh.apply(UiEvent::OpenPreferences);
    let t = tree(&mut sh);
    assert_consistent(&t);
    assert!(t.node(LINE_ID).is_none(), "the preferences cover the textbox");
    let group = by_name(&t, A11yRole::Group, &sh.texts().get("ui.prefs.title"));
    assert!(t.node(t.focus).is_some() && t.focus != ROOT_ID && t.focus != group);

    // Increment the master volume slider through the screen reader.
    let label = sh.texts().get("ui.prefs.volume_master");
    let slider = by_name(&t, A11yRole::Slider, &label);
    let before = sh.prefs().volumes.master;
    sh.a11y_action(slider, A11yAction::Decrement);
    assert!(sh.prefs().volumes.master < before);
    let after = tree(&mut sh);
    assert_eq!(after.focus, slider);

    // Back closes the screen: the line is exposed again.
    let back = by_name(&after, A11yRole::Button, &sh.texts().get("ui.menu.back"));
    sh.a11y_action(back, A11yAction::Click);
    assert_eq!(sh.top(), None);
    assert!(tree(&mut sh).node(LINE_ID).is_some());
}

#[test]
fn the_preference_turns_the_support_off_and_back_on() {
    let mut sh = shell();
    assert!(sh.prefs().screen_reader, "on by default");
    sh.apply(UiEvent::SetScreenReader(false));
    assert!(!sh.prefs().screen_reader);
    let t = tree(&mut sh);
    assert_eq!(t.nodes.len(), 1, "only the window is left");
    // Actions are ignored too.
    let id = orbit_ui::a11y::widget_id(SLOT_MAIN, 0);
    sh.a11y_action(id, A11yAction::Click);
    assert!(!sh.has_game());
    sh.apply(UiEvent::SetScreenReader(true));
    assert!(tree(&mut sh).nodes.len() > 1);
}

#[test]
fn toasts_are_announced() {
    let mut sh = shell();
    sh.apply(UiEvent::Toast("Saved".into()));
    let t = tree(&mut sh);
    assert!(t.announcements().contains(&"Saved"));
}

#[test]
fn stale_ids_are_ignored() {
    let mut sh = shell();
    sh.a11y_action(orbit_ui::a11y::widget_id(40, 3), A11yAction::Click);
    sh.a11y_action(CHOICE_BASE, A11yAction::Click);
    sh.a11y_action(ROOT_ID, A11yAction::Click);
    assert!(!sh.has_game());
}
