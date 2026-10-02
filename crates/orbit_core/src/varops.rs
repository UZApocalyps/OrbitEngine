//! No-code variable operations and conditions (schema v7).
//!
//! [`VarOp`] ("add 1 to `gold`", "toggle `met_alice`") and [`Condition`]
//! ("`gold` >= 10", all / any of several rows) are what visual editors and
//! UI screens manipulate. They are *views* over the expression language:
//! [`VarOp::to_expr`] / [`Condition::to_expr`] produce the [`Expr`] stored in
//! story nodes, and [`VarOp::from_expr`] / [`Condition::from_expr`] recognize
//! such expressions again (anything else stays an expression, shown in
//! "expression mode" by the editor). The UI system serializes the structured
//! forms directly.

use serde::{Deserialize, Serialize};

use crate::expr::{BinOp, EvalError, Expr, UnOp};
use crate::value::{Value, ValueType};

/// The value an operation or comparison works with: a literal or another
/// variable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Operand {
    Lit(Value),
    Var(String),
}

impl Operand {
    pub fn to_expr(&self) -> Expr {
        match self {
            Operand::Lit(v) => Expr::Lit(v.clone()),
            Operand::Var(n) => Expr::Var(n.clone()),
        }
    }

    fn from_expr(e: &Expr) -> Option<Operand> {
        match e {
            Expr::Lit(v) => Some(Operand::Lit(v.clone())),
            Expr::Var(n) => Some(Operand::Var(n.clone())),
            _ => None,
        }
    }

    /// Static type of the operand (`None` for an unknown variable).
    pub fn value_type(&self, ty_of: &dyn Fn(&str) -> Option<ValueType>) -> Option<ValueType> {
        match self {
            Operand::Lit(v) => Some(v.value_type()),
            Operand::Var(n) => ty_of(n),
        }
    }
}

/// What an operation does to its variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum OpKind {
    Set,
    /// Number: add. Text: append.
    Add,
    Sub,
    Mul,
    /// Dividing by zero does nothing.
    Div,
    /// Bool only; the operand is ignored.
    Toggle,
}

impl OpKind {
    /// Operations offered for a variable of type `ty`.
    pub fn for_type(ty: ValueType) -> &'static [OpKind] {
        match ty {
            ValueType::Int | ValueType::Float => &[
                OpKind::Set,
                OpKind::Add,
                OpKind::Sub,
                OpKind::Mul,
                OpKind::Div,
            ],
            ValueType::Bool => &[OpKind::Set, OpKind::Toggle],
            ValueType::Str => &[OpKind::Set, OpKind::Add],
        }
    }

    pub fn supports(self, ty: ValueType) -> bool {
        Self::for_type(ty).contains(&self)
    }

    /// Stable key, for UI strings (`varop.<key>`).
    pub fn key(self) -> &'static str {
        match self {
            OpKind::Set => "set",
            OpKind::Add => "add",
            OpKind::Sub => "sub",
            OpKind::Mul => "mul",
            OpKind::Div => "div",
            OpKind::Toggle => "toggle",
        }
    }
}

/// Why an operation could not be applied. The caller leaves the variable
/// unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpError {
    UnknownVariable(String),
    Type(String),
    DivisionByZero,
    Overflow,
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::UnknownVariable(n) => write!(f, "unknown variable `{n}`"),
            OpError::Type(m) => write!(f, "type mismatch: {m}"),
            OpError::DivisionByZero => f.write_str("division by zero"),
            OpError::Overflow => f.write_str("integer overflow"),
        }
    }
}

impl std::error::Error for OpError {}

impl From<EvalError> for OpError {
    fn from(e: EvalError) -> Self {
        match e {
            EvalError::UnknownVariable(n) => OpError::UnknownVariable(n),
            EvalError::Type(m) => OpError::Type(m),
            EvalError::Overflow => OpError::Overflow,
            EvalError::DivisionByZero => OpError::DivisionByZero,
        }
    }
}

/// A typed, no-code operation on one variable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VarOp {
    pub var: String,
    pub op: OpKind,
    /// Ignored by [`OpKind::Toggle`].
    pub value: Operand,
}

impl VarOp {
    pub fn new(var: impl Into<String>, op: OpKind, value: Operand) -> Self {
        Self {
            var: var.into(),
            op,
            value,
        }
    }

    pub fn set(var: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::new(var, OpKind::Set, Operand::Lit(value.into()))
    }

    pub fn toggle(var: impl Into<String>) -> Self {
        Self::new(var, OpKind::Toggle, Operand::Lit(Value::Bool(true)))
    }

    /// The expression assigned to the variable (what a `SetVar` node stores).
    pub fn to_expr(&self) -> Expr {
        let target = || Expr::Var(self.var.clone());
        let bin = |op| Expr::binary(op, target(), self.value.to_expr());
        match self.op {
            OpKind::Set => self.value.to_expr(),
            OpKind::Add => bin(BinOp::Add),
            OpKind::Sub => bin(BinOp::Sub),
            OpKind::Mul => bin(BinOp::Mul),
            OpKind::Div => bin(BinOp::Div),
            OpKind::Toggle => Expr::negate(target()),
        }
    }

    /// Recognizes the expression assigned to `var`; `None` when it is more
    /// than a no-code operation (the editor then stays in expression mode).
    pub fn from_expr(var: &str, expr: &Expr) -> Option<VarOp> {
        let make = |op, value| Some(VarOp::new(var, op, value));
        match expr {
            Expr::Lit(_) | Expr::Var(_) => make(OpKind::Set, Operand::from_expr(expr)?),
            Expr::Unary(UnOp::Not, inner) if matches!(&**inner, Expr::Var(n) if n == var) => {
                Some(VarOp::toggle(var))
            }
            Expr::Binary(op, l, r) if matches!(&**l, Expr::Var(n) if n == var) => {
                let kind = match op {
                    BinOp::Add => OpKind::Add,
                    BinOp::Sub => OpKind::Sub,
                    BinOp::Mul => OpKind::Mul,
                    BinOp::Div => OpKind::Div,
                    _ => return None,
                };
                make(kind, Operand::from_expr(r)?)
            }
            _ => None,
        }
    }

    /// The new value of the variable, given its current one through `get`.
    /// The result keeps the variable's type (a float result stored in an
    /// integer variable is rounded). Dividing by zero is an error: callers
    /// leave the variable unchanged.
    pub fn apply(&self, get: &dyn Fn(&str) -> Option<Value>) -> Result<Value, OpError> {
        let current =
            get(&self.var).ok_or_else(|| OpError::UnknownVariable(self.var.clone()))?;
        let result = self.to_expr().eval(get)?;
        coerce(result, current.value_type())
    }

    /// Problems of this operation given the declared variable types: unknown
    /// variables, operation not available for the type, operand of another
    /// type, literal division by zero.
    pub fn issues(&self, ty_of: &dyn Fn(&str) -> Option<ValueType>) -> Vec<String> {
        let mut out = Vec::new();
        let Some(ty) = ty_of(&self.var) else {
            out.push(format!("unknown variable `{}`", self.var));
            return out;
        };
        if !self.op.supports(ty) {
            out.push(format!(
                "operation `{}` is not available for {ty} variable `{}`",
                self.op.key(),
                self.var
            ));
            return out;
        }
        if self.op == OpKind::Toggle {
            return out;
        }
        if let Operand::Var(n) = &self.value
            && ty_of(n).is_none()
        {
            out.push(format!("unknown variable `{n}`"));
            return out;
        }
        if let Some(vt) = self.value.value_type(ty_of)
            && vt != ty
            && !(vt.is_numeric() && ty.is_numeric())
        {
            out.push(format!("`{}` is {ty} but the value is {vt}", self.var));
        }
        let zero = matches!(&self.value, Operand::Lit(Value::Int(0)))
            || matches!(&self.value, Operand::Lit(Value::Float(f)) if *f == 0.0);
        if self.op == OpKind::Div && zero {
            out.push("division by zero".to_owned());
        }
        out
    }
}

/// Keeps `value` within `ty` (the declared type of a variable): numbers convert between int and float (a float
/// stored in an int is rounded); anything else must already match.
pub fn coerce(value: Value, ty: ValueType) -> Result<Value, OpError> {
    match (value, ty) {
        (Value::Float(f), ValueType::Int) if f.is_finite() => {
            let r = f.round();
            if r >= i64::MIN as f64 && r <= i64::MAX as f64 {
                Ok(Value::Int(r as i64))
            } else {
                Err(OpError::Overflow)
            }
        }
        (Value::Int(i), ValueType::Float) => Ok(Value::Float(i as f64)),
        (v, t) if v.value_type() == t => Ok(v),
        (v, t) => Err(OpError::Type(format!(
            "cannot store a {} in a {t} variable",
            v.value_type()
        ))),
    }
}

/// Comparison operator of a [`Condition::Cmp`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    /// Comparisons offered for a variable of type `ty` (booleans: is / is
    /// not; ordering only for numbers and text).
    pub fn for_type(ty: ValueType) -> &'static [Cmp] {
        match ty {
            ValueType::Bool => &[Cmp::Eq, Cmp::Ne],
            _ => &[Cmp::Eq, Cmp::Ne, Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge],
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
        }
    }

    fn bin_op(self) -> BinOp {
        match self {
            Cmp::Eq => BinOp::Eq,
            Cmp::Ne => BinOp::Ne,
            Cmp::Lt => BinOp::Lt,
            Cmp::Le => BinOp::Le,
            Cmp::Gt => BinOp::Gt,
            Cmp::Ge => BinOp::Ge,
        }
    }

    fn from_bin_op(op: BinOp) -> Option<Cmp> {
        Some(match op {
            BinOp::Eq => Cmp::Eq,
            BinOp::Ne => Cmp::Ne,
            BinOp::Lt => Cmp::Lt,
            BinOp::Le => Cmp::Le,
            BinOp::Gt => Cmp::Gt,
            BinOp::Ge => Cmp::Ge,
            _ => return None,
        })
    }
}

/// A no-code condition on variables.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Condition {
    /// `var <cmp> value`. A boolean variable "is true" is `Eq` with `true`.
    Cmp {
        var: String,
        cmp: Cmp,
        value: Operand,
    },
    /// Every member holds (an empty group is true).
    All(Vec<Condition>),
    /// At least one member holds (an empty group is false).
    Any(Vec<Condition>),
    /// Advanced: a raw expression.
    Expr(Expr),
}

impl Condition {
    pub fn cmp(var: impl Into<String>, cmp: Cmp, value: impl Into<Value>) -> Self {
        Condition::Cmp {
            var: var.into(),
            cmp,
            value: Operand::Lit(value.into()),
        }
    }

    /// `var` is true.
    pub fn is_true(var: impl Into<String>) -> Self {
        Self::cmp(var, Cmp::Eq, true)
    }

    pub fn to_expr(&self) -> Expr {
        match self {
            Condition::Cmp { var, cmp, value } => {
                Expr::binary(cmp.bin_op(), Expr::Var(var.clone()), value.to_expr())
            }
            Condition::All(items) => fold(items, BinOp::And, true),
            Condition::Any(items) => fold(items, BinOp::Or, false),
            Condition::Expr(e) => e.clone(),
        }
    }

    /// Recognizes structured conditions inside an expression; the rest is
    /// kept as [`Condition::Expr`].
    pub fn from_expr(expr: &Expr) -> Condition {
        match expr {
            Expr::Binary(BinOp::And, ..) => {
                let mut items = Vec::new();
                flatten(expr, BinOp::And, &mut items);
                Condition::All(items)
            }
            Expr::Binary(BinOp::Or, ..) => {
                let mut items = Vec::new();
                flatten(expr, BinOp::Or, &mut items);
                Condition::Any(items)
            }
            Expr::Binary(op, l, r) => match (Cmp::from_bin_op(*op), &**l, Operand::from_expr(r)) {
                (Some(cmp), Expr::Var(var), Some(value)) => Condition::Cmp {
                    var: var.clone(),
                    cmp,
                    value,
                },
                _ => Condition::Expr(expr.clone()),
            },
            Expr::Lit(Value::Bool(true)) => Condition::All(Vec::new()),
            Expr::Lit(Value::Bool(false)) => Condition::Any(Vec::new()),
            Expr::Var(var) => Condition::is_true(var.clone()),
            Expr::Unary(UnOp::Not, inner) => match &**inner {
                Expr::Var(var) => Condition::cmp(var.clone(), Cmp::Eq, false),
                _ => Condition::Expr(expr.clone()),
            },
            _ => Condition::Expr(expr.clone()),
        }
    }

    /// `true` when no part is a raw expression.
    pub fn is_structured(&self) -> bool {
        match self {
            Condition::Cmp { .. } => true,
            Condition::All(v) | Condition::Any(v) => v.iter().all(Condition::is_structured),
            Condition::Expr(_) => false,
        }
    }

    /// Evaluates the condition; `get` reads a variable.
    pub fn eval(&self, get: &dyn Fn(&str) -> Option<Value>) -> Result<bool, EvalError> {
        match self.to_expr().eval(get)? {
            Value::Bool(b) => Ok(b),
            other => Err(EvalError::Type(format!(
                "condition must be bool, got {}",
                other.value_type()
            ))),
        }
    }

    /// Names of the variables read.
    pub fn variables(&self) -> Vec<String> {
        self.to_expr()
            .variables()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// Problems given the declared variable types: unknown variables, type
    /// mismatch, ordering on a boolean.
    pub fn issues(&self, ty_of: &dyn Fn(&str) -> Option<ValueType>) -> Vec<String> {
        match self {
            Condition::Cmp { var, cmp, value } => {
                let Some(ty) = ty_of(var) else {
                    return vec![format!("unknown variable `{var}`")];
                };
                if let Operand::Var(n) = value
                    && ty_of(n).is_none()
                {
                    return vec![format!("unknown variable `{n}`")];
                }
                let mut out = Vec::new();
                if !Cmp::for_type(ty).contains(cmp) {
                    out.push(format!("`{}` cannot be used on {ty} `{var}`", cmp.symbol()));
                }
                if let Some(vt) = value.value_type(ty_of)
                    && vt != ty
                    && !(vt.is_numeric() && ty.is_numeric())
                {
                    out.push(format!("`{var}` is {ty} but is compared with {vt}"));
                }
                out
            }
            Condition::All(v) | Condition::Any(v) => {
                v.iter().flat_map(|c| c.issues(ty_of)).collect()
            }
            Condition::Expr(e) => match e.type_of(ty_of) {
                Err(err) => vec![err.0],
                Ok(_) => e
                    .variables()
                    .into_iter()
                    .filter(|n| ty_of(n).is_none())
                    .map(|n| format!("unknown variable `{n}`"))
                    .collect(),
            },
        }
    }
}

fn fold(items: &[Condition], op: BinOp, empty: bool) -> Expr {
    let mut iter = items.iter().map(Condition::to_expr);
    match iter.next() {
        None => Expr::Lit(Value::Bool(empty)),
        Some(first) => iter.fold(first, |acc, e| Expr::binary(op, acc, e)),
    }
}

fn flatten(expr: &Expr, op: BinOp, out: &mut Vec<Condition>) {
    match expr {
        Expr::Binary(o, l, r) if *o == op => {
            flatten(l, op, out);
            flatten(r, op, out);
        }
        other => out.push(Condition::from_expr(other)),
    }
}

impl ValueType {
    /// A neutral value of this type (`0`, `false`, empty text).
    pub fn default_value(self) -> Value {
        match self {
            ValueType::Bool => Value::Bool(false),
            ValueType::Int => Value::Int(0),
            ValueType::Float => Value::Float(0.0),
            ValueType::Str => Value::Str(String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(v: &[(&str, Value)]) -> impl Fn(&str) -> Option<Value> + use<> {
        let v: Vec<(String, Value)> = v.iter().map(|(k, x)| (k.to_string(), x.clone())).collect();
        move |n| v.iter().find(|(k, _)| k == n).map(|(_, x)| x.clone())
    }

    #[test]
    fn number_ops() {
        let get = vars(&[("n", Value::Int(10)), ("f", Value::Float(1.5)), ("m", Value::Int(3))]);
        let run = |op, v: Operand, var: &str| VarOp::new(var, op, v).apply(&get);
        assert_eq!(run(OpKind::Set, Operand::Lit(Value::Int(4)), "n"), Ok(Value::Int(4)));
        assert_eq!(run(OpKind::Add, Operand::Lit(Value::Int(2)), "n"), Ok(Value::Int(12)));
        assert_eq!(run(OpKind::Sub, Operand::Var("m".into()), "n"), Ok(Value::Int(7)));
        assert_eq!(run(OpKind::Mul, Operand::Lit(Value::Int(3)), "n"), Ok(Value::Int(30)));
        assert_eq!(run(OpKind::Div, Operand::Lit(Value::Int(3)), "n"), Ok(Value::Int(3)));
        // A float result stored in an int variable is rounded.
        assert_eq!(run(OpKind::Mul, Operand::Lit(Value::Float(1.26)), "n"), Ok(Value::Int(13)));
        // An int operand into a float variable stays a float.
        assert_eq!(run(OpKind::Add, Operand::Lit(Value::Int(1)), "f"), Ok(Value::Float(2.5)));
        assert_eq!(run(OpKind::Div, Operand::Lit(Value::Int(0)), "n"), Err(OpError::DivisionByZero));
        assert_eq!(run(OpKind::Div, Operand::Lit(Value::Float(0.0)), "f"), Err(OpError::DivisionByZero));
        assert!(matches!(run(OpKind::Set, Operand::Lit(Value::Str("x".into())), "n"), Err(OpError::Type(_))));
        assert_eq!(run(OpKind::Add, Operand::Lit(Value::Int(1)), "ghost"), Err(OpError::UnknownVariable("ghost".into())));
    }

    #[test]
    fn bool_and_text_ops() {
        let get = vars(&[("b", Value::Bool(true)), ("s", Value::Str("ab".into()))]);
        assert_eq!(VarOp::toggle("b").apply(&get), Ok(Value::Bool(false)));
        assert_eq!(VarOp::set("b", false).apply(&get), Ok(Value::Bool(false)));
        assert_eq!(VarOp::set("s", "x").apply(&get), Ok(Value::Str("x".into())));
        let append = VarOp::new("s", OpKind::Add, Operand::Lit(Value::Str("c".into())));
        assert_eq!(append.apply(&get), Ok(Value::Str("abc".into())));
        assert!(VarOp::toggle("s").apply(&get).is_err());
    }

    #[test]
    fn ops_by_type() {
        assert!(OpKind::Div.supports(ValueType::Float));
        assert!(!OpKind::Div.supports(ValueType::Str));
        assert!(!OpKind::Toggle.supports(ValueType::Int));
        assert_eq!(OpKind::for_type(ValueType::Bool), &[OpKind::Set, OpKind::Toggle]);
        assert_eq!(OpKind::for_type(ValueType::Str), &[OpKind::Set, OpKind::Add]);
    }

    #[test]
    fn varop_expr_round_trip() {
        for op in [
            VarOp::set("a", 3i64),
            VarOp::new("a", OpKind::Set, Operand::Var("b".into())),
            VarOp::new("a", OpKind::Add, Operand::Lit(Value::Int(1))),
            VarOp::new("a", OpKind::Sub, Operand::Var("b".into())),
            VarOp::new("a", OpKind::Mul, Operand::Lit(Value::Float(2.0))),
            VarOp::new("a", OpKind::Div, Operand::Lit(Value::Int(2))),
            VarOp::toggle("a"),
        ] {
            assert_eq!(VarOp::from_expr("a", &op.to_expr()), Some(op));
        }
        // Not a no-code shape: stays an expression.
        let e = Expr::parse("a + b * 2").unwrap();
        assert_eq!(VarOp::from_expr("a", &e), None);
        let other = Expr::parse("b + 1").unwrap();
        assert_eq!(VarOp::from_expr("a", &other), None);
    }

    #[test]
    fn varop_issues() {
        let ty = |n: &str| match n {
            "n" => Some(ValueType::Int),
            "b" => Some(ValueType::Bool),
            "s" => Some(ValueType::Str),
            _ => None,
        };
        assert!(VarOp::set("n", 1i64).issues(&ty).is_empty());
        assert_eq!(VarOp::set("ghost", 1i64).issues(&ty).len(), 1);
        assert_eq!(VarOp::toggle("n").issues(&ty).len(), 1);
        assert_eq!(VarOp::set("n", "x").issues(&ty).len(), 1);
        assert_eq!(VarOp::new("n", OpKind::Div, Operand::Lit(Value::Int(0))).issues(&ty).len(), 1);
        assert_eq!(VarOp::new("n", OpKind::Add, Operand::Var("zz".into())).issues(&ty).len(), 1);
        assert!(VarOp::new("n", OpKind::Add, Operand::Lit(Value::Float(0.5))).issues(&ty).is_empty());
    }

    #[test]
    fn conditions_evaluate() {
        let get = vars(&[
            ("gold", Value::Int(10)),
            ("met", Value::Bool(true)),
            ("name", Value::Str("bob".into())),
        ]);
        let c = |cond: Condition| cond.eval(&get);
        assert_eq!(c(Condition::cmp("gold", Cmp::Ge, 10i64)), Ok(true));
        assert_eq!(c(Condition::cmp("gold", Cmp::Lt, 10i64)), Ok(false));
        assert_eq!(c(Condition::cmp("gold", Cmp::Gt, 2.5f64)), Ok(true));
        assert_eq!(c(Condition::is_true("met")), Ok(true));
        assert_eq!(c(Condition::cmp("name", Cmp::Ne, "al")), Ok(true));
        let other = Condition::Cmp {
            var: "gold".into(),
            cmp: Cmp::Eq,
            value: Operand::Var("gold".into()),
        };
        assert_eq!(c(other), Ok(true));
        let all = Condition::All(vec![Condition::is_true("met"), Condition::cmp("gold", Cmp::Lt, 5i64)]);
        let any = Condition::Any(vec![Condition::is_true("met"), Condition::cmp("gold", Cmp::Lt, 5i64)]);
        assert_eq!(c(all), Ok(false));
        assert_eq!(c(any), Ok(true));
        assert_eq!(c(Condition::All(vec![])), Ok(true));
        assert_eq!(c(Condition::Any(vec![])), Ok(false));
        assert_eq!(c(Condition::Expr(Expr::parse("gold * 2 == 20").unwrap())), Ok(true));
        assert!(c(Condition::cmp("ghost", Cmp::Eq, 1i64)).is_err());
        assert!(c(Condition::Expr(Expr::lit(3i64))).is_err());
    }

    #[test]
    fn condition_expr_round_trip() {
        let cond = Condition::Any(vec![
            Condition::All(vec![
                Condition::cmp("a", Cmp::Ge, 1i64),
                Condition::cmp("b", Cmp::Ne, "x"),
            ]),
            Condition::cmp("c", Cmp::Eq, false),
        ]);
        let back = Condition::from_expr(&cond.to_expr());
        assert_eq!(back, cond);
        assert!(back.is_structured());
        // A bare bool variable reads back as "is true".
        assert_eq!(Condition::from_expr(&Expr::var("m")), Condition::is_true("m"));
        // Anything else stays an expression.
        let raw = Expr::parse("a + 1 > 2").unwrap();
        assert_eq!(Condition::from_expr(&raw), Condition::Expr(raw.clone()));
        assert!(!Condition::from_expr(&raw).is_structured());
    }

    #[test]
    fn condition_issues() {
        let ty = |n: &str| match n {
            "n" => Some(ValueType::Int),
            "b" => Some(ValueType::Bool),
            _ => None,
        };
        assert!(Condition::cmp("n", Cmp::Gt, 1i64).issues(&ty).is_empty());
        assert_eq!(Condition::cmp("ghost", Cmp::Gt, 1i64).issues(&ty).len(), 1);
        assert_eq!(Condition::cmp("n", Cmp::Gt, "x").issues(&ty).len(), 1);
        assert_eq!(Condition::cmp("b", Cmp::Lt, true).issues(&ty).len(), 1);
        let group = Condition::All(vec![Condition::cmp("zz", Cmp::Eq, 1i64), Condition::is_true("b")]);
        assert_eq!(group.issues(&ty).len(), 1);
    }
}
