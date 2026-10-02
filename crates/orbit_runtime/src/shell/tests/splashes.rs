//! Splash sequence in front of the shell (GPU free): engine splash, then the
//! project's, then the menu or a new game; skipping; hot reload.

use super::*;
use crate::splash::{AfterLoad, ENGINE_DURATION, ENGINE_SKIP_AFTER, SplashPhase, SplashState};
use orbit_ui::{SplashContent, SplashDef};

fn project_splash(skippable: bool, content: SplashContent) -> SplashDef {
    SplashDef {
        content,
        duration: 2.0,
        fade_in: 0.2,
        fade_out: 0.2,
        skippable,
        background: [0, 0, 0, 255],
    }
}

fn phase(project: &[SplashDef], start_game: bool) -> SplashPhase {
    SplashPhase::new(Box::new(shell()), SplashState::new(project), start_game)
}

/// Runs frames until the splashes end (panics after a bound).
fn play_out(mut sp: SplashPhase) -> Box<Shell> {
    for _ in 0..10_000 {
        match sp.update(0.1) {
            Ok(shell) => return shell,
            Err(back) => sp = back,
        }
    }
    panic!("the splashes never ended");
}

fn advance(sp: SplashPhase, dt: f32) -> SplashPhase {
    match sp.update(dt) {
        Err(sp) => sp,
        Ok(_) => panic!("the splashes ended too soon"),
    }
}

#[test]
fn engine_then_project_then_the_main_menu() {
    let project = [project_splash(true, SplashContent::Text("Studio".into()))];
    let mut sp = phase(&project, false);
    assert_eq!(sp.state.steps().len(), 2);
    let mut order = vec![sp.state.index()];
    let shell = loop {
        match sp.update(0.1) {
            Ok(shell) => break shell,
            Err(back) => {
                sp = back;
                if order.last() != Some(&sp.state.index()) {
                    order.push(sp.state.index());
                }
            }
        }
    };
    assert_eq!(order, [0, 1], "the engine first, then the project");
    assert!(!shell.has_game(), "no auto start: the main menu shows");
    assert_eq!(shell.top(), None);
}

#[test]
fn auto_start_begins_a_new_game_after_the_splashes() {
    let shell = play_out(phase(&[], true));
    assert!(shell.has_game());
}

#[test]
fn the_shell_receives_nothing_while_the_splashes_play() {
    let mut sp = phase(&[], true);
    sp.input(&ui(UiInput::Confirm));
    sp.input(&InputAction::Number(1));
    assert!(!sp.shell.has_game(), "the game starts only at the end");
}

#[test]
fn a_click_skips_only_once_the_engine_splash_allows_it() {
    let mut sp = phase(&[], false);
    sp.input(&ui(UiInput::Confirm));
    assert_eq!(sp.state.time(), 0.0, "too early: nothing happens");
    let mut sp = advance(sp, ENGINE_SKIP_AFTER + 0.1);
    let before = sp.state.time();
    sp.input(&ui(UiInput::Confirm));
    assert!(sp.state.time() > before, "the fade out starts");
    assert!(sp.state.time() < ENGINE_DURATION);
    let shell = play_out(sp);
    assert!(!shell.has_game());
}

#[test]
fn a_splash_without_image_still_plays_and_ends() {
    let project = [project_splash(
        true,
        SplashContent::Image("missing/logo.png".into()),
    )];
    let shell = play_out(phase(&project, false));
    assert!(!shell.has_game());
}

#[test]
fn a_non_skippable_project_splash_ignores_clicks() {
    let project = [project_splash(false, SplashContent::Text("Studio".into()))];
    let mut sp = phase(&project, false);
    while sp.state.index() == 0 {
        sp = advance(sp, 0.1);
    }
    let before = sp.state.time();
    sp.input(&ui(UiInput::Confirm));
    assert_eq!(sp.state.time(), before);
    assert_eq!(sp.state.index(), 1);
}

#[test]
fn a_reload_in_a_running_session_does_not_replay_the_splashes() {
    assert_eq!(AfterLoad::decide(true, true), AfterLoad::RunNow);
    assert_eq!(AfterLoad::decide(true, false), AfterLoad::RunNow);
    assert_eq!(AfterLoad::decide(false, true), AfterLoad::Splash);
    assert_eq!(AfterLoad::decide(false, false), AfterLoad::Ready);
}
