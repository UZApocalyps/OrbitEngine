use std::io::Cursor;
use std::path::{Path, PathBuf};

use orbit_core::Story;

fn demo_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stories/demo.ron")
}

struct Output {
    code: u8,
    out: String,
    err: String,
}

fn orbit(args: &[&str], stdin: &str) -> Output {
    let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let mut input = Cursor::new(stdin.as_bytes().to_vec());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = orbit_cli::run(&args, &mut input, &mut out, &mut err);
    Output {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orbit_cli_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn validate_accepts_the_demo() {
    let demo = demo_path();
    let r = orbit(&["validate", demo.to_str().unwrap()], "");
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("0 error(s), 0 warning(s)"), "{}", r.out);
}

#[test]
fn validate_checks_files_only_on_request() {
    let demo = demo_path();
    let demo = demo.to_str().unwrap();
    let dir = temp_dir("assets");

    // Without --check-files, missing files are not an error.
    assert_eq!(orbit(&["validate", demo], "").code, 0);

    let r = orbit(
        &[
            "validate",
            demo,
            "--check-files",
            "--root",
            dir.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(r.code, 1);
    assert_eq!(r.err.matches("does not exist").count(), 10, "{}", r.err);

    // Create every asset: now it passes.
    let story = Story::from_ron(&std::fs::read_to_string(demo).unwrap()).unwrap();
    for asset in &story.assets.assets {
        let file = dir.join(&asset.path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"x").unwrap();
    }
    let r = orbit(
        &[
            "validate",
            demo,
            "--check-files",
            "--root",
            dir.to_str().unwrap(),
        ],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn validate_reports_errors_and_warnings_for_a_project_directory() {
    let dir = temp_dir("project");
    let mut story = Story::from_ron(&std::fs::read_to_string(demo_path()).unwrap()).unwrap();
    // Break it: a Say pointing to a missing string; plus an undefined variable.
    story.strings.entries.clear();
    std::fs::write(dir.join("story.ron"), story.to_ron().unwrap()).unwrap();
    let r = orbit(&["validate", dir.to_str().unwrap()], "");
    assert_eq!(r.code, 1);
    assert!(r.err.contains("error: "), "{}", r.err);
    assert!(r.err.contains("unknown string"), "{}", r.err);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn validate_reports_unparsable_files_and_bad_usage() {
    let dir = temp_dir("garbage");
    let file = dir.join("bad.ron");
    std::fs::write(&file, "(((").unwrap();
    assert_eq!(orbit(&["validate", file.to_str().unwrap()], "").code, 1);
    assert_eq!(orbit(&["validate"], "").code, 2);
    assert_eq!(orbit(&["validate", "does/not/exist.ron"], "").code, 2);
    assert_eq!(orbit(&["validate", "a", "--wat"], "").code, 2);
    assert_eq!(orbit(&["frobnicate"], "").code, 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn run_text_plays_both_branches() {
    let demo = demo_path();
    let demo = demo.to_str().unwrap();

    // Take the job, buy the chip, break in, hack by listening, sell out to Helix.
    let r = orbit(&["run-text", demo], "1\n1\n1\n1\n1\n1\n3\n1\n");
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.contains("1) Take the job for the standard fee."),
        "{}",
        r.out
    );
    assert!(r.out.contains("ENDING 2"), "{}", r.out);
    assert!(
        r.out.contains("[music assets/demo/music_theme.wav"),
        "{}",
        r.out
    );
    assert!(r.out.contains("with fade to #000000 1s"), "{}", r.out);
    assert!(r.out.contains("with dissolve 0.5s"), "{}", r.out);
    assert!(r.out.trim_end().ends_with("[end]"), "{}", r.out);

    // Refuse the job: the short ending.
    let r = orbit(&["run-text", demo], "4\n");
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("ENDING 1"), "{}", r.out);
    assert!(!r.out.contains("ENDING 2"), "{}", r.out);
}

#[test]
fn run_text_reprompts_on_invalid_input_and_fails_on_eof() {
    let demo = demo_path();
    let demo = demo.to_str().unwrap();

    let r = orbit(&["run-text", demo], "abc\n0\n9\n\n4\n");
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out
            .matches("please enter a number between 1 and 4")
            .count(),
        4
    );

    let r = orbit(&["run-text", demo], "");
    assert_eq!(r.code, 1);
    assert!(r.err.contains("input ended"), "{}", r.err);
}

#[test]
fn run_text_step_waits_for_enter() {
    let demo = demo_path();
    let demo = demo.to_str().unwrap();
    // Without enough Enters the run stops at EOF.
    let r = orbit(&["run-text", demo, "--step"], "\n\n");
    assert_eq!(r.code, 1);
    // One Enter per line, then the choice, then enters for the rest.
    let input = format!("{}4\n{}", "\n".repeat(7), "\n".repeat(40));
    let r = orbit(&["run-text", demo, "--step"], &input);
    assert_eq!(r.code, 0, "{}", r.err);
}

#[test]
fn run_text_refuses_invalid_stories() {
    let dir = temp_dir("invalid");
    let mut story = Story::from_ron(&std::fs::read_to_string(demo_path()).unwrap()).unwrap();
    story.strings.entries.clear();
    let file = dir.join("s.ron");
    std::fs::write(&file, story.to_ron().unwrap()).unwrap();
    let r = orbit(&["run-text", file.to_str().unwrap()], "1\n");
    assert_eq!(r.code, 1);
    assert!(r.err.contains("not playing"), "{}", r.err);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mcp_needs_a_project_and_help_works() {
    let r = orbit(&["mcp"], "");
    assert_eq!(r.code, 2);
    assert!(r.err.contains("--project"));
    let r = orbit(&["mcp", "--project", "/definitely/not/here"], "");
    assert_eq!(r.code, 2, "{}", r.err);
    let r = orbit(&["help"], "");
    assert_eq!(r.code, 0);
    assert!(r.out.contains("orbit validate"));
    assert_eq!(orbit(&[], "").code, 0);
}

#[test]
fn run_text_rollback_replays_the_previous_line() {
    let demo = demo_path();
    let demo = demo.to_str().unwrap();
    // At the choice: roll back one step (re-shows the last line, waits),
    // Enter goes forward to the choice again, then pick option 1.
    let r = orbit(&["run-text", demo], "b\n\n4\n");
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("[rollback]"), "{}", r.out);
    assert_eq!(r.out.matches("A neural chip, prototype").count(), 2);
    assert_eq!(r.out.matches("4) Refuse and go back to sleep.").count(), 2);
    assert!(r.out.trim_end().ends_with("[end]"), "{}", r.out);
    // Nothing before the first line to go back to is reported, not fatal.
    let r = orbit(&["run-text", demo], "b\nb 99\nb\nf 99\n4\n");
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("[rollback]"), "{}", r.out);
}

#[test]
fn run_text_saves_loads_and_keeps_persistent_data() {
    let demo = demo_path();
    let demo = demo.to_str().unwrap();
    let dir = temp_dir("saves");
    let dir_arg = dir.to_str().unwrap();

    // First game: save at the choice, then finish (refusing the job).
    let r = orbit(&["run-text", demo, "--save-dir", dir_arg], "s 1\ns q\n4\n");
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("[saved to manual #1]"), "{}", r.out);
    assert!(r.out.contains("[saved to quicksave]"), "{}", r.out);
    assert!(!r.out.contains("Welcome back"), "{}", r.out);
    assert!(dir.join("saves/manual_001.sav").is_file());
    assert!(dir.join("persistent.ron").is_file());

    // Second game: the persistent counter shows a welcome line, a bad slot is
    // reported, and loading the save returns to the choice (then refuse again).
    let r = orbit(&["run-text", demo, "--save-dir", dir_arg], "l 7\nl 1\n4\n");
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("Welcome back"), "{}", r.out);
    assert!(
        r.out.contains("[load failed: slot manual #7 is empty]"),
        "{}",
        r.out
    );
    assert!(r.out.contains("[loaded manual #1]"), "{}", r.out);
    assert!(r.out.contains("ENDING 1"), "{}", r.out);
    assert!(!r.out.contains("ENDING 2"), "{}", r.out);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_text_without_save_dir_keeps_everything_in_memory() {
    let demo = demo_path();
    let demo = demo.to_str().unwrap();
    let r = orbit(&["run-text", demo], "s 1\nl 1\nh\n4\n");
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("[loaded manual #1]"), "{}", r.out);
    assert!(
        r.out.contains("  | "),
        "history lines are printed: {}",
        r.out
    );
    let r = orbit(&["run-text", demo, "--save-dir"], "");
    assert_eq!(r.code, 2);
}
