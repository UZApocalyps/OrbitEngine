//! Ren'Py text tags to the engine markup (`orbit_text`): `{b}`, `{i}`,
//! `{color}` and `{w=…}` exist on both sides; the rest is dropped (content
//! kept) and reported.

/// What the conversion of one text met.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TextNotes {
    /// Tags with no engine equivalent (dropped, their content kept).
    pub dropped_tags: Vec<String>,
    /// `[var]` interpolations, kept as written (the engine has none).
    pub interpolations: Vec<String>,
}

/// Converts a Ren'Py dialogue string.
pub fn convert(text: &str, notes: &mut TextNotes) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' && chars.get(i + 1) == Some(&'{') {
            out.push_str("{{");
            i += 2;
            continue;
        }
        if c == '[' && chars.get(i + 1) == Some(&'[') {
            out.push('[');
            i += 2;
            continue;
        }
        if c == '%' && chars.get(i + 1) == Some(&'%') {
            out.push('%');
            i += 2;
            continue;
        }
        if c == '['
            && let Some(end) = chars[i..].iter().position(|c| *c == ']')
        {
            let var: String = chars[i + 1..i + end].iter().collect();
            notes.interpolations.push(var);
            out.extend(&chars[i..=i + end]);
            i += end + 1;
            continue;
        }
        if c == '{'
            && let Some(end) = chars[i..].iter().position(|c| *c == '}')
        {
            let tag: String = chars[i + 1..i + end].iter().collect();
            out.push_str(&convert_tag(&tag, notes));
            i += end + 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn convert_tag(tag: &str, notes: &mut TextNotes) -> String {
    let (name, value) = match tag.split_once('=') {
        Some((n, v)) => (n.trim(), Some(v.trim())),
        None => (tag.trim(), None),
    };
    match (name, value) {
        ("b" | "/b" | "i" | "/i" | "/color", None) => format!("{{{name}}}"),
        ("color", Some(v)) => format!("{{color={v}}}"),
        ("w", None) => "{w=0.5}".to_owned(),
        ("w", Some(v)) => format!("{{w={v}}}"),
        ("p", None) => "\n".to_owned(),
        ("p", Some(v)) => format!("{{w={v}}}\n"),
        // Display-speed hints: the engine typewriter has its own settings.
        ("nw" | "fast", None) => String::new(),
        _ if name.starts_with('#') => String::new(),
        _ => {
            let base = name.trim_start_matches('/');
            if !notes.dropped_tags.iter().any(|t| t == base) {
                notes.dropped_tags.push(base.to_owned());
            }
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        let mut n = TextNotes::default();
        assert_eq!(
            convert(
                "{b}Hi{/b} {color=#f00}red{/color}{w} [name]{p}{size=+4}x{/size}",
                &mut n
            ),
            "{b}Hi{/b} {color=#f00}red{/color}{w=0.5} [name]\nx"
        );
        assert_eq!(n.dropped_tags, vec!["size".to_owned()]);
        assert_eq!(n.interpolations, vec!["name".to_owned()]);
        let mut n = TextNotes::default();
        assert_eq!(convert("{{a}} [[b] 100%%", &mut n), "{{a}} [b] 100%");
        assert!(n.interpolations.is_empty());
    }
}
