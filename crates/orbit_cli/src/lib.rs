//! The `orbit` command line.
//!
//! * `orbit validate <project|file> [--check-files] [--root <dir>]`
//! * `orbit run-text <file> [--step]`
//! * `orbit mcp` (placeholder until M3)
//!
//! The logic lives in [`run`], which takes its streams as parameters so it can
//! be tested without spawning a process.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use orbit_core::{CoreError, Story, TransitionKind, ValidationReport, migrate};
use orbit_vm::{PresentationEvent, StepResult, Vm};

const USAGE: &str = "\
orbit - OrbitEngine command line

USAGE:
    orbit validate <project|file> [--check-files] [--root <dir>]
        Validate a story (a .ron file, or a project directory containing
        story.ron). Exit code is non-zero when errors are found.
        --check-files   also check that every manifest asset exists on disk
        --root <dir>    directory the manifest paths are relative to
                        (default: the project directory, else the current one)
    orbit run-text <file> [--step]
        Play a story in the terminal. Choices are read from stdin.
        --step          wait for Enter after each line
    orbit mcp
        Placeholder (not implemented, planned for M3).
    orbit help
";

/// Runs the CLI and returns the process exit code.
pub fn run(
    args: &[String],
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let result = match args.first().map(String::as_str) {
        Some("validate") => validate_cmd(&args[1..], out, err),
        Some("run-text") => run_text_cmd(&args[1..], input, out, err),
        Some("mcp") => {
            let _ = writeln!(err, "orbit mcp: not implemented (M3)");
            Ok(1)
        }
        Some("help" | "--help" | "-h") | None => {
            let _ = write!(out, "{USAGE}");
            Ok(0)
        }
        Some(other) => Err(format!("unknown command `{other}` (try `orbit help`)")),
    };
    match result {
        Ok(code) => code,
        Err(message) => {
            let _ = writeln!(err, "error: {message}");
            2
        }
    }
}

// ---------------------------------------------------------------- loading

/// A `.ron` file, or a directory holding `story.ron` (or a single `.ron`).
fn resolve_story_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Ok(path.to_owned());
    }
    let default = path.join("story.ron");
    if default.is_file() {
        return Ok(default);
    }
    let mut ron_files: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ron"))
        .collect();
    match ron_files.len() {
        1 => Ok(ron_files.remove(0)),
        0 => Err(format!("no story.ron found in {}", path.display())),
        _ => Err(format!(
            "several .ron files in {}: pass the story file explicitly",
            path.display()
        )),
    }
}

fn read_story_source(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Parses (and migrates) without validating. Returns the story and the schema
/// version the file was written with.
fn parse_story(source: &str, path: &Path) -> Result<(Story, u32), String> {
    let describe = |e: CoreError| format!("{}: {e}", path.display());
    let version = migrate::read_version(source).map_err(describe)?;
    let story = Story::from_ron_unchecked(source).map_err(describe)?;
    Ok((story, version))
}

fn print_report(report: &ValidationReport, err: &mut dyn Write) {
    for e in &report.errors {
        let _ = writeln!(err, "error: {e}");
    }
    for w in &report.warnings {
        let _ = writeln!(err, "warning: {w}");
    }
}

// --------------------------------------------------------------- validate

fn validate_cmd(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<u8, String> {
    let mut target: Option<&str> = None;
    let mut check_files = false;
    let mut root: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--check-files" => check_files = true,
            "--root" => {
                root = Some(PathBuf::from(it.next().ok_or("--root needs a directory")?));
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            path if target.is_none() => target = Some(path),
            _ => return Err("validate takes a single path".into()),
        }
    }
    let target = Path::new(target.ok_or("usage: orbit validate <project|file>")?);
    let story_path = resolve_story_path(target)?;
    let source = read_story_source(&story_path)?;

    let (story, version) = match parse_story(&source, &story_path) {
        Ok(parsed) => parsed,
        Err(message) => {
            let _ = writeln!(err, "error: {message}");
            return Ok(1);
        }
    };
    if version < story.schema_version {
        let _ = writeln!(
            err,
            "note: schema version {version} migrated in memory to {}",
            story.schema_version
        );
    }

    let mut report = orbit_core::validate(&story);
    if check_files {
        let root = root.unwrap_or_else(|| {
            if target.is_dir() {
                target.to_owned()
            } else {
                PathBuf::from(".")
            }
        });
        report
            .errors
            .extend(orbit_core::check_asset_files(&story, |p| {
                root.join(p).is_file()
            }));
    }
    print_report(&report, err);
    let _ = writeln!(
        out,
        "{}: {} error(s), {} warning(s)",
        story_path.display(),
        report.errors.len(),
        report.warnings.len()
    );
    Ok(if report.errors.is_empty() { 0 } else { 1 })
}

// --------------------------------------------------------------- run-text

fn run_text_cmd(
    args: &[String],
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, String> {
    let mut target: Option<&str> = None;
    let mut step = false;
    for a in args {
        match a.as_str() {
            "--step" => step = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            path if target.is_none() => target = Some(path),
            _ => return Err("run-text takes a single file".into()),
        }
    }
    let target = Path::new(target.ok_or("usage: orbit run-text <file>")?);
    let story_path = resolve_story_path(target)?;
    let source = read_story_source(&story_path)?;
    let (story, _) = parse_story(&source, &story_path)?;

    let report = orbit_core::validate(&story);
    print_report(&report, err);
    if !report.errors.is_empty() {
        let _ = writeln!(err, "error: story is invalid, not playing it");
        return Ok(1);
    }
    play(story, step, input, out, err).map_err(|e| format!("I/O error: {e}"))
}

fn play(
    story: Story,
    step: bool,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> std::io::Result<u8> {
    let mut vm = match Vm::from_start(story) {
        Ok(vm) => vm,
        Err(e) => {
            writeln!(err, "error: {e}")?;
            return Ok(1);
        }
    };
    let mut line = String::new();
    loop {
        let result = vm.step();
        for event in vm.take_events() {
            writeln!(out, "  {}", describe_event(vm.story(), &event))?;
        }
        match result {
            StepResult::Say {
                speaker, string_id, ..
            } => {
                let text = vm.text(string_id).unwrap_or("<missing text>");
                match speaker.and_then(|c| vm.character_name(c)) {
                    Some(name) => writeln!(out, "{name}: {text}")?,
                    None => writeln!(out, "{text}")?,
                }
                if step {
                    line.clear();
                    if input.read_line(&mut line)? == 0 {
                        writeln!(err, "error: input ended")?;
                        return Ok(1);
                    }
                }
            }
            StepResult::Choice { options, .. } => {
                for (i, option) in options.iter().enumerate() {
                    let text = vm.text(option.string_id).unwrap_or("<missing text>");
                    writeln!(out, "  {}) {text}", i + 1)?;
                }
                loop {
                    write!(out, "> ")?;
                    out.flush()?;
                    line.clear();
                    if input.read_line(&mut line)? == 0 {
                        writeln!(out)?;
                        writeln!(err, "error: input ended while waiting for a choice")?;
                        return Ok(1);
                    }
                    let picked = line
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .filter(|n| (1..=options.len()).contains(n));
                    match picked {
                        Some(n) => {
                            if let Err(e) = vm.choose(n - 1) {
                                writeln!(err, "error: {e}")?;
                                return Ok(1);
                            }
                            let chosen = vm.text(options[n - 1].string_id).unwrap_or("?");
                            writeln!(out, "  -> {chosen}")?;
                            break;
                        }
                        None => {
                            writeln!(out, "please enter a number between 1 and {}", options.len())?
                        }
                    }
                }
            }
            StepResult::WaitTime(seconds) => writeln!(out, "  [wait {seconds}s]")?,
            StepResult::WaitTransition { duration } => {
                writeln!(out, "  [transition {duration}s]")?;
            }
            StepResult::End => {
                writeln!(out, "[end]")?;
                return Ok(0);
            }
            StepResult::Error(e) => {
                writeln!(err, "error: {e}")?;
                return Ok(1);
            }
        }
    }
}

fn describe_transition(t: &Option<orbit_core::Transition>) -> String {
    match t {
        None => String::new(),
        Some(t) => match t.kind {
            TransitionKind::Fade(c) => {
                format!(
                    " with fade to #{:02x}{:02x}{:02x} {}s",
                    c.r, c.g, c.b, t.duration
                )
            }
            TransitionKind::Dissolve => format!(" with dissolve {}s", t.duration),
        },
    }
}

fn describe_event(story: &Story, event: &PresentationEvent) -> String {
    let asset = |id| story.asset_path(id).unwrap_or("<unknown asset>").to_owned();
    let name = |id| {
        story
            .character(id)
            .and_then(|c| story.text(c.name_id))
            .unwrap_or("<unknown character>")
            .to_owned()
    };
    match event {
        PresentationEvent::Background {
            asset: a,
            transition,
        } => {
            format!(
                "[background {}{}]",
                asset(*a),
                describe_transition(transition)
            )
        }
        PresentationEvent::Show {
            character,
            expression,
            position,
            x,
            y,
            transition,
            ..
        } => format!(
            "[show {} ({expression}) at {position:?} ({x}, {y}){}]",
            name(*character),
            describe_transition(transition)
        ),
        PresentationEvent::Hide {
            character,
            transition,
        } => format!(
            "[hide {}{}]",
            name(*character),
            describe_transition(transition)
        ),
        PresentationEvent::PlayMusic {
            asset: a,
            fade_in,
            looping,
        } => format!(
            "[music {} fade-in {fade_in}s{}]",
            asset(*a),
            if *looping { ", looping" } else { "" }
        ),
        PresentationEvent::StopMusic { fade_out } => format!("[music stop fade-out {fade_out}s]"),
        PresentationEvent::PlaySound { asset: a } => format!("[sound {}]", asset(*a)),
    }
}
