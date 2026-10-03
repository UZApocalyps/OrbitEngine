//! Gallery unlocks and scene replay (M7-03, M7-04): what a playthrough
//! unlocks, the data the gallery / replay screens read, and a replay that
//! runs on its own variables, saves nothing and comes back to its menu.

use orbit_core::SceneId;

use super::*;

const PROLOGUE: &str = "01JBX0S0000000000000000001";
const EPILOGUE: &str = "01JBX0S0000000000000000002";

fn scene(id: &str) -> SceneId {
    id.parse().unwrap()
}

/// The stock UI (with the gallery and replay screens) on `storage`.
fn stock_shell(storage: &MemoryStorage) -> Shell {
    let mut sh = shell_with(storage, ShellConfig::default());
    sh.set_ui_bundle(&UiBundle::builtin());
    sh
}

/// A whole playthrough, back at the title.
fn played(storage: &MemoryStorage) -> Shell {
    let mut sh = stock_shell(storage);
    sh.apply(UiEvent::StartGame);
    play_to_end(&mut sh);
    sh.update(0.1);
    sh.apply(UiEvent::ReturnToMainMenu);
    assert!(!sh.has_game());
    sh
}

/// Steps a replay until it gives the hand back.
fn finish_replay(sh: &mut Shell) {
    for _ in 0..2000 {
        if !sh.replaying() {
            return;
        }
        // Like `step`, but the game may be gone after any input.
        match kind(sh) {
            FlowKind::Choice => sh.input(InputAction::Number(1)),
            FlowKind::Say | FlowKind::Transition => sh.input(ui(UiInput::Confirm)),
            _ => sh.update(0.5),
        }
        sh.update(0.1);
    }
    panic!("the replay did not end");
}

fn is_top(sh: &Shell, id: &str) -> bool {
    matches!(sh.top(), Some(Overlay::Custom(i)) if sh.data[i].id() == id)
}

#[test]
fn nothing_is_unlocked_before_playing() {
    let sh = stock_shell(&MemoryStorage::new());
    let ctx = sh.data_context();
    assert!(!ctx.gallery.is_empty(), "the story shows backgrounds");
    assert!(ctx.gallery_rows(&[]).iter().all(|v| *v == Value::from("")));
    assert!(ctx.replay_rows(&[]).is_empty());
    assert_eq!(ctx.scenes.len(), 2);
}

#[test]
fn a_playthrough_unlocks_its_backgrounds_and_scenes_for_good() {
    let storage = MemoryStorage::new();
    let sh = played(&storage);
    let stored = sh.saves().load_persistent().unwrap();
    assert!(stored.has_seen(scene(PROLOGUE)) && stored.has_seen(scene(EPILOGUE)));
    let bg = &gallery_paths(&sh.story)[0];
    assert!(stored.is_unlocked(bg));
    // The screens read it, here and in a new process.
    for sh in [sh, stock_shell(&storage)] {
        let ctx = sh.data_context();
        assert_eq!(ctx.gallery_rows(&[])[0], Value::from(bg.as_str()));
        assert_eq!(
            ctx.replay_rows(&[]),
            vec![Value::from("prologue"), Value::from("epilogue")]
        );
        // An explicit list keeps only its (seen) entries.
        assert_eq!(ctx.replay_rows(&[EPILOGUE.into()]), vec![Value::from("epilogue")]);
        assert_eq!(ctx.gallery_rows(&["bg/never.png".into()]), vec![Value::from("")]);
    }
}

#[test]
fn a_replay_runs_on_isolated_variables_and_writes_nothing() {
    let storage = MemoryStorage::new();
    let mut sh = played(&storage);
    let before = sh.saves().load_persistent().unwrap();
    let latest = sh.saves().latest().unwrap().map(|(slot, _)| slot);
    let size = storage.total_size();
    assert_eq!(before.vars.get("plays"), Some(&Value::Int(1)));

    sh.apply(UiEvent::OpenScreen(REPLAY_SCREEN.into()));
    sh.apply(UiEvent::ReplayScene("epilogue".into()));
    assert!(sh.replaying());
    assert_eq!(sh.top(), None, "the replay plays at once");
    let vm = sh.game().unwrap().director.vm();
    // The variables of the first visit.
    assert_eq!(
        vm.variable("credits"),
        before.seen_scenes[&scene(EPILOGUE)].get("credits")
    );
    assert_eq!(vm.variable("plays"), Some(&Value::Int(1)));
    // No saves of any kind.
    assert!(!sh.can_save());
    sh.apply(UiEvent::QuickSave);
    sh.apply(UiEvent::SaveSlot(slot(5)));
    sh.update(5.0);
    sh.apply(UiEvent::VarOp(VarOp::new(
        "plays",
        orbit_core::OpKind::Add,
        orbit_core::Operand::Lit(Value::Int(10)),
    )));
    finish_replay(&mut sh);
    sh.update(0.1);

    // Back at the title, on the replay menu.
    assert!(!sh.has_game());
    assert!(is_top(&sh, REPLAY_SCREEN));
    // The epilogue's `plays + 1` and the button's +10 stayed in the replay.
    assert_eq!(sh.saves().load_persistent().unwrap(), before);
    assert_eq!(sh.persistent, before);
    assert_eq!(sh.saves().latest().unwrap().map(|(slot, _)| slot), latest);
    assert_eq!(storage.total_size(), size, "nothing was written");
    sh.on_exit();
    assert_eq!(sh.saves().load_persistent().unwrap().vars, before.vars);
}

#[test]
fn leaving_a_replay_by_the_menu_comes_back_to_the_replay_screen() {
    let storage = MemoryStorage::new();
    let mut sh = played(&storage);
    let before = sh.saves().load_persistent().unwrap();
    assert!(sh.start_replay(PROLOGUE));
    play_lines(&mut sh, 1);
    sh.apply(UiEvent::ReturnToMainMenu);
    assert!(!sh.has_game());
    assert!(is_top(&sh, REPLAY_SCREEN));
    assert_eq!(sh.saves().load_persistent().unwrap(), before);
    // The prologue replay ends when the story jumps to the epilogue.
    assert!(sh.start_replay(PROLOGUE));
    finish_replay(&mut sh);
    assert!(is_top(&sh, REPLAY_SCREEN));
}

#[test]
fn unseen_scenes_and_running_games_refuse_a_replay() {
    let mut sh = stock_shell(&MemoryStorage::new());
    assert!(!sh.start_replay("epilogue"), "never seen");
    assert!(!sh.start_replay("nowhere"));
    sh.apply(UiEvent::StartGame);
    play_lines(&mut sh, 1);
    // The prologue is seen now, but a replay would end the playthrough.
    assert!(!sh.start_replay("prologue"));
    assert!(sh.has_game() && !sh.replaying());
    // The running game counts as seen for the menus.
    assert_eq!(sh.data_context().replay_rows(&[]), vec![Value::from("prologue")]);
}

#[test]
fn the_stock_gallery_and_replay_screens_list_the_unlocks() {
    let storage = MemoryStorage::new();
    let mut sh = played(&storage);
    sh.apply(UiEvent::OpenScreen("replay".into()));
    assert!(is_top(&sh, "replay"));
    // The first row button replays the prologue.
    sh.input(ui(UiInput::Confirm));
    assert!(sh.replaying());
    assert_eq!(
        sh.game().unwrap().director.vm().replay_scene(),
        Some(scene(PROLOGUE))
    );
    sh.apply(UiEvent::ReturnToMainMenu);
    sh.apply(UiEvent::Back);
    sh.apply(UiEvent::OpenScreen("gallery".into()));
    assert!(is_top(&sh, "gallery"));
}
