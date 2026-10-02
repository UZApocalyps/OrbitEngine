//! Dynamic variable values.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A variable value stored in the game state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

/// Static type of a [`Value`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ValueType {
    Bool,
    Int,
    Float,
    Str,
}

impl ValueType {
    pub fn is_numeric(self) -> bool {
        matches!(self, ValueType::Int | ValueType::Float)
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ValueType::Bool => "bool",
            ValueType::Int => "int",
            ValueType::Float => "float",
            ValueType::Str => "string",
        })
    }
}

impl Value {
    /// Static type of this value.
    pub fn value_type(&self) -> ValueType {
        match self {
            Value::Bool(_) => ValueType::Bool,
            Value::Int(_) => ValueType::Int,
            Value::Float(_) => ValueType::Float,
            Value::Str(_) => ValueType::Str,
        }
    }

    /// Compares two values. Ints and floats are comparable with each other;
    /// other mixed-type comparisons are undefined and return `None`.
    pub fn partial_compare(&self, other: &Value) -> Option<Ordering> {
        match (self, other) {
            (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
            (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
            (Value::Int(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
            (Value::Float(a), Value::Int(b)) => a.partial_cmp(&(*b as f64)),
            (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(v) => write!(f, "{v}"),
            Value::Int(v) => write!(f, "{v}"),
            Value::Float(v) => write!(f, "{v}"),
            Value::Str(v) => write!(f, "{v}"),
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Str(v.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_cross_compare() {
        assert_eq!(
            Value::Int(2).partial_compare(&Value::Float(2.5)),
            Some(Ordering::Less)
        );
        assert_eq!(Value::Int(1).partial_compare(&Value::Str("1".into())), None);
    }
}
