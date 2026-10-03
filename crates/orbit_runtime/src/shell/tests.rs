//! Tests of the shell: no GPU, an in-memory (or temporary) storage.

use orbit_core::Value;
use orbit_save::{FileStorage, MemoryStorage};

use super::*;
use crate::director::Flow;
use orbit_ui::UiBundle;

mod a11y;
mod splashes;
mod replay;
mod story_screens;

const DEMO: &str = include_str!("../../../../examples/stories/test_minimal.ron");
const FR: &str = include_str!("../../../../examples/stories/i18n/test_minimal.fr.ron");

fn story() -> Arc<Story> {
    Arc::new(Story::from_ron(DEMO).unwrap())
}

fn localizer() -> Localizer {
    let mut loc = Localizer::from_story(&story());
    loc.add_translation_ron(FR).unwrap();
    loc
}

fn shell_with(storage: &MemoryStorage, cfg: ShellConfig) -> Shell {
    Shell::new(
        story(),
        Box::new(storage.clone()),
        localizer(),
        Theme::default(),
        cfg,
    )
}

fn shell() -> Shell {
    shell_with(&MemoryStorage::new(), ShellConfig::default())
}

fn ui(input: UiInput) -> InputAction {
    InputAction::Ui(input)
}

fn slot(index: u32) -> UiSlot {
    UiSlot {
        category: SlotCategory::Manual,
        index,
    }
}

fn kind(sh: &Shell) -> FlowKind {
    sh.game().expect("a game is running").kind()
}

/// One logical "go on": answers choices, lets timers run, reads and advances
/// lines; then a frame passes.
fn step(sh: &mut Shell) {
    match kind(sh) {
        FlowKind::Choice => sh.input(InputAction::Number(1)),
        FlowKind::Say => {
            sh.input(ui(UiInput::Confirm)); // completes the typewriter...
            if kind(sh) == FlowKind::Say {
                sh.input(ui(UiInput::Confirm)); // ...then goes on
            }
        }
        FlowKind::Transition => sh.input(ui(UiInput::Confirm)),
        _ => sh.update(0.5),
    }
    sh.update(0.02);
}

/// Plays until a line (not a transition) is on screen after `lines` lines.
fn play_lines(sh: &mut Shell, lines: usize) {
    let mut seen = 0;
    let mut last = u64::MAX;
    for _ in 0..2000 {
        if kind(sh) == FlowKind::Say {
            let generation = sh.game().unwrap().director.generation();
            if generation != last {
                last = generation;
                seen += 1;
                if seen > lines {
                    return;
                }
            }
        }
        step(sh);
    }
    panic!("the story did not reach line {lines}");
}

fn current_text(sh: &Shell) -> String {
    match sh.game().unwrap().director.flow() {
        Flow::Say(line) => line.text.clone(),
        other => panic!("expected a line, got {other:?}"),
    }
}

fn play_to_end(sh: &mut Shell) {
    for _ in 0..5000 {
        if kind(sh) == FlowKind::Ended {
            return;
        }
        step(sh);
        sh.update(0.1);
    }
    panic!("the story did not end");
}

// ------------------------------------------------------- the screen machine

#[test]
fn boots_on_the_main_menu_without_a_game() {
    let sh = shell();
    assert!(!sh.has_game());
    assert_eq!(sh.top(), None);
    assert!(!sh.can_continue(), "nothing to continue yet");
}

#[test]
fn new_game_then_escape_opens_and_closes_the_pause_menu() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    assert!(sh.has_game());
    assert_eq!(sh.top(), None);
    sh.input(ui(UiInput::Cancel));
    assert_eq!(sh.top(), Some(Overlay::Pause));
    // The pause menu turns Cancel into Resume.
    sh.input(ui(UiInput::Cancel));
    assert_eq!(sh.top(), None);
}

#[test]
fn screens_stack_and_back_returns_one_level() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    sh.apply(UiEvent::OpenPause);
    sh.apply(UiEvent::OpenPreferences);
    assert_eq!(sh.top(), Some(Overlay::Prefs));
    sh.apply(UiEvent::Back);
    assert_eq!(sh.top(), Some(Overlay::Pause));
    sh.apply(UiEvent::OpenHistory);
    sh.apply(UiEvent::OpenHistory); // not stacked twice
    sh.apply(UiEvent::Back);
    assert_eq!(sh.top(), Some(Overlay::Pause));
    sh.apply(UiEvent::Resume);
    assert_eq!(sh.top(), None);
}

#[test]
fn main_menu_opens_load_and_preferences_and_goes_back() {
    let mut sh = shell();
    sh.apply(UiEvent::OpenLoad);
    assert_eq!(sh.top(), Some(Overlay::Load));
    sh.input(ui(UiInput::Cancel));
    assert_eq!(sh.top(), None);
    sh.apply(UiEvent::OpenPreferences);
    assert_eq!(sh.top(), Some(Overlay::Prefs));
    sh.apply(UiEvent::Back);
    assert_eq!(sh.top(), None);
    assert!(!sh.has_game());
}

#[test]
fn game_only_screens_are_refused_on_the_main_menu() {
    let mut sh = shell();
    sh.apply(UiEvent::OpenPause);
    sh.apply(UiEvent::OpenHistory);
    sh.apply(UiEvent::OpenSave);
    assert_eq!(sh.top(), None);
}

#[test]
fn return_to_the_main_menu_closes_the_game_and_stops_the_music() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    sh.apply(UiEvent::OpenPause);
    sh.take_effects();
    sh.apply(UiEvent::ReturnToMainMenu);
    assert!(!sh.has_game());
    assert_eq!(sh.top(), None);
    assert!(
        sh.take_effects()
            .iter()
            .any(|e| matches!(e, Effect::Audio(AudioCommand::StopMusic { .. })))
    );
}

#[test]
fn quit_requests_the_host_to_close() {
    let mut sh = shell();
    sh.apply(UiEvent::Quit);
    assert!(sh.take_effects().contains(&Effect::Quit));
}

#[test]
fn the_end_screen_returns_to_the_main_menu() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    play_to_end(&mut sh);
    assert_eq!(kind(&sh), FlowKind::Ended);
    sh.input(ui(UiInput::Confirm));
    assert!(!sh.has_game());
}

// ------------------------------------------------------------------ rollback

#[test]
fn wheel_and_keys_roll_back_and_forward() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 4);
    let latest = current_text(&sh);
    sh.input(ui(UiInput::Scroll(1.0)));
    let before = current_text(&sh);
    assert_ne!(before, latest);
    assert!(sh.game().unwrap().dialogue.is_complete(), "shown whole");
    sh.input(ui(UiInput::Left));
    let earlier = current_text(&sh);
    assert_ne!(earlier, before);
    sh.input(ui(UiInput::PageUp));
    sh.input(ui(UiInput::Scroll(-1.0)));
    sh.input(ui(UiInput::Right));
    sh.input(ui(UiInput::PageDown));
    // Back to where we started after as many steps forward as back.
    assert_eq!(current_text(&sh), latest);
}

#[test]
fn rollback_to_the_very_first_line_stops_there() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 2);
    for _ in 0..20 {
        sh.input(ui(UiInput::Scroll(1.0)));
    }
    assert_eq!(kind(&sh), FlowKind::Say);
    assert!(!sh.game().unwrap().director.can_rollback());
}

#[test]
fn rollback_over_a_choice_shows_it_again() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    for _ in 0..2000 {
        if kind(&sh) == FlowKind::Choice {
            break;
        }
        step(&mut sh);
    }
    assert_eq!(kind(&sh), FlowKind::Choice);
    sh.input(ui(UiInput::Scroll(1.0)));
    assert_eq!(kind(&sh), FlowKind::Say);
    sh.input(ui(UiInput::Scroll(-1.0)));
    assert_eq!(kind(&sh), FlowKind::Choice);
    assert!(sh.game().unwrap().menu.is_some());
}

#[test]
fn history_lists_the_lines_and_rewinds_to_one() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 5);
    let view = sh.history_view();
    assert!(view.entries.len() >= 5, "{} entries", view.entries.len());
    assert!(view.entries.iter().any(|e| e.speaker.is_some()));
    assert!(view.entries.iter().all(|e| e.can_rollback));
    let target = &view.entries[1];
    let wanted = target.text.clone();
    sh.apply(UiEvent::OpenPause);
    sh.apply(UiEvent::OpenHistory);
    sh.apply(UiEvent::RollbackTo(target.id));
    assert_eq!(sh.top(), None, "history closes after a rewind");
    assert_eq!(current_text(&sh), wanted);
}

#[test]
fn history_is_localized() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    sh.apply(UiEvent::SetLanguage("fr".into()));
    let view = sh.history_view();
    assert!(
        view.entries.iter().any(|e| e.text.contains("pluie")),
        "{:?}",
        view.entries
    );
}

// ----------------------------------------------------------------- quick menu

#[test]
fn clicks_on_the_quick_menu_do_not_advance_the_dialogue() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    sh.update(0.0); // builds the quick menu view
    let generation = sh.game().unwrap().director.generation();
    let bar = sh.quick.bar_rect();
    // The right end of the bar is the "Menu" button.
    let on_bar = [bar[0] + bar[2] - 12.0, bar[1] + bar[3] * 0.5];
    sh.input(ui(UiInput::PointerMove(on_bar)));
    sh.input(ui(UiInput::PointerDown(on_bar)));
    sh.input(ui(UiInput::PointerUp(on_bar)));
    assert_eq!(sh.top(), Some(Overlay::Pause), "the button was pressed");
    assert_eq!(sh.game().unwrap().director.generation(), generation);
    assert!(
        !sh.game().unwrap().dialogue.is_complete(),
        "the click did not even complete the line"
    );
    // A click elsewhere does.
    sh.apply(UiEvent::Resume);
    let elsewhere = [900.0, 300.0];
    sh.input(ui(UiInput::PointerDown(elsewhere)));
    assert!(sh.game().unwrap().dialogue.is_complete());
}

#[test]
fn quick_menu_events_toggle_skip_and_auto_and_roll_back() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 3);
    sh.apply(UiEvent::ToggleAuto);
    assert!(sh.game().unwrap().auto_on());
    sh.apply(UiEvent::ToggleSkip);
    assert!(sh.game().unwrap().skip_button_on() && !sh.game().unwrap().auto_on());
    let latest = current_text(&sh);
    sh.apply(UiEvent::Rollback);
    assert_ne!(current_text(&sh), latest);
}

// --------------------------------------------------------------------- saves

#[test]
fn save_then_load_restores_the_screen() {
    let storage = MemoryStorage::new();
    let mut a = shell_with(&storage, ShellConfig::default());
    a.set_now(1_790_966_520_000);
    a.apply(UiEvent::StartGame);
    play_lines(&mut a, 6);
    let text = current_text(&a);
    let stage = a.game().unwrap().director.stage().clone();
    assert!(!stage.characters.is_empty(), "someone is on stage by now");
    a.apply(UiEvent::OpenPause);
    a.apply(UiEvent::OpenSave);
    a.apply(UiEvent::SaveSlot(slot(2)));

    // "Restart the program" on the same storage.
    let mut b = shell_with(&storage, ShellConfig::default());
    assert!(b.can_continue());
    b.apply(UiEvent::OpenLoad);
    b.apply(UiEvent::LoadSlot(slot(2)));
    assert!(b.has_game());
    assert_eq!(b.top(), None, "loading closes the screens");
    assert_eq!(current_text(&b), text);
    assert!(b.game().unwrap().dialogue.is_complete(), "no typewriter");
    assert_eq!(b.game().unwrap().director.stage(), &stage);
    assert!(
        b.game().unwrap().director.can_rollback(),
        "history was saved"
    );
    // The story goes on from there.
    b.input(ui(UiInput::Confirm));
    b.input(ui(UiInput::Confirm));
    assert_ne!(
        b.game().unwrap().director.flow(),
        &Flow::Say(crate::director::Line {
            speaker: None,
            text,
            already_read: true
        })
    );
}

#[test]
fn loading_restores_the_music() {
    let storage = MemoryStorage::new();
    let mut a = shell_with(&storage, ShellConfig::default());
    a.apply(UiEvent::StartGame);
    play_lines(&mut a, 3);
    a.apply(UiEvent::QuickSave);
    let mut b = shell_with(&storage, ShellConfig::default());
    b.apply(UiEvent::OpenLoad);
    b.take_effects();
    b.apply(UiEvent::LoadSlot(UiSlot {
        category: SlotCategory::Quick,
        index: 0,
    }));
    let effects = b.take_effects();
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::Audio(AudioCommand::PlayMusic { looping: true, .. })
        )),
        "{effects:?}"
    );
}

#[test]
fn quicksave_and_quickload_hotkeys() {
    let mut sh = shell();
    sh.input(InputAction::Hotkey(Hotkey::QuickLoad)); // not in a game: ignored
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 2);
    sh.input(InputAction::Hotkey(Hotkey::QuickLoad));
    assert!(!sh.quick_exists, "no quicksave yet");
    assert!(!sh.toasts.is_empty(), "a toast says so");
    let saved = current_text(&sh);
    sh.input(InputAction::Hotkey(Hotkey::QuickSave));
    assert!(sh.quick_exists);
    play_lines(&mut sh, 3);
    assert_ne!(current_text(&sh), saved);
    sh.input(InputAction::Hotkey(Hotkey::QuickLoad));
    assert_eq!(current_text(&sh), saved);
}

#[test]
fn delete_slot_empties_it() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    sh.apply(UiEvent::SaveSlot(slot(0)));
    assert!(storage.read("saves/manual_001.sav").unwrap().is_some());
    sh.apply(UiEvent::DeleteSlot(slot(0)));
    assert!(storage.read("saves/manual_001.sav").unwrap().is_none());
}

#[test]
fn slot_views_show_date_preview_and_incompatibility() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(
        &storage,
        ShellConfig {
            utc_offset_minutes: 60,
            ..Default::default()
        },
    );
    sh.set_now(1_790_966_520_000); // 2026-10-02 18:42 UTC
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 2);
    sh.apply(UiEvent::SaveSlot(slot(4)));
    let views = sh.category_views();
    let manual = &views[0];
    assert_eq!(manual.slots.len(), 20);
    let used = &manual.slots[4];
    assert_eq!(used.state, UiSlotState::Used);
    assert!(
        used.timestamp.starts_with("2026-10-02 19:42"),
        "{}",
        used.timestamp
    );
    assert!(!used.preview.is_empty());
    assert_eq!(manual.slots[0].state, UiSlotState::Empty);
    // Quick and autosave categories exist too.
    assert_eq!(views[1].category, SlotCategory::Auto);
    assert_eq!(views[2].slots.len(), 1);
}

#[test]
fn a_corrupt_save_is_a_toast_not_a_crash() {
    let storage = MemoryStorage::new();
    storage.put_raw("saves/manual_001.sav", b"definitely not a save".to_vec());
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.apply(UiEvent::OpenLoad);
    let views = sh.category_views();
    assert_eq!(
        views[0].slots[0].state,
        UiSlotState::Used,
        "listed, loadable"
    );
    let before = sh.toasts.len();
    sh.apply(UiEvent::LoadSlot(slot(0)));
    assert!(!sh.has_game());
    assert_eq!(sh.toasts.len(), before + 1);
    assert!(!sh.can_continue(), "a damaged save is not 'continue'");
    // Loading an empty slot is a toast too.
    sh.apply(UiEvent::LoadSlot(slot(7)));
    assert!(!sh.has_game());
    // Continue without any save: a toast.
    sh.apply(UiEvent::Continue);
    assert!(!sh.has_game());
}

#[test]
fn a_failing_storage_reports_and_keeps_playing() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    storage.set_fail_writes(true);
    let before = sh.toasts.len();
    sh.apply(UiEvent::SaveSlot(slot(0)));
    assert_eq!(sh.toasts.len(), before + 1);
    assert!(sh.has_game());
    sh.update(10.0); // preferences write fails silently too
}

#[test]
fn continue_loads_the_latest_save() {
    let storage = MemoryStorage::new();
    let mut a = shell_with(&storage, ShellConfig::default());
    a.set_now(1_000);
    a.apply(UiEvent::StartGame);
    play_lines(&mut a, 2);
    a.apply(UiEvent::SaveSlot(slot(0)));
    let older = current_text(&a);
    play_lines(&mut a, 6);
    a.set_now(2_000);
    a.apply(UiEvent::SaveSlot(slot(1)));
    let newer = current_text(&a);
    assert_ne!(older, newer);
    let mut b = shell_with(&storage, ShellConfig::default());
    b.apply(UiEvent::Continue);
    assert!(b.has_game());
    assert_eq!(current_text(&b), newer);
}

// ------------------------------------------------------------------ autosave

#[test]
fn a_new_game_autosaves_when_the_first_scene_starts() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.apply(UiEvent::StartGame);
    assert!(storage.read("saves/auto_0.sav").unwrap().is_none());
    play_lines(&mut sh, 1);
    sh.update(0.1);
    assert!(storage.read("saves/auto_0.sav").unwrap().is_some());
    assert!(
        storage.read("saves/auto_1.sav").unwrap().is_none(),
        "one so far"
    );
    // ... and a fresh shell can continue from it.
    let mut b = shell_with(&storage, ShellConfig::default());
    assert!(b.can_continue());
    b.apply(UiEvent::Continue);
    assert!(b.has_game());
}

#[test]
fn autosave_repeats_every_n_lines_and_rotates() {
    let storage = MemoryStorage::new();
    let cfg = ShellConfig {
        autosave: AutosaveConfig {
            enabled: true,
            on_scene_start: false,
            every_lines: Some(3),
            min_interval: 0.0,
        },
        ..Default::default()
    };
    let mut sh = shell_with(&storage, cfg);
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 12);
    sh.update(0.1);
    for n in 0..3 {
        assert!(
            storage
                .read(&format!("saves/auto_{n}.sav"))
                .unwrap()
                .is_some(),
            "autosave {n}"
        );
    }
    assert!(
        storage.read("saves/auto_3.sav").unwrap().is_none(),
        "only 3 are kept"
    );
}

#[test]
fn autosave_can_be_disabled_and_loading_does_not_autosave() {
    let storage = MemoryStorage::new();
    let cfg = ShellConfig {
        autosave: AutosaveConfig {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut sh = shell_with(&storage, cfg);
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 8);
    sh.update(0.1);
    assert!(storage.read("saves/auto_0.sav").unwrap().is_none());
    sh.apply(UiEvent::SaveSlot(slot(0)));

    // Loading a save must not write an autosave of the line it lands on.
    let mut b = shell_with(&storage, ShellConfig::default());
    b.apply(UiEvent::LoadSlot(slot(0)));
    b.update(0.1);
    assert!(storage.read("saves/auto_0.sav").unwrap().is_none());
}

#[test]
fn closing_the_window_autosaves_the_running_game() {
    let storage = MemoryStorage::new();
    let cfg = ShellConfig {
        autosave: AutosaveConfig {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut sh = shell_with(&storage, cfg);
    sh.on_exit(); // no game: nothing to save
    assert!(storage.read("saves/auto_0.sav").unwrap().is_none());
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 3);
    sh.on_exit();
    assert!(storage.read("saves/auto_0.sav").unwrap().is_some());
    assert!(storage.read("persistent.ron").unwrap().is_some());
}

// ----------------------------------------------------------------- thumbnails

#[test]
fn a_save_waits_for_its_thumbnail_then_embeds_it() {
    let storage = MemoryStorage::new();
    let cfg = ShellConfig {
        capture_thumbnails: true,
        ..Default::default()
    };
    let mut sh = shell_with(&storage, cfg);
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 2);
    sh.apply(UiEvent::SaveSlot(slot(0)));
    assert!(
        storage.read("saves/manual_001.sav").unwrap().is_none(),
        "not written before the capture"
    );
    let requests = sh.take_capture_requests();
    // The autosave asked too; the manual save is the last request.
    let request = requests
        .last()
        .expect("the manual save asked for the stage");
    assert!(request.stage.background.is_some());
    assert!(sh.take_capture_requests().is_empty(), "asked only once");
    // The game moved on meanwhile: the save is the frozen moment anyway.
    step(&mut sh);
    sh.complete_capture(request.id, Some(vec![1, 2, 3]));
    let bytes = storage.read("saves/manual_001.sav").unwrap().unwrap();
    let meta = SaveData::decode_meta(&bytes).unwrap();
    assert_eq!(meta.thumbnail.as_deref(), Some(&[1u8, 2, 3][..]));
    // The list offers the PNG to the runtime to turn into a texture.
    let thumbs = sh.take_thumbnail_requests();
    assert_eq!(thumbs.len(), 1);
    assert_eq!(thumbs[0].key.0, slot(0));
}

#[test]
fn a_save_without_thumbnail_is_written_after_the_timeout() {
    let storage = MemoryStorage::new();
    let cfg = ShellConfig {
        capture_thumbnails: true,
        ..Default::default()
    };
    let mut sh = shell_with(&storage, cfg);
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 2);
    sh.apply(UiEvent::SaveSlot(slot(0)));
    sh.take_capture_requests(); // the runtime never answers
    sh.update(1.0);
    assert!(storage.read("saves/manual_001.sav").unwrap().is_none());
    sh.update(CAPTURE_TIMEOUT);
    let bytes = storage.read("saves/manual_001.sav").unwrap().unwrap();
    assert!(SaveData::decode_meta(&bytes).unwrap().thumbnail.is_none());
}

// ---------------------------------------------------------------- persistent

#[test]
fn persistent_variables_are_saved_when_the_story_ends() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.apply(UiEvent::StartGame);
    assert_eq!(
        sh.game().unwrap().director.vm().variable("plays"),
        Some(&Value::Int(0))
    );
    play_to_end(&mut sh);
    sh.update(0.1);
    let stored = sh.saves().load_persistent().unwrap();
    assert_eq!(stored.vars.get("plays"), Some(&Value::Int(1)));
    assert!(!stored.read_nodes.is_empty(), "read lines are kept too");

    // A new playthrough, even in a new process, starts from there.
    sh.input(ui(UiInput::Confirm)); // end screen -> main menu
    sh.apply(UiEvent::StartGame);
    assert_eq!(
        sh.game().unwrap().director.vm().variable("plays"),
        Some(&Value::Int(1))
    );
    let mut again = shell_with(&storage, ShellConfig::default());
    again.apply(UiEvent::StartGame);
    assert_eq!(
        again.game().unwrap().director.vm().variable("plays"),
        Some(&Value::Int(1))
    );
    // Lines read before count as read: the first one is skippable.
    let Flow::Transition { .. } = again.game().unwrap().director.flow() else {
        panic!("opening transition expected");
    };
    play_to_end(&mut again);
    again.update(0.1);
    assert_eq!(
        again.saves().load_persistent().unwrap().vars.get("plays"),
        Some(&Value::Int(2))
    );
}

#[test]
fn persistent_variables_survive_loading_a_save() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 2);
    sh.apply(UiEvent::SaveSlot(slot(0)));
    play_to_end(&mut sh); // plays -> 1
    sh.update(0.1);
    // Loading the old save keeps plays = 1: persistent data is not rolled back.
    sh.apply(UiEvent::LoadSlot(slot(0)));
    assert_eq!(
        sh.game().unwrap().director.vm().variable("plays"),
        Some(&Value::Int(1))
    );
}

// --------------------------------------------------------------- preferences

#[test]
fn preferences_are_applied_and_persisted() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    // Startup applies the saved (default) volumes.
    let initial = sh.take_effects();
    assert!(
        initial
            .iter()
            .filter(|e| matches!(e, Effect::SetVolume(..)))
            .count()
            == 4
    );
    sh.apply(UiEvent::SetVolume(AudioBus::Music, 0.3));
    sh.apply(UiEvent::SetTextSpeed(90.0));
    sh.apply(UiEvent::SetAutoDelay(4.0));
    sh.apply(UiEvent::SetSkipMode(UiSkipMode::All));
    assert!(
        sh.take_effects()
            .contains(&Effect::SetVolume(AudioBus::Music, 0.3))
    );
    // Not written on every change, but shortly after.
    assert!(storage.read("prefs.ron").unwrap().is_none());
    sh.update(PREFS_SAVE_DELAY + 0.1);
    let again = shell_with(&storage, ShellConfig::default());
    let p = again.prefs();
    assert_eq!(p.volumes.music, 0.3);
    assert_eq!(p.text_speed, 90.0);
    assert_eq!(p.auto_delay, 4.0);
    assert_eq!(p.skip_mode, orbit_save::SkipMode::All);
}

#[test]
fn preferences_reach_the_running_game() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    sh.apply(UiEvent::SetSkipMode(UiSkipMode::All));
    sh.apply(UiEvent::SetTextSpeed(0.5));
    sh.apply(UiEvent::SetTextSpeed(120.0));
    sh.apply(UiEvent::ToggleSkip);
    for _ in 0..400 {
        sh.update(0.1);
        if kind(&sh) == FlowKind::Choice {
            break;
        }
    }
    assert_eq!(
        kind(&sh),
        FlowKind::Choice,
        "skip-all read through the text"
    );
}

#[test]
fn closing_the_preferences_screen_writes_them() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.apply(UiEvent::OpenPreferences);
    sh.apply(UiEvent::SetAutoDelay(5.0));
    sh.apply(UiEvent::Back);
    assert!(storage.read("prefs.ron").unwrap().is_some());
}

#[test]
fn reset_preferences_restores_the_defaults() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.apply(UiEvent::SetVolume(AudioBus::Master, 0.1));
    sh.apply(UiEvent::SetLanguage("fr".into()));
    sh.take_effects();
    sh.apply(UiEvent::ResetPreferences);
    assert_eq!(sh.prefs(), &Preferences::default());
    assert_eq!(sh.language(), "en");
    assert!(
        sh.take_effects()
            .contains(&Effect::SetVolume(AudioBus::Master, 1.0))
    );
    assert_eq!(sh.texts().get("ui.menu.new_game"), "New Game");
}

#[test]
fn damaged_preferences_fall_back_to_defaults() {
    let storage = MemoryStorage::new();
    storage.put_raw("prefs.ron", b"(((".to_vec());
    storage.put_raw("persistent.ron", b"nope".to_vec());
    let sh = shell_with(&storage, ShellConfig::default());
    assert_eq!(sh.prefs(), &Preferences::default());
}

#[test]
fn fullscreen_is_only_offered_where_supported() {
    let mut sh = shell();
    assert!(sh.prefs_view().fullscreen.is_none());
    sh.input(InputAction::Hotkey(Hotkey::ToggleFullscreen));
    assert!(!sh.take_effects().contains(&Effect::SetFullscreen(true)));

    let cfg = ShellConfig {
        fullscreen_supported: true,
        ..Default::default()
    };
    let mut sh = shell_with(&MemoryStorage::new(), cfg);
    assert_eq!(sh.prefs_view().fullscreen, Some(false));
    sh.take_effects();
    sh.input(InputAction::Hotkey(Hotkey::ToggleFullscreen));
    assert!(sh.take_effects().contains(&Effect::SetFullscreen(true)));
    assert!(sh.prefs().fullscreen);
    sh.apply(UiEvent::SetFullscreen(false));
    assert!(!sh.prefs().fullscreen);
}

// ---------------------------------------------------------------------- i18n

#[test]
fn language_change_translates_ui_and_the_current_line() {
    let mut sh = shell();
    assert_eq!(sh.language(), "en");
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    let english = current_text(&sh);
    sh.apply(UiEvent::SetLanguage("fr".into()));
    assert_eq!(sh.language(), "fr");
    assert_eq!(sh.texts().get("ui.menu.new_game"), "Nouvelle partie");
    let french = current_text(&sh);
    assert_ne!(english, french);
    assert!(sh.game().unwrap().dialogue.is_complete());
    // The speaker name is translated as well.
    sh.apply(UiEvent::SetLanguage("en".into()));
    assert_eq!(current_text(&sh), english);
    // An unknown language changes nothing.
    sh.apply(UiEvent::SetLanguage("zz".into()));
    assert_eq!(sh.language(), "en");
}

#[test]
fn the_language_choice_is_remembered() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.apply(UiEvent::SetLanguage("fr".into()));
    sh.update(PREFS_SAVE_DELAY + 0.1);
    let again = shell_with(&storage, ShellConfig::default());
    assert_eq!(again.language(), "fr");
    assert_eq!(again.texts().get("ui.menu.quit"), "Quitter");
}

#[test]
fn initial_language_is_the_preference_then_the_system_then_the_source() {
    let with_system = |system: Option<&str>| ShellConfig {
        system_language: system.map(str::to_owned),
        ..Default::default()
    };
    // System language, regional variant included.
    let sh = shell_with(&MemoryStorage::new(), with_system(Some("fr-FR")));
    assert_eq!(sh.language(), "fr");
    // Unknown system language: the source language.
    let sh = shell_with(&MemoryStorage::new(), with_system(Some("de-DE")));
    assert_eq!(sh.language(), "en");
    let sh = shell_with(&MemoryStorage::new(), with_system(None));
    assert_eq!(sh.language(), "en");
    // A stored preference beats the system.
    let storage = MemoryStorage::new();
    let mut first = shell_with(&storage, with_system(Some("fr")));
    first.apply(UiEvent::SetLanguage("en".into()));
    first.update(PREFS_SAVE_DELAY + 0.1);
    let second = shell_with(&storage, with_system(Some("fr")));
    assert_eq!(second.language(), "en");
}

#[test]
fn the_game_can_override_interface_texts() {
    let mut ui_overrides = HashMap::new();
    ui_overrides.insert(
        "en".to_owned(),
        vec![("ui.menu.new_game".to_owned(), "Begin".to_owned())],
    );
    let cfg = ShellConfig {
        ui_overrides,
        ..Default::default()
    };
    let mut sh = shell_with(&MemoryStorage::new(), cfg);
    assert_eq!(sh.texts().get("ui.menu.new_game"), "Begin");
    assert_eq!(sh.texts().get("ui.menu.quit"), "Quit");
    sh.apply(UiEvent::SetLanguage("fr".into()));
    assert_eq!(sh.texts().get("ui.menu.new_game"), "Nouvelle partie");
}

#[test]
fn the_language_list_names_every_language() {
    let sh = shell();
    let view = sh.prefs_view();
    let names: Vec<_> = view.languages.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["English", "Fran\u{e7}ais"]);
}

// -------------------------------------------------------------- real files

#[test]
fn saves_land_in_the_data_directory() {
    let dir = std::env::temp_dir().join(format!("orbit_shell_files_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut sh = Shell::new(
            story(),
            Box::new(FileStorage::new(&dir)),
            localizer(),
            Theme::default(),
            ShellConfig::default(),
        );
        sh.apply(UiEvent::StartGame);
        play_lines(&mut sh, 2);
        sh.update(0.1);
        sh.apply(UiEvent::SaveSlot(slot(0)));
        sh.on_exit();
    }
    assert!(dir.join("saves").join("auto_0.sav").is_file());
    assert!(dir.join("saves").join("manual_001.sav").is_file());
    assert!(dir.join("persistent.ron").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------- embedded (preview)

fn embedded() -> Shell {
    let mut sh = shell_with(
        &MemoryStorage::new(),
        ShellConfig {
            embedded: true,
            capture_thumbnails: false,
            ..ShellConfig::default()
        },
    );
    let vm = Vm::from_start(story()).unwrap();
    sh.install_vm(vm, false);
    sh
}

#[test]
fn an_embedded_shell_plays_at_once_and_never_shows_the_main_menu() {
    let mut sh = embedded();
    assert!(sh.has_game());
    play_to_end(&mut sh);
    // Confirming on the end screen starts the story again instead of going
    // to a main menu.
    sh.input(ui(UiInput::Confirm));
    assert!(sh.has_game());
    assert_ne!(kind(&sh), FlowKind::Ended);
    assert_eq!(sh.top(), None);
    // The pause menu's "main menu" does the same.
    sh.apply(UiEvent::ReturnToMainMenu);
    assert!(sh.has_game());
}

#[test]
fn an_embedded_shell_does_not_keep_persistent_data_between_playthroughs() {
    let mut sh = embedded();
    play_to_end(&mut sh);
    sh.input(ui(UiInput::Confirm)); // restart
    // `plays` is persistent in the demo: it is back to its initial value.
    let plays = sh.game().unwrap().director.vm().variable("plays").cloned();
    assert_eq!(plays, Some(Value::Int(0)));
}

#[test]
fn a_line_waiting_for_the_player_is_a_still_image() {
    let mut sh = embedded();
    // The first background fades in: the picture moves.
    assert!(sh.is_animating());
    for _ in 0..20 {
        sh.update(30.0);
        if kind(&sh) == FlowKind::Say {
            break;
        }
    }
    assert_eq!(kind(&sh), FlowKind::Say);
    assert!(sh.is_animating(), "the line is being typed");
    sh.update(30.0);
    assert!(!sh.is_animating(), "a complete line waiting for a click");
    for _ in 0..50 {
        sh.update(0.016);
        assert!(!sh.is_animating());
    }
    // A choice waits too.
    while kind(&sh) != FlowKind::Choice {
        step(&mut sh);
        sh.update(0.5);
    }
    sh.update(5.0);
    assert!(!sh.is_animating());
}

#[test]
fn timers_transitions_skip_and_toasts_keep_the_picture_moving() {
    let mut sh = embedded();
    // The first background has a fade transition somewhere before a line.
    let mut saw_transition = false;
    for _ in 0..40 {
        if matches!(kind(&sh), FlowKind::Transition | FlowKind::Waiting) {
            saw_transition = true;
            assert!(sh.is_animating());
        }
        step(&mut sh);
    }
    assert!(saw_transition);
    // Skip runs on its own...
    let mut sh = embedded();
    sh.set_skip(true);
    assert!(sh.is_animating());
    // ...and stops at the first choice, leaving a still image.
    for _ in 0..400 {
        sh.update(0.1);
        if kind(&sh) == FlowKind::Choice {
            break;
        }
    }
    assert_eq!(kind(&sh), FlowKind::Choice);
    sh.update(1.0);
    assert!(!sh.is_animating());
    // A toast (quick save) fades: animating until it is gone.
    sh.apply(UiEvent::QuickSave);
    sh.update(0.016);
    assert!(sh.is_animating());
    sh.update(10.0);
    assert!(!sh.is_animating());
}

#[test]
fn the_embedded_shell_reports_the_node_on_screen() {
    let mut sh = embedded();
    for _ in 0..20 {
        sh.update(30.0);
        if kind(&sh) == FlowKind::Say {
            break;
        }
    }
    let shown = sh.shown_node().expect("a line is shown");
    let story = story();
    let (s, n) = story.locate(shown).unwrap();
    assert!(matches!(
        story.scenes[s].nodes[n].kind,
        orbit_core::NodeKind::Say { .. }
    ));
}

// --------------------------------------------------- data-driven screens

fn with_bundle(bundle: &UiBundle) -> Shell {
    let mut sh = shell();
    sh.set_ui_bundle(bundle);
    sh
}

fn bundle_of(screens: &[&str]) -> UiBundle {
    UiBundle::from_parts(None, screens).unwrap()
}

const HUD: &str = "(id: \"hud\", modal: true, root: (kind: Stack(dir: Vertical)))";
const BLOCKER: &str = "(id: \"blocker\", pauses_game: true, on_cancel: [Close], \
    root: (kind: Stack(dir: Vertical), children: [(kind: Button(text: Literal(\"x\"), action: [Close]))]))";

#[test]
fn a_data_main_menu_replaces_the_stock_one() {
    let mut sh = with_bundle(&UiBundle::builtin());
    assert!(sh.sys_main.is_some() && sh.sys_pause.is_some() && sh.sys_prefs.is_some());
    sh.input(ui(UiInput::Confirm)); // first button: New game
    assert!(sh.has_game());
}

#[test]
fn stock_screens_stay_when_the_bundle_provides_none() {
    let mut sh = with_bundle(&bundle_of(&[HUD]));
    assert!(sh.sys_main.is_none() && sh.sys_pause.is_none() && sh.sys_prefs.is_none());
    sh.input(ui(UiInput::Confirm));
    assert!(sh.has_game(), "the Rust main menu still starts the game");
    sh.apply(UiEvent::OpenPause);
    assert_eq!(sh.top(), Some(Overlay::Pause));
}

#[test]
fn the_data_pause_menu_pauses_the_game_and_cancel_resumes() {
    let mut sh = with_bundle(&UiBundle::builtin());
    sh.start_new_game();
    sh.update(0.5);
    sh.apply(UiEvent::OpenPause);
    let Some(Overlay::Custom(_)) = sh.top() else {
        panic!("{:?}", sh.top())
    };
    let before = sh.play_time;
    sh.update(0.5);
    assert_eq!(sh.play_time, before, "the game is paused");
    sh.input(ui(UiInput::Cancel));
    assert_eq!(sh.top(), None, "on_cancel: CloseAll");
    sh.update(0.5);
    assert!(sh.play_time > before);
}

#[test]
fn a_screen_without_pauses_game_lets_the_game_run() {
    let mut sh = with_bundle(&bundle_of(&[HUD, BLOCKER]));
    sh.start_new_game();
    sh.apply(UiEvent::OpenScreen("hud".into()));
    assert!(matches!(sh.top(), Some(Overlay::Custom(_))));
    let t = sh.play_time;
    sh.update(0.5);
    assert!(sh.play_time > t, "hud does not pause");
    sh.apply(UiEvent::ReplaceScreen("blocker".into()));
    let t = sh.play_time;
    sh.update(0.5);
    assert_eq!(sh.play_time, t, "blocker pauses");
}

#[test]
fn screens_open_replace_and_close_by_id_with_stock_fallbacks() {
    let mut sh = with_bundle(&bundle_of(&[HUD, BLOCKER]));
    sh.start_new_game();
    sh.apply(UiEvent::OpenScreen("preferences".into()));
    assert_eq!(
        sh.top(),
        Some(Overlay::Prefs),
        "no data screen: the stock one"
    );
    sh.apply(UiEvent::ReplaceScreen("hud".into()));
    assert!(matches!(sh.top(), Some(Overlay::Custom(_))));
    assert_eq!(sh.stack.len(), 1);
    sh.apply(UiEvent::OpenScreen("blocker".into()));
    sh.apply(UiEvent::OpenScreen("nope".into()));
    assert_eq!(sh.stack.len(), 2, "unknown ids are ignored");
    sh.apply(UiEvent::CloseAll);
    assert_eq!(sh.top(), None);
}

#[test]
fn data_preferences_edit_the_player_settings() {
    let mut sh = with_bundle(&UiBundle::builtin());
    sh.take_effects();
    sh.apply(UiEvent::OpenPreferences);
    assert!(matches!(sh.top(), Some(Overlay::Custom(_))));
    sh.input(ui(UiInput::Left)); // the first widget is the master slider
    assert!(sh.prefs().volumes.master < 1.0);
    let effects = sh.take_effects();
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SetVolume(AudioBus::Master, v) if *v < 1.0)),
        "{effects:?}"
    );
    sh.input(ui(UiInput::Cancel));
    assert_eq!(sh.top(), None, "Close");
}

#[test]
fn play_sound_and_custom_actions_become_effects() {
    let mut sh = shell();
    sh.apply(UiEvent::PlaySound("click.ogg".into()));
    sh.apply(UiEvent::Custom {
        name: "hi".into(),
        args: vec!["a".into()],
    });
    let effects = sh.take_effects();
    assert!(effects.contains(&Effect::PlayAsset("click.ogg".into())));
    assert!(effects.contains(&Effect::Custom {
        name: "hi".into(),
        args: vec!["a".into()]
    }));
}

#[test]
fn data_save_and_load_screens_stand_in_for_the_stock_ones() {
    // A bundle without them: the Rust screens stay.
    let mut sh = with_bundle(&bundle_of(&[HUD]));
    assert!(sh.sys_save.is_none() && sh.sys_load.is_none() && !sh.slots_wanted);
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    sh.apply(UiEvent::OpenSave);
    assert_eq!(sh.top(), Some(Overlay::Save));

    // The built-in bundle provides both.
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.set_ui_bundle(&UiBundle::builtin());
    let (save, load) = (sh.sys_save.unwrap(), sh.sys_load.unwrap());
    assert!(sh.slots_wanted);
    // On the title nothing can be saved, even by id.
    sh.apply(UiEvent::OpenScreen("save".into()));
    assert_eq!(sh.top(), None);
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    sh.apply(UiEvent::OpenSave);
    assert_eq!(sh.top(), Some(Overlay::Custom(save)));
    // The first card is the empty slot 1: picking it saves.
    sh.input(ui(UiInput::Confirm));
    for _ in 0..20 {
        sh.update(0.1);
    }
    assert!(storage.read("saves/manual_001.sav").unwrap().is_some());
    assert_eq!(
        sh.data_context().saves[0].slots[0].state,
        UiSlotState::Used,
        "the screen's slots follow the storage"
    );
    sh.input(ui(UiInput::Cancel));
    assert_eq!(sh.top(), None, "on_cancel: Close");
    sh.apply(UiEvent::OpenLoad);
    assert_eq!(sh.top(), Some(Overlay::Custom(load)));
}

// ----------------------------------- every action of a button, end to end

/// A shell showing the screen `t` (one button running `action`, a RON
/// action list entry) and a second screen `p2`; `setup` runs first. The
/// button is then pressed with the keyboard.
fn press(actions: &str, setup: impl FnOnce(&mut Shell)) -> (Shell, MemoryStorage) {
    let t = format!(
        "(id: \"t\", on_cancel: [Close], root: (kind: Stack(dir: Vertical), children: [\
         (kind: Button(text: Literal(\"x\"), action: [{actions}]))]))"
    );
    let p2 = "(id: \"p2\", root: (kind: Stack(dir: Vertical), children: [\
              (kind: Button(text: Literal(\"y\"), action: [Close]))]))";
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.set_ui_bundle(&bundle_of(&[&t, p2]));
    setup(&mut sh);
    sh.apply(UiEvent::OpenScreen("t".into()));
    assert!(
        matches!(sh.top(), Some(Overlay::Custom(_))),
        "{:?}",
        sh.top()
    );
    sh.take_effects();
    sh.input(ui(UiInput::Confirm));
    (sh, storage)
}

fn in_game(sh: &mut Shell) {
    sh.apply(UiEvent::StartGame);
    play_lines(sh, 1);
}

fn is_screen(sh: &Shell, id: &str) -> bool {
    matches!(sh.top(), Some(Overlay::Custom(i)) if sh.data[i].id() == id)
}

#[test]
fn a_button_opens_replaces_and_closes_screens() {
    let (sh, _) = press("Open(\"p2\")", |_| {});
    assert!(is_screen(&sh, "p2") && sh.stack.len() == 2);
    let (sh, _) = press("Replace(\"p2\")", |_| {});
    assert!(is_screen(&sh, "p2") && sh.stack.len() == 1);
    let (sh, _) = press("Close", |_| {});
    assert_eq!(sh.top(), None);
    let (sh, _) = press("CloseAll", |sh| sh.apply(UiEvent::OpenScreen("p2".into())));
    assert_eq!(sh.top(), None, "every screen is closed");
}

#[test]
fn a_button_drives_the_game_flow() {
    let (sh, _) = press("GameNew", |_| {});
    assert!(sh.has_game());
    let (sh, _) = press("ReturnToTitle", |sh| sh.apply(UiEvent::StartGame));
    assert!(!sh.has_game());
    let (mut sh, effects) = press("GameQuit", |_| {});
    let _ = effects;
    assert!(sh.take_effects().contains(&Effect::Quit));
}

#[test]
fn a_button_continues_from_the_last_save() {
    let (sh, storage) = press("GameContinue", |sh| {
        in_game(sh);
        sh.apply(UiEvent::SaveSlot(slot(0)));
        for _ in 0..20 {
            sh.update(0.1);
        }
        sh.apply(UiEvent::ReturnToMainMenu);
    });
    assert!(storage.read("saves/manual_001.sav").unwrap().is_some());
    assert!(sh.has_game(), "Continue loaded the save");
}

#[test]
fn a_button_saves_and_loads_quick_and_opens_the_slot_screens() {
    let (mut sh, _) = press("QuickSave", in_game);
    for _ in 0..20 {
        sh.update(0.1);
    }
    assert_eq!(sh.category_views()[2].slots[0].state, UiSlotState::Used);
    let (sh, _) = press("QuickLoad", |_| {});
    assert!(!sh.toasts.is_empty(), "no quicksave: the player is told");
    let (sh, _) = press("OpenSave", in_game);
    assert_eq!(sh.top(), Some(Overlay::Save));
    let (sh, _) = press("OpenLoad", in_game);
    assert_eq!(sh.top(), Some(Overlay::Load));
}

#[test]
fn a_button_resets_preferences_plays_sounds_and_calls_hooks() {
    let (sh, _) = press("ResetPreferences", |sh| {
        sh.apply(UiEvent::SetVolume(AudioBus::Master, 0.3));
    });
    assert_eq!(
        sh.prefs().volumes.master,
        Preferences::default().volumes.master
    );
    let (mut sh, _) = press(
        "PlaySound(\"click.ogg\"), Custom(name: \"hi\", args: [\"a\"])",
        |_| {},
    );
    let effects = sh.take_effects();
    assert!(
        effects.contains(&Effect::PlayAsset("click.ogg".into())),
        "{effects:?}"
    );
    assert!(effects.contains(&Effect::Custom {
        name: "hi".into(),
        args: vec!["a".into()]
    }));
}

const VARS_SCREEN: &str = r#"(id: "vars", pauses_game: true, on_cancel: [Close],
    root: (kind: Stack(dir: Vertical), children: [
        (kind: Button(text: Literal("+5"), action: [VarSet((var: "credits", op: Add, value: Lit(Int(5))))])),
        (kind: Button(text: Literal("plays"), action: [VarSet((var: "plays", op: Add, value: Lit(Int(1))))])),
        (kind: Button(text: Literal("bad"), action: [VarSet((var: "credits", op: Div, value: Lit(Int(0))))])),
        (kind: Label(text: Bind(Var("credits")))),
    ]))"#;

fn game_var(sh: &Shell, name: &str) -> Option<Value> {
    sh.game().unwrap().director.vm().variable(name).cloned()
}

#[test]
fn a_var_button_changes_the_running_game_and_persistent_data() {
    let mut sh = with_bundle(&bundle_of(&[VARS_SCREEN]));
    sh.start_new_game();
    assert_eq!(game_var(&sh, "credits"), Some(Value::Int(100)));
    sh.apply(UiEvent::OpenScreen("vars".into()));
    sh.input(ui(UiInput::Confirm)); // the first (focused) button: +5
    assert_eq!(game_var(&sh, "credits"), Some(Value::Int(105)));
    // The screen reads the new value back from the game.
    let ctx = sh.data_context();
    assert_eq!(ctx.vars.get("credits"), Some(&Value::Int(105)));
    // Persistent variables are written to the store.
    sh.apply(UiEvent::VarOp(VarOp::new(
        "plays",
        orbit_core::OpKind::Add,
        orbit_core::Operand::Lit(Value::Int(1)),
    )));
    assert_eq!(sh.persistent.vars.get("plays"), Some(&Value::Int(1)));
    // A division by zero is ignored.
    sh.apply(UiEvent::VarOp(VarOp::new(
        "credits",
        orbit_core::OpKind::Div,
        orbit_core::Operand::Lit(Value::Int(0)),
    )));
    assert_eq!(game_var(&sh, "credits"), Some(Value::Int(105)));
}

#[test]
fn at_the_title_only_persistent_variables_change() {
    let mut sh = with_bundle(&bundle_of(&[VARS_SCREEN]));
    assert!(!sh.has_game());
    let add = |name: &str| {
        UiEvent::VarOp(VarOp::new(
            name,
            orbit_core::OpKind::Add,
            orbit_core::Operand::Lit(Value::Int(1)),
        ))
    };
    sh.apply(add("credits"));
    sh.apply(add("plays"));
    let vars = sh.data_context().vars;
    assert_eq!(
        vars.get("credits"),
        Some(&Value::Int(100)),
        "no playthrough"
    );
    assert_eq!(vars.get("plays"), Some(&Value::Int(1)));
    // The next game starts with it.
    sh.start_new_game();
    assert_eq!(game_var(&sh, "plays"), Some(Value::Int(1)));
}

fn shown_node(sh: &Shell) -> Option<orbit_core::NodeId> {
    sh.game()?.director.shown().map(|i| i.node())
}

#[test]
fn a_scene_goto_button_jumps_closes_the_screens_and_rolls_back() {
    let epilogue_line: orbit_core::NodeId = "01JBX0N0000000000000000021".parse().unwrap();
    let mut before = None;
    let (mut sh, _) = press("SceneGoto(\"epilogue\")", |sh| {
        in_game(sh);
        before = shown_node(sh);
    });
    assert!(before.is_some());
    assert_eq!(sh.top(), None, "the screens are closed");
    assert_eq!(shown_node(&sh), Some(epilogue_line));
    // Saved after the jump, rolled back, then loaded: back after the jump.
    sh.apply(UiEvent::SaveSlot(slot(0)));
    sh.update(5.0);
    sh.apply(UiEvent::Rollback);
    assert_eq!(shown_node(&sh), before, "rollback returns before the jump");
    sh.apply(UiEvent::LoadSlot(slot(0)));
    sh.update(0.02);
    assert_eq!(shown_node(&sh), Some(epilogue_line));
    sh.apply(UiEvent::Rollback);
    assert_eq!(shown_node(&sh), before, "the history survives the load");

    // By id; unknown scenes change nothing.
    sh.apply(UiEvent::SceneGoto("01JBX0S0000000000000000002".into()));
    assert_eq!(shown_node(&sh), Some(epilogue_line));
    sh.apply(UiEvent::SceneGoto("nowhere".into()));
    assert_eq!(shown_node(&sh), Some(epilogue_line));
}

#[test]
fn a_scene_goto_at_the_title_is_ignored() {
    let (sh, _) = press("SceneGoto(\"epilogue\")", |_| {});
    assert!(!sh.has_game());
}

// ------------------------------------------------ accessibility (M7-07)

#[test]
fn accessibility_defaults_leave_the_theme_untouched() {
    let sh = shell();
    assert_eq!(sh.theme, Theme::default());
    assert_eq!(sh.prefs().text_scale, 1.0);
    assert!(!sh.prefs().high_contrast && !sh.prefs().reduced_motion);
    assert!(!sh.prefs_view().high_contrast);
}

#[test]
fn text_scale_and_high_contrast_re_skin_the_game_and_persist() {
    let storage = MemoryStorage::new();
    let mut sh = shell_with(&storage, ShellConfig::default());
    sh.apply(UiEvent::StartGame);
    sh.apply(UiEvent::SetTextScale(1.5));
    assert_eq!(sh.theme.textbox.text_size, Theme::default().textbox.text_size * 1.5);
    assert!(sh.theme.textbox.rect[3] > Theme::default().textbox.rect[3]);
    sh.apply(UiEvent::SetHighContrast(true));
    assert_eq!(sh.theme.textbox.background, [0, 0, 0, 255]);
    // The scale survives the contrast switch, and out-of-range values clamp.
    assert_eq!(sh.theme.textbox.text_size, Theme::default().textbox.text_size * 1.5);
    sh.apply(UiEvent::SetTextScale(9.0));
    assert_eq!(sh.prefs().text_scale, 1.6);
    sh.update(PREFS_SAVE_DELAY + 0.1);

    // A new session reads them back and starts skinned.
    let again = shell_with(&storage, ShellConfig::default());
    assert_eq!(again.prefs().text_scale, 1.6);
    assert!(again.prefs().high_contrast);
    assert_eq!(again.theme.textbox.background, [0, 0, 0, 255]);
    assert_eq!(again.base_theme, Theme::default());

    // Defaults bring the project theme back.
    sh.apply(UiEvent::ResetPreferences);
    assert_eq!(sh.theme, Theme::default());
}

#[test]
fn old_preferences_files_keep_todays_look() {
    let storage = MemoryStorage::new();
    storage.put_raw(
        "prefs.ron",
        b"(volumes: (master: 0.5, music: 0.8, sfx: 1.0, voice: 1.0), text_speed: 40.0, \
          auto_delay: 2.0, skip_mode: ReadOnly, fullscreen: false, language: None)"
            .to_vec(),
    );
    let sh = shell_with(&storage, ShellConfig::default());
    assert_eq!(sh.theme, Theme::default());
    assert!(!sh.prefs().reduced_motion);
}

#[test]
fn reduced_motion_cuts_the_running_transition_and_reaches_new_games() {
    let mut sh = shell();
    sh.apply(UiEvent::StartGame);
    assert_eq!(kind(&sh), FlowKind::Transition, "the demo opens with a fade");
    sh.apply(UiEvent::SetReducedMotion(true));
    assert_ne!(kind(&sh), FlowKind::Transition);
    assert!(sh.game().unwrap().director.transition().is_none());

    let mut sh = shell();
    sh.apply(UiEvent::SetReducedMotion(true));
    sh.apply(UiEvent::StartGame);
    assert_ne!(kind(&sh), FlowKind::Transition);
}

#[test]
fn the_data_preferences_screen_follows_the_accessibility_settings() {
    let mut sh = with_bundle(&UiBundle::builtin());
    sh.apply(UiEvent::OpenPreferences);
    assert!(matches!(sh.top(), Some(Overlay::Custom(_))));
    // The data screen is rebuilt in high contrast without losing its place.
    sh.apply(UiEvent::SetHighContrast(true));
    assert!(sh.prefs().high_contrast);
    assert!(matches!(sh.top(), Some(Overlay::Custom(_))));
    sh.apply(UiEvent::SetTextScale(1.6));
    assert_eq!(sh.theme.widgets.text_size, Theme::default().widgets.text_size * 1.6);
    sh.apply(UiEvent::SetScreenReader(false));
    assert!(!sh.prefs().screen_reader);
}
