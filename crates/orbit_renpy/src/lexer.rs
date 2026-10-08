//! Ren'Py source to a tree of logical lines, and a small tokenizer for one
//! line.
//!
//! A *logical line* runs on while a bracket or a string is open (Ren'Py
//! joins them); a line indented deeper than the previous one is its child.

/// One logical line of a `.rpy` file, with the lines indented under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// 1-based line number where the statement starts.
    pub line: usize,
    pub indent: usize,
    /// The statement, comments removed (or the comment itself, `#` kept,
    /// when comments are kept).
    pub text: String,
    pub children: Vec<Line>,
}

/// Splits `source` into logical lines and builds the indentation tree.
/// `keep_comments`: comment-only lines are kept as `# ...` lines (the
/// translation files carry the original statement in a comment).
pub fn parse_tree(source: &str, keep_comments: bool) -> Vec<Line> {
    let flat = logical_lines(source, keep_comments);
    let mut roots: Vec<Line> = Vec::new();
    // Stack of (indent, path) to the open parents.
    let mut stack: Vec<usize> = Vec::new();
    for line in flat {
        while let Some(&indent) = stack.last() {
            if indent >= line.indent {
                stack.pop();
            } else {
                break;
            }
        }
        let mut level = &mut roots;
        for _ in 0..stack.len() {
            level = &mut level.last_mut().expect("open parent").children;
        }
        stack.push(line.indent);
        level.push(line);
    }
    roots
}

fn logical_lines(source: &str, keep_comments: bool) -> Vec<Line> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut out = Vec::new();
    let mut lines = source.lines().enumerate().peekable();
    while let Some((index, raw)) = lines.next() {
        let indent = raw
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .map(|c| if c == '\t' { 4 } else { 1 })
            .sum::<usize>();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            if keep_comments {
                out.push(Line {
                    line: index + 1,
                    indent,
                    text: trimmed.to_owned(),
                    children: Vec::new(),
                });
            }
            continue;
        }
        let mut text = String::new();
        let mut state = ScanState::default();
        let mut current = raw.trim_start().to_owned();
        loop {
            let (kept, open) = scan(&current, &mut state);
            text.push_str(&kept);
            if !open {
                break;
            }
            match lines.next() {
                Some((_, next)) => {
                    // A string spanning lines: Ren'Py joins with one space.
                    text.push(' ');
                    current = next.trim().to_owned();
                }
                None => break,
            }
        }
        let text = text.trim().to_owned();
        if !text.is_empty() {
            out.push(Line {
                line: index + 1,
                indent,
                text,
                children: Vec::new(),
            });
        }
    }
    out
}

#[derive(Default)]
struct ScanState {
    depth: i32,
    /// Open quote: the quote char and whether it is a triple quote.
    quote: Option<(char, bool)>,
}

/// Copies `line` without its trailing comment; returns whether the logical
/// line goes on (an open bracket or string).
fn scan(line: &str, st: &mut ScanState) -> (String, bool) {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match st.quote {
            Some((q, triple)) => {
                if c == '\\' && i + 1 < chars.len() {
                    out.push(c);
                    out.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if c == q {
                    if !triple {
                        st.quote = None;
                    } else if chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q) {
                        st.quote = None;
                        out.push(c);
                        out.push(c);
                        i += 2;
                    }
                }
                out.push(c);
            }
            None => match c {
                '#' => break,
                '"' | '\'' | '`' => {
                    let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
                    st.quote = Some((c, triple));
                    out.push(c);
                    if triple {
                        out.push(c);
                        out.push(c);
                        i += 2;
                    }
                }
                '(' | '[' | '{' => {
                    st.depth += 1;
                    out.push(c);
                }
                ')' | ']' | '}' => {
                    st.depth -= 1;
                    out.push(c);
                }
                _ => out.push(c),
            },
        }
        i += 1;
    }
    let open = st.quote.is_some() || st.depth > 0;
    (out, open)
}

/// A token of one logical line.
#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// Name, possibly dotted (`persistent.seen`, `.local`, `renpy.pause`).
    Ident(String),
    /// String literal, unescaped.
    Str(String),
    Num(String),
    /// Operator or punctuation (`==`, `+=`, `(`, `:`, ...).
    Op(String),
}

impl Tok {
    pub fn ident(&self) -> Option<&str> {
        match self {
            Tok::Ident(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_op(&self, op: &str) -> bool {
        matches!(self, Tok::Op(o) if o == op)
    }

    pub fn is_word(&self, word: &str) -> bool {
        matches!(self, Tok::Ident(o) if o == word)
    }
}

const OPS: [&str; 22] = [
    "**", "//", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "<", ">", "=", "+", "-", "*", "/",
    "%", "(", ")", ",", ":",
];

/// Tokenizes a logical line. Unknown characters become one-char operators.
pub fn tokenize(text: &str) -> Vec<Tok> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // String prefixes (r"", u"", f"") are dropped.
        let prefixed = matches!(c, 'r' | 'u' | 'f' | 'R' | 'U' | 'F')
            && matches!(chars.get(i + 1), Some('"' | '\''));
        if prefixed {
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
            let mut j = i + if triple { 3 } else { 1 };
            let mut s = String::new();
            while j < chars.len() {
                let d = chars[j];
                if d == '\\' && j + 1 < chars.len() {
                    match chars[j + 1] {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        other => s.push(other),
                    }
                    j += 2;
                    continue;
                }
                if d == c
                    && (!triple || (chars.get(j + 1) == Some(&c) && chars.get(j + 2) == Some(&c)))
                {
                    j += if triple { 3 } else { 1 };
                    break;
                }
                s.push(d);
                j += 1;
            }
            out.push(Tok::Str(collapse_spaces(&s)));
            i = j;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.') {
                j += 1;
            }
            out.push(Tok::Num(chars[i..j].iter().collect()));
            i = j;
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '.' {
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '.')
            {
                j += 1;
            }
            out.push(Tok::Ident(chars[i..j].iter().collect()));
            i = j;
            continue;
        }
        let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
        if let Some(op) = OPS.iter().find(|op| rest.starts_with(**op)) {
            out.push(Tok::Op((*op).to_owned()));
            i += op.chars().count();
        } else {
            out.push(Tok::Op(c.to_string()));
            i += 1;
        }
    }
    out
}

/// Ren'Py collapses runs of whitespace (a string spanning lines) to one space.
fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c == ' ' || c == '\r' {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

/// Splits tokens on top-level commas (inside one pair of brackets).
pub fn split_args(toks: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Op(o) if o == "(" || o == "[" || o == "{" => depth += 1,
            Tok::Op(o) if o == ")" || o == "]" || o == "}" => depth -= 1,
            Tok::Op(o) if o == "," && depth == 0 => {
                out.push(&toks[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < toks.len() {
        out.push(&toks[start..]);
    }
    out
}

/// Index of the bracket closing the one opened at `open`.
pub fn matching(toks: &[Tok], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, t) in toks.iter().enumerate().skip(open) {
        match t {
            Tok::Op(o) if o == "(" || o == "[" || o == "{" => depth += 1,
            Tok::Op(o) if o == ")" || o == "]" || o == "}" => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_and_continuations() {
        let src = "label start:\n    e \"Hi\" # c\n    menu:\n        \"A\":\n            jump a\n    $ x = (1 +\n        2)\n# top\nlabel a:\n    return\n";
        let tree = parse_tree(src, false);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].children.len(), 3);
        assert_eq!(tree[0].children[0].text, "e \"Hi\"");
        assert_eq!(tree[0].children[1].children[0].children[0].text, "jump a");
        assert_eq!(tree[0].children[2].text, "$ x = (1 + 2)");
    }

    #[test]
    fn tokens() {
        let t = tokenize(r#"e happy "Say \"hi\"" with dissolve"#);
        assert_eq!(t[0], Tok::Ident("e".into()));
        assert_eq!(t[2], Tok::Str("Say \"hi\"".into()));
        let t = tokenize("$ persistent.seen += 1.5");
        assert_eq!(t[1], Tok::Ident("persistent.seen".into()));
        assert_eq!(t[2], Tok::Op("+=".into()));
        assert_eq!(t[3], Tok::Num("1.5".into()));
    }
}
