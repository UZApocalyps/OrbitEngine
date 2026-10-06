//! A minimal XML tag scanner: enough for OpenRaster `stack.xml` and Krita
//! `maindoc.xml` (elements and attributes only; text, comments, doctype and
//! processing instructions are skipped).

pub struct Tag {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    /// A start tag (also true for `<a/>`).
    pub open: bool,
    /// An end tag, or a self-closing one.
    pub close: bool,
}

impl Tag {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let ent = &rest[1..end];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let n = ent
                    .strip_prefix("#x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()));
                match n.and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => out.push_str(&rest[..=end]),
                }
            }
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

pub fn tags(text: &str) -> Vec<Tag> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        if let Some(r) = rest.strip_prefix("!--") {
            rest = r.find("-->").map_or("", |e| &r[e + 3..]);
            continue;
        }
        if let Some(r) = rest.strip_prefix("![CDATA[") {
            rest = r.find("]]>").map_or("", |e| &r[e + 3..]);
            continue;
        }
        // End of the tag, honoring quoted attribute values.
        let mut quote = None;
        let mut end = rest.len();
        for (k, c) in rest.char_indices() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (None, '"' | '\'') => quote = Some(c),
                (None, '>') => {
                    end = k;
                    break;
                }
                _ => {}
            }
        }
        let body = &rest[..end];
        rest = rest.get(end + 1..).unwrap_or("");
        if body.starts_with('?') || body.starts_with('!') {
            continue;
        }
        if let Some(name) = body.strip_prefix('/') {
            out.push(Tag { name: name.trim().to_owned(), attrs: Vec::new(), open: false, close: true });
            continue;
        }
        let selfclose = body.ends_with('/');
        let body = body.trim_end_matches('/');
        let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
        let mut attrs = Vec::new();
        let mut a = body[name_end..].trim_start();
        while let Some(eq) = a.find('=') {
            let key = a[..eq].trim().to_owned();
            let v = a[eq + 1..].trim_start();
            let Some(q) = v.chars().next().filter(|c| matches!(c, '"' | '\'')) else { break };
            let Some(close) = v[1..].find(q) else { break };
            attrs.push((key, unescape(&v[1..1 + close])));
            a = v[close + 2..].trim_start();
        }
        out.push(Tag { name: body[..name_end].to_owned(), attrs, open: true, close: selfclose });
    }
    out
}
