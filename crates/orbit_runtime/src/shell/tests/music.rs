//! The music of the screens (`ui_version` 9) through the shell: the title
//! music, a menu over the game replacing or ducking the game's music, the
//! game's music coming back, and the buttons.

use super::*;
use crate::ui_music::MusicOp;

const MAIN: &str = "(id: \"main_menu\", root: (kind: Stack(dir: Vertical), children: [\
    (kind: Button(text: Literal(\"New\"), action: [GameNew]))]), \
    music: Some((asset: \"music/title.ogg\", fade_in_ms: 800)))";

fn pause(music: &str) -> String {
    format!(
        "(id: \"pause\", modal: true, pauses_game: true, on_cancel: [Close], \
         root: (kind: Stack(dir: Vertical), children: [\
         (kind: Button(text: Literal(\"Back\"), action: [Close])), \
         (kind: Button(text: Literal(\"Title\"), action: [ReturnToTitle])), \
         (kind: Button(text: Literal(\"Jingle\"), action: [PlayMusic(\"music/jingle.ogg\")])), \
         (kind: Button(text: Literal(\"Silence\"), action: [StopMusic]))]), \
         music: Some(({music})))"
    )
}

fn plays(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::UiMusic(MusicOp::Play { path, .. }) => Some(path.clone()),
            _ => None,
        })
        .collect()
}

fn ducks(effects: &[Effect]) -> Vec<f32> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::UiMusic(MusicOp::Duck(level)) => Some(*level),
            _ => None,
        })
        .collect()
}

fn game_music_plays(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::Audio(AudioCommand::PlayMusic { .. })))
}

fn stops(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Audio(AudioCommand::StopMusic { .. })))
        .count()
}

/// A running game (three lines in, so the story's music is on) with the
/// screens `main` and `pause`; effects drained.
fn running(pause_music: &str) -> Shell {
    let mut sh = shell();
    sh.set_ui_bundle(&bundle_of(&[MAIN, &pause(pause_music)]));
    sh.update(0.0);
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 3);
    sh.take_effects();
    sh
}

#[test]
fn the_title_screen_music_plays_from_the_start_screen_and_stops_with_the_game() {
    let mut sh = shell();
    sh.set_ui_bundle(&bundle_of(&[MAIN, &pause("asset: \"music/p.ogg\"")]));
    sh.update(0.0);
    let effects = sh.take_effects();
    assert_eq!(plays(&effects), ["music/title.ogg"], "{effects:?}");
    assert!(matches!(
        effects.iter().find(|e| matches!(e, Effect::UiMusic(_))),
        Some(Effect::UiMusic(MusicOp::Play { fade_in_ms: 800, looping: true, .. }))
    ));
    // Nothing more while the title stays.
    sh.update(0.5);
    assert!(sh.take_effects().is_empty());
    // Opening a game stops the title track (the game brings its own music).
    sh.apply(UiEvent::StartGame);
    let effects = sh.take_effects();
    assert!(stops(&effects) >= 1, "{effects:?}");
    assert!(plays(&effects).is_empty());
    sh.update(0.1);
    assert!(plays(&sh.take_effects()).is_empty(), "the title music does not come back in game");
}

#[test]
fn a_menu_over_the_game_replaces_its_music_and_closing_restores_it() {
    let mut sh = running("asset: \"music/p.ogg\"");
    sh.apply(UiEvent::OpenPause);
    sh.update(0.0);
    let effects = sh.take_effects();
    assert_eq!(plays(&effects), ["music/p.ogg"], "{effects:?}");
    assert!(!game_music_plays(&effects));
    // Closing puts the story's track back, from the director's state.
    sh.apply(UiEvent::Back);
    sh.update(0.0);
    let effects = sh.take_effects();
    assert!(game_music_plays(&effects), "{effects:?}");
    assert!(plays(&effects).is_empty());
    // A second time works the same.
    sh.apply(UiEvent::OpenPause);
    sh.update(0.0);
    assert_eq!(plays(&sh.take_effects()), ["music/p.ogg"]);
}

#[test]
fn without_resume_the_game_music_stays_off() {
    let mut sh = running("asset: \"music/p.ogg\", resume_game_music: false");
    sh.apply(UiEvent::OpenPause);
    sh.update(0.0);
    sh.take_effects();
    sh.apply(UiEvent::Back);
    sh.update(0.0);
    let effects = sh.take_effects();
    assert!(!game_music_plays(&effects) && stops(&effects) == 1, "{effects:?}");
}

#[test]
fn a_ducking_menu_lowers_the_game_music_and_restores_the_level() {
    let mut sh = running("over_game: Duck, duck_volume: 0.2");
    sh.apply(UiEvent::OpenPause);
    sh.update(0.0);
    let effects = sh.take_effects();
    assert_eq!(ducks(&effects), [0.2], "{effects:?}");
    assert!(plays(&effects).is_empty() && !game_music_plays(&effects));
    sh.apply(UiEvent::Back);
    sh.update(0.0);
    assert_eq!(ducks(&sh.take_effects()), [1.0]);
}

#[test]
fn returning_to_the_title_starts_the_title_music_again() {
    let mut sh = running("asset: \"music/p.ogg\"");
    sh.apply(UiEvent::OpenPause);
    sh.update(0.0);
    sh.take_effects();
    sh.apply(UiEvent::ReturnToMainMenu);
    sh.update(0.0);
    let effects = sh.take_effects();
    assert_eq!(plays(&effects), ["music/title.ogg"], "{effects:?}");
    assert!(stops(&effects) >= 1, "the menu's and the game's tracks are stopped");
    assert!(!game_music_plays(&effects), "no game music to restore at the title");
}

#[test]
fn music_buttons_play_and_stop_until_the_screen_changes() {
    let mut sh = shell();
    sh.set_ui_bundle(&bundle_of(&[MAIN, &pause("asset: \"music/p.ogg\"")]));
    sh.update(0.0);
    sh.take_effects();
    sh.apply(UiEvent::PlayMusic("music/jingle.ogg".into()));
    assert_eq!(plays(&sh.take_effects()), ["music/jingle.ogg"]);
    // The screen on top did not change: the jingle is left alone.
    sh.update(0.5);
    assert!(sh.take_effects().is_empty());
    sh.apply(UiEvent::StopMusic);
    assert_eq!(stops(&sh.take_effects()), 1);
    sh.update(0.5);
    assert!(sh.take_effects().is_empty());
}
