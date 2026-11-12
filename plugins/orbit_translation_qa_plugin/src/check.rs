//! The checks themselves: pure functions on texts, unit tested.
//!
//! The syntaxes are the engine's:
//!
//! * `[name]` interpolates a variable (`orbit_core::interpolate_vars`: the
//!   name is made of letters, digits, `_` and `.`; `[[` is a literal `[`);
//! * `{b}`, `{i}`, `{color=...}`, `{w=...}`, `{bubble=...}` and their closing
//!   tags are the text markup (`orbit_text::parse_markup`; `{{` and `}}` are
//!   literal braces, an unknown tag stays literal text).

use std::collections::BTreeMap;

use orbit_editor_sdk::orbit_core::{NodeId, StringId};

/// Where a string is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A line of dialogue.
    Line,
    /// The label of a choice.
    Choice,
    /// Any other text (notification, ...), or one nothing uses.
    Other,
}

/// A source string and where it lives in the story.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: StringId,
    pub source: String,
    pub origin: Origin,
    /// The node that owns the string (to show it in the editor).
    pub node: Option<NodeId>,
    pub scene: String,
}

/// What a check found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Placeholders,
    Tags,
    Unpaired,
    Identical,
    Empty,
    Whitespace,
    Punctuation,
    Length,
    Orphan,
}

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::Placeholders,
        Kind::Tags,
        Kind::Unpaired,
        Kind::Identical,
        Kind::Empty,
        Kind::Whitespace,
        Kind::Punctuation,
        Kind::Length,
        Kind::Orphan,
    ];

    /// Stable word, used in text keys and in the CSV.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Placeholders => "placeholders",
            Kind::Tags => "tags",
            Kind::Unpaired => "unpaired",
            Kind::Identical => "identical",
            Kind::Empty => "empty",
            Kind::Whitespace => "whitespace",
            Kind::Punctuation => "punctuation",
            Kind::Length => "length",
            Kind::Orphan => "orphan",
        }
    }
}

/// The facts behind an issue, rendered by the UI in the editor language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detail {
    None,
    /// Items the source has and the translation lacks, and the reverse.
    Diff { missing: Vec<String>, extra: Vec<String> },
    /// Tags opened and never closed (or the reverse) in the translation.
    Unpaired(Vec<String>),
    /// Which end differs in spaces.
    Ends { leading: bool, trailing: bool },
    /// Final punctuation of the source and of the translation (`""` for none).
    Punctuation { source: String, found: String },
    /// Visible length of both texts and the ratio in percent.
    Length { source: usize, found: usize, percent: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub kind: Kind,
    pub detail: Detail,
    /// Index into the items of the report (`None` for an orphaned entry).
    pub item: Option<usize>,
    pub id: StringId,
    /// The translation text the issue is about.
    pub text: String,
}

/// The settings of the checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub identical: bool,
    pub punctuation: bool,
    pub length: bool,
    /// A translation longer than this percentage of the source is reported.
    pub length_percent: u32,
}

pub const DEFAULT_LENGTH_PERCENT: u32 = 150;
pub const MIN_LENGTH_PERCENT: u32 = 101;
pub const MAX_LENGTH_PERCENT: u32 = 1000;

impl Default for Settings {
    fn default() -> Self {
        Self { identical: true, punctuation: true, length: true, length_percent: DEFAULT_LENGTH_PERCENT }
    }
}

// ------------------------------------------------------------ interpolation

/// The `[name]` variables of `text`, in reading order (the rules of
/// `orbit_core::interpolate_vars`).
pub fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('[') {
        let tail = &rest[at + 1..];
        if let Some(after) = tail.strip_prefix('[') {
            rest = after;
            continue;
        }
        match tail.find(']') {
            Some(end) if end > 0 && tail[..end].chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') => {
                out.push(tail[..end].to_owned());
                rest = &tail[end + 1..];
            }
            _ => rest = tail,
        }
    }
    out
}

// ------------------------------------------------------------------- markup

/// A markup tag as `parse_markup` accepts it (the trimmed text between the
/// braces), or `None` for text that stays literal.
fn valid_tag(inner: &str) -> Option<String> {
    let t = inner.trim();
    let ok = match t {
        "b" | "/b" | "i" | "/i" | "/color" => true,
        _ => {
            if let Some(v) = t.strip_prefix("bubble=") {
                matches!(v.trim(), "speech" | "thought" | "shout")
            } else if let Some(v) = t.strip_prefix("color=") {
                !v.trim().is_empty()
            } else if let Some(v) = t.strip_prefix("w=") {
                v.trim().parse::<f32>().is_ok_and(|s| s.is_finite() && s >= 0.0)
            } else {
                false
            }
        }
    };
    ok.then(|| t.to_owned())
}

/// Splits `text` into its tags and its visible text (`{{` and `}}` become
/// braces), like `parse_markup`.
fn scan_markup(text: &str) -> (Vec<String>, String) {
    let mut tags = Vec::new();
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(['{', '}']) {
        plain.push_str(&rest[..pos]);
        let after = &rest[pos..];
        if after.starts_with("{{") || after.starts_with("}}") {
            plain.push_str(&after[..1]);
            rest = &after[2..];
            continue;
        }
        if after.starts_with('{')
            && let Some(end) = after.find('}')
            && let Some(tag) = valid_tag(&after[1..end])
        {
            tags.push(tag);
            rest = &after[end + 1..];
            continue;
        }
        plain.push_str(&after[..1]);
        rest = &after[1..];
    }
    plain.push_str(rest);
    (tags, plain)
}

/// The markup tags of `text`, in reading order.
#[cfg(test)]
pub fn tags(text: &str) -> Vec<String> {
    scan_markup(text).0
}

/// The text as the player reads it, without tags.
pub fn visible_text(text: &str) -> String {
    scan_markup(text).1
}

/// The tags of the pairs `{b}`/`{/b}`, `{i}`/`{/i}`, `{color=..}`/`{/color}`
/// that are not matched: a closing tag with nothing open is `/x`, an opening
/// tag never closed is `x`.
pub fn unpaired(tags: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (open, close, name) in [("b", "/b", "b"), ("i", "/i", "i"), ("color=", "/color", "color")] {
        let mut depth = 0usize;
        for t in tags {
            if t == close {
                if depth == 0 {
                    out.push(format!("{{{close}}}"));
                } else {
                    depth -= 1;
                }
            } else if (open.ends_with('=') && t.starts_with(open)) || t == open {
                depth += 1;
            }
        }
        for _ in 0..depth {
            out.push(format!("{{{name}}}"));
        }
    }
    out
}

// ------------------------------------------------------------------ helpers

/// `(in a, not in b)` and `(in b, not in a)`, counting repetitions.
fn multiset_diff(a: &[String], b: &[String]) -> (Vec<String>, Vec<String>) {
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for x in a {
        *counts.entry(x).or_default() += 1;
    }
    for x in b {
        *counts.entry(x).or_default() -= 1;
    }
    let mut only_a = Vec::new();
    let mut only_b = Vec::new();
    for (k, n) in counts {
        for _ in 0..n.unsigned_abs() {
            if n > 0 { &mut only_a } else { &mut only_b }.push(k.to_owned());
        }
    }
    (only_a, only_b)
}

fn is_closing(c: char) -> bool {
    matches!(c, '"' | '\'' | '\u{201d}' | '\u{2019}' | '\u{bb}' | '\u{203a}' | ')' | ']' | '\u{300d}' | '\u{300f}')
}

/// A sentence-final mark, brought to its ASCII form so that `…` and `...`,
/// `。` and `.` compare equal.
fn punct(c: char) -> Option<&'static str> {
    Some(match c {
        '.' | '\u{3002}' | '\u{964}' | '\u{ff0e}' => ".",
        '!' | '\u{ff01}' => "!",
        '?' | '\u{ff1f}' | '\u{61f}' => "?",
        '\u{2026}' => "...",
        ':' | '\u{ff1a}' => ":",
        ';' | '\u{ff1b}' => ";",
        ',' | '\u{ff0c}' | '\u{3001}' => ",",
        _ => return None,
    })
}

/// The final punctuation of the visible text (closing quotes and brackets
/// skipped), normalised; `""` for none.
pub fn final_punctuation(text: &str) -> String {
    let plain = visible_text(text);
    let t = plain.trim_end().trim_end_matches(is_closing).trim_end();
    let mut marks: Vec<&'static str> = Vec::new();
    for c in t.chars().rev() {
        match punct(c) {
            Some(m) => marks.push(m),
            None => break,
        }
    }
    marks.iter().rev().copied().collect()
}

fn leading_ws(s: &str) -> &str {
    &s[..s.len() - s.trim_start().len()]
}

fn trailing_ws(s: &str) -> &str {
    &s[s.trim_end().len()..]
}

fn primary_language(tag: &str) -> String {
    tag.split(['-', '_']).next().unwrap_or(tag).to_ascii_lowercase()
}

// ------------------------------------------------------------------- checks

/// What is wrong with the translation `found` of `source` (never called for a
/// missing entry: that is the Translations panel's business). `same_language`:
/// the translation is in the language of the source (a regional variant), so
/// an identical text is expected.
pub fn check_text(source: &str, found: &str, settings: &Settings, same_language: bool) -> Vec<(Kind, Detail)> {
    let mut out = Vec::new();
    if source.trim().is_empty() {
        return out;
    }
    if found.trim().is_empty() {
        out.push((Kind::Empty, Detail::None));
        return out;
    }

    let (missing, extra) = multiset_diff(&placeholders(source), &placeholders(found));
    if !missing.is_empty() || !extra.is_empty() {
        let wrap = |v: Vec<String>| v.into_iter().map(|n| format!("[{n}]")).collect::<Vec<_>>();
        out.push((Kind::Placeholders, Detail::Diff { missing: wrap(missing), extra: wrap(extra) }));
    }

    let (source_tags, source_plain) = scan_markup(source);
    let (found_tags, found_plain) = scan_markup(found);
    let (missing, extra) = multiset_diff(&source_tags, &found_tags);
    if !missing.is_empty() || !extra.is_empty() {
        let wrap = |v: Vec<String>| v.into_iter().map(|n| format!("{{{n}}}")).collect::<Vec<_>>();
        out.push((Kind::Tags, Detail::Diff { missing: wrap(missing), extra: wrap(extra) }));
    }
    // Pairs broken in the translation but not in the source.
    let broken = unpaired(&found_tags);
    if !broken.is_empty() && unpaired(&source_tags) != broken {
        out.push((Kind::Unpaired, Detail::Unpaired(broken)));
    }

    if settings.identical
        && !same_language
        && source.trim() == found.trim()
        && source_plain.chars().any(char::is_alphabetic)
        // One word is usually a name or a cognate (Alice, Menu): not worth a report.
        && source.split_whitespace().nth(1).is_some()
    {
        out.push((Kind::Identical, Detail::None));
    }

    let (leading, trailing) = (leading_ws(source) != leading_ws(found), trailing_ws(source) != trailing_ws(found));
    if leading || trailing {
        out.push((Kind::Whitespace, Detail::Ends { leading, trailing }));
    }

    if settings.punctuation {
        let (s, f) = (final_punctuation(source), final_punctuation(found));
        if s != f {
            out.push((Kind::Punctuation, Detail::Punctuation { source: s, found: f }));
        }
    }

    if settings.length {
        let (s, f) = (source_plain.trim().chars().count(), found_plain.trim().chars().count());
        // A few characters more on a very short text is not worth a report.
        if s >= 4 && f >= s + 5 {
            let percent = (f as u64 * 100 / s as u64) as u32;
            if percent > settings.length_percent {
                out.push((Kind::Length, Detail::Length { source: s, found: f, percent }));
            }
        }
    }
    out
}

/// Checks every entry of one translation. `items` are the strings of the
/// story; an entry missing from `entries` is skipped, an entry whose id is
/// not in `items` is an orphan.
pub fn analyse(
    items: &[Item],
    entries: &BTreeMap<StringId, String>,
    source_lang: &str,
    lang: &str,
    settings: &Settings,
) -> Vec<Issue> {
    let same_language = primary_language(source_lang) == primary_language(lang);
    let mut out = Vec::new();
    let mut known = std::collections::HashSet::new();
    for (index, item) in items.iter().enumerate() {
        known.insert(item.id);
        let Some(found) = entries.get(&item.id) else { continue };
        for (kind, detail) in check_text(&item.source, found, settings, same_language) {
            out.push(Issue { kind, detail, item: Some(index), id: item.id, text: found.clone() });
        }
    }
    for (id, text) in entries {
        if !known.contains(id) {
            out.push(Issue { kind: Kind::Orphan, detail: Detail::None, item: None, id: *id, text: text.clone() });
        }
    }
    out
}

/// Counts per kind.
pub fn counts(issues: &[Issue]) -> BTreeMap<Kind, usize> {
    let mut out = BTreeMap::new();
    for i in issues {
        *out.entry(i.kind).or_default() += 1;
    }
    out
}

// ---------------------------------------------------------------------- csv

fn field(value: &str, sep: char) -> String {
    // A spreadsheet reads a cell starting with these as a formula: a leading
    // apostrophe makes it text (same rule as the story statistics export).
    let value = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{value}") } else { value.to_owned() };
    if value.contains([sep, '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

/// One row of the CSV, already rendered.
pub struct CsvRow {
    pub lang: String,
    pub kind: String,
    pub scene: String,
    pub id: String,
    pub origin: String,
    pub source: String,
    pub translation: String,
    pub problem: String,
}

pub const CSV_COLUMNS: usize = 8;

/// UTF-8 with a byte order mark (Excel then reads accents correctly), `\r\n`
/// line ends, `sep` between the fields. `header` has [`CSV_COLUMNS`] titles.
pub fn to_csv(header: &[String], rows: &[CsvRow], sep: char) -> String {
    let mut out = String::from('\u{feff}');
    let mut line = |cells: &[&str]| {
        let cells: Vec<String> = cells.iter().map(|c| field(c, sep)).collect();
        out.push_str(&cells.join(&sep.to_string()));
        out.push_str("\r\n");
    };
    line(&header.iter().map(String::as_str).collect::<Vec<_>>());
    for r in rows {
        line(&[&r.lang, &r.kind, &r.scene, &r.id, &r.origin, &r.source, &r.translation, &r.problem]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_cells_cannot_be_formulas() {
        assert_eq!(field("=1+1", ';'), "'=1+1");
        assert_eq!(field("@SUM(A1)", ';'), "'@SUM(A1)");
        assert_eq!(field("-2", ';'), "'-2");
        assert_eq!(field("+1", ';'), "'+1");
        assert_eq!(field("\tx", ';'), "'\tx");
        assert_eq!(field("\rx", ';'), "\"'\rx\"");
        assert_eq!(field("a=b", ';'), "a=b");
        assert_eq!(field("=a;b", ';'), "\"'=a;b\"");
    }

    fn s() -> Settings {
        Settings::default()
    }

    fn kinds(source: &str, found: &str) -> Vec<Kind> {
        check_text(source, found, &s(), false).into_iter().map(|(k, _)| k).collect()
    }

    #[test]
    fn placeholders_follow_the_engine_rules() {
        assert_eq!(placeholders("Hi [name], you have [gold] coins"), ["name", "gold"]);
        assert_eq!(placeholders("[[gold] is literal, [x.y_1] is not"), ["x.y_1"]);
        assert_eq!(placeholders("[ x ] [] [a-b] ["), Vec::<String>::new());
        assert_eq!(placeholders("[a][a]"), ["a", "a"]);
    }

    #[test]
    fn missing_and_extra_variables_are_found() {
        let r = check_text("Hello [name], [gold] coins", "Salut [nom], [gold] pieces", &s(), false);
        assert!(r.contains(&(
            Kind::Placeholders,
            Detail::Diff { missing: vec!["[name]".into()], extra: vec!["[nom]".into()] }
        )));
        // A repeated variable counts.
        assert_eq!(kinds("[a] and [a]", "[a] et moi"), [Kind::Placeholders]);
        // Order does not matter.
        assert!(kinds("[a] then [b]", "[b] puis [a]").is_empty());
    }

    #[test]
    fn tags_are_compared_and_pairs_checked() {
        assert!(kinds("{b}Hi{/b} {w=0.5}you", "{b}Salut{/b} {w=0.5}toi").is_empty());
        let r = check_text("{b}Hi{/b}", "Salut", &s(), false);
        assert!(r.iter().any(|(k, d)| *k == Kind::Tags
            && *d == Detail::Diff { missing: vec!["{/b}".into(), "{b}".into()], extra: vec![] }));
        // Same tags as the source but a pair broken in both: not reported twice.
        assert!(!kinds("{b}Hi", "{b}Salut").contains(&Kind::Unpaired));
        // A translation that opens and never closes while the source is fine.
        let r = check_text("{b}Hi{/b}", "{b}Salut", &s(), false);
        assert!(r.iter().any(|(k, d)| *k == Kind::Unpaired && *d == Detail::Unpaired(vec!["{b}".into()])));
        let r = check_text("Hi", "Salut{/i}", &s(), false);
        assert!(r.iter().any(|(k, _)| *k == Kind::Unpaired));
    }

    #[test]
    fn unknown_braces_are_text_not_tags() {
        assert!(tags("{{b}} {size=3}x{nope}").is_empty());
        assert_eq!(visible_text("a {{b}} {color=red}c{/color}"), "a {b} c");
        assert_eq!(tags("{ b }x{color=#fff}{bubble=thought}"), ["b", "color=#fff", "bubble=thought"]);
        assert_eq!(unpaired(&tags("{color=red}x{color=blue}y{/color}")), ["{color}"]);
    }

    #[test]
    fn identical_translations_are_reported_unless_meaningless() {
        assert_eq!(kinds("Welcome home", "Welcome home"), [Kind::Identical]);
        assert!(kinds("...", "...").is_empty());
        assert!(kinds("42", "42").is_empty());
        assert!(kinds("Alice", "Alice").is_empty());
        // A regional variant of the source language is expected to match.
        assert!(check_text("Colour", "Colour", &s(), true).is_empty());
        let off = Settings { identical: false, ..s() };
        assert!(check_text("Welcome home", "Welcome home", &off, false).is_empty());
    }

    #[test]
    fn empty_or_blank_translation_is_reported_alone() {
        assert_eq!(kinds("Hi [x]", "   \n"), [Kind::Empty]);
        assert_eq!(kinds("Hi", ""), [Kind::Empty]);
        // A blank source has nothing to check.
        assert!(kinds("  ", "").is_empty());
    }

    #[test]
    fn spaces_at_the_ends_are_compared() {
        let r = check_text("Hello", " Bonjour", &s(), false);
        assert!(r.contains(&(Kind::Whitespace, Detail::Ends { leading: true, trailing: false })));
        let r = check_text("Hello ", "Bonjour", &s(), false);
        assert!(r.contains(&(Kind::Whitespace, Detail::Ends { leading: false, trailing: true })));
        assert!(!kinds("  Hello ", "  Bonjour ").contains(&Kind::Whitespace));
    }

    #[test]
    fn final_punctuation_is_normalised() {
        assert_eq!(final_punctuation("Really?!"), "?!");
        assert_eq!(final_punctuation("Vraiment ?!"), "?!");
        assert_eq!(final_punctuation("Wait\u{2026}"), "...");
        assert_eq!(final_punctuation("Wait..."), "...");
        assert_eq!(final_punctuation("\u{300c}\u{4f55}\u{ff1f}\u{300d}"), "?");
        assert_eq!(final_punctuation("He said \"stop.\""), ".");
        assert_eq!(final_punctuation("Done{/b}!"), "!");
        assert_eq!(final_punctuation("No mark"), "");
    }

    #[test]
    fn punctuation_differences_and_the_switch() {
        assert_eq!(kinds("Stop!", "Arrete."), [Kind::Punctuation]);
        assert_eq!(kinds("Stop", "Arrete."), [Kind::Punctuation]);
        assert!(kinds("Stop\u{2026}", "Arrete...").is_empty());
        assert!(kinds("Why are you here?", "Pourquoi es-tu ici ?").is_empty());
        let off = Settings { punctuation: false, ..s() };
        assert!(check_text("Stop!", "Arrete.", &off, false).is_empty());
    }

    #[test]
    fn length_uses_the_threshold_and_ignores_tags_and_tiny_texts() {
        let long = "x".repeat(40);
        let r = check_text(&"y".repeat(20), &long, &s(), false);
        assert!(r.contains(&(Kind::Length, Detail::Length { source: 20, found: 40, percent: 200 })));
        // 140 % is under the default 150 % but over a 120 % setting.
        let found = "z".repeat(28);
        assert!(check_text(&"y".repeat(20), &found, &s(), false).is_empty());
        let strict = Settings { length_percent: 120, ..s() };
        assert!(!check_text(&"y".repeat(20), &found, &strict, false).is_empty());
        // Tags do not count, nor do tiny texts.
        let tagged = format!("{{color=#ff0000}}{}{{/color}}", "z".repeat(29));
        assert!(!check_text(&"y".repeat(20), &tagged, &s(), false).iter().any(|(k, _)| *k == Kind::Length));
        assert!(check_text("Yes", "Oui, bien sur", &s(), false).is_empty());
        let off = Settings { length: false, ..s() };
        assert!(check_text(&"y".repeat(20), &long, &off, false).is_empty());
    }

    fn item(source: &str) -> Item {
        Item { id: StringId::new(), source: source.into(), origin: Origin::Line, node: None, scene: "s".into() }
    }

    #[test]
    fn analyse_skips_missing_entries_and_finds_orphans() {
        let (a, b) = (item("Hello [x]"), item("Bye"));
        let ghost = StringId::new();
        let mut entries = BTreeMap::new();
        entries.insert(a.id, "Salut".to_owned());
        entries.insert(ghost, "Fantome".to_owned());
        let issues = analyse(&[a.clone(), b], &entries, "en", "fr", &s());
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert!(issues.iter().any(|i| i.kind == Kind::Placeholders && i.item == Some(0)));
        let orphan = issues.iter().find(|i| i.kind == Kind::Orphan).unwrap();
        assert_eq!((orphan.id, orphan.item), (ghost, None));
        assert_eq!(counts(&issues)[&Kind::Orphan], 1);
    }

    #[test]
    fn csv_has_a_bom_quotes_and_crlf() {
        let header: Vec<String> = (0..CSV_COLUMNS).map(|i| format!("h{i}")).collect();
        let row = CsvRow {
            lang: "fr".into(),
            kind: "tags".into(),
            scene: "Intro; 1".into(),
            id: "ID".into(),
            origin: "line".into(),
            source: "say \"hi\"\nnow".into(),
            translation: "caf\u{e9}".into(),
            problem: String::new(),
        };
        let csv = to_csv(&header, &[row], ';');
        assert!(csv.starts_with("\u{feff}h0;h1"));
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(lines[0], "\u{feff}h0;h1;h2;h3;h4;h5;h6;h7");
        assert!(lines[1].starts_with("fr;tags;\"Intro; 1\";ID;line;\"say \"\"hi\"\"\nnow\""));
        assert!(csv.contains("caf\u{e9}"));
        assert!(csv.ends_with("\r\n"));
    }
}
