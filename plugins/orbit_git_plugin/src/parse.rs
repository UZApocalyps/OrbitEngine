//! Reading what git prints: pure functions, unit tested.
//!
//! Only stable, machine formats are read: `git status --porcelain=v1 -z
//! --branch` and `git log -z` with an explicit format.

/// What happened to a file, for display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    New,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    /// Unmerged (a merge or a rebase is in progress).
    Conflict,
}

/// One line of `git status --porcelain=v1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Index column (`' '` when untouched, `'?'` for an untracked file).
    pub x: char,
    /// Work tree column.
    pub y: char,
    /// Path, relative to the repository root, `/` separated.
    pub path: String,
    /// The old path of a rename or a copy.
    pub from: Option<String>,
}

impl Entry {
    pub fn untracked(&self) -> bool {
        self.x == '?'
    }

    /// Both sides are unmerged paths (`UU`, `AA`, `DD`, `DU`...).
    pub fn conflicted(&self) -> bool {
        matches!((self.x, self.y), ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D'))
    }

    pub fn state(&self) -> FileState {
        if self.conflicted() {
            return FileState::Conflict;
        }
        if self.untracked() {
            return FileState::New;
        }
        let has = |c: char| self.x == c || self.y == c;
        if has('D') {
            FileState::Deleted
        } else if has('R') {
            FileState::Renamed
        } else if has('C') {
            FileState::Copied
        } else if has('A') {
            FileState::New
        } else if has('T') {
            FileState::TypeChanged
        } else {
            FileState::Modified
        }
    }

    /// Already (partly) in the index.
    pub fn staged(&self) -> bool {
        !matches!(self.x, ' ' | '?' | '!')
    }
}

/// The branch line of `git status --branch`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Branch {
    /// The branch name; `None` when HEAD is detached.
    pub name: Option<String>,
    /// The branch has no commit yet.
    pub no_commits: bool,
    /// `origin/main`: shown as information, never contacted.
    pub upstream: Option<String>,
}

/// The result of one `git status`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: Branch,
    pub entries: Vec<Entry>,
}

fn parse_branch(header: &str) -> Branch {
    // `## main...origin/main [ahead 1]`, `## main`, `## No commits yet on main`,
    // `## Initial commit on main` (older git), `## HEAD (no branch)`.
    for prefix in ["No commits yet on ", "Initial commit on "] {
        if let Some(name) = header.strip_prefix(prefix) {
            return Branch { name: Some(name.trim().to_owned()), no_commits: true, upstream: None };
        }
    }
    if header.starts_with("HEAD (no branch)") {
        return Branch::default();
    }
    let head = match header.find(" [") {
        Some(at) if header.ends_with(']') => &header[..at],
        _ => header,
    };
    match head.split_once("...") {
        Some((name, upstream)) => {
            Branch { name: Some(name.to_owned()), no_commits: false, upstream: Some(upstream.to_owned()) }
        }
        None => Branch { name: Some(head.to_owned()), no_commits: false, upstream: None },
    }
}

/// Parses the output of `git status --porcelain=v1 -z --branch`: records end
/// with a NUL; a rename or a copy (`R `, `C `) is followed by one more record
/// holding the old path. Paths are raw (no quoting) thanks to `-z`.
pub fn parse_status(out: &[u8]) -> Status {
    let mut status = Status::default();
    let mut records = out.split(|&b| b == 0).map(|r| String::from_utf8_lossy(r).into_owned());
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if let Some(header) = record.strip_prefix("## ") {
            status.branch = parse_branch(header);
            continue;
        }
        let mut chars = record.chars();
        let (Some(x), Some(y), Some(' ')) = (chars.next(), chars.next(), chars.next()) else { continue };
        let path = chars.as_str().to_owned();
        if path.is_empty() {
            continue;
        }
        let from = if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') { records.next() } else { None };
        status.entries.push(Entry { x, y, path, from });
    }
    status
}

/// A commit of the history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub short: String,
    pub author: String,
    /// Already formatted by git (`%Y-%m-%d %H:%M`, local time).
    pub date: String,
    pub subject: String,
}

/// The `--format` of the log read by [`parse_log`]: fields separated by the
/// ASCII unit separator, commits by NUL (`-z`).
pub const LOG_FORMAT: &str = "--format=%h%x1f%an%x1f%ad%x1f%s";

/// Parses `git log -z` with [`LOG_FORMAT`].
pub fn parse_log(out: &[u8]) -> Vec<Commit> {
    String::from_utf8_lossy(out)
        .split('\0')
        .filter_map(|record| {
            let record = record.trim_start_matches('\n');
            let mut f = record.splitn(4, '\u{1f}');
            let (short, author, date, subject) = (f.next()?, f.next()?, f.next()?, f.next()?);
            Some(Commit {
                short: short.to_owned(),
                author: author.to_owned(),
                date: date.to_owned(),
                subject: subject.to_owned(),
            })
        })
        .collect()
}

/// The paths git must be given to record `entries`: a rename or a copy also
/// needs its old path (git then sees the deletion and the new file together),
/// each path once, in order.
pub fn paths_to_commit<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for e in entries {
        for p in std::iter::once(&e.path).chain(e.from.iter()) {
            if seen.insert(p.clone()) {
                out.push(p.clone());
            }
        }
    }
    out
}

/// The paths as git's `--pathspec-from-file=- --pathspec-file-nul` reads them.
pub fn pathspec_stdin(paths: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in paths {
        out.extend_from_slice(p.as_bytes());
        out.push(0);
    }
    out
}

/// What the editor writes in a project folder, and the export folders: the
/// `.gitignore` of a new repository. `.orbit/` holds the editor's state
/// (lock file, recovery copies, thumbnails); `export/` is the export dialog's
/// default output folder (`.orbit-*` are the work folders an export leaves
/// inside its output folder).
pub const GITIGNORE: &str = "\
# OrbitEngine editor state: lock, recovery copies, thumbnails.
.orbit/

# Default output folder of the export dialog.
export/
.orbit-*/
";

/// The lines of [`GITIGNORE`] an existing `.gitignore` does not have (a
/// repository created on a project that already has one keeps it; the
/// missing lines are appended).
pub fn missing_ignore_lines(existing: &str) -> Vec<&'static str> {
    let have: std::collections::HashSet<&str> = existing.lines().map(str::trim).collect();
    GITIGNORE
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !have.contains(l))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(x: char, y: char, path: &str) -> Entry {
        Entry { x, y, path: path.into(), from: None }
    }

    #[test]
    fn status_reads_states_names_with_spaces_and_accents() {
        let out = "## main...origin/main [ahead 2]\0 M story.ron\0?? assets/caf\u{e9} noir.png\0A  new file.txt\0 D gone.txt\0MM both.txt\0";
        let s = parse_status(out.as_bytes());
        assert_eq!(s.branch.name.as_deref(), Some("main"));
        assert_eq!(s.branch.upstream.as_deref(), Some("origin/main"));
        assert!(!s.branch.no_commits);
        assert_eq!(s.entries.len(), 5);
        assert_eq!(s.entries[0], entry(' ', 'M', "story.ron"));
        assert_eq!(s.entries[1].path, "assets/caf\u{e9} noir.png");
        assert_eq!(s.entries[1].state(), FileState::New);
        assert!(s.entries[1].untracked() && !s.entries[1].staged());
        assert_eq!(s.entries[2].state(), FileState::New);
        assert!(s.entries[2].staged());
        assert_eq!(s.entries[3].state(), FileState::Deleted);
        assert_eq!(s.entries[4].state(), FileState::Modified);
    }

    #[test]
    fn renames_carry_their_old_path_and_do_not_swallow_the_next_record() {
        let out = "## main\0R  new name.txt\0old name.txt\0 M other.txt\0C  copy.txt\0orig.txt\0";
        let s = parse_status(out.as_bytes());
        assert_eq!(s.entries.len(), 3);
        assert_eq!(s.entries[0].path, "new name.txt");
        assert_eq!(s.entries[0].from.as_deref(), Some("old name.txt"));
        assert_eq!(s.entries[0].state(), FileState::Renamed);
        assert_eq!(s.entries[1], entry(' ', 'M', "other.txt"));
        assert_eq!(s.entries[2].state(), FileState::Copied);
        assert_eq!(s.entries[2].from.as_deref(), Some("orig.txt"));
        // A rename in the work tree column (intent-to-add) is read the same way.
        let s = parse_status(b" R b\0a\0");
        assert_eq!(s.entries[0].from.as_deref(), Some("a"));
    }

    #[test]
    fn conflicts_are_recognised() {
        let s = parse_status(b"UU a\0AA b\0DD c\0AU d\0UD e\0");
        assert!(s.entries.iter().all(|e| e.state() == FileState::Conflict));
        assert!(!entry('M', ' ', "x").conflicted());
    }

    #[test]
    fn branch_header_variants() {
        let b = |s: &str| parse_status(s.as_bytes()).branch;
        assert_eq!(b("## main\0").name.as_deref(), Some("main"));
        assert_eq!(b("## feature/x...origin/feature/x\0").name.as_deref(), Some("feature/x"));
        assert_eq!(b("## main...origin/main [gone]\0").upstream.as_deref(), Some("origin/main"));
        let fresh = b("## No commits yet on master\0");
        assert!(fresh.no_commits);
        assert_eq!(fresh.name.as_deref(), Some("master"));
        assert!(b("## Initial commit on main\0").no_commits);
        let detached = b("## HEAD (no branch)\0");
        assert_eq!(detached.name, None);
        assert!(!detached.no_commits);
    }

    #[test]
    fn empty_and_malformed_output() {
        assert_eq!(parse_status(b""), Status::default());
        assert!(parse_status(b"\0\0xy\0?\0").entries.is_empty());
    }

    #[test]
    fn log_records_split_on_nul_and_unit_separator() {
        let out = "abc1234\u{1f}Jos\u{e9} Dupont\u{1f}2026-11-12 09:30\u{1f}Fix: the \"intro\"\0def5678\u{1f}Ann\u{1f}2026-11-11 18:00\u{1f}Second | line\0";
        let log = parse_log(out.as_bytes());
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].author, "Jos\u{e9} Dupont");
        assert_eq!(log[0].subject, "Fix: the \"intro\"");
        assert_eq!(log[1].short, "def5678");
        assert_eq!(log[1].date, "2026-11-11 18:00");
        assert!(parse_log(b"").is_empty());
        // A subject is kept whole even if it contains the field separator.
        let odd = parse_log("a\u{1f}b\u{1f}c\u{1f}d\u{1f}e\0".as_bytes());
        assert_eq!(odd[0].subject, "d\u{1f}e");
    }

    #[test]
    fn commit_paths_include_old_names_once() {
        let e = vec![
            Entry { x: 'R', y: ' ', path: "b".into(), from: Some("a".into()) },
            entry('?', '?', "c"),
            entry(' ', 'M', "b"),
        ];
        assert_eq!(paths_to_commit(&e), ["b", "a", "c"]);
        assert_eq!(pathspec_stdin(&["x y".into(), "z".into()]), b"x y\0z\0");
    }

    #[test]
    fn gitignore_covers_editor_state_and_exports() {
        assert!(GITIGNORE.lines().any(|l| l == ".orbit/"));
        assert!(GITIGNORE.lines().any(|l| l == "export/"));
        assert!(missing_ignore_lines("").contains(&".orbit/"));
        assert_eq!(missing_ignore_lines("node_modules/\n.orbit/\r\nexport/\n.orbit-*/\n"), Vec::<&str>::new());
        assert_eq!(missing_ignore_lines(".orbit/\n"), ["export/", ".orbit-*/"]);
    }
}
