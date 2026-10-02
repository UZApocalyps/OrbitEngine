//! Line lexer. The DSL is line based: every statement fits on one line
//! (strings escape their newlines), so a lexing error only loses its line.

/// A token kind.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tok {
    /// A bare word: `[alpha_][alnum_.]*`.
    Ident(String),
    /// A quoted string, unescaped.
    Str(String),
    /// A number literal as written (no sign). `float` when it has a `.` or an
    /// exponent.
    Num { text: String, float: bool },
    /// A symbol (`->`, `+=`, `(`, …).
    Sym(&'static str),
    /// `@<id>`: a reference by id.
    At(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Token {
    pub tok: Tok,
    /// 1-based column, in characters.
    pub col: usize,
}

/// A lexed line: its tokens, plus the `#@<id>` annotation if any.
#[derive(Clone, Debug, Default)]
pub(crate) struct Line {
    pub tokens: Vec<Token>,
    pub annotation: Option<(String, usize)>,
}

/// Lexing error: column and message.
pub(crate) type LexError = (usize, String);

const SYMBOLS: &[&str] = &[
    "->", "+=", "-=", "*=", "/=", "==", "!=", "<=", ">=", "&&", "||", "<", ">", "!", "+", "-", "*",
    "/", "=", "(", ")", ",", "[", "]", "{", "}", ":", "$",
];

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// `true` when `s` lexes as a single identifier.
pub(crate) fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(is_ident_start) && chars.all(is_ident_char)
}

pub(crate) fn lex_line(src: &str) -> Result<Line, LexError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut line = Line::default();
    while i < chars.len() {
        let c = chars[i];
        let col = i + 1;
        if c.is_whitespace() {
            i += 1;
        } else if c == '#' {
            if chars.get(i + 1) == Some(&'@') {
                let start = i + 2;
                let mut j = start;
                while j < chars.len() && chars[j].is_ascii_alphanumeric() {
                    j += 1;
                }
                if line.annotation.is_some() {
                    return Err((col, "duplicate `#@` id annotation".into()));
                }
                line.annotation = Some((chars[start..j].iter().collect(), col));
                i = j;
                // Only a comment may follow an annotation.
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                if i < chars.len() && chars[i] != '#' {
                    return Err((i + 1, "unexpected text after the `#@` id annotation".into()));
                }
            } else {
                // Comment until the end of the line.
                break;
            }
        } else if c == '@' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_alphanumeric() {
                j += 1;
            }
            if j == i + 1 {
                return Err((col, "expected an id after `@`".into()));
            }
            line.tokens.push(Token {
                tok: Tok::At(chars[i + 1..j].iter().collect()),
                col,
            });
            i = j;
        } else if c == '"' {
            let (s, next) = lex_string(&chars, i)?;
            line.tokens.push(Token {
                tok: Tok::Str(s),
                col,
            });
            i = next;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let mut float = false;
            if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                float = true;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    float = true;
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            if i < chars.len() && is_ident_char(chars[i]) {
                return Err((col, "malformed number".into()));
            }
            line.tokens.push(Token {
                tok: Tok::Num {
                    text: chars[start..i].iter().collect(),
                    float,
                },
                col,
            });
        } else if is_ident_start(c) {
            let start = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            line.tokens.push(Token {
                tok: Tok::Ident(chars[start..i].iter().collect()),
                col,
            });
        } else if let Some(sym) = SYMBOLS.iter().find(|s| {
            let n = s.chars().count();
            i + n <= chars.len() && chars[i..i + n].iter().copied().eq(s.chars())
        }) {
            line.tokens.push(Token {
                tok: Tok::Sym(sym),
                col,
            });
            i += sym.chars().count();
        } else {
            return Err((col, format!("unexpected character `{c}`")));
        }
    }
    Ok(line)
}

fn lex_string(chars: &[char], open: usize) -> Result<(String, usize), LexError> {
    let mut i = open + 1;
    let mut s = String::new();
    loop {
        let Some(&c) = chars.get(i) else {
            return Err((open + 1, "unterminated string".into()));
        };
        i += 1;
        match c {
            '"' => return Ok((s, i)),
            '\\' => {
                let Some(&e) = chars.get(i) else {
                    return Err((open + 1, "unterminated string".into()));
                };
                i += 1;
                match e {
                    'n' => s.push('\n'),
                    'r' => s.push('\r'),
                    't' => s.push('\t'),
                    '0' => s.push('\0'),
                    '"' => s.push('"'),
                    '\\' => s.push('\\'),
                    'u' => {
                        if chars.get(i) != Some(&'{') {
                            return Err((i, "expected `{` after `\\u`".into()));
                        }
                        let start = i + 1;
                        let mut j = start;
                        while j < chars.len() && chars[j] != '}' {
                            j += 1;
                        }
                        let hex: String = chars[start..j.min(chars.len())].iter().collect();
                        let ch = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
                        match (ch, chars.get(j)) {
                            (Some(ch), Some('}')) => s.push(ch),
                            _ => return Err((i - 1, "invalid `\\u{…}` escape".into())),
                        }
                        i = j + 1;
                    }
                    other => return Err((i - 1, format!("unknown escape `\\{other}`"))),
                }
            }
            c => s.push(c),
        }
    }
}

/// Quotes and escapes a string for the DSL.
pub(crate) fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_a_line_with_annotation_and_comment() {
        let l = lex_line(r#"  alice "Hi \"you\"\n" #@01ABC # note"#).unwrap();
        assert_eq!(l.tokens.len(), 2);
        assert_eq!(l.tokens[0].col, 3);
        assert_eq!(l.tokens[1].tok, Tok::Str("Hi \"you\"\n".into()));
        assert_eq!(l.annotation, Some(("01ABC".into(), 24)));
    }

    #[test]
    fn numbers_and_symbols() {
        let l = lex_line("x += 1.5e-3 -> -2").unwrap();
        let toks: Vec<_> = l.tokens.into_iter().map(|t| t.tok).collect();
        assert_eq!(
            toks,
            vec![
                Tok::Ident("x".into()),
                Tok::Sym("+="),
                Tok::Num {
                    text: "1.5e-3".into(),
                    float: true
                },
                Tok::Sym("->"),
                Tok::Sym("-"),
                Tok::Num {
                    text: "2".into(),
                    float: false
                },
            ]
        );
    }

    #[test]
    fn errors_have_columns() {
        assert_eq!(lex_line("a \"oops").unwrap_err().0, 3);
        assert_eq!(lex_line("a ~").unwrap_err().0, 3);
        assert!(lex_line("#@X y").is_err());
    }

    #[test]
    fn quote_round_trips() {
        let s = "a\"b\\c\nd\te\u{1}é";
        let l = lex_line(&quote(s)).unwrap();
        assert_eq!(l.tokens[0].tok, Tok::Str(s.into()));
    }
}
