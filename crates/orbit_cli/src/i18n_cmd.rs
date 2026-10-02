//! `orbit i18n`: PO/CSV exchange and stale-translation checks.

use std::io::Write;
use std::path::{Path, PathBuf};

use orbit_i18n::{ExchangeFormat, Translation, stale_entries};

pub const USAGE: &str = "\
    orbit i18n export <story> --lang <tag> [--format po|csv] [--out <file>]
        Write the strings of the story with their translation (an empty
        template when the language has no file yet) for a translation tool.
        Default out: <story dir>/i18n/<stem>.<lang>.<po|csv>. Stale entries
        (source edited since translated) are flagged (`#, fuzzy` / `stale`).
    orbit i18n import <file.po|file.csv> <story> [--lang <tag>] [--name <text>]
        Merge a translated file into <story dir>/i18n/<stem>.<lang>.ron
        (created if needed). Non-empty texts win. The language comes from
        --lang, else the PO `Language:` header.
    orbit i18n stale <story> [--lang <tag>]
        List translations whose source text changed (exit 1 if any).
    orbit i18n accept <story> --lang <tag>
        Mark the stale translations of a language as reviewed.
    orbit i18n stamp <story> --lang <tag>
        Adopt a legacy translation file: record the current source text of
        every translated string, so later edits are detected as stale.
";

struct Args {
    positional: Vec<String>,
    lang: Option<String>,
    format: Option<ExchangeFormat>,
    out: Option<PathBuf>,
    name: Option<String>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        positional: Vec::new(),
        lang: None,
        format: None,
        out: None,
        name: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |n: &str| it.next().cloned().ok_or(format!("{n} needs a value"));
        match arg.as_str() {
            "--lang" => a.lang = Some(value("--lang")?),
            "--name" => a.name = Some(value("--name")?),
            "--out" => a.out = Some(PathBuf::from(value("--out")?)),
            "--format" => {
                let v = value("--format")?;
                a.format = Some(
                    ExchangeFormat::from_name(&v)
                        .ok_or(format!("unknown format `{v}` (po or csv)"))?,
                );
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            other => a.positional.push(other.to_owned()),
        }
    }
    Ok(a)
}

fn load_story(path: &str) -> Result<(orbit_core::Story, PathBuf), String> {
    let story_path = super::resolve_story_path(Path::new(path))?;
    let source = super::read_story_source(&story_path)?;
    let (story, _) = super::parse_story(&source, &story_path)?;
    Ok((story, story_path))
}

fn format_of(path: &Path) -> Option<ExchangeFormat> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(ExchangeFormat::from_name)
}

pub fn i18n_cmd(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<u8, String> {
    let (sub, rest) = args
        .split_first()
        .ok_or("usage: orbit i18n <export|import|stale|accept|stamp> (see `orbit help`)")?;
    let a = parse(rest)?;
    match sub.as_str() {
        "export" => {
            let [story] = a.positional.as_slice() else {
                return Err("usage: orbit i18n export <story> --lang <tag>".into());
            };
            let lang = a.lang.clone().ok_or("--lang is required")?;
            let (story, story_path) = load_story(story)?;
            let format = a
                .format
                .or_else(|| a.out.as_deref().and_then(format_of))
                .unwrap_or(ExchangeFormat::Po);
            let t = orbit_i18n::files::load(&orbit_i18n::files::translation_path(&story_path, &lang))?
                .unwrap_or_else(|| Translation::new(&lang, a.name.clone().unwrap_or_default()));
            let target = a.out.clone().unwrap_or_else(|| {
                orbit_i18n::files::translation_path(&story_path, &lang).with_extension(format.extension())
            });
            orbit_i18n::files::write_text(&target, &t.export(&story.strings, format))?;
            let _ = writeln!(
                out,
                "{}: {} string(s)",
                target.display(),
                story.strings.entries.len()
            );
            Ok(0)
        }
        "import" => {
            let [file, story] = a.positional.as_slice() else {
                return Err("usage: orbit i18n import <file.po|file.csv> <story>".into());
            };
            let file = Path::new(file);
            let format = a
                .format
                .or_else(|| format_of(file))
                .ok_or("cannot tell the format: use --format po|csv")?;
            let text = std::fs::read_to_string(file)
                .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            let (story, story_path) = load_story(story)?;
            let imported = Translation::import(
                &text,
                format,
                a.lang.as_deref().unwrap_or(""),
                a.name.as_deref().unwrap_or(""),
            )
            .map_err(|e| format!("{}: {e}", file.display()))?;
            if imported.lang.trim().eq_ignore_ascii_case(&story.strings.source_lang) {
                return Err(format!(
                    "`{}` is the source language of the story, it cannot be a translation",
                    imported.lang
                ));
            }
            let target = orbit_i18n::files::translation_path(&story_path, &imported.lang);
            let mut t = orbit_i18n::files::load(&target)?
                .unwrap_or_else(|| Translation::new(&imported.lang, ""));
            let changed = t.merge(imported);
            orbit_i18n::files::save(&target, &t)?;
            let stale = stale_entries(&story.strings, &t).len();
            let _ = writeln!(
                out,
                "{}: {changed} string(s) updated, {stale} stale",
                target.display()
            );
            Ok(0)
        }
        "stale" | "accept" | "stamp" => {
            let [story] = a.positional.as_slice() else {
                return Err(format!("usage: orbit i18n {sub} <story> [--lang <tag>]"));
            };
            let (story, story_path) = load_story(story)?;
            let langs = match &a.lang {
                Some(l) => vec![l.clone()],
                None if sub == "stale" => orbit_i18n::files::list_languages(&story_path),
                None => return Err("--lang is required".into()),
            };
            let mut any = false;
            for lang in langs {
                let path = orbit_i18n::files::translation_path(&story_path, &lang);
                let Some(mut t) = orbit_i18n::files::load(&path)? else {
                    return Err(format!("no translation file {}", path.display()));
                };
                match sub.as_str() {
                    "stale" => {
                        let stale = stale_entries(&story.strings, &t);
                        any |= !stale.is_empty();
                        for s in &stale {
                            let _ = writeln!(
                                err,
                                "stale: {lang} {}: \"{}\" (source is now \"{}\")",
                                s.id, s.translation, s.source
                            );
                        }
                        let _ = writeln!(out, "{lang}: {} stale", stale.len());
                    }
                    "accept" => {
                        let n = t.accept_stale(&story.strings);
                        orbit_i18n::files::save(&path, &t)?;
                        let _ = writeln!(out, "{lang}: {n} marked as reviewed");
                    }
                    _ => {
                        let n = t.stamp_missing(&story.strings);
                        orbit_i18n::files::save(&path, &t)?;
                        let _ = writeln!(out, "{lang}: {n} string(s) stamped");
                    }
                }
            }
            Ok(u8::from(any))
        }
        other => Err(format!("unknown i18n command `{other}`")),
    }
}

