//! Expressions in the DSL.
//!
//! The surface syntax is the one of [`orbit_core::Expr::parse`] (`||`, `&&`,
//! `== !=`, `< <= > >=`, `+ -`, `* /`, unary `! -`), but printing and parsing
//! are exact inverses on every finite expression tree, which the core text
//! form does not guarantee: a negation of a literal is printed `-(3)` (the
//! parser only folds a minus written directly before a number), floats keep
//! their exponent (`1e-7`), and `i64::MIN` is accepted.

use orbit_core::{BinOp, Expr, Func, ScalarType, UnOp, Value, fold_list, fold_map};

use crate::lexer::{Tok, Token, quote};

// ---------------------------------------------------------------- printing

pub(crate) fn fmt_f64(v: f64) -> String {
    format!("{v:?}")
}

pub(crate) fn fmt_f32(v: f32) -> String {
    format!("{v:?}")
}

fn symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}

pub(crate) fn print_expr(e: &Expr) -> String {
    let mut out = String::new();
    write_expr(&mut out, e);
    out
}

fn write_expr(out: &mut String, e: &Expr) {
    match e {
        Expr::Lit(Value::Bool(b)) => out.push_str(if *b { "true" } else { "false" }),
        Expr::Lit(Value::Int(i)) => out.push_str(&i.to_string()),
        Expr::Lit(Value::Float(f)) => out.push_str(&fmt_f64(*f)),
        Expr::Lit(Value::Str(s)) => out.push_str(&quote(s)),
        Expr::Lit(Value::List(t, l)) if l.is_empty() => {
            out.push_str(&format!("[:{}]", t.keyword()));
        }
        Expr::Lit(Value::Map(t, m)) if m.is_empty() => {
            out.push_str(&format!("#{{:{}}}", t.keyword()));
        }
        Expr::Lit(Value::List(_, l)) => {
            out.push('[');
            for (i, v) in l.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_expr(out, &Expr::Lit(v.clone()));
            }
            out.push(']');
        }
        Expr::Lit(Value::Map(_, m)) => {
            out.push_str("#{");
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&quote(k));
                out.push_str(": ");
                write_expr(out, &Expr::Lit(v.clone()));
            }
            out.push('}');
        }
        Expr::List(items) => {
            out.push('[');
            for (i, e) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_expr(out, e);
            }
            out.push(']');
        }
        Expr::Map(entries) => {
            out.push_str("#{");
            for (i, (k, e)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&quote(k));
                out.push_str(": ");
                write_expr(out, e);
            }
            out.push('}');
        }
        Expr::Index(c, i) => {
            let bare = matches!(
                **c,
                Expr::Var(_) | Expr::Call(..) | Expr::Index(..) | Expr::List(_) | Expr::Map(_)
            );
            write_operand(out, c, !bare);
            out.push('[');
            write_expr(out, i);
            out.push(']');
        }
        Expr::Call(f, args) => {
            out.push_str(f.name());
            out.push('(');
            for (i, e) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_expr(out, e);
            }
            out.push(')');
        }
        Expr::Var(name) => out.push_str(name),
        Expr::Unary(op, inner) => {
            out.push(if *op == UnOp::Not { '!' } else { '-' });
            // `-3` would parse back as the literal -3: protect a
            // non-negative number behind a negation.
            let protect = match &**inner {
                Expr::Binary(..) => true,
                Expr::Lit(Value::Int(i)) => *op == UnOp::Neg && *i >= 0,
                Expr::Lit(Value::Float(f)) => *op == UnOp::Neg && f.is_sign_positive(),
                _ => false,
            };
            write_operand(out, inner, protect);
        }
        Expr::Binary(op, l, r) => {
            write_operand(out, l, matches!(**l, Expr::Binary(..)));
            out.push(' ');
            out.push_str(symbol(*op));
            out.push(' ');
            write_operand(out, r, matches!(**r, Expr::Binary(..)));
        }
    }
}

fn write_operand(out: &mut String, e: &Expr, paren: bool) {
    if paren {
        out.push('(');
        write_expr(out, e);
        out.push(')');
    } else {
        write_expr(out, e);
    }
}

// ----------------------------------------------------------------- parsing

/// Parse failure: 1-based column and message.
pub(crate) type ExprError = (usize, String);

/// Parses an expression from `tokens[*at..]`, stopping at the first token
/// that cannot continue it (e.g. `->`). `end_col` is reported for errors at
/// the end of the line.
pub(crate) fn parse_expr(
    tokens: &[Token],
    at: &mut usize,
    end_col: usize,
) -> Result<Expr, ExprError> {
    let mut p = P {
        tokens,
        at: *at,
        end_col,
    };
    let e = p.or()?;
    *at = p.at;
    Ok(e)
}

struct P<'a> {
    tokens: &'a [Token],
    at: usize,
    end_col: usize,
}

const LEVELS: &[&[&str]] = &[
    &["||"],
    &["&&"],
    &["==", "!="],
    &["<", "<=", ">", ">="],
    &["+", "-"],
    &["*", "/"],
];

fn bin_op(sym: &str) -> BinOp {
    match sym {
        "+" => BinOp::Add,
        "-" => BinOp::Sub,
        "*" => BinOp::Mul,
        "/" => BinOp::Div,
        "==" => BinOp::Eq,
        "!=" => BinOp::Ne,
        "<" => BinOp::Lt,
        "<=" => BinOp::Le,
        ">" => BinOp::Gt,
        ">=" => BinOp::Ge,
        "&&" => BinOp::And,
        _ => BinOp::Or,
    }
}

impl P<'_> {
    fn col(&self) -> usize {
        self.tokens.get(self.at).map_or(self.end_col, |t| t.col)
    }

    fn peek_sym(&self, syms: &[&str]) -> Option<&'static str> {
        match self.tokens.get(self.at).map(|t| &t.tok) {
            Some(Tok::Sym(s)) if syms.contains(s) => Some(s),
            _ => None,
        }
    }

    fn or(&mut self) -> Result<Expr, ExprError> {
        self.level(0)
    }

    fn level(&mut self, n: usize) -> Result<Expr, ExprError> {
        if n == LEVELS.len() {
            return self.unary();
        }
        let mut lhs = self.level(n + 1)?;
        while let Some(op) = self.peek_sym(LEVELS[n]) {
            self.at += 1;
            let rhs = self.level(n + 1)?;
            lhs = Expr::binary(bin_op(op), lhs, rhs);
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, ExprError> {
        if self.peek_sym(&["!"]).is_some() {
            self.at += 1;
            return Ok(Expr::negate(self.unary()?));
        }
        if self.peek_sym(&["-"]).is_some() {
            self.at += 1;
            // A minus written directly before a number is part of the literal.
            if let Some(Token {
                tok: Tok::Num { text, float },
                col,
            }) = self.tokens.get(self.at)
            {
                self.at += 1;
                return number(text, *float, true, *col).map(Expr::Lit);
            }
            return Ok(Expr::Unary(UnOp::Neg, Box::new(self.unary()?)));
        }
        self.postfix()
    }

    fn eat(&mut self, sym: &str) -> bool {
        let found = self.peek_sym(&[sym]).is_some();
        if found {
            self.at += 1;
        }
        found
    }

    fn expect(&mut self, sym: &str) -> Result<(), ExprError> {
        if self.eat(sym) {
            Ok(())
        } else {
            Err((self.col(), format!("expected `{sym}`")))
        }
    }

    fn postfix(&mut self) -> Result<Expr, ExprError> {
        let mut e = self.primary()?;
        while self.eat("[") {
            let index = self.or()?;
            self.expect("]")?;
            e = Expr::index(e, index);
        }
        Ok(e)
    }

    /// `:type` right after an opening bracket: a typed empty literal.
    fn typed_empty(&mut self) -> Result<Option<ScalarType>, ExprError> {
        if !self.eat(":") {
            return Ok(None);
        }
        let col = self.col();
        match self.tokens.get(self.at).map(|t| &t.tok) {
            Some(Tok::Ident(name)) => match ScalarType::from_keyword(name) {
                Some(t) => {
                    self.at += 1;
                    Ok(Some(t))
                }
                None => Err((col, format!("unknown element type `{name}`"))),
            },
            _ => Err((col, "expected an element type (bool, int, float, string)".into())),
        }
    }

    /// Comma-separated items until `close` (already past the opening).
    fn items<T>(
        &mut self,
        close: &str,
        mut item: impl FnMut(&mut Self) -> Result<T, ExprError>,
    ) -> Result<Vec<T>, ExprError> {
        let mut out = Vec::new();
        if self.eat(close) {
            return Ok(out);
        }
        loop {
            out.push(item(self)?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(close)?;
        Ok(out)
    }

    fn primary(&mut self) -> Result<Expr, ExprError> {
        let col = self.col();
        let Some(tok) = self.tokens.get(self.at) else {
            return Err((col, "expected an expression".into()));
        };
        self.at += 1;
        match &tok.tok {
            Tok::Num { text, float } => number(text, *float, false, col).map(Expr::Lit),
            Tok::Str(s) => Ok(Expr::Lit(Value::Str(s.clone()))),
            Tok::Ident(name) if self.peek_sym(&["("]).is_some() => {
                let Some(func) = Func::from_name(name) else {
                    return Err((col, format!("unknown function `{name}`")));
                };
                self.at += 1;
                let args = self.items(")", Self::or)?;
                if args.len() != func.arity() {
                    return Err((
                        col,
                        format!("`{name}` takes {} argument(s), got {}", func.arity(), args.len()),
                    ));
                }
                Ok(Expr::Call(func, args))
            }
            Tok::Sym("[") => {
                if let Some(t) = self.typed_empty()? {
                    self.expect("]")?;
                    return Ok(Expr::Lit(Value::empty_list(t)));
                }
                Ok(fold_list(self.items("]", Self::or)?))
            }
            Tok::Sym("#{") => {
                if let Some(t) = self.typed_empty()? {
                    self.expect("}")?;
                    return Ok(Expr::Lit(Value::empty_map(t)));
                }
                let entries = self.items("}", |p| {
                    let kcol = p.col();
                    let key = match p.tokens.get(p.at).map(|t| &t.tok) {
                        Some(Tok::Str(k)) => k.clone(),
                        _ => return Err((kcol, "expected a text key (\"key\")".to_owned())),
                    };
                    p.at += 1;
                    p.expect(":")?;
                    Ok((key, kcol, p.or()?))
                })?;
                let mut out: Vec<(String, Expr)> = Vec::with_capacity(entries.len());
                for (k, kcol, e) in entries {
                    if out.iter().any(|(seen, _)| *seen == k) {
                        return Err((kcol, format!("duplicate key {}", quote(&k))));
                    }
                    out.push((k, e));
                }
                Ok(fold_map(out))
            }
            Tok::Ident(name) => Ok(match name.as_str() {
                "true" => Expr::Lit(Value::Bool(true)),
                "false" => Expr::Lit(Value::Bool(false)),
                _ => Expr::Var(name.clone()),
            }),
            Tok::Sym("(") => {
                let e = self.or()?;
                if self.peek_sym(&[")"]).is_some() {
                    self.at += 1;
                    Ok(e)
                } else {
                    Err((self.col(), "expected `)`".into()))
                }
            }
            Tok::Sym(s) => Err((col, format!("unexpected `{s}` in expression"))),
            Tok::At(_) => Err((col, "unexpected `@` reference in expression".into())),
        }
    }
}

/// A numeric literal (`negative` when a minus was written before it).
pub(crate) fn number(
    text: &str,
    float: bool,
    negative: bool,
    col: usize,
) -> Result<Value, ExprError> {
    if float {
        let signed = if negative {
            format!("-{text}")
        } else {
            text.to_owned()
        };
        return signed
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| (col, "invalid number".into()));
    }
    let magnitude: u64 = text
        .parse()
        .map_err(|_| (col, "number out of range".to_owned()))?;
    let value = if negative {
        if magnitude == 1u64 << 63 {
            i64::MIN
        } else {
            -(i64::try_from(magnitude).map_err(|_| (col, "number out of range".to_owned()))?)
        }
    } else {
        i64::try_from(magnitude).map_err(|_| (col, "number out of range".to_owned()))?
    };
    Ok(Value::Int(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex_line;

    fn round(e: &Expr) -> Expr {
        let text = print_expr(e);
        let line = lex_line(&text).unwrap();
        let mut at = 0;
        let back = parse_expr(&line.tokens, &mut at, text.len() + 1).unwrap();
        assert_eq!(at, line.tokens.len(), "{text}");
        back
    }

    #[test]
    fn tricky_literals_round_trip() {
        let cases = [
            Expr::Unary(UnOp::Neg, Box::new(Expr::lit(3i64))),
            Expr::Unary(UnOp::Neg, Box::new(Expr::lit(-3i64))),
            Expr::lit(i64::MIN),
            Expr::lit(1e-7),
            Expr::lit(-1.5e300),
            Expr::Unary(UnOp::Neg, Box::new(Expr::lit(0.0))),
            Expr::binary(BinOp::Sub, Expr::var("a"), Expr::lit(-3i64)),
            Expr::binary(
                BinOp::Add,
                Expr::var("a"),
                Expr::binary(BinOp::Add, Expr::var("b"), Expr::var("c")),
            ),
            Expr::negate(Expr::binary(BinOp::And, Expr::var("x"), Expr::lit(true))),
        ];
        for e in cases {
            assert_eq!(round(&e), e, "{}", print_expr(&e));
        }
    }

    #[test]
    fn collections_round_trip_and_match_core() {
        use orbit_core::{Func, ScalarType};
        let cases = [
            "[1, 2]",
            "[-1.5, 2e-7]",
            "[:string]",
            "#{:int}",
            "#{\"a b\": 1, \"c\": -2}",
            "[]",
            "#{}",
            "[n, 1 + 2]",
            "#{\"k\": n}",
            "bag[0]",
            "bag[i][j]",
            "-bag[0]",
            "!flags[\"x\"]",
            "(a + b)[0]",
            "len(bag) >= 2 && contains(bag, \"sword\")",
            "insert(m, \"k\", 1)",
            "remove_key(m, k)",
            "push(bag, \"x\")",
            "remove(bag, 1)",
            "remove_at(bag, 0)",
            "clear(m)",
            "has_key(m, \"k\")",
        ];
        for src in cases {
            let line = lex_line(src).unwrap();
            let mut at = 0;
            let e = parse_expr(&line.tokens, &mut at, src.len() + 1)
                .unwrap_or_else(|err| panic!("{src}: {err:?}"));
            assert_eq!(at, line.tokens.len(), "{src}");
            assert_eq!(round(&e), e, "{src}");
            // Same tree as the core parser (which has no exponent syntax).
            if !src.contains("e-") {
                assert_eq!(Expr::parse(src).unwrap(), e, "{src}");
            }
        }
        // Trees that are not produced by parsing round-trip too.
        let built = [
            Expr::Lit(Value::empty_list(ScalarType::Bool)),
            Expr::Lit(Value::map(ScalarType::Float, [("x", Value::Float(1.0))])),
            Expr::index(Expr::lit(-3i64), Expr::lit(0i64)),
            Expr::Unary(UnOp::Neg, Box::new(Expr::index(Expr::var("a"), Expr::lit(0i64)))),
            Expr::call(Func::Len, vec![Expr::List(vec![Expr::var("a")])]),
        ];
        for e in built {
            assert_eq!(round(&e), e, "{}", print_expr(&e));
        }
        for bad in ["[1, 2", "[:list]", "#{1: 2}", "#{\"a\": 1, \"a\": 2}", "len(a, b)", "nope(a)"] {
            let line = lex_line(bad).unwrap();
            let mut at = 0;
            let r = parse_expr(&line.tokens, &mut at, bad.len() + 1);
            assert!(r.is_err() || at < line.tokens.len(), "{bad:?} should fail");
        }
    }

    #[test]
    fn same_surface_as_core() {
        let src = "route == \"forest\" && (affection > 2 || !met)";
        let core = Expr::parse(src).unwrap();
        assert_eq!(round(&core), core);
        assert_eq!(Expr::parse(&print_expr(&core)).unwrap(), core);
    }
}
