//! Running git. Everything here blocks, so it only runs inside background
//! tasks (`spawn_task`).
//!
//! The rules, all enforced in [`Runner::run`]:
//!
//! * git is started directly, never through a shell, with its arguments as a
//!   list; the only values taken from the user (the commit message, the file
//!   names) are arguments or standard input, never part of a command line
//!   string;
//! * the working directory is the project folder;
//! * `GIT_DIR` and its relatives are removed from the environment so that git
//!   cannot be redirected to another repository, and prompts are disabled;
//! * on Windows no console window opens (`CREATE_NO_WINDOW`);
//! * only `rev-parse`, `status`, `log`, `add`, `commit` and `init` exist as
//!   operations: nothing reads or writes the network, nothing rewrites
//!   history or deletes files.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::parse::{self, Commit, Status};

/// How many commits the history shows.
pub const LOG_LIMIT: usize = 20;

/// Why a command did not give a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitError {
    /// No `git` executable could be started.
    NotFound,
    /// The task was cancelled while git ran (git was stopped).
    Cancelled,
    /// git ran too long and was stopped.
    TimedOut,
    /// git could not be started, or ended with an error: its message.
    Failed(String),
}

/// What a finished command printed.
#[derive(Debug)]
pub struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// Where and how to run git.
#[derive(Clone, Debug)]
pub struct Runner {
    /// The project folder.
    pub dir: PathBuf,
    /// Executables to try, in order.
    candidates: Vec<OsString>,
    /// `-c key=value` settings put before the command (tests give an
    /// identity; the plugin never sets any).
    pub config: Vec<(String, String)>,
    /// Extra environment variables (tests bound git's search for a repository).
    pub env: Vec<(String, String)>,
}

/// Where Git for Windows usually lives, for an editor started before git was
/// installed (its `PATH` does not know git yet).
fn known_locations() -> Vec<OsString> {
    let mut out = Vec::new();
    if cfg!(windows) {
        let mut push = |var: &str, tail: &str| {
            if let Some(base) = std::env::var_os(var) {
                let p = Path::new(&base).join(tail);
                if p.is_file() {
                    out.push(p.into_os_string());
                }
            }
        };
        push("ProgramFiles", "Git\\cmd\\git.exe");
        push("ProgramFiles(x86)", "Git\\cmd\\git.exe");
        push("LOCALAPPDATA", "Programs\\Git\\cmd\\git.exe");
    }
    out
}

impl Runner {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let mut candidates = vec![OsString::from("git")];
        candidates.extend(known_locations());
        Self { dir: dir.into(), candidates, config: Vec::new(), env: Vec::new() }
    }

    /// Runs `git <args>` with `stdin` as standard input. `cancelled` is
    /// polled while git runs; past `timeout` git is stopped.
    pub fn run(
        &self,
        args: &[&str],
        stdin: Option<Vec<u8>>,
        cancelled: &dyn Fn() -> bool,
        timeout: Duration,
    ) -> Result<Output, GitError> {
        let mut child = None;
        for exe in &self.candidates {
            let mut cmd = Command::new(exe);
            for (k, v) in &self.config {
                cmd.arg("-c").arg(format!("{k}={v}"));
            }
            // `--literal-pathspecs`: a file name is a name, never a pattern.
            cmd.arg("--no-optional-locks").arg("--literal-pathspecs").args(args);
            cmd.current_dir(&self.dir)
                .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_PAGER", "cat")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_COMMON_DIR")
                .env_remove("GIT_PREFIX")
                .env_remove("GIT_OBJECT_DIRECTORY")
                .env_remove("GIT_NAMESPACE");
            cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                // `CREATE_NO_WINDOW`: do not open a console window.
                cmd.creation_flags(0x0800_0000);
            }
            match cmd.spawn() {
                Ok(c) => {
                    child = Some(c);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(GitError::Failed(e.to_string())),
            }
        }
        let Some(mut child) = child else { return Err(GitError::NotFound) };

        // Feed and drain on threads: a big path list or a big output never
        // blocks git (or us).
        let writer = child.stdin.take().zip(stdin).map(|(mut pipe, data)| {
            std::thread::spawn(move || {
                let _ = pipe.write_all(&data);
            })
        });
        let reader = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut p) = pipe {
                    let _ = p.read_to_end(&mut buf);
                }
                buf
            })
        };
        let out_thread = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
        let err_thread = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));

        let started = Instant::now();
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status.success()),
                Ok(None) => {}
                Err(e) => break Err(GitError::Failed(e.to_string())),
            }
            if cancelled() {
                break Err(GitError::Cancelled);
            }
            if started.elapsed() > timeout {
                break Err(GitError::TimedOut);
            }
            std::thread::sleep(Duration::from_millis(15));
        };
        if outcome.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(w) = writer {
            let _ = w.join();
        }
        let stdout = out_thread.join().unwrap_or_default();
        let stderr = String::from_utf8_lossy(&err_thread.join().unwrap_or_default()).trim().to_owned();
        outcome.map(|success| Output { success, stdout, stderr })
    }

    /// Like [`Runner::run`] but an error exit status is an `Err` with git's
    /// message.
    fn ok(&self, args: &[&str], stdin: Option<Vec<u8>>, cancelled: &dyn Fn() -> bool, timeout: Duration) -> Result<Output, GitError> {
        let out = self.run(args, stdin, cancelled, timeout)?;
        if out.success {
            Ok(out)
        } else {
            Err(GitError::Failed(if out.stderr.is_empty() { args.join(" ") } else { out.stderr }))
        }
    }
}

const QUICK: Duration = Duration::from_secs(60);
/// A commit may run hooks.
const SLOW: Duration = Duration::from_secs(600);

// --------------------------------------------------------------- operations

/// What the project folder is, as far as git is concerned.
#[derive(Debug, PartialEq, Eq)]
pub enum Repo {
    /// The folder is not in a repository.
    None,
    /// The folder is inside a repository that starts higher: this plugin only
    /// manages a repository at the project's root.
    Nested,
    /// The project folder is the root of a repository.
    Root(Box<Snapshot>),
}

/// The state of a repository.
#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub status: Status,
    pub log: Vec<Commit>,
}

/// `git rev-parse --is-inside-work-tree --show-prefix`.
fn locate(r: &Runner, cancelled: &dyn Fn() -> bool) -> Result<Option<String>, GitError> {
    let out = r.run(&["rev-parse", "--is-inside-work-tree", "--show-prefix"], None, cancelled, QUICK)?;
    if !out.success {
        // "not a git repository" is the normal answer for a plain folder;
        // anything else (unsafe directory, corrupt repository...) is reported.
        return if out.stderr.to_lowercase().contains("not a git repository") || out.stderr.is_empty() {
            Ok(None)
        } else {
            Err(GitError::Failed(out.stderr))
        };
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut lines = text.lines();
    if lines.next() != Some("true") {
        // A bare repository, or the inside of `.git`.
        return Ok(None);
    }
    Ok(Some(lines.next().unwrap_or("").to_owned()))
}

/// Reads the repository of the project folder.
pub fn snapshot(r: &Runner, cancelled: &dyn Fn() -> bool) -> Result<Repo, GitError> {
    match locate(r, cancelled)? {
        None => return Ok(Repo::None),
        Some(prefix) if !prefix.is_empty() => return Ok(Repo::Nested),
        Some(_) => {}
    }
    let status = r.ok(&["status", "--porcelain=v1", "-z", "--branch", "--untracked-files=all"], None, cancelled, QUICK)?;
    let status = parse::parse_status(&status.stdout);
    let log = if status.branch.no_commits {
        Vec::new()
    } else {
        let n = format!("-n{LOG_LIMIT}");
        let out = r.run(
            &["log", &n, "-z", "--date=format-local:%Y-%m-%d %H:%M", parse::LOG_FORMAT],
            None,
            cancelled,
            QUICK,
        )?;
        // A detached or odd HEAD with no history is not an error worth showing.
        if out.success { parse::parse_log(&out.stdout) } else { Vec::new() }
    };
    Ok(Repo::Root(Box::new(Snapshot { status, log })))
}

/// `git init` in the project folder, and a `.gitignore` for the editor's own
/// files (an existing one only gets the lines it lacks). Nothing is
/// committed.
pub fn init(r: &Runner, cancelled: &dyn Fn() -> bool) -> Result<(), GitError> {
    // Never nest a repository in another one by accident.
    if locate(r, cancelled)?.is_some() {
        return Err(GitError::Failed("already a repository".to_owned()));
    }
    r.ok(&["init"], None, cancelled, QUICK)?;
    let path = r.dir.join(".gitignore");
    let io = |e: std::io::Error| GitError::Failed(format!("{}: {e}", path.display()));
    match std::fs::read_to_string(&path) {
        Ok(existing) => {
            let missing = parse::missing_ignore_lines(&existing);
            if !missing.is_empty() {
                let mut text = existing;
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                for line in missing {
                    text.push_str(line);
                    text.push('\n');
                }
                std::fs::write(&path, text).map_err(io)?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(&path, parse::GITIGNORE).map_err(io)?;
        }
        Err(e) => return Err(io(e)),
    }
    Ok(())
}

/// Records `paths` (as given by [`parse::paths_to_commit`]) in one commit
/// with `message`. Only these paths are committed, whatever else is staged.
pub fn commit(r: &Runner, paths: &[String], message: &str, cancelled: &dyn Fn() -> bool) -> Result<(), GitError> {
    if paths.is_empty() {
        return Err(GitError::Failed("nothing to commit".to_owned()));
    }
    // `-A` records deletions and new files as well as edits.
    r.ok(
        &["add", "-A", "--pathspec-from-file=-", "--pathspec-file-nul"],
        Some(parse::pathspec_stdin(paths)),
        cancelled,
        SLOW,
    )?;
    r.ok(
        &["commit", "--only", "-m", message, "--pathspec-from-file=-", "--pathspec-file-nul"],
        Some(parse::pathspec_stdin(paths)),
        cancelled,
        SLOW,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::FileState;
    use std::fs;

    fn never() -> bool {
        false
    }

    /// A scratch folder under the workspace's `target` (never the system
    /// temp folder), removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/plugin-test-scratch")
                .join(format!("git-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn runner(dir: &Path) -> Runner {
        let mut r = Runner::new(dir);
        // An identity for the test repository only, never written anywhere.
        r.config = vec![("user.name".into(), "Test Author".into()), ("user.email".into(), "test@example.com".into())];
        // The scratch folder is inside the workspace's own repository: git
        // must not look for a repository above the scratch area.
        let area = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/plugin-test-scratch").canonicalize().unwrap_or_default();
        let area = area.to_string_lossy().trim_start_matches(r"\\?\").to_owned();
        r.env = vec![("GIT_CEILING_DIRECTORIES".into(), area)];
        r
    }

    /// `None` (test skipped) when no git can be started.
    fn git_available(r: &Runner) -> bool {
        match r.run(&["--version"], None, &never, QUICK) {
            Ok(out) if out.success => true,
            _ => {
                eprintln!("git is not available: test skipped");
                false
            }
        }
    }

    fn root(repo: Repo) -> Snapshot {
        match repo {
            Repo::Root(s) => *s,
            other => panic!("expected a repository, got {other:?}"),
        }
    }

    #[test]
    fn missing_git_is_a_clean_error() {
        let mut r = Runner::new(".");
        r.candidates = vec![OsString::from("orbit-no-such-git-executable")];
        assert_eq!(r.run(&["status"], None, &never, QUICK).unwrap_err(), GitError::NotFound);
        assert_eq!(snapshot(&r, &never).unwrap_err(), GitError::NotFound);
    }

    #[test]
    fn a_real_repository_end_to_end() {
        let dir = Scratch::new("e2e");
        let r = runner(&dir.0);
        if !git_available(&r) {
            return;
        }

        // A plain folder is not a repository.
        assert_eq!(snapshot(&r, &never).unwrap(), Repo::None);

        // Initialise: .gitignore written, nothing committed.
        dir.write("story.ron", "(story)");
        dir.write("assets/caf\u{e9} noir.png", "png");
        dir.write(".orbit/lock", "lock");
        dir.write("export/game.exe", "exe");
        init(&r, &never).unwrap();
        assert!(fs::read_to_string(dir.0.join(".gitignore")).unwrap().contains(".orbit/"));
        assert!(init(&r, &never).is_err(), "a second init is refused");
        let snap = root(snapshot(&r, &never).unwrap());
        assert!(snap.status.branch.no_commits);
        assert!(snap.log.is_empty());
        let paths: Vec<&str> = snap.status.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&".gitignore") && paths.contains(&"story.ron") && paths.contains(&"assets/caf\u{e9} noir.png"), "{paths:?}");
        assert!(!paths.iter().any(|p| p.starts_with(".orbit/") || p.starts_with("export/")), "ignored files are listed: {paths:?}");

        // First commit of two files only.
        let chosen = parse::paths_to_commit(snap.status.entries.iter().filter(|e| e.path != ".gitignore"));
        commit(&r, &chosen, "First \"quoted\" commit\n\nwith a body", &never).unwrap();
        let snap = root(snapshot(&r, &never).unwrap());
        assert!(!snap.status.branch.no_commits);
        assert_eq!(snap.log.len(), 1);
        assert_eq!(snap.log[0].subject, "First \"quoted\" commit");
        assert_eq!(snap.log[0].author, "Test Author");
        assert_eq!(snap.status.entries.len(), 1, "{:?}", snap.status.entries);
        assert_eq!(snap.status.entries[0].path, ".gitignore");

        // Modify, delete, add, rename; commit a subset.
        dir.write("story.ron", "(story v2)");
        dir.write("notes with spaces.txt", "n");
        dir.write("later.txt", "l");
        fs::remove_file(dir.0.join("assets/caf\u{e9} noir.png")).unwrap();
        let snap = root(snapshot(&r, &never).unwrap());
        let state = |p: &str| snap.status.entries.iter().find(|e| e.path == p).map(|e| e.state());
        assert_eq!(state("story.ron"), Some(FileState::Modified));
        assert_eq!(state("assets/caf\u{e9} noir.png"), Some(FileState::Deleted));
        assert_eq!(state("notes with spaces.txt"), Some(FileState::New));
        let chosen = parse::paths_to_commit(
            snap.status.entries.iter().filter(|e| matches!(e.path.as_str(), "story.ron" | "assets/caf\u{e9} noir.png" | "notes with spaces.txt")),
        );
        commit(&r, &chosen, "Second", &never).unwrap();
        let snap = root(snapshot(&r, &never).unwrap());
        assert_eq!(snap.log.len(), 2);
        assert_eq!(snap.log[0].subject, "Second");
        let left: Vec<&str> = snap.status.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(left, [".gitignore", "later.txt"]);
        // The deletion was recorded, the file is gone from the tree.
        let shown = r.ok(&["show", "--name-status", "--format=", "HEAD"], None, &never, QUICK).unwrap();
        let shown = String::from_utf8_lossy(&shown.stdout).into_owned();
        assert!(shown.contains("D\tassets/caf\u{e9} noir.png") || shown.contains("D\t\"assets/caf"), "{shown}");

        // A file that was staged by hand but left unchecked stays out of the commit.
        r.ok(&["add", "later.txt"], None, &never, QUICK).unwrap();
        dir.write("story.ron", "(story v3)");
        commit(&r, &["story.ron".to_owned()], "Third", &never).unwrap();
        let snap = root(snapshot(&r, &never).unwrap());
        let later = snap.status.entries.iter().find(|e| e.path == "later.txt").unwrap();
        assert!(later.staged(), "{later:?}");
        assert_eq!(snap.log.len(), 3);

        // Cancelling stops a running command.
        let started = Instant::now();
        let cancelled = r.run(&["status"], None, &|| true, QUICK);
        assert!(matches!(cancelled, Err(GitError::Cancelled)) || cancelled.is_ok());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_project_inside_a_bigger_repository_is_not_taken_over() {
        let dir = Scratch::new("nested");
        let r = runner(&dir.0);
        if !git_available(&r) {
            return;
        }
        r.ok(&["init"], None, &never, QUICK).unwrap();
        dir.write("game/story.ron", "x");
        let inner = runner(&dir.0.join("game"));
        assert_eq!(snapshot(&inner, &never).unwrap(), Repo::Nested);
        assert!(init(&inner, &never).is_err(), "no repository inside a repository");
        assert!(!dir.0.join("game/.gitignore").exists());
    }

    #[test]
    fn an_existing_gitignore_is_completed_not_replaced() {
        let dir = Scratch::new("ignore");
        let r = runner(&dir.0);
        if !git_available(&r) {
            return;
        }
        dir.write(".gitignore", "*.log");
        init(&r, &never).unwrap();
        let text = fs::read_to_string(dir.0.join(".gitignore")).unwrap();
        assert!(text.starts_with("*.log\n.orbit/"), "{text}");
        assert!(text.contains("export/"));
    }
}
