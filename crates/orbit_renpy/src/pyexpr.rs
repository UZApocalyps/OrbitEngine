//! Python expressions (the simple subset used by Ren'Py scripts) to
//! OrbitEngine expressions and literal values.

use std::collections::BTreeMap;

use orbit_core::{Expr, ScalarType, Value};

#[cfg(test)]
use crate::lexer::tokenize;
use crate::lexer::{Tok, matching, split_args};

/// Name of a Ren'Py variable in the story: `persistent.x` -> `persistent_x`
/// (persistent variables become [`orbit_core::VarScope::Persistent`]).
pub fn var_name(name: &str) -> String {
    match name.strip_prefix("persistent.") {
        Some(rest) => format!("persistent_{}", rest.replace('.', "_")),
        None => name.replace('.', "_"),
    }
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The engine's text form of a Python expression (`and` -> `&&`, `True` ->
/// `true`, `'a'` -> `"a"`...). `None` when it uses something the engine has
/// no equivalent for (`**`, `//`, `in`, `None`, lambdas...).
pub fn to_engine_text(toks: &[Tok]) -> Option<String> {
    let mut out = String::new();
    for (i, t) in toks.iter().enumerate() {
        let piece = match t {
            Tok::Ident(w) => match w.as_str() {
                "and" => "&&".to_owned(),
                "or" => "||".to_owned(),
                "not" => "!".to_owned(),
                "True" => "true".to_owned(),
                "False" => "false".to_owned(),
                "None" | "in" | "is" | "lambda" | "if" | "else" | "for" => return None,
                "len" if toks.get(i + 1).is_some_and(|t| t.is_op("(")) => "len".to_owned(),
                other if toks.get(i + 1).is_some_and(|t| t.is_op("(")) => {
                    let _ = other;
                    return None;
                }
                other => var_name(other),
            },
            Tok::Str(s) => quote(s),
            Tok::Num(n) => n.clone(),
            Tok::Op(o) => match o.as_str() {
                "**" | "//" | "%" => return None,
                "{" => "#{".to_owned(),
                other => other.to_owned(),
            },
        };
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&piece);
    }
    Some(out)
}

/// A Python expression as an engine expression, when it maps.
#[cfg(test)]
pub fn parse_expr(source: &str) -> Option<Expr> {
    parse_expr_toks(&tokenize(source))
}

/// [`parse_expr`] on tokens.
pub fn parse_expr_toks(toks: &[Tok]) -> Option<Expr> {
    if toks.is_empty() {
        return None;
    }
    let text = to_engine_text(toks)?;
    let expr = Expr::parse(&text).ok()?;
    if matches!(expr, Expr::Rhai(_)) {
        return None;
    }
    Some(expr)
}

/// A Python expression as Rhai code reading story variables through
/// `get_var` (the fallback for conditions the engine syntax cannot hold).
#[cfg(test)]
pub fn to_rhai(source: &str, known_vars: &dyn Fn(&str) -> bool) -> String {
    to_rhai_toks(&tokenize(source), known_vars)
}

/// [`to_rhai`] on tokens.
pub fn to_rhai_toks(toks: &[Tok], known_vars: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::new();
    for (i, t) in toks.iter().enumerate() {
        let piece = match t {
            Tok::Ident(w) => match w.as_str() {
                "and" => "&&".to_owned(),
                "or" => "||".to_owned(),
                "not" => "!".to_owned(),
                "True" => "true".to_owned(),
                "False" => "false".to_owned(),
                "None" => "()".to_owned(),
                other => {
                    let name = var_name(other);
                    let call = toks.get(i + 1).is_some_and(|t| t.is_op("("));
                    if !call && known_vars(&name) {
                        format!("get_var({})", quote(&name))
                    } else {
                        name
                    }
                }
            },
            Tok::Str(s) => quote(s),
            Tok::Num(n) => n.clone(),
            Tok::Op(o) => match o.as_str() {
                "{" => "#{".to_owned(),
                other => other.to_owned(),
            },
        };
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&piece);
    }
    out
}

/// A literal Python value (`True`, `3`, `-1.5`, `"text"`, `[1, 2]`,
/// `{"a": 1}`), for `default` / `define`. Lists and dicts must hold scalars
/// of one type (ints widen to floats); an empty one is a list of text.
pub fn parse_literal(toks: &[Tok]) -> Option<Value> {
    match toks {
        [Tok::Ident(w)] if w == "True" => Some(Value::Bool(true)),
        [Tok::Ident(w)] if w == "False" => Some(Value::Bool(false)),
        [Tok::Str(s)] => Some(Value::Str(s.clone())),
        [Tok::Ident(w), Tok::Op(o), Tok::Str(s), Tok::Op(c)]
            if w == "_" && o == "(" && c == ")" =>
        {
            Some(Value::Str(s.clone()))
        }
        [Tok::Num(n)] => number(n, false),
        [Tok::Op(m), Tok::Num(n)] if m == "-" => number(n, true),
        [Tok::Op(o), .., Tok::Op(c)]
            if (o == "[" || o == "(")
                && matching(toks, 0) == Some(toks.len() - 1)
                && (c == "]" || c == ")") =>
        {
            let items = split_args(&toks[1..toks.len() - 1])
                .into_iter()
                .map(parse_literal)
                .collect::<Option<Vec<_>>>()?;
            let elem = common(&items)?;
            Some(Value::list(elem, items))
        }
        [Tok::Op(o), .., Tok::Op(c)]
            if o == "{" && c == "}" && matching(toks, 0) == Some(toks.len() - 1) =>
        {
            let mut map = BTreeMap::new();
            for entry in split_args(&toks[1..toks.len() - 1]) {
                let colon = entry.iter().position(|t| t.is_op(":"))?;
                let key = match &entry[..colon] {
                    [Tok::Str(k)] => k.clone(),
                    _ => return None,
                };
                map.insert(key, parse_literal(&entry[colon + 1..])?);
            }
            let items: Vec<Value> = map.values().cloned().collect();
            let elem = common(&items)?;
            Some(Value::map(elem, map))
        }
        _ => None,
    }
}

fn number(n: &str, negative: bool) -> Option<Value> {
    let n = n.replace('_', "");
    if let Ok(i) = n.parse::<i64>() {
        return Some(Value::Int(if negative { -i } else { i }));
    }
    let f: f64 = n.parse().ok()?;
    Some(Value::Float(if negative { -f } else { f }))
}

fn common(items: &[Value]) -> Option<ScalarType> {
    let mut ty: Option<ScalarType> = None;
    for v in items {
        let t = match v {
            Value::Bool(_) => ScalarType::Bool,
            Value::Int(_) => ScalarType::Int,
            Value::Float(_) => ScalarType::Float,
            Value::Str(_) => ScalarType::Str,
            _ => return None,
        };
        ty = match (ty, t) {
            (None, t) => Some(t),
            (Some(a), b) if a == b => Some(a),
            (Some(ScalarType::Int), ScalarType::Float)
            | (Some(ScalarType::Float), ScalarType::Int) => Some(ScalarType::Float),
            _ => return None,
        };
    }
    Some(ty.unwrap_or(ScalarType::Str))
}

/// The zero value of the type of `v` (declaration of a variable first
/// assigned in a label).
pub fn zero_like(v: &Value) -> Value {
    match v {
        Value::Bool(_) => Value::Bool(false),
        Value::Int(_) => Value::Int(0),
        Value::Float(_) => Value::Float(0.0),
        Value::Str(_) => Value::Str(String::new()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions() {
        assert!(parse_expr("points >= 2 and not met").is_some());
        assert!(parse_expr("name == 'Bob' or True").is_some());
        assert!(parse_expr("x ** 2").is_none());
        assert!(parse_expr("renpy.random.randint(1, 3)").is_none());
        assert_eq!(var_name("persistent.seen"), "persistent_seen");
    }

    #[test]
    fn literals() {
        assert_eq!(parse_literal(&tokenize("-3")), Some(Value::Int(-3)));
        assert_eq!(parse_literal(&tokenize("0.5")), Some(Value::Float(0.5)));
        assert_eq!(parse_literal(&tokenize("True")), Some(Value::Bool(true)));
        let l = parse_literal(&tokenize("[1, 2.5]")).unwrap();
        assert!(matches!(l, Value::List(ScalarType::Float, _)));
        let m = parse_literal(&tokenize("{\"a\": \"x\"}")).unwrap();
        assert!(matches!(m, Value::Map(ScalarType::Str, _)));
        assert_eq!(parse_literal(&tokenize("foo()")), None);
    }

    #[test]
    fn rhai_fallback_reads_known_variables() {
        let code = to_rhai("x // 2 == 1", &|n| n == "x");
        assert_eq!(code, "get_var(\"x\") // 2 == 1");
    }
}
