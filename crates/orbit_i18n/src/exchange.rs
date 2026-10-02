//! Exchange with translation tools: gettext PO and CSV export/import, and
//! detection of translations made stale by an edit of the source text.
//!
//! Each exchanged row is `(id, source text, translation)`. The string id is
//! the PO `msgctxt` / the first CSV column, so imports never depend on the
//! source wording. An import records the hash of the source text the row
//! carries: if the story was edited since the export, the entry is stale.

use std::collections::BTreeSet;

use orbit_core::{StringId, StringTable};

use crate::{I18nError, Translation};

/// FNV-1a 64 of the text (CRLF normalized): stable across platforms and
/// releases, so it can be stored in files.
pub fn source_hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.replace("\r\n", "\n").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A translation whose source text changed since it was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaleEntry {
    pub id: StringId,
    /// The current source text.
    pub source: String,
    pub translation: String,
}

/// Entries translated from a source text that is no longer the current one.
/// Entries without a recorded hash (legacy files) are never stale.
pub fn stale_entries(source: &StringTable, t: &Translation) -> Vec<StaleEntry> {
    t.source_hashes
        .iter()
        .filter_map(|(id, hash)| {
            let current = source.entries.get(id)?;
            let text = t.entries.get(id)?;
            (!text.trim().is_empty() && source_hash(current) != *hash).then(|| StaleEntry {
                id: *id,
                source: current.clone(),
                translation: text.clone(),
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExchangeFormat {
    Po,
    Csv,
}

impl ExchangeFormat {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "po" | "pot" => Some(Self::Po),
            "csv" => Some(Self::Csv),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Po => "po",
            Self::Csv => "csv",
        }
    }
}

impl Translation {
    /// Records, for every translated entry without a hash, the hash of the
    /// current source (adopts a legacy file as up to date). Returns the
    /// number of entries stamped.
    pub fn stamp_missing(&mut self, source: &StringTable) -> usize {
        let mut n = 0;
        for (id, text) in &self.entries {
            if text.trim().is_empty() || self.source_hashes.contains_key(id) {
                continue;
            }
            if let Some(src) = source.entries.get(id) {
                self.source_hashes.insert(*id, source_hash(src));
                n += 1;
            }
        }
        n
    }

    /// Marks every stale entry as reviewed (up to date with the current
    /// source). Returns the number of entries changed.
    pub fn accept_stale(&mut self, source: &StringTable) -> usize {
        let stale = stale_entries(source, self);
        for s in &stale {
            self.source_hashes.insert(s.id, source_hash(&s.source));
        }
        stale.len()
    }

    /// Serializes in `format`: every source string with its translation;
    /// stale ones are flagged (`#, fuzzy` in PO, `stale` column in CSV).
    pub fn export(&self, source: &StringTable, format: ExchangeFormat) -> String {
        let stale: BTreeSet<StringId> = stale_entries(source, self)
            .into_iter()
            .map(|s| s.id)
            .collect();
        match format {
            ExchangeFormat::Po => self.to_po(source, &stale),
            ExchangeFormat::Csv => self.to_csv(source, &stale),
        }
    }

    /// Parses an exchange file. `lang`/`name` apply when given; when `lang`
    /// is empty the PO `Language:` header is used.
    pub fn import(
        text: &str,
        format: ExchangeFormat,
        lang: &str,
        name: &str,
    ) -> Result<Self, I18nError> {
        let (header_lang, rows) = match format {
            ExchangeFormat::Po => parse_po(text)?,
            ExchangeFormat::Csv => (None, parse_csv(text)?),
        };
        let lang = if lang.trim().is_empty() {
            header_lang.unwrap_or_default()
        } else {
            lang.to_owned()
        };
        if lang.trim().is_empty() {
            return Err(I18nError::EmptyLanguage);
        }
        let mut t = Translation::new(lang, name);
        for row in rows {
            let Ok(id) = row.id.parse::<StringId>() else {
                continue; // not one of ours
            };
            t.entries.insert(id, row.translation);
            if let Some(src) = row.source {
                t.source_hashes.insert(id, source_hash(&src));
            }
        }
        Ok(t)
    }

    /// Merges an imported translation over this one (same language): its
    /// non-empty texts win; empty ones keep the existing text. Returns the
    /// number of entries changed.
    pub fn merge(&mut self, imported: Translation) -> usize {
        let mut n = 0;
        for (id, text) in imported.entries {
            if text.trim().is_empty() {
                continue;
            }
            if self.entries.get(&id) != Some(&text) {
                n += 1;
            }
            self.entries.insert(id, text);
            match imported.source_hashes.get(&id) {
                Some(h) => self.source_hashes.insert(id, *h),
                None => self.source_hashes.remove(&id),
            };
        }
        if self.name.is_empty() {
            self.name = imported.name;
        }
        n
    }

    fn to_po(&self, source: &StringTable, stale: &BTreeSet<StringId>) -> String {
        let mut out = String::new();
        out.push_str("msgid \"\"\nmsgstr \"\"\n");
        out.push_str("\"Content-Type: text/plain; charset=UTF-8\\n\"\n");
        out.push_str(&format!("\"Language: {}\\n\"\n", self.lang));
        out.push_str(&format!(
            "\"X-Source-Language: {}\\n\"\n",
            source.source_lang
        ));
        for (id, src) in &source.entries {
            let tr = self.entries.get(id).map_or("", String::as_str);
            out.push('\n');
            if stale.contains(id) {
                out.push_str("#, fuzzy\n");
            }
            out.push_str(&format!("msgctxt \"{id}\"\n"));
            po_field(&mut out, "msgid", src);
            po_field(&mut out, "msgstr", tr);
        }
        out
    }

    fn to_csv(&self, source: &StringTable, stale: &BTreeSet<StringId>) -> String {
        let mut out = String::from("id,source,translation,stale\r\n");
        for (id, src) in &source.entries {
            let tr = self.entries.get(id).map_or("", String::as_str);
            let flag = if stale.contains(id) { "stale" } else { "" };
            out.push_str(&format!(
                "{id},{},{},{flag}\r\n",
                csv_field(src),
                csv_field(tr)
            ));
        }
        out
    }
}

struct Row {
    id: String,
    /// The source text the translator saw, when the file carries it.
    source: Option<String>,
    translation: String,
}

fn po_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o
}

fn po_field(out: &mut String, keyword: &str, text: &str) {
    if text.contains('\n') && text.len() > 1 {
        out.push_str(&format!("{keyword} \"\"\n"));
        let mut rest = text;
        while !rest.is_empty() {
            let end = rest.find('\n').map_or(rest.len(), |i| i + 1);
            out.push_str(&format!("\"{}\"\n", po_escape(&rest[..end])));
            rest = &rest[end..];
        }
    } else {
        out.push_str(&format!("{keyword} \"{}\"\n", po_escape(text)));
    }
}

fn po_unquote(s: &str, line_no: usize) -> Result<String, I18nError> {
    let err = |m: &str| I18nError::Parse(format!("PO line {line_no}: {m}"));
    let s = s.trim();
    let inner = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| err("expected a quoted string"))?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            _ => return Err(err("invalid escape sequence")),
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq)]
enum Cur {
    None,
    Ctxt,
    Id,
    Str,
}

#[derive(Default)]
struct PoEntry {
    ctxt: Option<String>,
    msgid: String,
    msgstr: String,
    open: bool,
}

fn parse_po(text: &str) -> Result<(Option<String>, Vec<Row>), I18nError> {
    let mut rows = Vec::new();
    let mut header_lang = None;
    let mut e = PoEntry::default();
    let mut cur = Cur::None;

    let mut flush = |e: &mut PoEntry| {
        let done = std::mem::take(e);
        match done.ctxt {
            Some(id) => rows.push(Row {
                id,
                source: Some(done.msgid),
                translation: done.msgstr,
            }),
            None if done.msgid.is_empty() => {
                for line in done.msgstr.lines() {
                    if let Some(l) = line.strip_prefix("Language:") {
                        header_lang = Some(l.trim().to_owned());
                    }
                }
            }
            None => {}
        }
    };

    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let n = i + 1;
        if line.is_empty() {
            if e.open {
                flush(&mut e);
                cur = Cur::None;
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(v) = line.strip_prefix("msgctxt") {
            if e.open {
                flush(&mut e);
            }
            e.open = true;
            e.ctxt = Some(po_unquote(v, n)?);
            cur = Cur::Ctxt;
        } else if let Some(v) = line.strip_prefix("msgid") {
            if e.open && cur != Cur::Ctxt {
                flush(&mut e);
            }
            e.open = true;
            e.msgid = po_unquote(v, n)?;
            cur = Cur::Id;
        } else if let Some(v) = line.strip_prefix("msgstr") {
            e.msgstr = po_unquote(v, n)?;
            cur = Cur::Str;
        } else if line.starts_with('"') {
            let s = po_unquote(line, n)?;
            match cur {
                Cur::Ctxt => e.ctxt.get_or_insert_with(String::new).push_str(&s),
                Cur::Id => e.msgid.push_str(&s),
                Cur::Str => e.msgstr.push_str(&s),
                Cur::None => {
                    return Err(I18nError::Parse(format!("PO line {n}: stray string")));
                }
            }
        } else {
            return Err(I18nError::Parse(format!(
                "PO line {n}: unexpected `{line}`"
            )));
        }
    }
    if e.open {
        flush(&mut e);
    }
    Ok((header_lang, rows))
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// RFC 4180 records (quoted fields may hold commas, quotes, newlines).
fn csv_records(text: &str) -> Result<Vec<Vec<String>>, I18nError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut records = Vec::new();
    let mut rec: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => rec.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                rec.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut rec));
            }
            c => field.push(c),
        }
    }
    if quoted {
        return Err(I18nError::Parse("CSV: unterminated quoted field".into()));
    }
    if !field.is_empty() || !rec.is_empty() {
        rec.push(field);
        records.push(rec);
    }
    records.retain(|r| !(r.len() == 1 && r[0].is_empty()));
    Ok(records)
}

fn parse_csv(text: &str) -> Result<Vec<Row>, I18nError> {
    let mut it = csv_records(text)?.into_iter();
    let header = it
        .next()
        .ok_or_else(|| I18nError::Parse("CSV: empty file".into()))?;
    let col = |name: &str| {
        header
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
    };
    let id = col("id").ok_or_else(|| I18nError::Parse("CSV: missing `id` column".into()))?;
    let tr = col("translation")
        .ok_or_else(|| I18nError::Parse("CSV: missing `translation` column".into()))?;
    let src = col("source");
    Ok(it
        .map(|r| Row {
            id: r.get(id).map_or_else(String::new, |s| s.trim().to_owned()),
            source: src.and_then(|i| r.get(i)).cloned(),
            translation: r.get(tr).cloned().unwrap_or_default(),
        })
        .collect())
}
