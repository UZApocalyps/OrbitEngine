//! A small typed expression language used by conditions and `SetVar`.
//!
//! The canonical form is the [`Expr`] AST (serialized as RON like the rest of
//! the IR). A text form (`route == "forest" && affection > 2`) can be parsed
//! with [`Expr::parse`] and printed with `Display`, for editors and
//! hand-written files.
//!
//! Operators, from lowest to highest precedence: `||`, `&&`, `== !=`,
//! `< <= > >=`, `+ -`, unary `! -`.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::value::{Value, ValueType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnOp {
    Not,
    Neg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
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
}

/// Expression AST.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    Lit(Value),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
}

/// Runtime evaluation failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EvalError {
    #[error("unknown variable `{0}`")]
    UnknownVariable(String),
    #[error("type mismatch: {0}")]
    Type(String),
    #[error("integer overflow")]
    Overflow,
}

/// Static typing failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct TypeError(pub String);

/// Failure of [`Expr::parse`]; `pos` is a byte offset in the source.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("parse error at {pos}: {message}")]
pub struct ParseError {
    pub pos: usize,
    pub message: String,
}

impl Expr {
    pub fn lit(v: impl Into<Value>) -> Expr {
        Expr::Lit(v.into())
    }

    pub fn var(name: impl Into<String>) -> Expr {
        Expr::Var(name.into())
    }

    pub fn negate(e: Expr) -> Expr {
        Expr::Unary(UnOp::Not, Box::new(e))
    }

    pub fn binary(op: BinOp, l: Expr, r: Expr) -> Expr {
        Expr::Binary(op, Box::new(l), Box::new(r))
    }

    /// Parses the text form. See the module docs for the grammar.
    pub fn parse(source: &str) -> Result<Expr, ParseError> {
        Parser::new(source)?.parse_all()
    }

    /// Names of every variable read by the expression (with repeats).
    pub fn variables(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_vars(&mut out);
        out
    }

    fn collect_vars<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Expr::Lit(_) => {}
            Expr::Var(v) => out.push(v),
            Expr::Unary(_, e) => e.collect_vars(out),
            Expr::Binary(_, l, r) => {
                l.collect_vars(out);
                r.collect_vars(out);
            }
        }
    }

    /// Evaluates the expression. `vars` looks a variable up by name.
    /// `&&` and `||` short-circuit.
    pub fn eval(&self, vars: &dyn Fn(&str) -> Option<Value>) -> Result<Value, EvalError> {
        match self {
            Expr::Lit(v) => Ok(v.clone()),
            Expr::Var(name) => vars(name).ok_or_else(|| EvalError::UnknownVariable(name.clone())),
            Expr::Unary(UnOp::Not, e) => Ok(Value::Bool(!as_bool(&e.eval(vars)?, "!")?)),
            Expr::Unary(UnOp::Neg, e) => match e.eval(vars)? {
                Value::Int(i) => i.checked_neg().map(Value::Int).ok_or(EvalError::Overflow),
                Value::Float(f) => Ok(Value::Float(-f)),
                other => Err(EvalError::Type(format!(
                    "`-` expects a number, got {}",
                    other.value_type()
                ))),
            },
            Expr::Binary(BinOp::And, l, r) => {
                if !as_bool(&l.eval(vars)?, "&&")? {
                    return Ok(Value::Bool(false));
                }
                Ok(Value::Bool(as_bool(&r.eval(vars)?, "&&")?))
            }
            Expr::Binary(BinOp::Or, l, r) => {
                if as_bool(&l.eval(vars)?, "||")? {
                    return Ok(Value::Bool(true));
                }
                Ok(Value::Bool(as_bool(&r.eval(vars)?, "||")?))
            }
            Expr::Binary(op, l, r) => eval_binary(*op, l.eval(vars)?, r.eval(vars)?),
        }
    }

    /// Static type of the expression. `var_type` gives the known type of a
    /// variable; `None` means "unknown", which is compatible with anything
    /// (the result is then `Ok(None)` when it cannot be determined).
    pub fn type_of(
        &self,
        var_type: &dyn Fn(&str) -> Option<ValueType>,
    ) -> Result<Option<ValueType>, TypeError> {
        match self {
            Expr::Lit(v) => Ok(Some(v.value_type())),
            Expr::Var(name) => Ok(var_type(name)),
            Expr::Unary(UnOp::Not, e) => {
                expect(
                    e.type_of(var_type)?,
                    |t| t == ValueType::Bool,
                    "`!`",
                    "bool",
                )?;
                Ok(Some(ValueType::Bool))
            }
            Expr::Unary(UnOp::Neg, e) => {
                let t = e.type_of(var_type)?;
                expect(t, ValueType::is_numeric, "`-`", "a number")?;
                Ok(t)
            }
            Expr::Binary(op, l, r) => {
                let (lt, rt) = (l.type_of(var_type)?, r.type_of(var_type)?);
                let name = format!("`{}`", op.symbol());
                match op {
                    BinOp::And | BinOp::Or => {
                        expect(lt, |t| t == ValueType::Bool, &name, "bool")?;
                        expect(rt, |t| t == ValueType::Bool, &name, "bool")?;
                        Ok(Some(ValueType::Bool))
                    }
                    BinOp::Add | BinOp::Sub => {
                        let ok = |t: ValueType| {
                            t.is_numeric() || (*op == BinOp::Add && t == ValueType::Str)
                        };
                        expect(lt, ok, &name, "numbers (or strings for `+`)")?;
                        expect(rt, ok, &name, "numbers (or strings for `+`)")?;
                        match (lt, rt) {
                            (Some(a), Some(b)) if a.is_numeric() && b.is_numeric() => {
                                Ok(Some(if a == ValueType::Int && b == ValueType::Int {
                                    ValueType::Int
                                } else {
                                    ValueType::Float
                                }))
                            }
                            (Some(a), Some(b)) if a == b => Ok(Some(a)),
                            (Some(a), Some(b)) => {
                                Err(TypeError(format!("{name} cannot mix {a} and {b}")))
                            }
                            // One side unknown: only a string operand pins the result.
                            (Some(ValueType::Str), None) | (None, Some(ValueType::Str)) => {
                                Ok(Some(ValueType::Str))
                            }
                            _ => Ok(None),
                        }
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        if let (Some(a), Some(b)) = (lt, rt)
                            && a != b
                            && !(a.is_numeric() && b.is_numeric())
                        {
                            return Err(TypeError(format!("{name} cannot compare {a} with {b}")));
                        }
                        if !matches!(op, BinOp::Eq | BinOp::Ne) {
                            let ok = |t: ValueType| t != ValueType::Bool;
                            expect(lt, ok, &name, "numbers or strings")?;
                            expect(rt, ok, &name, "numbers or strings")?;
                        }
                        Ok(Some(ValueType::Bool))
                    }
                }
            }
        }
    }
}

fn as_bool(v: &Value, op: &str) -> Result<bool, EvalError> {
    match v {
        Value::Bool(b) => Ok(*b),
        other => Err(EvalError::Type(format!(
            "`{op}` expects bool, got {}",
            other.value_type()
        ))),
    }
}

fn expect(
    t: Option<ValueType>,
    ok: impl Fn(ValueType) -> bool,
    op: &str,
    wanted: &str,
) -> Result<(), TypeError> {
    match t {
        Some(t) if !ok(t) => Err(TypeError(format!("{op} expects {wanted}, got {t}"))),
        _ => Ok(()),
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

fn eval_binary(op: BinOp, l: Value, r: Value) -> Result<Value, EvalError> {
    let (lt, rt) = (l.value_type(), r.value_type());
    match op {
        BinOp::Add | BinOp::Sub => match (&l, &r) {
            (Value::Int(a), Value::Int(b)) => {
                let res = if op == BinOp::Add {
                    a.checked_add(*b)
                } else {
                    a.checked_sub(*b)
                };
                res.map(Value::Int).ok_or(EvalError::Overflow)
            }
            (Value::Str(a), Value::Str(b)) if op == BinOp::Add => Ok(Value::Str(format!("{a}{b}"))),
            _ => match (as_f64(&l), as_f64(&r)) {
                (Some(a), Some(b)) => {
                    Ok(Value::Float(if op == BinOp::Add { a + b } else { a - b }))
                }
                _ => Err(EvalError::Type(format!(
                    "`{}` cannot combine {lt} and {rt}",
                    op.symbol()
                ))),
            },
        },
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            if lt != rt && !(lt.is_numeric() && rt.is_numeric()) {
                return Err(EvalError::Type(format!(
                    "`{}` cannot compare {lt} with {rt}",
                    op.symbol()
                )));
            }
            if !matches!(op, BinOp::Eq | BinOp::Ne) && lt == ValueType::Bool {
                return Err(EvalError::Type(format!(
                    "`{}` cannot order booleans",
                    op.symbol()
                )));
            }
            // `None` only happens for NaN: every comparison is false except `!=`.
            let ordering = l.partial_compare(&r);
            Ok(Value::Bool(match op {
                BinOp::Eq => ordering == Some(Ordering::Equal),
                BinOp::Ne => ordering != Some(Ordering::Equal),
                BinOp::Lt => ordering == Some(Ordering::Less),
                BinOp::Le => matches!(ordering, Some(Ordering::Less | Ordering::Equal)),
                BinOp::Gt => ordering == Some(Ordering::Greater),
                _ => matches!(ordering, Some(Ordering::Greater | Ordering::Equal)),
            }))
        }
        BinOp::And | BinOp::Or => unreachable!("short-circuit operators are handled by the caller"),
    }
}

// ---------------------------------------------------------------- printing

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Lit(Value::Str(s)) => {
                f.write_str("\"")?;
                for c in s.chars() {
                    match c {
                        '"' => f.write_str("\\\"")?,
                        '\\' => f.write_str("\\\\")?,
                        '\n' => f.write_str("\\n")?,
                        c => write!(f, "{c}")?,
                    }
                }
                f.write_str("\"")
            }
            Expr::Lit(Value::Float(v)) => write!(f, "{v:?}"),
            Expr::Lit(v) => write!(f, "{v}"),
            Expr::Var(name) => f.write_str(name),
            Expr::Unary(op, e) => {
                f.write_str(if *op == UnOp::Not { "!" } else { "-" })?;
                write_operand(f, e)
            }
            Expr::Binary(op, l, r) => {
                write_operand(f, l)?;
                write!(f, " {} ", op.symbol())?;
                write_operand(f, r)
            }
        }
    }
}

fn write_operand(f: &mut fmt::Formatter<'_>, e: &Expr) -> fmt::Result {
    if matches!(e, Expr::Binary(..)) {
        write!(f, "({e})")
    } else {
        write!(f, "{e}")
    }
}

impl FromStr for Expr {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Expr::parse(s)
    }
}

// ----------------------------------------------------------------- parsing

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
}

struct Parser {
    toks: Vec<(usize, Tok)>,
    at: usize,
    len: usize,
}

fn perr<T>(pos: usize, message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError {
        pos,
        message: message.into(),
    })
}

fn next_char(src: &str, i: usize) -> Option<char> {
    src.get(i..).and_then(|r| r.chars().next())
}

fn tokenize(src: &str) -> Result<Vec<(usize, Tok)>, ParseError> {
    const OPS: &[&str] = &["==", "!=", "<=", ">=", "&&", "||", "<", ">", "!", "+", "-"];
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while let Some(c) = next_char(src, i) {
        if c.is_whitespace() {
            i += c.len_utf8();
        } else if c == '(' {
            out.push((i, Tok::LParen));
            i += 1;
        } else if c == ')' {
            out.push((i, Tok::RParen));
            i += 1;
        } else if c == '"' {
            let start = i;
            i += 1;
            let mut s = String::new();
            loop {
                let Some(ch) = next_char(src, i) else {
                    return perr(start, "unterminated string");
                };
                i += ch.len_utf8();
                match ch {
                    '"' => break,
                    '\\' => {
                        let Some(esc) = next_char(src, i) else {
                            return perr(start, "unterminated string");
                        };
                        i += esc.len_utf8();
                        s.push(match esc {
                            'n' => '\n',
                            '"' => '"',
                            '\\' => '\\',
                            other => return perr(i - 1, format!("unknown escape `\\{other}`")),
                        });
                    }
                    ch => s.push(ch),
                }
            }
            out.push((start, Tok::Str(s)));
        } else if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let mut is_float = false;
            if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                is_float = true;
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text = &src[start..i];
            let tok = if is_float {
                text.parse().ok().map(Tok::Float)
            } else {
                text.parse().ok().map(Tok::Int)
            };
            match tok {
                Some(t) => out.push((start, t)),
                None => return perr(start, "number out of range"),
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while let Some(ch) = next_char(src, i) {
                if ch.is_alphanumeric() || ch == '_' || ch == '.' {
                    i += ch.len_utf8();
                } else {
                    break;
                }
            }
            out.push((start, Tok::Ident(src[start..i].to_owned())));
        } else if let Some(op) = OPS.iter().find(|op| src[i..].starts_with(**op)) {
            out.push((i, Tok::Op(op)));
            i += op.len();
        } else {
            return perr(i, format!("unexpected character `{c}`"));
        }
    }
    Ok(out)
}

impl Parser {
    fn new(src: &str) -> Result<Self, ParseError> {
        Ok(Self {
            toks: tokenize(src)?,
            at: 0,
            len: src.len(),
        })
    }

    fn pos(&self) -> usize {
        self.toks.get(self.at).map_or(self.len, |(p, _)| *p)
    }

    fn peek_op(&self, ops: &[&'static str]) -> Option<&'static str> {
        match self.toks.get(self.at) {
            Some((_, Tok::Op(o))) if ops.contains(o) => Some(o),
            _ => None,
        }
    }

    fn parse_all(mut self) -> Result<Expr, ParseError> {
        if self.toks.is_empty() {
            return perr(0, "empty expression");
        }
        let e = self.or()?;
        if self.at < self.toks.len() {
            return perr(self.pos(), "unexpected trailing input");
        }
        Ok(e)
    }

    fn binary_level(
        &mut self,
        ops: &[&'static str],
        next: fn(&mut Self) -> Result<Expr, ParseError>,
    ) -> Result<Expr, ParseError> {
        let mut lhs = next(self)?;
        while let Some(op) = self.peek_op(ops) {
            self.at += 1;
            let rhs = next(self)?;
            lhs = Expr::binary(bin_op(op), lhs, rhs);
        }
        Ok(lhs)
    }

    fn or(&mut self) -> Result<Expr, ParseError> {
        self.binary_level(&["||"], Self::and)
    }
    fn and(&mut self) -> Result<Expr, ParseError> {
        self.binary_level(&["&&"], Self::equality)
    }
    fn equality(&mut self) -> Result<Expr, ParseError> {
        self.binary_level(&["==", "!="], Self::comparison)
    }
    fn comparison(&mut self) -> Result<Expr, ParseError> {
        self.binary_level(&["<", "<=", ">", ">="], Self::additive)
    }
    fn additive(&mut self) -> Result<Expr, ParseError> {
        self.binary_level(&["+", "-"], Self::unary)
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        let pos = self.pos();
        if self.peek_op(&["!"]).is_some() {
            self.at += 1;
            return Ok(Expr::negate(self.unary()?));
        }
        if self.peek_op(&["-"]).is_some() {
            self.at += 1;
            // A minus directly on a numeric literal folds into the literal, so
            // that `-3` parses to `Lit(Int(-3))`.
            return Ok(match self.unary()? {
                Expr::Lit(Value::Int(i)) => match i.checked_neg() {
                    Some(n) => Expr::Lit(Value::Int(n)),
                    None => return perr(pos, "number out of range"),
                },
                Expr::Lit(Value::Float(f)) => Expr::Lit(Value::Float(-f)),
                other => Expr::Unary(UnOp::Neg, Box::new(other)),
            });
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        let pos = self.pos();
        let Some((_, tok)) = self.toks.get(self.at).cloned() else {
            return perr(pos, "unexpected end of expression");
        };
        self.at += 1;
        match tok {
            Tok::Int(i) => Ok(Expr::Lit(Value::Int(i))),
            Tok::Float(f) => Ok(Expr::Lit(Value::Float(f))),
            Tok::Str(s) => Ok(Expr::Lit(Value::Str(s))),
            Tok::Ident(name) => Ok(match name.as_str() {
                "true" => Expr::Lit(Value::Bool(true)),
                "false" => Expr::Lit(Value::Bool(false)),
                _ => Expr::Var(name),
            }),
            Tok::LParen => {
                let e = self.or()?;
                match self.toks.get(self.at) {
                    Some((_, Tok::RParen)) => {
                        self.at += 1;
                        Ok(e)
                    }
                    _ => perr(self.pos(), "expected `)`"),
                }
            }
            Tok::RParen => perr(pos, "unexpected `)`"),
            Tok::Op(o) => perr(pos, format!("unexpected operator `{o}`")),
        }
    }
}

fn bin_op(sym: &str) -> BinOp {
    match sym {
        "+" => BinOp::Add,
        "-" => BinOp::Sub,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str, vars: &[(&str, Value)]) -> Result<Value, EvalError> {
        let e = Expr::parse(src).unwrap();
        e.eval(&|n| vars.iter().find(|(k, _)| *k == n).map(|(_, v)| v.clone()))
    }

    #[test]
    fn precedence_and_arithmetic() {
        assert_eq!(eval("1 + 2 - 3", &[]), Ok(Value::Int(0)));
        assert_eq!(eval("1 + 2 == 3 && !false", &[]), Ok(Value::Bool(true)));
        assert_eq!(eval("true || false && false", &[]), Ok(Value::Bool(true)));
        assert_eq!(eval("-2 + 5", &[]), Ok(Value::Int(3)));
        assert_eq!(eval("1 + 0.5", &[]), Ok(Value::Float(1.5)));
        assert_eq!(eval("\"a\" + \"b\" == \"ab\"", &[]), Ok(Value::Bool(true)));
        assert_eq!(
            eval("2 < 3 && 3 >= 3 && 4 > 3 && 3 <= 3 && 1 != 2", &[]),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn variables_and_short_circuit() {
        let vars = [("n", Value::Int(3))];
        assert_eq!(eval("n > 2", &vars), Ok(Value::Bool(true)));
        // The right-hand side is never evaluated.
        assert_eq!(eval("false && missing", &vars), Ok(Value::Bool(false)));
        assert_eq!(eval("true || missing", &vars), Ok(Value::Bool(true)));
        assert_eq!(
            eval("missing", &vars),
            Err(EvalError::UnknownVariable("missing".into()))
        );
    }

    #[test]
    fn runtime_type_errors() {
        assert!(matches!(eval("1 + true", &[]), Err(EvalError::Type(_))));
        assert!(matches!(eval("1 == \"1\"", &[]), Err(EvalError::Type(_))));
        assert!(matches!(eval("!1", &[]), Err(EvalError::Type(_))));
        assert!(matches!(eval("true < false", &[]), Err(EvalError::Type(_))));
        assert_eq!(
            eval("9223372036854775807 + 1", &[]),
            Err(EvalError::Overflow)
        );
    }

    #[test]
    fn display_roundtrips() {
        for src in [
            "route == \"forest\"",
            "(a + b) - c",
            "!(x && y) || z",
            "-n < 3.5",
            "name != \"a \\\"q\\\" b\"",
            "-(a + 1)",
        ] {
            let e = Expr::parse(src).unwrap();
            let printed = e.to_string();
            assert_eq!(Expr::parse(&printed).unwrap(), e, "{src} -> {printed}");
        }
    }

    #[test]
    fn parse_errors_have_positions() {
        for bad in ["", "1 +", "(1", "1 )", "a $ b", "\"x", "1 2", "&& 1"] {
            assert!(Expr::parse(bad).is_err(), "{bad:?} should fail");
        }
        assert_eq!(Expr::parse("1 + $").unwrap_err().pos, 4);
    }

    #[test]
    fn static_typing() {
        let types = |n: &str| match n {
            "i" => Some(ValueType::Int),
            "s" => Some(ValueType::Str),
            "b" => Some(ValueType::Bool),
            _ => None,
        };
        let ty = |src: &str| Expr::parse(src).unwrap().type_of(&types);
        assert_eq!(ty("i + 1"), Ok(Some(ValueType::Int)));
        assert_eq!(ty("i + 1.5"), Ok(Some(ValueType::Float)));
        assert_eq!(ty("s == \"x\" && b"), Ok(Some(ValueType::Bool)));
        assert_eq!(ty("unknown + 1"), Ok(None));
        assert_eq!(ty("unknown == 1"), Ok(Some(ValueType::Bool)));
        assert!(ty("s + 1").is_err());
        assert!(ty("i && b").is_err());
        assert!(ty("s == 1").is_err());
        assert!(ty("b < b").is_err());
        assert!(ty("!i").is_err());
    }
}
