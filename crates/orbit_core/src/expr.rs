//! A small typed expression language used by conditions and `SetVar`.
//!
//! The canonical form is the [`Expr`] AST (serialized as RON like the rest of
//! the IR). A text form (`route == "forest" && affection > 2`) can be parsed
//! with [`Expr::parse`] and printed with `Display`, for editors and
//! hand-written files.
//!
//! Operators, from lowest to highest precedence: `||`, `&&`, `== !=`,
//! `< <= > >=`, `+ -`, `* /`, unary `! -`, indexing `x[i]`.
//!
//! Collections (schema v11, ADR 0016): list literals `[1, 2]`, map literals
//! `#{"sword": 1}`, typed empty literals `[:int]` / `#{:string}` (a bare `[]`
//! or `#{}` takes the type of the variable it is stored in), indexing
//! `list[0]` / `map["key"]`, and the functions of [`Func`]
//! (`len(x)`, `contains(list, v)`, `has_key(map, k)`, `push(list, v)`...).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::value::{ScalarType, Value, ValueType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum UnOp {
    Not,
    Neg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum BinOp {
    Add,
    Sub,
    /// Added in schema v7.
    Mul,
    /// Added in schema v7. Integer division when both sides are integers.
    Div,
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
}

/// Built-in functions on collections (schema v11). The modifying ones
/// (`push`, `remove`...) return a new collection: `SetVar` stores it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Func {
    /// `len(x)`: elements of a list or map, characters of a text.
    Len,
    /// `contains(list, v)`; `contains(text, part)`.
    Contains,
    /// `has_key(map, key)`.
    HasKey,
    /// `push(list, v)`: `v` appended.
    Push,
    /// `remove(list, v)`: the first `v` removed (unchanged when absent).
    Remove,
    /// `remove_at(list, i)`: the element at `i` removed.
    RemoveAt,
    /// `insert(map, key, v)`: `key` set to `v`.
    Insert,
    /// `remove_key(map, key)`: `key` removed (unchanged when absent).
    RemoveKey,
    /// `clear(x)`: an empty collection of the same type.
    Clear,
}

impl Func {
    pub const ALL: [Func; 9] = [
        Func::Len,
        Func::Contains,
        Func::HasKey,
        Func::Push,
        Func::Remove,
        Func::RemoveAt,
        Func::Insert,
        Func::RemoveKey,
        Func::Clear,
    ];

    /// Name in the text form.
    pub fn name(self) -> &'static str {
        match self {
            Func::Len => "len",
            Func::Contains => "contains",
            Func::HasKey => "has_key",
            Func::Push => "push",
            Func::Remove => "remove",
            Func::RemoveAt => "remove_at",
            Func::Insert => "insert",
            Func::RemoveKey => "remove_key",
            Func::Clear => "clear",
        }
    }

    pub fn from_name(name: &str) -> Option<Func> {
        Func::ALL.into_iter().find(|f| f.name() == name)
    }

    /// Number of arguments.
    pub fn arity(self) -> usize {
        match self {
            Func::Len | Func::Clear => 1,
            Func::Insert => 3,
            _ => 2,
        }
    }
}

/// Expression AST.
///
/// New variants are appended at the end: game packs encode the story with
/// postcard (variant indices), so older packs decode unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Expr {
    Lit(Value),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// A list built from expressions (schema v11). Literal lists whose
    /// elements are constants of one type are [`Expr::Lit`] instead.
    List(Vec<Expr>),
    /// A map built from expressions (schema v11).
    Map(Vec<(String, Expr)>),
    /// `collection[index]` (schema v11).
    Index(Box<Expr>, Box<Expr>),
    /// A built-in function call (schema v11).
    Call(Func, Vec<Expr>),
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
    #[error("division by zero")]
    DivisionByZero,
    /// A list index outside `0..len`.
    #[error("index {index} is out of bounds (length {len})")]
    IndexOutOfBounds { index: i64, len: usize },
    /// A map key that is not there.
    #[error("no key `{0}` in the map")]
    MissingKey(String),
    /// A collection would exceed its size limits (ADR 0016).
    #[error(
        "collection too large (at most {} elements and {} bytes of text)",
        crate::value::MAX_COLLECTION_LEN,
        crate::value::MAX_COLLECTION_TEXT
    )]
    TooLarge,
}

impl EvalError {
    /// Errors that depend on the data rather than on the story's logic
    /// (division by zero, index out of bounds, missing key, size limit): a
    /// `SetVar` hitting one leaves its variable unchanged with a warning
    /// instead of stopping the game.
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            EvalError::DivisionByZero
                | EvalError::IndexOutOfBounds { .. }
                | EvalError::MissingKey(_)
                | EvalError::TooLarge
        )
    }
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

    pub fn call(f: Func, args: Vec<Expr>) -> Expr {
        Expr::Call(f, args)
    }

    pub fn index(collection: Expr, index: Expr) -> Expr {
        Expr::Index(Box::new(collection), Box::new(index))
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
            Expr::Binary(_, l, r) | Expr::Index(l, r) => {
                l.collect_vars(out);
                r.collect_vars(out);
            }
            Expr::List(items) | Expr::Call(_, items) => {
                items.iter().for_each(|e| e.collect_vars(out));
            }
            Expr::Map(entries) => entries.iter().for_each(|(_, e)| e.collect_vars(out)),
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
            Expr::List(items) => {
                let values = items
                    .iter()
                    .map(|e| e.eval(vars))
                    .collect::<Result<Vec<_>, _>>()?;
                let elem = common_scalar(values.iter())?;
                limited(Value::list(elem, values))
            }
            Expr::Map(entries) => {
                let mut map = BTreeMap::new();
                for (k, e) in entries {
                    map.insert(k.clone(), e.eval(vars)?);
                }
                let elem = common_scalar(map.values())?;
                limited(Value::map(elem, map))
            }
            Expr::Index(c, i) => match (c.eval(vars)?, i.eval(vars)?) {
                (Value::List(_, l), Value::Int(i)) => usize::try_from(i)
                    .ok()
                    .and_then(|u| l.get(u).cloned())
                    .ok_or(EvalError::IndexOutOfBounds { index: i, len: l.len() }),
                (Value::Map(_, m), Value::Str(k)) => {
                    m.get(&k).cloned().ok_or(EvalError::MissingKey(k))
                }
                (c, i) => Err(EvalError::Type(format!(
                    "cannot index a {} with a {}",
                    c.value_type(),
                    i.value_type()
                ))),
            },
            Expr::Call(f, args) => {
                if args.len() != f.arity() {
                    return Err(EvalError::Type(format!(
                        "`{}` takes {} argument(s), got {}",
                        f.name(),
                        f.arity(),
                        args.len()
                    )));
                }
                let args = args
                    .iter()
                    .map(|e| e.eval(vars))
                    .collect::<Result<Vec<_>, _>>()?;
                call(*f, args)
            }
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
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
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
                            && !ValueType::compatible(a, b)
                        {
                            return Err(TypeError(format!("{name} cannot compare {a} with {b}")));
                        }
                        if !matches!(op, BinOp::Eq | BinOp::Ne) {
                            let ok = |t: ValueType| t != ValueType::Bool && !t.is_collection();
                            expect(lt, ok, &name, "numbers or strings")?;
                            expect(rt, ok, &name, "numbers or strings")?;
                        }
                        Ok(Some(ValueType::Bool))
                    }
                }
            }
            Expr::List(items) => {
                let types = items
                    .iter()
                    .map(|e| e.type_of(var_type))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(common_type(&types, "a list")?.map(ValueType::List))
            }
            Expr::Map(entries) => {
                let types = entries
                    .iter()
                    .map(|(_, e)| e.type_of(var_type))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(common_type(&types, "a map")?.map(ValueType::Map))
            }
            Expr::Index(c, i) => {
                let (ct, it) = (c.type_of(var_type)?, i.type_of(var_type)?);
                match ct {
                    None => Ok(None),
                    Some(ValueType::List(t)) => {
                        expect(it, |t| t == ValueType::Int, "a list index", "an int")?;
                        Ok(Some(t.value_type()))
                    }
                    Some(ValueType::Map(t)) => {
                        expect(it, |t| t == ValueType::Str, "a map key", "a string")?;
                        Ok(Some(t.value_type()))
                    }
                    Some(other) => Err(TypeError(format!("cannot index a {other}"))),
                }
            }
            Expr::Call(f, args) => {
                if args.len() != f.arity() {
                    return Err(TypeError(format!(
                        "`{}` takes {} argument(s), got {}",
                        f.name(),
                        f.arity(),
                        args.len()
                    )));
                }
                let types = args
                    .iter()
                    .map(|e| e.type_of(var_type))
                    .collect::<Result<Vec<_>, _>>()?;
                call_type(*f, &types)
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

fn limited(v: Value) -> Result<Value, EvalError> {
    if v.within_limits() {
        Ok(v)
    } else {
        Err(EvalError::TooLarge)
    }
}

/// Element type of a collection built from `values`: all of one scalar type,
/// or numbers (float as soon as one is). An empty collection is `int` until
/// stored (a variable keeps its declared type, see `coerce`).
fn common_scalar<'a>(values: impl Iterator<Item = &'a Value>) -> Result<ScalarType, EvalError> {
    let mut elem: Option<ScalarType> = None;
    for v in values {
        let Some(t) = v.value_type().scalar() else {
            return Err(EvalError::Type(
                "collections cannot contain collections".to_owned(),
            ));
        };
        elem = Some(match elem {
            None => t,
            Some(e) if e == t => e,
            Some(e) if e.value_type().is_numeric() && t.value_type().is_numeric() => {
                ScalarType::Float
            }
            Some(e) => {
                return Err(EvalError::Type(format!(
                    "a collection cannot mix {e} and {t}"
                )));
            }
        });
    }
    Ok(elem.unwrap_or(ScalarType::Int))
}

/// Static counterpart of [`common_scalar`]; `Ok(None)` when unknown (empty, or
/// only unknown elements).
fn common_type(types: &[Option<ValueType>], what: &str) -> Result<Option<ScalarType>, TypeError> {
    let mut elem: Option<ScalarType> = None;
    for t in types.iter().flatten() {
        let Some(s) = t.scalar() else {
            return Err(TypeError(format!("{what} cannot contain a {t}")));
        };
        elem = Some(match elem {
            None => s,
            Some(e) if e == s => e,
            Some(e) if e.value_type().is_numeric() && s.value_type().is_numeric() => {
                ScalarType::Float
            }
            Some(e) => return Err(TypeError(format!("{what} cannot mix {e} and {s}"))),
        });
    }
    Ok(elem)
}

fn call(f: Func, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let type_err = |args: &[Value]| {
        let types: Vec<String> = args.iter().map(|a| a.value_type().to_string()).collect();
        EvalError::Type(format!("`{}` cannot take ({})", f.name(), types.join(", ")))
    };
    let elem_value = |elem: ScalarType, v: Value| {
        v.clone()
            .into_scalar(elem)
            .ok_or_else(|| EvalError::Type(format!("cannot store a {} in a list of {elem}", v.value_type())))
    };
    match (f, args.as_mut_slice()) {
        (Func::Len, [v]) => v
            .len()
            .map(|n| Value::Int(n as i64))
            .ok_or_else(|| type_err(&args)),
        (Func::Contains, [Value::List(_, l), v]) => Ok(Value::Bool(
            l.iter().any(|x| x.partial_compare(v) == Some(Ordering::Equal)),
        )),
        (Func::Contains, [Value::Str(s), Value::Str(part)]) => {
            Ok(Value::Bool(s.contains(part.as_str())))
        }
        (Func::HasKey, [Value::Map(_, m), Value::Str(k)]) => Ok(Value::Bool(m.contains_key(k))),
        (Func::Push, [Value::List(t, l), v]) => {
            let v = elem_value(*t, v.clone())?;
            Arc::make_mut(l).push(v);
            limited(args.swap_remove(0))
        }
        (Func::Remove, [Value::List(_, l), v]) => {
            if let Some(i) = l
                .iter()
                .position(|x| x.partial_compare(v) == Some(Ordering::Equal))
            {
                Arc::make_mut(l).remove(i);
            }
            Ok(args.swap_remove(0))
        }
        (Func::RemoveAt, [Value::List(_, l), Value::Int(i)]) => {
            let index = *i;
            match usize::try_from(index).ok().filter(|u| *u < l.len()) {
                Some(u) => {
                    Arc::make_mut(l).remove(u);
                    Ok(args.swap_remove(0))
                }
                None => Err(EvalError::IndexOutOfBounds { index, len: l.len() }),
            }
        }
        (Func::Insert, [Value::Map(t, m), Value::Str(k), v]) => {
            let v = elem_value(*t, v.clone())?;
            Arc::make_mut(m).insert(k.clone(), v);
            limited(args.swap_remove(0))
        }
        (Func::RemoveKey, [Value::Map(_, m), Value::Str(k)]) => {
            if m.contains_key(k.as_str()) {
                Arc::make_mut(m).remove(k.as_str());
            }
            Ok(args.swap_remove(0))
        }
        (Func::Clear, [Value::List(t, _)]) => Ok(Value::empty_list(*t)),
        (Func::Clear, [Value::Map(t, _)]) => Ok(Value::empty_map(*t)),
        _ => Err(type_err(&args)),
    }
}

fn call_type(f: Func, types: &[Option<ValueType>]) -> Result<Option<ValueType>, TypeError> {
    let name = format!("`{}`", f.name());
    let elem_fits = |coll: Option<ValueType>, v: Option<ValueType>| match (coll.and_then(ValueType::element), v) {
        (Some(e), Some(v)) if !ValueType::compatible(e, v) => Err(TypeError(format!(
            "{name} expects a {e}, got {v}"
        ))),
        _ => Ok(()),
    };
    let is_list = |t: ValueType| matches!(t, ValueType::List(_));
    let is_map = |t: ValueType| matches!(t, ValueType::Map(_));
    match f {
        Func::Len => {
            expect(types[0], |t| t.is_collection() || t == ValueType::Str, &name, "a list, a map or a string")?;
            Ok(Some(ValueType::Int))
        }
        Func::Contains => {
            match types[0] {
                Some(ValueType::Str) => expect(types[1], |t| t == ValueType::Str, &name, "a string")?,
                t => {
                    expect(t, is_list, &name, "a list or a string")?;
                    elem_fits(t, types[1])?;
                }
            }
            Ok(Some(ValueType::Bool))
        }
        Func::HasKey => {
            expect(types[0], is_map, &name, "a map")?;
            expect(types[1], |t| t == ValueType::Str, &name, "a string key")?;
            Ok(Some(ValueType::Bool))
        }
        Func::Push | Func::Remove => {
            expect(types[0], is_list, &name, "a list")?;
            elem_fits(types[0], types[1])?;
            Ok(types[0])
        }
        Func::RemoveAt => {
            expect(types[0], is_list, &name, "a list")?;
            expect(types[1], |t| t == ValueType::Int, &name, "an int index")?;
            Ok(types[0])
        }
        Func::Insert => {
            expect(types[0], is_map, &name, "a map")?;
            expect(types[1], |t| t == ValueType::Str, &name, "a string key")?;
            elem_fits(types[0], types[2])?;
            Ok(types[0])
        }
        Func::RemoveKey => {
            expect(types[0], is_map, &name, "a map")?;
            expect(types[1], |t| t == ValueType::Str, &name, "a string key")?;
            Ok(types[0])
        }
        Func::Clear => {
            expect(types[0], ValueType::is_collection, &name, "a list or a map")?;
            Ok(types[0])
        }
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
        BinOp::Mul | BinOp::Div => match (&l, &r) {
            (Value::Int(a), Value::Int(b)) => {
                if op == BinOp::Div && *b == 0 {
                    return Err(EvalError::DivisionByZero);
                }
                let res = if op == BinOp::Mul {
                    a.checked_mul(*b)
                } else {
                    a.checked_div(*b)
                };
                res.map(Value::Int).ok_or(EvalError::Overflow)
            }
            _ => match (as_f64(&l), as_f64(&r)) {
                (Some(a), Some(b)) => {
                    if op == BinOp::Div && b == 0.0 {
                        return Err(EvalError::DivisionByZero);
                    }
                    Ok(Value::Float(if op == BinOp::Mul { a * b } else { a / b }))
                }
                _ => Err(EvalError::Type(format!(
                    "`{}` expects numbers, got {lt} and {rt}",
                    op.symbol()
                ))),
            },
        },
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            // Two lists (or two maps) compare by content: an empty `[]`
            // literal equals an empty list of any element type.
            let same_kind = matches!(
                (lt, rt),
                (ValueType::List(_), ValueType::List(_)) | (ValueType::Map(_), ValueType::Map(_))
            );
            if !ValueType::compatible(lt, rt) && !same_kind {
                return Err(EvalError::Type(format!(
                    "`{}` cannot compare {lt} with {rt}",
                    op.symbol()
                )));
            }
            if !matches!(op, BinOp::Eq | BinOp::Ne) && (lt == ValueType::Bool || lt.is_collection()) {
                return Err(EvalError::Type(format!(
                    "`{}` cannot order {lt} values",
                    op.symbol()
                )));
            }
            // `None` only happens for NaN (and differing maps): every
            // comparison is false except `!=`.
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

fn write_str_lit(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
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

fn write_scalar(f: &mut fmt::Formatter<'_>, v: &Value) -> fmt::Result {
    match v {
        Value::Str(s) => write_str_lit(f, s),
        Value::Float(v) => write!(f, "{v:?}"),
        v => write!(f, "{v}"),
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Lit(Value::List(t, l)) if l.is_empty() => write!(f, "[:{}]", t.keyword()),
            Expr::Lit(Value::Map(t, m)) if m.is_empty() => write!(f, "#{{:{}}}", t.keyword()),
            Expr::Lit(Value::List(_, l)) => {
                f.write_str("[")?;
                for (i, v) in l.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write_scalar(f, v)?;
                }
                f.write_str("]")
            }
            Expr::Lit(Value::Map(_, m)) => {
                f.write_str("#{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write_str_lit(f, k)?;
                    f.write_str(": ")?;
                    write_scalar(f, v)?;
                }
                f.write_str("}")
            }
            Expr::Lit(v) => write_scalar(f, v),
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
            Expr::List(items) => {
                f.write_str("[")?;
                for (i, e) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{e}")?;
                }
                f.write_str("]")
            }
            Expr::Map(entries) => {
                f.write_str("#{")?;
                for (i, (k, e)) in entries.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write_str_lit(f, k)?;
                    write!(f, ": {e}")?;
                }
                f.write_str("}")
            }
            Expr::Index(c, i) => {
                if matches!(
                    **c,
                    Expr::Var(_) | Expr::Call(..) | Expr::Index(..) | Expr::List(_) | Expr::Map(_)
                ) {
                    write!(f, "{c}")?;
                } else {
                    write!(f, "({c})")?;
                }
                write!(f, "[{i}]")
            }
            Expr::Call(func, args) => {
                write!(f, "{}(", func.name())?;
                for (i, e) in args.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{e}")?;
                }
                f.write_str(")")
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

/// Folds a list literal whose elements are all constants of one scalar type
/// into a single [`Expr::Lit`] (the canonical form, as for `-3`).
pub fn fold_list(items: Vec<Expr>) -> Expr {
    let elem = match items.first() {
        Some(Expr::Lit(v)) => v.value_type().scalar(),
        _ => None,
    };
    match elem {
        Some(t)
            if items
                .iter()
                .all(|e| matches!(e, Expr::Lit(v) if v.value_type() == t.value_type())) =>
        {
            let values = items
                .into_iter()
                .map(|e| match e {
                    Expr::Lit(v) => v,
                    _ => unreachable!(),
                })
                .collect();
            Expr::Lit(Value::List(t, Arc::new(values)))
        }
        _ => Expr::List(items),
    }
}

/// [`fold_list`] for map literals.
pub fn fold_map(entries: Vec<(String, Expr)>) -> Expr {
    let elem = match entries.first() {
        Some((_, Expr::Lit(v))) => v.value_type().scalar(),
        _ => None,
    };
    match elem {
        Some(t)
            if entries
                .iter()
                .all(|(_, e)| matches!(e, Expr::Lit(v) if v.value_type() == t.value_type())) =>
        {
            let map = entries
                .into_iter()
                .map(|(k, e)| match e {
                    Expr::Lit(v) => (k, v),
                    _ => unreachable!(),
                })
                .collect();
            Expr::Lit(Value::Map(t, Arc::new(map)))
        }
        _ => Expr::Map(entries),
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
    const OPS: &[&str] = &[
        "==", "!=", "<=", ">=", "&&", "||", "#{", "<", ">", "!", "+", "-", "*", "/", "[", "]",
        "}", ",", ":",
    ];
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

    fn eat_op(&mut self, op: &'static str) -> bool {
        let found = self.peek_op(&[op]).is_some();
        if found {
            self.at += 1;
        }
        found
    }

    fn expect_op(&mut self, op: &'static str) -> Result<(), ParseError> {
        if self.eat_op(op) {
            Ok(())
        } else {
            perr(self.pos(), format!("expected `{op}`"))
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
        self.binary_level(&["+", "-"], Self::multiplicative)
    }
    fn multiplicative(&mut self) -> Result<Expr, ParseError> {
        self.binary_level(&["*", "/"], Self::unary)
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
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.primary()?;
        while self.eat_op("[") {
            let index = self.or()?;
            self.expect_op("]")?;
            e = Expr::index(e, index);
        }
        Ok(e)
    }

    /// `:type` after an opening bracket: a typed empty literal.
    fn typed_empty(&mut self) -> Result<Option<ScalarType>, ParseError> {
        if !self.eat_op(":") {
            return Ok(None);
        }
        let pos = self.pos();
        match self.toks.get(self.at).cloned() {
            Some((_, Tok::Ident(name))) => match ScalarType::from_keyword(&name) {
                Some(t) => {
                    self.at += 1;
                    Ok(Some(t))
                }
                None => perr(pos, format!("unknown element type `{name}`")),
            },
            _ => perr(pos, "expected an element type (bool, int, float, string)"),
        }
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
            Tok::Ident(name) if matches!(self.toks.get(self.at), Some((_, Tok::LParen))) => {
                let Some(func) = Func::from_name(&name) else {
                    return perr(pos, format!("unknown function `{name}`"));
                };
                self.at += 1;
                let mut args = Vec::new();
                if !matches!(self.toks.get(self.at), Some((_, Tok::RParen))) {
                    loop {
                        args.push(self.or()?);
                        if !self.eat_op(",") {
                            break;
                        }
                    }
                }
                match self.toks.get(self.at) {
                    Some((_, Tok::RParen)) => self.at += 1,
                    _ => return perr(self.pos(), "expected `)`"),
                }
                if args.len() != func.arity() {
                    return perr(
                        pos,
                        format!("`{name}` takes {} argument(s), got {}", func.arity(), args.len()),
                    );
                }
                Ok(Expr::Call(func, args))
            }
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
            Tok::Op("[") => {
                if let Some(t) = self.typed_empty()? {
                    self.expect_op("]")?;
                    return Ok(Expr::Lit(Value::empty_list(t)));
                }
                let mut items = Vec::new();
                if !self.eat_op("]") {
                    loop {
                        items.push(self.or()?);
                        if !self.eat_op(",") {
                            break;
                        }
                    }
                    self.expect_op("]")?;
                }
                Ok(fold_list(items))
            }
            Tok::Op("#{") => {
                if let Some(t) = self.typed_empty()? {
                    self.expect_op("}")?;
                    return Ok(Expr::Lit(Value::empty_map(t)));
                }
                let mut entries: Vec<(String, Expr)> = Vec::new();
                if !self.eat_op("}") {
                    loop {
                        let kpos = self.pos();
                        let key = match self.toks.get(self.at).cloned() {
                            Some((_, Tok::Str(k))) => k,
                            _ => return perr(kpos, "expected a text key (\"key\")"),
                        };
                        self.at += 1;
                        if entries.iter().any(|(k, _)| *k == key) {
                            return perr(kpos, format!("duplicate key \"{key}\""));
                        }
                        self.expect_op(":")?;
                        entries.push((key, self.or()?));
                        if !self.eat_op(",") {
                            break;
                        }
                    }
                    self.expect_op("}")?;
                }
                Ok(fold_map(entries))
            }
            Tok::RParen => perr(pos, "unexpected `)`"),
            Tok::Op(o) => perr(pos, format!("unexpected `{o}`")),
        }
    }
}

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
    fn multiplication_and_division() {
        assert_eq!(eval("2 + 3 * 4", &[]), Ok(Value::Int(14)));
        assert_eq!(eval("7 / 2", &[]), Ok(Value::Int(3)));
        assert_eq!(eval("7 / 2.0", &[]), Ok(Value::Float(3.5)));
        assert_eq!(eval("1 / 0", &[]), Err(EvalError::DivisionByZero));
        assert_eq!(eval("1.0 / 0.0", &[]), Err(EvalError::DivisionByZero));
        assert!(matches!(eval("\"a\" * 2", &[]), Err(EvalError::Type(_))));
        let e = Expr::parse("(a + 1) * b / 2").unwrap();
        assert_eq!(Expr::parse(&e.to_string()).unwrap(), e);
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

    fn inv() -> Vec<(&'static str, Value)> {
        vec![
            (
                "bag",
                Value::list(ScalarType::Str, [Value::from("sword"), Value::from("key")]),
            ),
            (
                "gold",
                Value::map(ScalarType::Int, [("alice", Value::Int(3)), ("bob", Value::Int(5))]),
            ),
            ("nums", Value::list(ScalarType::Float, [Value::Float(1.5)])),
            ("n", Value::Int(1)),
        ]
    }

    #[test]
    fn collection_literals_and_functions() {
        let v = inv();
        assert_eq!(eval("len(bag)", &v), Ok(Value::Int(2)));
        assert_eq!(eval("len(gold) + len(\"abc\")", &v), Ok(Value::Int(5)));
        assert_eq!(eval("contains(bag, \"key\")", &v), Ok(Value::Bool(true)));
        assert_eq!(eval("contains(bag, \"map\")", &v), Ok(Value::Bool(false)));
        assert_eq!(eval("contains(\"hello\", \"ell\")", &v), Ok(Value::Bool(true)));
        assert_eq!(eval("has_key(gold, \"bob\")", &v), Ok(Value::Bool(true)));
        assert_eq!(eval("bag[1]", &v), Ok(Value::from("key")));
        assert_eq!(eval("gold[\"alice\"] * 2", &v), Ok(Value::Int(6)));
        assert_eq!(
            eval("push(bag, \"map\")", &v),
            Ok(Value::list(ScalarType::Str, ["sword", "key", "map"].map(Value::from)))
        );
        assert_eq!(
            eval("remove(bag, \"sword\")", &v),
            Ok(Value::list(ScalarType::Str, [Value::from("key")]))
        );
        // Removing what is not there changes nothing.
        assert_eq!(eval("remove(bag, \"zz\") == bag", &v), Ok(Value::Bool(true)));
        assert_eq!(
            eval("remove_at(bag, 0)", &v),
            Ok(Value::list(ScalarType::Str, [Value::from("key")]))
        );
        assert_eq!(
            eval("insert(gold, \"carol\", 1)", &v).unwrap().len(),
            Some(3)
        );
        assert_eq!(eval("remove_key(gold, \"bob\")", &v).unwrap().len(), Some(1));
        assert_eq!(eval("clear(gold)", &v), Ok(Value::empty_map(ScalarType::Int)));
        // An int pushed in a float list becomes a float.
        assert_eq!(
            eval("push(nums, 2)", &v),
            Ok(Value::list(ScalarType::Float, [Value::Float(1.5), Value::Float(2.0)]))
        );
        assert_eq!(eval("[1, n + 1][1]", &v), Ok(Value::Int(2)));
        assert_eq!(eval("[1, 2.5]", &v), Ok(Value::list(ScalarType::Float, [Value::Float(1.0), Value::Float(2.5)])));
        assert_eq!(eval("#{\"a\": n}[\"a\"]", &v), Ok(Value::Int(1)));
        assert_eq!(eval("bag == [\"sword\", \"key\"]", &v), Ok(Value::Bool(true)));
        assert_eq!(eval("clear(bag) == []", &v), Ok(Value::Bool(true)));
        // The source is never modified (copy on write).
        assert_eq!(eval("len(bag)", &v), Ok(Value::Int(2)));
    }

    #[test]
    fn collection_errors() {
        let v = inv();
        assert_eq!(
            eval("bag[2]", &v),
            Err(EvalError::IndexOutOfBounds { index: 2, len: 2 })
        );
        assert_eq!(
            eval("bag[-1]", &v),
            Err(EvalError::IndexOutOfBounds { index: -1, len: 2 })
        );
        assert_eq!(
            eval("remove_at(bag, 5)", &v),
            Err(EvalError::IndexOutOfBounds { index: 5, len: 2 })
        );
        assert_eq!(eval("gold[\"zed\"]", &v), Err(EvalError::MissingKey("zed".into())));
        assert!(eval("gold[\"zed\"]", &v).unwrap_err().is_recoverable());
        assert!(matches!(eval("push(bag, 1)", &v), Err(EvalError::Type(_))));
        assert!(matches!(eval("[1, \"a\"]", &v), Err(EvalError::Type(_))));
        assert!(matches!(eval("[bag]", &v), Err(EvalError::Type(_))));
        assert!(matches!(eval("bag < bag", &v), Err(EvalError::Type(_))));
        assert!(matches!(eval("n[0]", &v), Err(EvalError::Type(_))));
        assert!(matches!(eval("has_key(bag, \"x\")", &v), Err(EvalError::Type(_))));
        // The size limit.
        let big = Value::list(
            ScalarType::Int,
            (0..crate::value::MAX_COLLECTION_LEN as i64).map(Value::Int),
        );
        let e = Expr::parse("push(big, 1)").unwrap();
        assert_eq!(
            e.eval(&|n| (n == "big").then(|| big.clone())),
            Err(EvalError::TooLarge)
        );
    }

    #[test]
    fn collection_parsing_and_printing() {
        // Constant literals fold into a single value.
        assert_eq!(
            Expr::parse("[1, 2]").unwrap(),
            Expr::Lit(Value::list(ScalarType::Int, [Value::Int(1), Value::Int(2)]))
        );
        assert_eq!(Expr::parse("[:string]").unwrap(), Expr::Lit(Value::empty_list(ScalarType::Str)));
        assert_eq!(Expr::parse("#{:int}").unwrap(), Expr::Lit(Value::empty_map(ScalarType::Int)));
        assert_eq!(Expr::parse("[]").unwrap(), Expr::List(vec![]));
        assert_eq!(Expr::parse("#{}").unwrap(), Expr::Map(vec![]));
        assert!(matches!(Expr::parse("[1, n]").unwrap(), Expr::List(_)));
        for src in [
            "[1, 2]",
            "[1.5, 2.0]",
            "[\"a\", \"b\\\"c\"]",
            "[:int]",
            "[:bool]",
            "#{:float}",
            "#{\"a\": 1, \"b\": 2}",
            "#{\"a\": n + 1}",
            "[]",
            "#{}",
            "[n, 2]",
            "bag[0]",
            "bag[i + 1][0]",
            "(a + b)[0]",
            "-bag[0]",
            "!flags[\"x\"]",
            "len(bag) > 2 && contains(bag, \"sword\")",
            "push(bag, \"x\")",
            "insert(m, \"k\", 1 + 2)",
            "remove_key(m, \"k\")",
            "remove_at(bag, 0)",
            "clear(bag)",
            "has_key(m, k)",
            "remove(bag, \"x\")",
            "[1, 2][0]",
            "#{\"a\": 1}[\"a\"]",
        ] {
            let e = Expr::parse(src).unwrap_or_else(|err| panic!("{src}: {err}"));
            let printed = e.to_string();
            assert_eq!(Expr::parse(&printed).unwrap(), e, "{src} -> {printed}");
        }
        for bad in [
            "[1, 2",
            "[:list]",
            "#{1: 2}",
            "#{\"a\": 1, \"a\": 2}",
            "len(a, b)",
            "nope(a)",
            "a[0",
            "#{\"a\" 1}",
            "]",
        ] {
            assert!(Expr::parse(bad).is_err(), "{bad:?} should fail");
        }
        // Variables are found inside the new forms.
        let e = Expr::parse("insert(m, \"k\", [a][b])").unwrap();
        assert_eq!(e.variables(), vec!["m", "a", "b"]);
    }

    #[test]
    fn collection_typing() {
        let types = |n: &str| match n {
            "bag" => Some(ValueType::List(ScalarType::Str)),
            "gold" => Some(ValueType::Map(ScalarType::Int)),
            "i" => Some(ValueType::Int),
            "s" => Some(ValueType::Str),
            _ => None,
        };
        let ty = |src: &str| Expr::parse(src).unwrap().type_of(&types);
        assert_eq!(ty("len(bag)"), Ok(Some(ValueType::Int)));
        assert_eq!(ty("bag[i]"), Ok(Some(ValueType::Str)));
        assert_eq!(ty("gold[s] + 1"), Ok(Some(ValueType::Int)));
        assert_eq!(ty("contains(bag, s)"), Ok(Some(ValueType::Bool)));
        assert_eq!(ty("has_key(gold, \"x\")"), Ok(Some(ValueType::Bool)));
        assert_eq!(ty("push(bag, \"x\")"), Ok(Some(ValueType::List(ScalarType::Str))));
        assert_eq!(ty("insert(gold, \"x\", 2.5)"), Ok(Some(ValueType::Map(ScalarType::Int))));
        assert_eq!(ty("[1, 2.5]"), Ok(Some(ValueType::List(ScalarType::Float))));
        assert_eq!(ty("[]"), Ok(None));
        assert_eq!(ty("bag == []"), Ok(Some(ValueType::Bool)));
        assert_eq!(ty("unknown[0]"), Ok(None));
        assert!(ty("bag[s]").is_err());
        assert!(ty("gold[i]").is_err());
        assert!(ty("i[0]").is_err());
        assert!(ty("push(bag, 1)").is_err());
        assert!(ty("push(gold, 1)").is_err());
        assert!(ty("has_key(bag, s)").is_err());
        assert!(ty("[bag]").is_err());
        assert!(ty("[1, \"a\"]").is_err());
        assert!(ty("bag < bag").is_err());
        assert!(ty("bag + bag").is_err());
        assert!(ty("len(i)").is_err());
        assert!(ty("bag == gold").is_err());
    }

    #[test]
    fn collection_exprs_survive_ron() {
        let e = Expr::parse("push(bag, \"x\") == [\"a\"] && gold[\"k\"] > len(#{\"a\": n})").unwrap();
        let ron = ron::to_string(&e).unwrap();
        assert_eq!(ron::from_str::<Expr>(&ron).unwrap(), e);
    }
}
