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
//!
//! Schema v11 (ADR 0016) adds the collection operations (push, remove,
//! remove at, clear, set/remove a map key) and conditions (contains, has
//! key, length comparisons).

use serde::{Deserialize, Serialize};

use crate::expr::{BinOp, EvalError, Expr, Func, UnOp};
use crate::value::{ScalarType, Value, ValueType};

/// The value an operation or comparison works with: a literal or another
/// variable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Operand {
    Lit(Value),
    Var(String),
    /// Inside a generated list (`WidgetKind::Repeat`, UI-15): the element of
    /// the row (the value, for a map). The four contextual operands were
    /// appended in schema v12 (ADR 0016): existing indices are unchanged.
    Item,
    /// The 0-based position of the row in a generated list.
    ItemIndex,
    /// The key of the row in a list generated from a map (text).
    ItemKey,
    /// A parameter the story gave the screen (`NodeKind::OpenScreen`, UI-14).
    Param(String),
}

/// Reserved variable names the contextual operands evaluate as. A screen
/// supplies them in its variable lookup; anywhere else they are unknown
/// variables (a variable name never starts with `@`).
pub const ITEM_VAR: &str = "@item";
pub const ITEM_INDEX_VAR: &str = "@index";
pub const ITEM_KEY_VAR: &str = "@key";
pub const PARAM_VAR_PREFIX: &str = "@param.";

impl Operand {
    pub fn to_expr(&self) -> Expr {
        match self {
            Operand::Lit(v) => Expr::Lit(v.clone()),
            Operand::Var(n) => Expr::Var(n.clone()),
            Operand::Item => Expr::Var(ITEM_VAR.into()),
            Operand::ItemIndex => Expr::Var(ITEM_INDEX_VAR.into()),
            Operand::ItemKey => Expr::Var(ITEM_KEY_VAR.into()),
            Operand::Param(p) => Expr::Var(format!("{PARAM_VAR_PREFIX}{p}")),
        }
    }

    fn from_expr(e: &Expr) -> Option<Operand> {
        match e {
            Expr::Lit(v) => Some(Operand::Lit(v.clone())),
            Expr::Var(n) => Some(Operand::from_var_name(n)),
            _ => None,
        }
    }

    /// `Var(name)`, or the contextual operand a reserved name stands for.
    pub fn from_var_name(n: &str) -> Operand {
        match n {
            ITEM_VAR => Operand::Item,
            ITEM_INDEX_VAR => Operand::ItemIndex,
            ITEM_KEY_VAR => Operand::ItemKey,
            _ => match n.strip_prefix(PARAM_VAR_PREFIX) {
                Some(p) => Operand::Param(p.to_owned()),
                None => Operand::Var(n.to_owned()),
            },
        }
    }

    /// Whether the operand only has a value inside a screen (a generated row
    /// or a screen parameter): story nodes refuse such operands.
    pub fn is_contextual(&self) -> bool {
        matches!(
            self,
            Operand::Item | Operand::ItemIndex | Operand::ItemKey | Operand::Param(_)
        )
    }

    /// The value of the operand: a literal as it is, a variable (or a
    /// contextual name) through `get`; `None` when `get` does not know it.
    pub fn eval(&self, get: &dyn Fn(&str) -> Option<Value>) -> Option<Value> {
        match self {
            Operand::Lit(v) => Some(v.clone()),
            other => match other.to_expr() {
                Expr::Var(n) => get(&n),
                _ => None,
            },
        }
    }

    /// Static type of the operand (`None` for an unknown variable, and for
    /// the contextual operands, only known in a screen at run time).
    pub fn value_type(&self, ty_of: &dyn Fn(&str) -> Option<ValueType>) -> Option<ValueType> {
        match self {
            Operand::Lit(v) => Some(v.value_type()),
            Operand::Var(n) => ty_of(n),
            Operand::Item | Operand::ItemIndex | Operand::ItemKey | Operand::Param(_) => None,
        }
    }
}

/// What an operation does to its variable.
///
/// New variants are appended at the end (stable indices).
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
    /// List: append the operand (schema v11).
    Push,
    /// List: remove the first element equal to the operand (schema v11).
    Remove,
    /// List: remove the element at the index given by the operand.
    RemoveAt,
    /// List or map: remove everything; the operand is ignored.
    Clear,
    /// Map: set the entry [`VarOp::key`] to the operand.
    MapSet,
    /// Map: remove the entry [`VarOp::key`]; the operand is ignored.
    MapRemove,
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
            ValueType::List(_) => &[
                OpKind::Push,
                OpKind::Remove,
                OpKind::RemoveAt,
                OpKind::Clear,
                OpKind::Set,
            ],
            ValueType::Map(_) => &[
                OpKind::MapSet,
                OpKind::MapRemove,
                OpKind::Clear,
                OpKind::Set,
            ],
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
            OpKind::Push => "push",
            OpKind::Remove => "remove",
            OpKind::RemoveAt => "remove_at",
            OpKind::Clear => "clear",
            OpKind::MapSet => "map_set",
            OpKind::MapRemove => "map_remove",
        }
    }

    /// Type of the operand for a variable of type `var`; `None` when the
    /// operand is ignored.
    pub fn operand_type(self, var: ValueType) -> Option<ValueType> {
        match self {
            OpKind::Toggle | OpKind::Clear | OpKind::MapRemove => None,
            OpKind::Push | OpKind::Remove | OpKind::MapSet => var.element(),
            OpKind::RemoveAt => Some(ValueType::Int),
            _ => Some(var),
        }
    }

    /// The operation uses [`VarOp::key`] (a text map key).
    pub fn uses_key(self) -> bool {
        matches!(self, OpKind::MapSet | OpKind::MapRemove)
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
    /// A collection error: index out of bounds, missing key, size limit
    /// (schema v11).
    Collection(EvalError),
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::UnknownVariable(n) => write!(f, "unknown variable `{n}`"),
            OpError::Type(m) => write!(f, "type mismatch: {m}"),
            OpError::DivisionByZero => f.write_str("division by zero"),
            OpError::Overflow => f.write_str("integer overflow"),
            OpError::Collection(e) => write!(f, "{e}"),
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
            e @ (EvalError::IndexOutOfBounds { .. }
            | EvalError::MissingKey(_)
            | EvalError::TooLarge) => OpError::Collection(e),
        }
    }
}

/// A typed, no-code operation on one variable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VarOp {
    pub var: String,
    pub op: OpKind,
    /// Ignored by [`OpKind::Toggle`], [`OpKind::Clear`] and
    /// [`OpKind::MapRemove`].
    pub value: Operand,
    /// The map key of [`OpKind::MapSet`] / [`OpKind::MapRemove`] (schema
    /// v11); `None` elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Operand>,
}

impl VarOp {
    pub fn new(var: impl Into<String>, op: OpKind, value: Operand) -> Self {
        Self {
            var: var.into(),
            op,
            value,
            key: None,
        }
    }

    /// The same operation on the map key `key`.
    pub fn with_key(mut self, key: Operand) -> Self {
        self.key = Some(key);
        self
    }

    pub fn set(var: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::new(var, OpKind::Set, Operand::Lit(value.into()))
    }

    pub fn toggle(var: impl Into<String>) -> Self {
        Self::new(var, OpKind::Toggle, Operand::Lit(Value::Bool(true)))
    }

    fn key_expr(&self) -> Expr {
        self.key
            .as_ref()
            .map_or_else(|| Expr::Lit(Value::Str(String::new())), Operand::to_expr)
    }

    /// The expression assigned to the variable (what a `SetVar` node stores).
    pub fn to_expr(&self) -> Expr {
        let target = || Expr::Var(self.var.clone());
        let bin = |op| Expr::binary(op, target(), self.value.to_expr());
        let call = |f, extra: Vec<Expr>| {
            let mut args = vec![target()];
            args.extend(extra);
            Expr::Call(f, args)
        };
        match self.op {
            OpKind::Set => self.value.to_expr(),
            OpKind::Add => bin(BinOp::Add),
            OpKind::Sub => bin(BinOp::Sub),
            OpKind::Mul => bin(BinOp::Mul),
            OpKind::Div => bin(BinOp::Div),
            OpKind::Toggle => Expr::negate(target()),
            OpKind::Push => call(Func::Push, vec![self.value.to_expr()]),
            OpKind::Remove => call(Func::Remove, vec![self.value.to_expr()]),
            OpKind::RemoveAt => call(Func::RemoveAt, vec![self.value.to_expr()]),
            OpKind::Clear => call(Func::Clear, vec![]),
            OpKind::MapSet => call(Func::Insert, vec![self.key_expr(), self.value.to_expr()]),
            OpKind::MapRemove => call(Func::RemoveKey, vec![self.key_expr()]),
        }
    }

    /// Recognizes the expression assigned to `var`; `None` when it is more
    /// than a no-code operation (the editor then stays in expression mode).
    pub fn from_expr(var: &str, expr: &Expr) -> Option<VarOp> {
        let make = |op, value| Some(VarOp::new(var, op, value));
        let is_target = |e: &Expr| matches!(e, Expr::Var(n) if n == var);
        // The operand of ops that ignore it.
        let ignored = || Operand::Lit(Value::Bool(true));
        match expr {
            Expr::Lit(_) | Expr::Var(_) => make(OpKind::Set, Operand::from_expr(expr)?),
            Expr::Unary(UnOp::Not, inner) if is_target(inner) => Some(VarOp::toggle(var)),
            Expr::Binary(op, l, r) if is_target(l) => {
                let kind = match op {
                    BinOp::Add => OpKind::Add,
                    BinOp::Sub => OpKind::Sub,
                    BinOp::Mul => OpKind::Mul,
                    BinOp::Div => OpKind::Div,
                    _ => return None,
                };
                make(kind, Operand::from_expr(r)?)
            }
            Expr::Call(f, args) if args.first().is_some_and(is_target) => match (f, &args[1..]) {
                (Func::Push, [v]) => make(OpKind::Push, Operand::from_expr(v)?),
                (Func::Remove, [v]) => make(OpKind::Remove, Operand::from_expr(v)?),
                (Func::RemoveAt, [v]) => make(OpKind::RemoveAt, Operand::from_expr(v)?),
                (Func::Clear, []) => make(OpKind::Clear, ignored()),
                (Func::Insert, [k, v]) => Some(
                    VarOp::new(var, OpKind::MapSet, Operand::from_expr(v)?)
                        .with_key(Operand::from_expr(k)?),
                ),
                (Func::RemoveKey, [k]) => Some(
                    VarOp::new(var, OpKind::MapRemove, ignored()).with_key(Operand::from_expr(k)?),
                ),
                _ => None,
            },
            _ => None,
        }
    }

    /// The new value of the variable, given its current one through `get`.
    /// The result keeps the variable's type (a float result stored in an
    /// integer variable is rounded). Dividing by zero, an index out of
    /// bounds or a collection over its limits is an error: callers leave
    /// the variable unchanged.
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
        if self.op.uses_key() {
            match &self.key {
                None => out.push(format!("no key given for map `{}`", self.var)),
                Some(Operand::Var(n)) if ty_of(n).is_none() => {
                    out.push(format!("unknown variable `{n}`"));
                }
                Some(k) => {
                    if let Some(kt) = k.value_type(ty_of)
                        && kt != ValueType::Str
                    {
                        out.push(format!("a map key must be text, got {kt}"));
                    }
                }
            }
        }
        let Some(want) = self.op.operand_type(ty) else {
            return out;
        };
        if let Operand::Var(n) = &self.value
            && ty_of(n).is_none()
        {
            out.push(format!("unknown variable `{n}`"));
            return out;
        }
        if let Some(vt) = self.value.value_type(ty_of)
            && !ValueType::compatible(vt, want)
            && !(want.is_collection() && same_kind_empty(&self.value, want))
        {
            if want == ty {
                out.push(format!("`{}` is {ty} but the value is {vt}", self.var));
            } else {
                out.push(format!(
                    "`{}` expects a {want} for `{}`, got {vt}",
                    self.var,
                    self.op.key()
                ));
            }
        }
        if let Operand::Lit(v) = &self.value
            && !v.is_well_formed()
        {
            out.push(format!("the value given to `{}` mixes element types", self.var));
        }
        let zero = matches!(&self.value, Operand::Lit(Value::Int(0)))
            || matches!(&self.value, Operand::Lit(Value::Float(f)) if *f == 0.0);
        if self.op == OpKind::Div && zero {
            out.push("division by zero".to_owned());
        }
        out
    }
}

/// An empty literal collection of the right kind fits any element type.
fn same_kind_empty(value: &Operand, want: ValueType) -> bool {
    match (value, want) {
        (Operand::Lit(Value::List(_, l)), ValueType::List(_)) => l.is_empty(),
        (Operand::Lit(Value::Map(_, m)), ValueType::Map(_)) => m.is_empty(),
        _ => false,
    }
}

/// Keeps `value` within `ty` (the declared type of a variable): numbers
/// convert between int and float (a float stored in an int is rounded);
/// collections convert their elements the same way (an empty collection
/// takes the declared element type); anything else must already match.
pub fn coerce(value: Value, ty: ValueType) -> Result<Value, OpError> {
    let mismatch = |v: &Value| {
        OpError::Type(format!(
            "cannot store a {} in a {ty} variable",
            v.value_type()
        ))
    };
    match (value, ty) {
        (v, t) if v.value_type() == t => Ok(v),
        (Value::Float(f), ValueType::Int) if f.is_finite() => Value::Float(f)
            .into_scalar(ScalarType::Int)
            .ok_or(OpError::Overflow),
        (Value::List(from, l), ValueType::List(to)) => {
            let items = l
                .iter()
                .map(|v| v.clone().into_scalar(to))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| mismatch(&Value::List(from, l.clone())))?;
            Ok(Value::List(to, items.into()))
        }
        (Value::Map(from, m), ValueType::Map(to)) => {
            let entries = m
                .iter()
                .map(|(k, v)| Some((k.clone(), v.clone().into_scalar(to)?)))
                .collect::<Option<std::collections::BTreeMap<_, _>>>()
                .ok_or_else(|| mismatch(&Value::Map(from, m.clone())))?;
            Ok(Value::Map(to, entries.into()))
        }
        (v, t) => match t.scalar().and_then(|s| v.clone().into_scalar(s)) {
            Some(v) => Ok(v),
            None => Err(mismatch(&v)),
        },
    }
}

/// Comparison operator of a [`Condition::Cmp`].
///
/// New variants are appended at the end (stable indices).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// List: holds the operand (schema v11).
    Contains,
    /// List: does not hold the operand.
    NotContains,
    /// Map: has the key given by the operand.
    HasKey,
    /// Map: does not have the key.
    NotHasKey,
    /// Collection: number of elements `==` the operand.
    LenEq,
    LenNe,
    LenLt,
    LenLe,
    LenGt,
    LenGe,
}

impl Cmp {
    /// Comparisons offered for a variable of type `ty` (booleans: is / is
    /// not; ordering only for numbers and text).
    pub fn for_type(ty: ValueType) -> &'static [Cmp] {
        match ty {
            ValueType::Bool => &[Cmp::Eq, Cmp::Ne],
            ValueType::List(_) => &[
                Cmp::Contains,
                Cmp::NotContains,
                Cmp::LenEq,
                Cmp::LenNe,
                Cmp::LenLt,
                Cmp::LenLe,
                Cmp::LenGt,
                Cmp::LenGe,
            ],
            ValueType::Map(_) => &[
                Cmp::HasKey,
                Cmp::NotHasKey,
                Cmp::LenEq,
                Cmp::LenNe,
                Cmp::LenLt,
                Cmp::LenLe,
                Cmp::LenGt,
                Cmp::LenGe,
            ],
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
            Cmp::Contains => "contains",
            Cmp::NotContains => "does not contain",
            Cmp::HasKey => "has key",
            Cmp::NotHasKey => "has no key",
            Cmp::LenEq => "length ==",
            Cmp::LenNe => "length !=",
            Cmp::LenLt => "length <",
            Cmp::LenLe => "length <=",
            Cmp::LenGt => "length >",
            Cmp::LenGe => "length >=",
        }
    }

    /// Stable key, for UI strings (`cmp.<key>`).
    pub fn key(self) -> &'static str {
        match self {
            Cmp::Eq => "eq",
            Cmp::Ne => "ne",
            Cmp::Lt => "lt",
            Cmp::Le => "le",
            Cmp::Gt => "gt",
            Cmp::Ge => "ge",
            Cmp::Contains => "contains",
            Cmp::NotContains => "not_contains",
            Cmp::HasKey => "has_key",
            Cmp::NotHasKey => "not_has_key",
            Cmp::LenEq => "len_eq",
            Cmp::LenNe => "len_ne",
            Cmp::LenLt => "len_lt",
            Cmp::LenLe => "len_le",
            Cmp::LenGt => "len_gt",
            Cmp::LenGe => "len_ge",
        }
    }

    /// Type of the operand compared with a variable of type `var`.
    pub fn operand_type(self, var: ValueType) -> ValueType {
        match self {
            Cmp::Contains | Cmp::NotContains => var.element().unwrap_or(var),
            Cmp::HasKey | Cmp::NotHasKey => ValueType::Str,
            Cmp::LenEq | Cmp::LenNe | Cmp::LenLt | Cmp::LenLe | Cmp::LenGt | Cmp::LenGe => {
                ValueType::Int
            }
            _ => var,
        }
    }

    /// The scalar comparison applied to the length, for `Len*`.
    fn len_cmp(self) -> Option<Cmp> {
        Some(match self {
            Cmp::LenEq => Cmp::Eq,
            Cmp::LenNe => Cmp::Ne,
            Cmp::LenLt => Cmp::Lt,
            Cmp::LenLe => Cmp::Le,
            Cmp::LenGt => Cmp::Gt,
            Cmp::LenGe => Cmp::Ge,
            _ => return None,
        })
    }

    fn of_len(self) -> Cmp {
        match self {
            Cmp::Eq => Cmp::LenEq,
            Cmp::Ne => Cmp::LenNe,
            Cmp::Lt => Cmp::LenLt,
            Cmp::Le => Cmp::LenLe,
            Cmp::Gt => Cmp::LenGt,
            Cmp::Ge => Cmp::LenGe,
            other => other,
        }
    }

    fn bin_op(self) -> BinOp {
        match self {
            Cmp::Eq | Cmp::LenEq => BinOp::Eq,
            Cmp::Ne | Cmp::LenNe => BinOp::Ne,
            Cmp::Lt | Cmp::LenLt => BinOp::Lt,
            Cmp::Le | Cmp::LenLe => BinOp::Le,
            Cmp::Gt | Cmp::LenGt => BinOp::Gt,
            Cmp::Ge | Cmp::LenGe => BinOp::Ge,
            // Not binary operators: handled by `Condition::to_expr`.
            Cmp::Contains | Cmp::NotContains | Cmp::HasKey | Cmp::NotHasKey => BinOp::Eq,
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
                let target = Expr::Var(var.clone());
                let call = |f| Expr::Call(f, vec![target.clone(), value.to_expr()]);
                match cmp {
                    Cmp::Contains => call(Func::Contains),
                    Cmp::NotContains => Expr::negate(call(Func::Contains)),
                    Cmp::HasKey => call(Func::HasKey),
                    Cmp::NotHasKey => Expr::negate(call(Func::HasKey)),
                    c if c.len_cmp().is_some() => Expr::binary(
                        c.bin_op(),
                        Expr::Call(Func::Len, vec![target.clone()]),
                        value.to_expr(),
                    ),
                    c => Expr::binary(c.bin_op(), target.clone(), value.to_expr()),
                }
            }
            Condition::All(items) => fold(items, BinOp::And, true),
            Condition::Any(items) => fold(items, BinOp::Or, false),
            Condition::Expr(e) => e.clone(),
        }
    }

    /// `var contains value` / `var has key value` read from a call.
    fn from_call(expr: &Expr, negated: bool) -> Option<Condition> {
        let Expr::Call(f, args) = expr else {
            return None;
        };
        let [Expr::Var(var), v] = args.as_slice() else {
            return None;
        };
        let cmp = match (f, negated) {
            (Func::Contains, false) => Cmp::Contains,
            (Func::Contains, true) => Cmp::NotContains,
            (Func::HasKey, false) => Cmp::HasKey,
            (Func::HasKey, true) => Cmp::NotHasKey,
            _ => return None,
        };
        Some(Condition::Cmp {
            var: var.clone(),
            cmp,
            value: Operand::from_expr(v)?,
        })
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
                (Some(cmp), Expr::Call(Func::Len, args), Some(value)) => match args.as_slice() {
                    [Expr::Var(var)] => Condition::Cmp {
                        var: var.clone(),
                        cmp: cmp.of_len(),
                        value,
                    },
                    _ => Condition::Expr(expr.clone()),
                },
                _ => Condition::Expr(expr.clone()),
            },
            Expr::Lit(Value::Bool(true)) => Condition::All(Vec::new()),
            Expr::Lit(Value::Bool(false)) => Condition::Any(Vec::new()),
            Expr::Var(var) => Condition::is_true(var.clone()),
            Expr::Call(..) => {
                Self::from_call(expr, false).unwrap_or_else(|| Condition::Expr(expr.clone()))
            }
            Expr::Unary(UnOp::Not, inner) => match &**inner {
                Expr::Var(var) => Condition::cmp(var.clone(), Cmp::Eq, false),
                call @ Expr::Call(..) => {
                    Self::from_call(call, true).unwrap_or_else(|| Condition::Expr(expr.clone()))
                }
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
                    return out;
                }
                let want = cmp.operand_type(ty);
                if let Some(vt) = value.value_type(ty_of)
                    && !ValueType::compatible(vt, want)
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
    /// A neutral value of this type (`0`, `false`, empty text, empty
    /// collection).
    pub fn default_value(self) -> Value {
        match self {
            ValueType::Bool => Value::Bool(false),
            ValueType::Int => Value::Int(0),
            ValueType::Float => Value::Float(0.0),
            ValueType::Str => Value::Str(String::new()),
            ValueType::List(t) => Value::empty_list(t),
            ValueType::Map(t) => Value::empty_map(t),
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

    fn coll_vars() -> impl Fn(&str) -> Option<Value> + use<> {
        vars(&[
            ("bag", Value::list(ScalarType::Str, [Value::from("sword")])),
            ("gold", Value::map(ScalarType::Int, [("alice", Value::Int(3))])),
            ("i", Value::Int(0)),
            ("item", Value::from("key")),
        ])
    }

    fn coll_types(n: &str) -> Option<ValueType> {
        match n {
            "bag" => Some(ValueType::List(ScalarType::Str)),
            "gold" => Some(ValueType::Map(ScalarType::Int)),
            "i" => Some(ValueType::Int),
            "item" => Some(ValueType::Str),
            _ => None,
        }
    }

    #[test]
    fn collection_ops() {
        let get = coll_vars();
        let lit = |v: Value| Operand::Lit(v);
        let push = VarOp::new("bag", OpKind::Push, Operand::Var("item".into()));
        assert_eq!(
            push.apply(&get),
            Ok(Value::list(ScalarType::Str, ["sword", "key"].map(Value::from)))
        );
        let remove = VarOp::new("bag", OpKind::Remove, lit("sword".into()));
        assert_eq!(remove.apply(&get), Ok(Value::empty_list(ScalarType::Str)));
        let at = VarOp::new("bag", OpKind::RemoveAt, lit(Value::Int(4)));
        assert!(matches!(
            at.apply(&get),
            Err(OpError::Collection(EvalError::IndexOutOfBounds { index: 4, len: 1 }))
        ));
        let clear = VarOp::new("gold", OpKind::Clear, lit(Value::Bool(true)));
        assert_eq!(clear.apply(&get), Ok(Value::empty_map(ScalarType::Int)));
        // A float stored in an int map is rounded, like an int variable.
        let set = VarOp::new("gold", OpKind::MapSet, lit(Value::Float(2.6)))
            .with_key(lit("bob".into()));
        assert_eq!(
            set.apply(&get),
            Ok(Value::map(ScalarType::Int, [("alice", Value::Int(3)), ("bob", Value::Int(3))]))
        );
        let rm = VarOp::new("gold", OpKind::MapRemove, lit(Value::Bool(true)))
            .with_key(lit("alice".into()));
        assert_eq!(rm.apply(&get), Ok(Value::empty_map(ScalarType::Int)));
        // `Set` with an untyped empty literal takes the variable's type.
        let reset = VarOp::from_expr("bag", &Expr::parse("[]").unwrap());
        assert_eq!(reset, None, "`[]` is an expression, not an operand");
        let e = Expr::parse("[]").unwrap().eval(&get).unwrap();
        assert_eq!(
            coerce(e, ValueType::List(ScalarType::Str)),
            Ok(Value::empty_list(ScalarType::Str))
        );
        assert!(coerce(Value::list(ScalarType::Str, ["x".into()]), ValueType::List(ScalarType::Int)).is_err());
        assert_eq!(
            coerce(Value::list(ScalarType::Int, [Value::Int(1)]), ValueType::List(ScalarType::Float)),
            Ok(Value::list(ScalarType::Float, [Value::Float(1.0)]))
        );
    }

    #[test]
    fn collection_ops_round_trip_and_types() {
        let lit = |v: Value| Operand::Lit(v);
        for op in [
            VarOp::new("bag", OpKind::Push, lit("x".into())),
            VarOp::new("bag", OpKind::Remove, Operand::Var("item".into())),
            VarOp::new("bag", OpKind::RemoveAt, lit(Value::Int(0))),
            VarOp::new("bag", OpKind::Clear, lit(Value::Bool(true))),
            VarOp::new("gold", OpKind::MapSet, lit(Value::Int(1))).with_key(lit("k".into())),
            VarOp::new("gold", OpKind::MapRemove, lit(Value::Bool(true)))
                .with_key(Operand::Var("item".into())),
            VarOp::new("bag", OpKind::Set, lit(Value::list(ScalarType::Str, ["a".into()]))),
        ] {
            assert_eq!(VarOp::from_expr(&op.var, &op.to_expr()), Some(op.clone()));
            assert!(op.issues(&coll_types).is_empty(), "{op:?}: {:?}", op.issues(&coll_types));
            // The structured form survives RON, and `key` is only written when set.
            let ron = ron::to_string(&op).unwrap();
            assert_eq!(ron.contains("key"), op.key.is_some(), "{ron}");
            assert_eq!(ron::from_str::<VarOp>(&ron).unwrap(), op);
        }
        // Operations of the wrong kind or with operands of the wrong type.
        let bad = [
            VarOp::new("bag", OpKind::Push, lit(Value::Int(1))),
            VarOp::new("bag", OpKind::Add, lit("x".into())),
            VarOp::new("gold", OpKind::Push, lit(Value::Int(1))),
            VarOp::new("gold", OpKind::MapSet, lit(Value::Int(1))),
            VarOp::new("gold", OpKind::MapSet, lit(Value::Int(1))).with_key(lit(Value::Int(1))),
            VarOp::new("bag", OpKind::RemoveAt, lit("x".into())),
            VarOp::new("i", OpKind::Push, lit(Value::Int(1))),
            VarOp::new("bag", OpKind::Set, lit(Value::list(ScalarType::Int, [Value::Int(1)]))),
        ];
        for op in bad {
            assert!(!op.issues(&coll_types).is_empty(), "{op:?}");
        }
        // An empty literal of another element type is fine for `Set`.
        assert!(
            VarOp::new("bag", OpKind::Set, lit(Value::empty_list(ScalarType::Int)))
                .issues(&coll_types)
                .is_empty()
        );
        assert_eq!(OpKind::for_type(ValueType::List(ScalarType::Int))[0], OpKind::Push);
        assert!(OpKind::MapSet.supports(ValueType::Map(ScalarType::Str)));
        assert!(!OpKind::Push.supports(ValueType::Int));
        assert_eq!(OpKind::RemoveAt.operand_type(ValueType::List(ScalarType::Str)), Some(ValueType::Int));
        assert_eq!(OpKind::Push.operand_type(ValueType::List(ScalarType::Str)), Some(ValueType::Str));
        assert_eq!(OpKind::Clear.operand_type(ValueType::List(ScalarType::Str)), None);
        // An old operation (no key) still reads.
        let old: VarOp = ron::from_str("(var: \"n\", op: Add, value: Lit(Int(1)))").unwrap();
        assert_eq!(old, VarOp::new("n", OpKind::Add, Operand::Lit(Value::Int(1))));
    }

    #[test]
    fn collection_conditions() {
        let get = coll_vars();
        let c = |cond: Condition| cond.eval(&get);
        assert_eq!(c(Condition::cmp("bag", Cmp::Contains, "sword")), Ok(true));
        assert_eq!(c(Condition::cmp("bag", Cmp::NotContains, "sword")), Ok(false));
        assert_eq!(c(Condition::cmp("gold", Cmp::HasKey, "alice")), Ok(true));
        assert_eq!(c(Condition::cmp("gold", Cmp::NotHasKey, "alice")), Ok(false));
        assert_eq!(c(Condition::cmp("bag", Cmp::LenEq, 1i64)), Ok(true));
        assert_eq!(c(Condition::cmp("gold", Cmp::LenGt, 1i64)), Ok(false));
        for cond in [
            Condition::cmp("bag", Cmp::Contains, "x"),
            Condition::cmp("bag", Cmp::NotContains, "x"),
            Condition::cmp("gold", Cmp::HasKey, "x"),
            Condition::cmp("gold", Cmp::NotHasKey, "x"),
            Condition::cmp("bag", Cmp::LenGe, 2i64),
            Condition::cmp("gold", Cmp::LenNe, 0i64),
            Condition::All(vec![
                Condition::cmp("bag", Cmp::Contains, "x"),
                Condition::cmp("gold", Cmp::LenLt, 3i64),
            ]),
        ] {
            let back = Condition::from_expr(&cond.to_expr());
            assert_eq!(back, cond);
            assert!(back.is_structured());
            assert!(cond.issues(&coll_types).is_empty(), "{cond:?}");
        }
        assert_eq!(Condition::cmp("bag", Cmp::Contains, 1i64).issues(&coll_types).len(), 1);
        assert_eq!(Condition::cmp("bag", Cmp::HasKey, "x").issues(&coll_types).len(), 1);
        assert_eq!(Condition::cmp("bag", Cmp::Lt, "x").issues(&coll_types).len(), 1);
        assert_eq!(Condition::cmp("gold", Cmp::LenEq, "x").issues(&coll_types).len(), 1);
        assert_eq!(Condition::cmp("i", Cmp::Contains, 1i64).issues(&coll_types).len(), 1);
        assert_eq!(Cmp::for_type(ValueType::Map(ScalarType::Int))[0], Cmp::HasKey);
        // Raw expressions on collections are typed too.
        let raw = Condition::Expr(Expr::parse("bag[0] == 1").unwrap());
        assert_eq!(raw.issues(&coll_types).len(), 1);
    }
}
