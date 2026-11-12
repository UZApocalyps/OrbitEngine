//! The asset report, without any UI or disk access: what the gathering step
//! (`scan.rs`) found goes in as [`Inputs`], the numbers the panel shows and
//! the CSV come out. Pure, so it is unit tested.

use std::collections::{HashMap, HashSet};

use orbit_editor_sdk::orbit_core::{AssetId, AssetKind};

/// An entry of the story's asset manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredAsset {
    pub id: AssetId,
    pub kind: AssetKind,
    /// Manifest path (relative to the assets root, `/` separators).
    pub path: String,
}

/// A file found in the project's asset folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskFile {
    pub path: String,
    pub len: u64,
}

/// Everything the report is computed from.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    pub declared: Vec<DeclaredAsset>,
    /// How many places of the story (nodes, expressions, achievements) use
    /// each asset; absent: none.
    pub story_uses: HashMap<AssetId, usize>,
    /// Paths named by other project files (UI screens, rigs, 3D scenes,
    /// scripts), lower case, `/` separators.
    pub referenced_paths: HashSet<String>,
    /// Assets referenced by id outside the story (voices of translations).
    pub referenced_ids: HashSet<AssetId>,
    /// Size of every existing file (declared or not), by path.
    pub sizes: HashMap<String, u64>,
    /// Every file of the asset folder.
    pub disk: Vec<DiskFile>,
}

/// A declared asset with what is known about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: AssetId,
    pub kind: AssetKind,
    pub path: String,
    /// `None`: the file does not exist.
    pub size: Option<u64>,
    pub story_uses: usize,
    /// Named by a UI screen, a rig, a script or a translation.
    pub other_ref: bool,
}

impl Row {
    /// Declared but referenced by nothing we can see.
    pub fn is_unused(&self) -> bool {
        self.story_uses == 0 && !self.other_ref
    }

    pub fn is_missing(&self) -> bool {
        self.size.is_none()
    }
}

/// Count and size of a group of files. `kind: None` is the files that are
/// not declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindTotal {
    pub kind: Option<AssetKind>,
    pub count: usize,
    pub bytes: u64,
}

/// One of the biggest files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigFile {
    pub path: String,
    pub bytes: u64,
    pub declared: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub rows: Vec<Row>,
    /// Files of the asset folder the manifest does not mention, by path.
    pub undeclared: Vec<DiskFile>,
}

const KIND_ORDER: [AssetKind; 6] = [
    AssetKind::Image,
    AssetKind::Music,
    AssetKind::Sound,
    AssetKind::Font,
    AssetKind::Rig,
    AssetKind::Scene3d,
];

/// Position of a kind in the displays (stable, grouped by medium).
pub fn kind_rank(kind: AssetKind) -> usize {
    KIND_ORDER.iter().position(|k| *k == kind).unwrap_or(KIND_ORDER.len())
}

/// Normal form of a path for comparisons: `/` separators, no leading `./`,
/// lower case (Windows and macOS file systems ignore the case).
pub fn norm_key(path: &str) -> String {
    let p = path.replace('\\', "/");
    p.trim_start_matches("./").to_lowercase()
}

impl Report {
    pub fn build(inputs: Inputs) -> Report {
        let declared_keys: HashSet<String> = inputs.declared.iter().map(|a| norm_key(&a.path)).collect();
        let mut rows: Vec<Row> = inputs
            .declared
            .iter()
            .map(|a| Row {
                id: a.id,
                kind: a.kind,
                path: a.path.clone(),
                size: inputs.sizes.get(&a.path).copied(),
                story_uses: inputs.story_uses.get(&a.id).copied().unwrap_or(0),
                other_ref: inputs.referenced_ids.contains(&a.id)
                    || inputs.referenced_paths.contains(&norm_key(&a.path)),
            })
            .collect();
        rows.sort_by(|a, b| a.path.cmp(&b.path));
        let mut undeclared: Vec<DiskFile> = inputs
            .disk
            .into_iter()
            .filter(|f| !declared_keys.contains(&norm_key(&f.path)))
            .collect();
        undeclared.sort_by(|a, b| a.path.cmp(&b.path));
        Report { rows, undeclared }
    }

    pub fn unused(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|r| r.is_unused())
    }

    pub fn missing_count(&self) -> usize {
        self.rows.iter().filter(|r| r.is_missing()).count()
    }

    pub fn declared_bytes(&self) -> u64 {
        self.rows.iter().filter_map(|r| r.size).sum()
    }

    pub fn undeclared_bytes(&self) -> u64 {
        self.undeclared.iter().map(|f| f.len).sum()
    }

    /// Declared and undeclared files together.
    pub fn total_bytes(&self) -> u64 {
        self.declared_bytes() + self.undeclared_bytes()
    }

    pub fn unused_bytes(&self) -> u64 {
        self.unused().filter_map(|r| r.size).sum()
    }

    /// Size per type, biggest first; the undeclared files last when there are
    /// some. Types without a file are left out.
    pub fn totals_by_kind(&self) -> Vec<KindTotal> {
        let mut out: Vec<KindTotal> = KIND_ORDER
            .iter()
            .map(|k| {
                let rows = self.rows.iter().filter(|r| r.kind == *k && r.size.is_some());
                let (count, bytes) = rows.fold((0, 0), |(c, b), r| (c + 1, b + r.size.unwrap_or(0)));
                KindTotal { kind: Some(*k), count, bytes }
            })
            .filter(|t| t.count > 0)
            .collect();
        out.sort_by_key(|t| std::cmp::Reverse(t.bytes));
        if !self.undeclared.is_empty() {
            out.push(KindTotal { kind: None, count: self.undeclared.len(), bytes: self.undeclared_bytes() });
        }
        out
    }

    /// The `n` biggest files, declared or not, biggest first (ties by path).
    pub fn biggest(&self, n: usize) -> Vec<BigFile> {
        let mut all: Vec<BigFile> = self
            .rows
            .iter()
            .filter_map(|r| r.size.map(|b| BigFile { path: r.path.clone(), bytes: b, declared: true }))
            .chain(
                self.undeclared
                    .iter()
                    .map(|f| BigFile { path: f.path.clone(), bytes: f.len, declared: false }),
            )
            .collect();
        all.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
        all.truncate(n);
        all
    }
}

/// Column the lists are sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Path,
    Kind,
    Size,
    Uses,
}

/// Indices of `rows` in the order of `key` (`descending` reverses it); ties
/// keep the path order. A missing file sorts as size 0.
pub fn sort_rows(rows: &[&Row], key: SortKey, descending: bool) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..rows.len()).collect();
    idx.sort_by(|&a, &b| {
        let (x, y) = (rows[a], rows[b]);
        let ord = match key {
            SortKey::Path => x.path.to_lowercase().cmp(&y.path.to_lowercase()),
            SortKey::Kind => kind_rank(x.kind).cmp(&kind_rank(y.kind)),
            SortKey::Size => x.size.unwrap_or(0).cmp(&y.size.unwrap_or(0)),
            SortKey::Uses => x.story_uses.cmp(&y.story_uses),
        };
        let ord = if descending { ord.reverse() } else { ord };
        ord.then_with(|| x.path.cmp(&y.path))
    });
    idx
}

/// "1.5 MB", "312 B": binary units (1 KB = 1024 B), one decimal from KB up.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Quoted strings of a text file (`"..."`, with `\"` and `\\` escapes), the
/// way a RON, Rhai or JSON file writes the paths it names.
pub fn quoted_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        current = match (current.take(), c) {
            (None, '"') => Some(String::new()),
            (None, _) => None,
            (Some(s), '"') => {
                out.push(s);
                None
            }
            (Some(mut s), '\\') => {
                if let Some(next) = chars.next() {
                    s.push(next);
                }
                Some(s)
            }
            (Some(mut s), c) => {
                s.push(c);
                Some(s)
            }
        };
    }
    out
}

/// `dir` + `/` + `rel` with `.` and `..` resolved; `None` if it leaves the
/// root. `dir` is the folder of the file that names `rel`.
pub fn join_rel(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    let full = format!("{dir}/{rel}").replace('\\', "/");
    for p in full.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// One CSV field: quoted when it holds the separator, a quote or a line break.
fn field(value: &str, sep: char) -> String {
    if value.contains([sep, '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// The CSV of the report: UTF-8 with a byte order mark (so that Excel opens
/// it as UTF-8), `\r\n` line ends, `sep` between the fields. One line per
/// declared asset, then one per undeclared file. `header` has [`CSV_COLUMNS`]
/// titles; the values are language independent.
pub const CSV_COLUMNS: usize = 7;

pub fn to_csv(report: &Report, header: &[String], sep: char) -> String {
    let mut out = String::from('\u{feff}');
    let mut line = |cells: &[String]| {
        let cells: Vec<String> = cells.iter().map(|c| field(c, sep)).collect();
        out.push_str(&cells.join(&sep.to_string()));
        out.push_str("\r\n");
    };
    line(header);
    for r in &report.rows {
        line(&[
            "declared".into(),
            r.kind.to_string(),
            r.path.clone(),
            r.size.map(|s| s.to_string()).unwrap_or_default(),
            if r.size.is_some() { "present" } else { "missing" }.into(),
            r.story_uses.to_string(),
            if r.is_unused() { "unused" } else { "used" }.into(),
        ]);
    }
    for f in &report.undeclared {
        line(&[
            "undeclared".into(),
            String::new(),
            f.path.clone(),
            f.len.to_string(),
            "present".into(),
            String::new(),
            String::new(),
        ]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(kind: AssetKind, path: &str) -> DeclaredAsset {
        DeclaredAsset { id: AssetId::new(), kind, path: path.into() }
    }

    fn sample() -> (Inputs, [AssetId; 5]) {
        let a = asset(AssetKind::Image, "assets/bg.png");
        let b = asset(AssetKind::Image, "assets/unused.png");
        let c = asset(AssetKind::Sound, "assets/voice/fr/x.ogg");
        let d = asset(AssetKind::Image, "assets/gone.png");
        let e = asset(AssetKind::Image, "assets/ui_only.png");
        let ids = [a.id, b.id, c.id, d.id, e.id];
        let mut inputs = Inputs { declared: vec![a, b, c, d, e], ..Default::default() };
        inputs.story_uses.insert(ids[0], 2);
        inputs.referenced_ids.insert(ids[2]);
        inputs.referenced_paths.insert("assets/ui_only.png".into());
        for (p, n) in [("assets/bg.png", 1000u64), ("assets/unused.png", 5000), ("assets/voice/fr/x.ogg", 300), ("assets/ui_only.png", 70), ("assets/stray.png", 9000)] {
            inputs.sizes.insert(p.into(), n);
            inputs.disk.push(DiskFile { path: p.into(), len: n });
        }
        (inputs, ids)
    }

    #[test]
    fn finds_unused_missing_and_undeclared() {
        let (inputs, ids) = sample();
        let report = Report::build(inputs);
        let unused: Vec<_> = report.unused().map(|r| r.id).collect();
        // `gone.png` is declared, missing and referenced by nothing: unused too.
        assert!(unused.contains(&ids[1]) && unused.contains(&ids[3]));
        assert!(!unused.contains(&ids[0]), "used by the story");
        assert!(!unused.contains(&ids[2]), "used by a translation voice");
        assert!(!unused.contains(&ids[4]), "named by a UI file");
        assert_eq!(report.missing_count(), 1);
        assert_eq!(report.undeclared.len(), 1);
        assert_eq!(report.undeclared[0].path, "assets/stray.png");
        assert_eq!(report.unused_bytes(), 5000);
    }

    #[test]
    fn totals_and_biggest() {
        let (inputs, _) = sample();
        let report = Report::build(inputs);
        assert_eq!(report.declared_bytes(), 1000 + 5000 + 300 + 70);
        assert_eq!(report.undeclared_bytes(), 9000);
        assert_eq!(report.total_bytes(), 15370);
        let totals = report.totals_by_kind();
        assert_eq!(totals[0].kind, Some(AssetKind::Image));
        assert_eq!((totals[0].count, totals[0].bytes), (3, 6070));
        assert_eq!(totals[1].kind, Some(AssetKind::Sound));
        assert_eq!(totals.last().unwrap().kind, None);
        let big = report.biggest(2);
        assert_eq!(big[0].path, "assets/stray.png");
        assert!(!big[0].declared);
        assert_eq!(big[1].path, "assets/unused.png");
        assert_eq!(report.biggest(99).len(), 5);
    }

    #[test]
    fn declared_paths_match_whatever_the_case_or_the_separator() {
        let a = asset(AssetKind::Image, "assets/Bg.PNG");
        let mut inputs = Inputs { declared: vec![a], ..Default::default() };
        inputs.disk.push(DiskFile { path: "assets/bg.png".into(), len: 1 });
        inputs.referenced_paths.insert(norm_key(".\\assets\\BG.png"));
        let report = Report::build(inputs);
        assert!(report.undeclared.is_empty());
        assert!(!report.rows[0].is_unused());
    }

    #[test]
    fn sorting_is_stable_and_reversible() {
        let (inputs, _) = sample();
        let report = Report::build(inputs);
        let rows: Vec<&Row> = report.rows.iter().collect();
        let by_size = sort_rows(&rows, SortKey::Size, true);
        let sizes: Vec<_> = by_size.iter().map(|&i| rows[i].size.unwrap_or(0)).collect();
        assert_eq!(sizes, vec![5000, 1000, 300, 70, 0]);
        let asc = sort_rows(&rows, SortKey::Size, false);
        assert_eq!(asc.first().map(|&i| rows[i].size.unwrap_or(0)), Some(0));
        let by_kind = sort_rows(&rows, SortKey::Kind, false);
        assert_eq!(rows[*by_kind.last().unwrap()].kind, AssetKind::Sound);
        let by_path = sort_rows(&rows, SortKey::Path, false);
        assert_eq!(rows[by_path[0]].path, "assets/bg.png");
        assert!(sort_rows(&[], SortKey::Uses, true).is_empty());
    }

    #[test]
    fn byte_formatting() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536 * 1024), "1.5 MB");
        assert!(format_bytes(u64::MAX).ends_with("GB"));
    }

    #[test]
    fn quoted_strings_handle_escapes_and_empties() {
        let text = r#"(image: "assets/a.png", name: "say \"hi\"", empty: "", x: 3, p: "b\\c")"#;
        assert_eq!(quoted_strings(text), vec!["assets/a.png", "say \"hi\"", "", "b\\c"]);
        assert!(quoted_strings("no quotes here").is_empty());
        // An unterminated string yields nothing.
        assert!(quoted_strings("\"open").is_empty());
    }

    #[test]
    fn relative_joins() {
        assert_eq!(join_rel("assets/rigs", "../body.png").as_deref(), Some("assets/body.png"));
        assert_eq!(join_rel("assets/rigs", "./a/b.png").as_deref(), Some("assets/rigs/a/b.png"));
        assert_eq!(join_rel("assets", "../../x.png"), None);
        assert_eq!(join_rel("", "a.png").as_deref(), Some("a.png"));
    }

    #[test]
    fn csv_has_a_bom_quotes_fields_and_lists_everything() {
        let mut inputs = Inputs::default();
        inputs.declared.push(asset(AssetKind::Image, "assets/a;b \"x\".png"));
        inputs.sizes.insert("assets/a;b \"x\".png".into(), 12);
        inputs.disk.push(DiskFile { path: "assets/stray.png".into(), len: 7 });
        let report = Report::build(inputs);
        let header: Vec<String> = (0..CSV_COLUMNS).map(|i| format!("h{i}")).collect();
        let csv = to_csv(&report, &header, ';');
        assert!(csv.starts_with('\u{feff}'));
        let lines: Vec<&str> = csv.trim_end().split("\r\n").collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "\u{feff}h0;h1;h2;h3;h4;h5;h6");
        assert_eq!(lines[1], "declared;image;\"assets/a;b \"\"x\"\".png\";12;present;0;unused");
        assert_eq!(lines[2], "undeclared;;assets/stray.png;7;present;;");
        // With a comma the semicolon no longer needs quotes.
        let comma = to_csv(&report, &header, ',');
        assert!(comma.contains("\"assets/a;b \"\"x\"\".png\""));
    }

    #[test]
    fn an_empty_project_gives_an_empty_report() {
        let report = Report::build(Inputs::default());
        assert_eq!(report.total_bytes(), 0);
        assert!(report.totals_by_kind().is_empty());
        assert!(report.biggest(10).is_empty());
        let csv = to_csv(&report, &vec![String::new(); CSV_COLUMNS], ';');
        assert_eq!(csv.trim_end(), "\u{feff};;;;;;");
    }
}
