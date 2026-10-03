//! The `orbit` command line.
//!
//! * `orbit validate <project|file> [--check-files] [--root <dir>]`
//! * `orbit run-text <file> [--step] [--save-dir <dir>]`
//! * `orbit mcp --project <dir|file.orbit>` (MCP server over stdio, see `orbit_mcp`; `--force` takes a project the editor holds)
//! * `orbit i18n export|import|stale|accept|stamp` (PO/CSV, stale translations)
//! * `orbit build` and `orbit export <target>` (see `orbit_export`)
//!
//! The logic lives in [`run`], which takes its streams as parameters so it can
//! be tested without spawning a process.

mod export_cmd;
mod i18n_cmd;

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use orbit_core::{CoreError, Story, TransitionKind, ValidationReport, migrate};
use orbit_save::{
    CaptureInfo, Compat, DEFAULT_SAVED_HISTORY, FileStorage, MemoryStorage, PersistentData,
    SaveConfig, SaveData, SaveManager, SlotId, Storage,
};
use orbit_vm::{Interaction, Presentation, PresentationEvent, StepResult, Vm};

const USAGE: &str = "\
orbit - OrbitEngine command line

USAGE:
    orbit validate <project|file> [--check-files] [--root <dir>]
        Validate a story (a .orbit or .ron file, or a project directory containing
        story.orbit or story.ron). Exit code is non-zero when errors are found.
        --check-files   also check that every manifest asset exists on disk
        --root <dir>    directory the manifest paths are relative to
                        (default: the project directory, else the current one)
    orbit run-text <file> [--step] [--save-dir <dir>]
        Play a story in the terminal. Choices are read from stdin.
        --step          wait for Enter after each line
        --save-dir <d>  keep saves and persistent data in <d> (default: in memory)
        At a choice prompt, or after a line with --step, you can also type:
          b [n]   roll back n steps (default 1)     f [n]  roll forward
          s <slot>  save      l <slot>  load        h      show the history
        <slot> is a number (manual slot), q (quicksave) or a (last autosave).
    orbit mcp --project <dir|file.orbit> [--force]
        Serve the project to an AI client over the Model Context Protocol
        (stdio, no GPU). Every edit goes through the project command API, is
        saved atomically and is one undo entry; file access is limited to
        the project folder. The log is kept in <project>/.orbit/mcp.log.
        Protocol on stdout: nothing else is printed there.
        Refused while the editor (or another `orbit mcp`) holds the project
        (.orbit/lock): use the editor's live MCP instead, or --force.
{export_usage}{i18n_usage}    orbit help
";

/// The help text (the export commands are documented next to their code).
fn usage() -> String {
    USAGE
        .replace("{export_usage}", export_cmd::USAGE)
        .replace("{i18n_usage}", i18n_cmd::USAGE)
}

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
        Some("mcp") => mcp_cmd(&args[1..], err),
        Some("build") => export_cmd::build_cmd(&args[1..], out, err),
        Some("export") => export_cmd::export_cmd(&args[1..], out, err),
        Some("i18n") => i18n_cmd::i18n_cmd(&args[1..], out, err),
        Some("help" | "--help" | "-h") | None => {
            let _ = write!(out, "{}", usage());
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
    if let Some(default) = orbit_project::default_story_in(|f| path.join(f).is_file()) {
        return Ok(path.join(default));
    }
    // `.orbit` files take precedence over legacy `.ron` ones (themes, i18n
    // tables... are `.ron` too).
    let files_with = |ext: &str| -> Result<Vec<PathBuf>, String> {
        Ok(std::fs::read_dir(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == ext))
            .collect())
    };
    for ext in [orbit_project::STORY_EXT, orbit_project::LEGACY_STORY_EXT] {
        let mut found = files_with(ext)?;
        match found.len() {
            0 => {}
            1 => return Ok(found.remove(0)),
            _ => {
                return Err(format!(
                    "several .{ext} files in {}: pass the story file explicitly",
                    path.display()
                ));
            }
        }
    }
    Err(format!("no story.orbit found in {}", path.display()))
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

// -------------------------------------------------------------------- mcp

/// `orbit mcp --project <dir|file>`: blocks serving MCP on the process's own
/// stdin / stdout (the `input` / `out` streams of [`run`] are not used: stdout
/// belongs to the protocol).
fn mcp_cmd(args: &[String], err: &mut dyn Write) -> Result<u8, String> {
    let mut project: Option<PathBuf> = None;
    let mut force = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--project" => {
                project = Some(PathBuf::from(it.next().ok_or("--project needs a path")?));
            }
            "--force" => force = true,
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            _ => return Err("usage: orbit mcp --project <dir|file.orbit> [--force]".into()),
        }
    }
    let project = project.ok_or("usage: orbit mcp --project <dir|file.orbit> [--force]")?;
    let _ = writeln!(err, "orbit mcp: serving {} on stdio", project.display());
    orbit_mcp::serve_stdio_with(&project, force)?;
    Ok(0)
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
        // Same resolution as the editor and the MCP server: a story file
        // opened alone may keep its assets in a parent folder.
        let root = root.unwrap_or_else(|| {
            let base = if target.is_dir() {
                target
            } else {
                story_path.parent().unwrap_or(Path::new(""))
            };
            orbit_project::find_assets_root(base, &story)
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
    let mut save_dir: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--step" => step = true,
            "--save-dir" => {
                save_dir = Some(PathBuf::from(
                    it.next().ok_or("--save-dir needs a directory")?,
                ));
            }
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
    let storage: Box<dyn Storage> = match save_dir {
        Some(dir) => Box::new(FileStorage::new(dir)),
        None => Box::new(MemoryStorage::new()),
    };
    play(story, step, storage, input, out, err).map_err(|e| format!("I/O error: {e}"))
}

/// What the player typed at a prompt.
#[derive(Debug, PartialEq)]
enum Command {
    Back(usize),
    Forward(usize),
    Save(SlotId),
    Load(SlotId),
    History,
    Other,
}

/// `b [n]`, `f [n]`, `s <slot>`, `l <slot>`, `h`. A slot is a number
/// (manual), `q` (quicksave) or `a` (newest autosave).
fn parse_command(line: &str) -> Command {
    let mut words = line.split_whitespace();
    let (Some(cmd), arg) = (words.next(), words.next()) else {
        return Command::Other;
    };
    if words.next().is_some() {
        return Command::Other;
    }
    let count = || match arg {
        None => Some(1),
        Some(n) => n.parse::<usize>().ok().filter(|n| *n > 0),
    };
    let slot = || match arg? {
        "q" => Some(SlotId::Quick),
        "a" => Some(SlotId::Auto(0)),
        n => n.parse().ok().map(SlotId::Manual),
    };
    match cmd {
        "b" => count().map_or(Command::Other, Command::Back),
        "f" => count().map_or(Command::Other, Command::Forward),
        "s" => slot().map_or(Command::Other, Command::Save),
        "l" => slot().map_or(Command::Other, Command::Load),
        "h" if arg.is_none() => Command::History,
        _ => Command::Other,
    }
}

fn play(
    story: Story,
    step: bool,
    storage: Box<dyn Storage>,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> std::io::Result<u8> {
    let story = Arc::new(story);
    let mut vm = match Vm::from_start(Arc::clone(&story)) {
        Ok(vm) => vm,
        Err(e) => {
            writeln!(err, "error: {e}")?;
            return Ok(1);
        }
    };
    let saves = SaveManager::new(storage, Arc::clone(&story), SaveConfig::default());
    match saves.load_persistent() {
        Ok(data) => data.apply_to(&mut vm),
        Err(e) => writeln!(err, "warning: persistent data ignored: {e}")?,
    }
    let started_ms = now_ms();
    let mut line = String::new();
    // Set by rollback / load: show this again instead of stepping.
    let mut redisplay: Option<Interaction> = None;

    'outer: loop {
        let replayed = redisplay.is_some();
        let interaction = match redisplay.take() {
            Some(i) => i,
            None => {
                let result = vm.step();
                for event in vm.take_events() {
                    writeln!(out, "  {}", describe_event(vm.story(), &event))?;
                }
                match result {
                    StepResult::Say {
                        node,
                        speaker,
                        string_id,
                        ..
                    } => Interaction::Say {
                        node,
                        speaker,
                        string_id,
                    },
                    StepResult::Choice { node, options } => Interaction::Choice { node, options },
                    StepResult::WaitTime(seconds) => {
                        writeln!(out, "  [wait {seconds}s]")?;
                        continue;
                    }
                    StepResult::WaitTransition { duration } => {
                        writeln!(out, "  [transition {duration}s]")?;
                        continue;
                    }
                    StepResult::WaitAnimation { node, duration } => {
                        writeln!(out, "  [animation {duration}s]")?;
                        // No clock runs in the terminal: play it to its end.
                        vm.complete_anim(node);
                        continue;
                    }
                    StepResult::Screen { screen, .. } => {
                        // The terminal has no UI: the screen closes at once, with no value.
                        writeln!(out, "  [screen \"{screen}\": opened and closed]")?;
                        if let Err(e) = vm.close_screen(&screen, None) {
                            writeln!(err, "warning: cannot close screen: {e}")?;
                        }
                        continue;
                    }
                    StepResult::End => {
                        writeln!(out, "[end]")?;
                        let data = PersistentData::from_vm(&vm);
                        if let Err(e) = saves.save_persistent(&data) {
                            writeln!(err, "warning: cannot save persistent data: {e}")?;
                        }
                        return Ok(0);
                    }
                    StepResult::Error(e) => {
                        writeln!(err, "error: {e}")?;
                        return Ok(1);
                    }
                }
            }
        };

        let is_choice = matches!(interaction, Interaction::Choice { .. });
        match &interaction {
            Interaction::Say {
                speaker, string_id, ..
            } => {
                let text = vm.text(*string_id).unwrap_or("<missing text>");
                match speaker.and_then(|c| vm.character_name(c)) {
                    Some(name) => writeln!(out, "{name}: {text}")?,
                    None => writeln!(out, "{text}")?,
                }
                // A line shown again by a rollback or load always waits.
                if !step && !replayed {
                    continue;
                }
            }
            Interaction::Choice { options, .. } => {
                for (i, option) in options.iter().enumerate() {
                    let text = vm.text(option.string_id).unwrap_or("<missing text>");
                    writeln!(out, "  {}) {text}", i + 1)?;
                }
            }
            Interaction::Screen { screen, .. } => {
                // Shown again by a rollback or load: close it like the first time.
                writeln!(out, "  [screen \"{screen}\": opened and closed]")?;
                if let Err(e) = vm.close_screen(screen, None) {
                    writeln!(err, "warning: cannot close screen: {e}")?;
                }
                continue;
            }
        }

        loop {
            if is_choice {
                write!(out, "> ")?;
                out.flush()?;
            }
            line.clear();
            if input.read_line(&mut line)? == 0 {
                if is_choice {
                    writeln!(out)?;
                    writeln!(err, "error: input ended while waiting for a choice")?;
                } else {
                    writeln!(err, "error: input ended")?;
                }
                return Ok(1);
            }
            let text = line.trim();

            match parse_command(text) {
                Command::Back(n) => {
                    match vm.rollback(n) {
                        Some(p) => {
                            show_presentation(&vm, &p, "rollback", out)?;
                            redisplay = p.interaction;
                            continue 'outer;
                        }
                        None => writeln!(out, "  [nothing to roll back]")?,
                    }
                    continue;
                }
                Command::Forward(n) => {
                    match vm.roll_forward(n) {
                        Some(p) => {
                            show_presentation(&vm, &p, "roll forward", out)?;
                            redisplay = p.interaction;
                            continue 'outer;
                        }
                        None => writeln!(out, "  [nothing to roll forward]")?,
                    }
                    continue;
                }
                Command::Save(slot) => {
                    let mut info =
                        CaptureInfo::new(now_ms(), now_ms().saturating_sub(started_ms) / 1000);
                    info.max_history = Some(DEFAULT_SAVED_HISTORY);
                    let data = SaveData::capture(&vm, info);
                    match saves.save(slot, &data) {
                        Ok(()) => writeln!(out, "  [saved to {slot}]")?,
                        Err(e) => writeln!(out, "  [save failed: {e}]")?,
                    }
                    continue;
                }
                Command::Load(slot) => {
                    match saves.load(slot) {
                        Ok(loaded) => match loaded.data.apply_to(&mut vm) {
                            Ok(_) => {
                                if let Compat::CompatibleWithWarnings(w) = &loaded.compat {
                                    writeln!(out, "  [warning: {}]", w.join("; "))?;
                                }
                                let p = vm.presentation();
                                show_presentation(&vm, &p, &format!("loaded {slot}"), out)?;
                                redisplay = p.interaction;
                                continue 'outer;
                            }
                            Err(e) => writeln!(out, "  [load failed: {e}]")?,
                        },
                        Err(e) => writeln!(out, "  [load failed: {e}]")?,
                    }
                    continue;
                }
                Command::History => {
                    for item in vm.backlog() {
                        let who = item
                            .speaker
                            .and_then(|c| vm.character_name(c))
                            .unwrap_or("-");
                        let text = vm.text(item.string_id).unwrap_or("?");
                        writeln!(out, "  | {who}: {text}")?;
                    }
                    continue;
                }
                Command::Other => {}
            }

            if is_choice {
                let Interaction::Choice { options, .. } = &interaction else {
                    unreachable!("is_choice")
                };
                let picked = text
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
                        continue 'outer;
                    }
                    None => writeln!(out, "please enter a number between 1 and {}", options.len())?,
                }
            } else {
                // Any other input on a line (including Enter) advances.
                continue 'outer;
            }
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Prints the scene rebuilt after a rollback or a load.
fn show_presentation(
    vm: &Vm,
    p: &Presentation,
    what: &str,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    writeln!(out, "  [{what}]")?;
    for event in &p.events {
        writeln!(out, "  {}", describe_event(vm.story(), event))?;
    }
    Ok(())
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
        PresentationEvent::Scene3d {
            scene,
            camera,
            transition,
        } => format!(
            "[3d scene {}{}{}]",
            asset(*scene),
            camera
                .as_ref()
                .map_or_else(String::new, |c| format!(" camera {c}")),
            describe_transition(transition)
        ),
        PresentationEvent::HideScene3d { transition } => {
            format!("[hide 3d scene{}]", describe_transition(transition))
        }
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
            scale,
            flip_x,
            flip_y,
            transition,
            ..
        } => {
            // Only what differs from the standard look is spelled out.
            let mut look = String::new();
            if *scale != [1.0, 1.0] {
                if scale[0] == scale[1] {
                    look.push_str(&format!(", scale {}", scale[0]));
                } else {
                    look.push_str(&format!(", scale {}x{}", scale[0], scale[1]));
                }
            }
            if *flip_x {
                look.push_str(", mirrored x");
            }
            if *flip_y {
                look.push_str(", mirrored y");
            }
            format!(
                "[show {} ({expression}) at {position:?} ({x}, {y}){look}{}]",
                name(*character),
                describe_transition(transition)
            )
        }
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
        PresentationEvent::Custom {
            plugin_id, kind, ..
        } => format!("[custom {plugin_id}.{kind}]"),
    }
}
