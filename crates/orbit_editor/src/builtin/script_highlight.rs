//! Rhai syntax highlighting (ADR 0015): a pure tokenizer, [`highlight`],
//! and the egui glue ([`layout_job`]) the script code editors use.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, TextStyle, Ui};

/// What a run of source text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Keyword,
    /// A function of the script API (`get_var`, `branch`...).
    Api,
    Str,
    Number,
    Comment,
}

/// A run of the source: byte range and kind. Spans cover the whole text,
/// in order, without gaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

pub const KEYWORDS: &[&str] = &[
    "let", "const", "fn", "if", "else", "while", "loop", "for", "in", "return", "break",
    "continue", "import", "export", "as", "throw", "try", "catch", "true", "false", "switch",
    "do", "until", "private", "this",
];

pub const API: &[&str] = &[
    "get_var",
    "set_var",
    "has_var",
    "rand_int",
    "rand_float",
    "chance",
    "branch",
    "log",
    "print",
    "api_version",
];

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Classifies `src`. The spans concatenate back to the whole text.
pub fn highlight(src: &str) -> Vec<Span> {
    let bytes = src.as_bytes();
    let mut spans: Vec<Span> = Vec::new();
    let mut push = |start: usize, end: usize, kind: TokenKind| {
        if start >= end {
            return;
        }
        match spans.last_mut() {
            Some(last) if last.kind == kind && last.end == start && kind == TokenKind::Plain => {
                last.end = end;
            }
            _ => spans.push(Span { start, end, kind }),
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let start = i;
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            push(start, i, TokenKind::Comment);
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            // Rhai block comments nest.
            let mut depth = 0usize;
            while i < bytes.len() {
                if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            push(start, i.min(bytes.len()), TokenKind::Comment);
        } else if b == b'"' || b == b'`' || b == b'\'' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && b != b'`' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b {
                    i += 1;
                    break;
                }
                // A plain string or char stops at the end of the line
                // (unterminated): keeps the rest of the file readable.
                if bytes[i] == b'\n' && b != b'`' {
                    break;
                }
                i += 1;
            }
            push(start, i.min(bytes.len()), TokenKind::Str);
        } else if b.is_ascii_digit() {
            while i < bytes.len() {
                let c = bytes[i] as char;
                let exp_sign = (c == '+' || c == '-')
                    && i > start
                    && matches!(bytes[i - 1], b'e' | b'E')
                    && !src[start..i].starts_with("0x");
                let fraction = c == '.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
                if c.is_ascii_alphanumeric() || c == '_' || exp_sign || fraction {
                    i += 1;
                } else {
                    break;
                }
            }
            push(start, i, TokenKind::Number);
        } else if is_ident_start(b as char) {
            while i < bytes.len() && is_ident(bytes[i] as char) {
                i += 1;
            }
            let word = &src[start..i];
            let kind = if KEYWORDS.contains(&word) {
                TokenKind::Keyword
            } else if API.contains(&word) {
                TokenKind::Api
            } else {
                TokenKind::Plain
            };
            push(start, i, kind);
        } else {
            // One whole character (UTF-8 aware) of plain text.
            i += src[i..].chars().next().map_or(1, char::len_utf8);
            push(start, i, TokenKind::Plain);
        }
    }
    spans
}

/// The colored runs of `src` for a `TextEdit` layouter. The lines in
/// `error_lines` (1-based) get a red background.
pub fn layout_job(ui: &Ui, src: &str, error_lines: &[usize]) -> LayoutJob {
    let font: FontId = TextStyle::Monospace.resolve(ui.style());
    let dark = ui.visuals().dark_mode;
    let pick = |d: (u8, u8, u8), l: (u8, u8, u8)| {
        let (r, g, b) = if dark { d } else { l };
        Color32::from_rgb(r, g, b)
    };
    let keyword = pick((197, 134, 192), (140, 40, 150));
    let api = pick((97, 175, 239), (20, 90, 170));
    let string = pick((152, 195, 121), (30, 120, 50));
    let number = pick((209, 154, 102), (160, 80, 0));
    let comment = pick((120, 130, 140), (110, 120, 130));
    let normal = ui.visuals().text_color();
    let error_bg = Color32::from_rgba_unmultiplied(220, 60, 60, 50);

    let mut job = LayoutJob::default();
    let mut line = 1usize;
    for span in highlight(src) {
        let color = match span.kind {
            TokenKind::Plain => normal,
            TokenKind::Keyword => keyword,
            TokenKind::Api => api,
            TokenKind::Str => string,
            TokenKind::Number => number,
            TokenKind::Comment => comment,
        };
        // Split on line breaks so the error background follows the lines.
        for piece in src[span.start..span.end].split_inclusive('\n') {
            let bg = if error_lines.contains(&line) {
                error_bg
            } else {
                Color32::TRANSPARENT
            };
            job.append(
                piece,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color,
                    background: bg,
                    italics: span.kind == TokenKind::Comment,
                    ..TextFormat::default()
                },
            );
            if piece.ends_with('\n') {
                line += 1;
            }
        }
    }
    job
}

/// Char index (as `TextEdit` cursors count) of the 1-based `line`, `column`
/// of `src`; clamped to the text.
pub fn char_index_of(src: &str, line: usize, column: usize) -> usize {
    let mut index = 0;
    for (n, text) in src.split_inclusive('\n').enumerate() {
        if n + 1 == line.max(1) {
            let col = column.max(1) - 1;
            return index + text.trim_end_matches('\n').chars().count().min(col);
        }
        index += text.chars().count();
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(&str, TokenKind)> {
        // Plain runs are merged by the tokenizer: split them on blanks.
        highlight(src)
            .into_iter()
            .flat_map(|s| {
                let text = &src[s.start..s.end];
                if s.kind == TokenKind::Plain {
                    text.split_whitespace().map(|w| (w, s.kind)).collect()
                } else {
                    vec![(text, s.kind)]
                }
            })
            .collect()
    }

    #[test]
    fn spans_cover_the_whole_text() {
        let src = "let x = get_var(\"gold\") + 1.5; // é\n/* a /* b */ c */ \"ü\\\"\" `t\nu`";
        let mut at = 0;
        for s in highlight(src) {
            assert_eq!(s.start, at);
            assert!(s.end > s.start);
            at = s.end;
        }
        assert_eq!(at, src.len());
    }

    #[test]
    fn keywords_api_strings_numbers_comments() {
        use TokenKind::*;
        assert_eq!(
            kinds("let n = get_var(\"gold\") * 2; // x"),
            vec![
                ("let", Keyword),
                ("n", Plain),
                ("=", Plain),
                ("get_var", Api),
                ("(", Plain),
                ("\"gold\"", Str),
                (")", Plain),
                ("*", Plain),
                ("2", Number),
                (";", Plain),
                ("// x", Comment),
            ]
        );
    }

    #[test]
    fn numbers_floats_hex_exponents() {
        use TokenKind::*;
        assert_eq!(kinds("1.5e-3"), vec![("1.5e-3", Number)]);
        assert_eq!(kinds("0xFF_u"), vec![("0xFF_u", Number)]);
        // A method call on an integer literal is not part of the number.
        assert_eq!(kinds("a.b"), vec![("a.b", Plain)]);    }

    #[test]
    fn nested_and_unterminated_comments() {
        use TokenKind::*;
        assert_eq!(
            kinds("/* a /* b */ c */ x"),
            vec![("/* a /* b */ c */", Comment), ("x", Plain)]
        );
        assert_eq!(kinds("x /* open"), vec![("x", Plain), ("/* open", Comment)]);
    }

    #[test]
    fn strings_with_escapes_and_unterminated_ones() {
        use TokenKind::*;
        assert_eq!(kinds(r#""a\"b" c"#), vec![(r#""a\"b""#, Str), ("c", Plain)]);
        // A plain string stops at the end of its line; a backtick one does not.
        assert_eq!(kinds("\"open\nlet"), vec![("\"open", Str), ("let", Keyword)]);
        assert_eq!(kinds("`a\nb` x"), vec![("`a\nb`", Str), ("x", Plain)]);
    }

    #[test]
    fn identifiers_that_contain_keywords_are_plain() {
        use TokenKind::*;
        assert_eq!(kinds("letter iffy get_variable"), vec![
            ("letter", Plain),
            ("iffy", Plain),
            ("get_variable", Plain)
        ]);
    }

    #[test]
    fn char_index_follows_lines_and_clamps() {
        let src = "ab\ncdé\nf";
        assert_eq!(char_index_of(src, 1, 1), 0);
        assert_eq!(char_index_of(src, 2, 3), 5);
        assert_eq!(char_index_of(src, 2, 99), 6);
        assert_eq!(char_index_of(src, 3, 2), 8);
        assert_eq!(char_index_of(src, 99, 1), 8);
    }
}
