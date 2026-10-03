//! Dynamic variable values.
//!
//! Besides scalars (bool, int, float, text), a value can be a **list** or a
//! **map** (schema v11, ADR 0016): homogeneous collections of scalars, shared
//! copy-on-write through an [`Arc`] so that rollback snapshots stay cheap
//! (cloning a collection clones a pointer; modifying it copies only that
//! collection). Collections carry their element type, so an empty list still
//! knows what it holds. Nesting is not supported.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Most elements a list or map may hold (ADR 0016).
pub const MAX_COLLECTION_LEN: usize = 10_000;
/// Most bytes of text (elements and keys) a list or map may hold.
pub const MAX_COLLECTION_TEXT: usize = 64 * 1024;

/// A variable value stored in the game state.
///
/// New variants are appended at the end: postcard (saves, game packs)
/// encodes the variant index, so older data decodes unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// A list of scalars of the given type (schema v11).
    List(ScalarType, Arc<Vec<Value>>),
    /// Text keys to scalars of the given type, sorted by key (schema v11).
    Map(ScalarType, Arc<BTreeMap<String, Value>>),
}

/// Type of the elements of a collection: a scalar type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ScalarType {
    Bool,
    Int,
    Float,
    Str,
}

impl ScalarType {
    pub const ALL: [ScalarType; 4] = [
        ScalarType::Bool,
        ScalarType::Int,
        ScalarType::Float,
        ScalarType::Str,
    ];

    pub fn value_type(self) -> ValueType {
        match self {
            ScalarType::Bool => ValueType::Bool,
            ScalarType::Int => ValueType::Int,
            ScalarType::Float => ValueType::Float,
            ScalarType::Str => ValueType::Str,
        }
    }

    /// Name used by the text form of expressions (`[:int]`).
    pub fn keyword(self) -> &'static str {
        match self {
            ScalarType::Bool => "bool",
            ScalarType::Int => "int",
            ScalarType::Float => "float",
            ScalarType::Str => "string",
        }
    }

    pub fn from_keyword(s: &str) -> Option<ScalarType> {
        ScalarType::ALL.into_iter().find(|t| t.keyword() == s)
    }
}

/// Static type of a [`Value`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ValueType {
    Bool,
    Int,
    Float,
    Str,
    /// List of the given scalar type.
    List(ScalarType),
    /// Map from text to the given scalar type.
    Map(ScalarType),
}

impl ValueType {
    pub fn is_numeric(self) -> bool {
        matches!(self, ValueType::Int | ValueType::Float)
    }

    pub fn is_collection(self) -> bool {
        matches!(self, ValueType::List(_) | ValueType::Map(_))
    }

    /// The scalar type, for a scalar.
    pub fn scalar(self) -> Option<ScalarType> {
        match self {
            ValueType::Bool => Some(ScalarType::Bool),
            ValueType::Int => Some(ScalarType::Int),
            ValueType::Float => Some(ScalarType::Float),
            ValueType::Str => Some(ScalarType::Str),
            ValueType::List(_) | ValueType::Map(_) => None,
        }
    }

    /// Type of the elements, for a collection.
    pub fn element(self) -> Option<ValueType> {
        match self {
            ValueType::List(t) | ValueType::Map(t) => Some(t.value_type()),
            _ => None,
        }
    }

    /// `a` can be stored where `b` is expected without loss of meaning
    /// (same type, or two numbers).
    pub fn compatible(a: ValueType, b: ValueType) -> bool {
        a == b || (a.is_numeric() && b.is_numeric())
    }
}

impl fmt::Display for ScalarType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value_type().fmt(f)
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueType::Bool => f.write_str("bool"),
            ValueType::Int => f.write_str("int"),
            ValueType::Float => f.write_str("float"),
            ValueType::Str => f.write_str("string"),
            ValueType::List(t) => write!(f, "list of {t}"),
            ValueType::Map(t) => write!(f, "map of {t}"),
        }
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
            Value::List(t, _) => ValueType::List(*t),
            Value::Map(t, _) => ValueType::Map(*t),
        }
    }

    /// An empty list of `elem`.
    pub fn empty_list(elem: ScalarType) -> Value {
        Value::List(elem, Arc::new(Vec::new()))
    }

    /// An empty map of `elem`.
    pub fn empty_map(elem: ScalarType) -> Value {
        Value::Map(elem, Arc::new(BTreeMap::new()))
    }

    /// A list of `elem` holding `items` (converted to `elem` when they are
    /// numbers of the other kind).
    pub fn list(elem: ScalarType, items: impl IntoIterator<Item = Value>) -> Value {
        let items = items
            .into_iter()
            .map(|v| v.clone().into_scalar(elem).unwrap_or(v))
            .collect();
        Value::List(elem, Arc::new(items))
    }

    /// A map of `elem` holding `entries`.
    pub fn map<K: Into<String>>(
        elem: ScalarType,
        entries: impl IntoIterator<Item = (K, Value)>,
    ) -> Value {
        let entries = entries
            .into_iter()
            .map(|(k, v)| (k.into(), v.clone().into_scalar(elem).unwrap_or(v)))
            .collect();
        Value::Map(elem, Arc::new(entries))
    }

    /// Number of elements of a collection (characters of a text).
    pub fn len(&self) -> Option<usize> {
        match self {
            Value::List(_, l) => Some(l.len()),
            Value::Map(_, m) => Some(m.len()),
            Value::Str(s) => Some(s.chars().count()),
            _ => None,
        }
    }

    /// `true` for an empty collection or text.
    pub fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }

    /// This scalar converted to `ty`: numbers convert between int and float
    /// (a float stored in an int is rounded); anything else must match.
    /// `None` when it cannot be converted.
    pub fn into_scalar(self, ty: ScalarType) -> Option<Value> {
        match (self, ty) {
            (Value::Float(f), ScalarType::Int) => {
                let r = f.round();
                (f.is_finite() && r >= i64::MIN as f64 && r <= i64::MAX as f64)
                    .then_some(Value::Int(r as i64))
            }
            (Value::Int(i), ScalarType::Float) => Some(Value::Float(i as f64)),
            (v, t) if v.value_type() == t.value_type() => Some(v),
            _ => None,
        }
    }

    /// Whether this collection stays within [`MAX_COLLECTION_LEN`] and
    /// [`MAX_COLLECTION_TEXT`]. Scalars always do.
    pub fn within_limits(&self) -> bool {
        let text = |v: &Value| match v {
            Value::Str(s) => s.len(),
            _ => 0,
        };
        match self {
            Value::List(_, l) => {
                l.len() <= MAX_COLLECTION_LEN
                    && l.iter().map(text).sum::<usize>() <= MAX_COLLECTION_TEXT
            }
            Value::Map(_, m) => {
                m.len() <= MAX_COLLECTION_LEN
                    && m.iter().map(|(k, v)| k.len() + text(v)).sum::<usize>()
                        <= MAX_COLLECTION_TEXT
            }
            _ => true,
        }
    }

    /// Whether a collection only holds scalars of its declared element type.
    pub fn is_well_formed(&self) -> bool {
        match self {
            Value::List(t, l) => l.iter().all(|v| v.value_type() == t.value_type()),
            Value::Map(t, m) => m.values().all(|v| v.value_type() == t.value_type()),
            _ => true,
        }
    }

    /// Compares two values. Ints and floats are comparable with each other;
    /// lists compare element by element; maps are only equal or not
    /// (`None` when they differ); other mixed-type comparisons are undefined
    /// and return `None`.
    pub fn partial_compare(&self, other: &Value) -> Option<Ordering> {
        match (self, other) {
            (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
            (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
            (Value::Int(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
            (Value::Float(a), Value::Int(b)) => a.partial_cmp(&(*b as f64)),
            (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
            (Value::List(_, a), Value::List(_, b)) => {
                for (x, y) in a.iter().zip(b.iter()) {
                    match x.partial_compare(y)? {
                        Ordering::Equal => {}
                        o => return Some(o),
                    }
                }
                Some(a.len().cmp(&b.len()))
            }
            (Value::Map(_, a), Value::Map(_, b)) => {
                let equal = a.len() == b.len()
                    && a.iter().zip(b.iter()).all(|((ka, va), (kb, vb))| {
                        ka == kb && va.partial_compare(vb) == Some(Ordering::Equal)
                    });
                equal.then_some(Ordering::Equal)
            }
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    /// Text shown to players (UI bindings, interpolation): a list is its
    /// elements separated by commas, a map its `key: value` entries.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(v) => write!(f, "{v}"),
            Value::Int(v) => write!(f, "{v}"),
            Value::Float(v) => write!(f, "{v}"),
            Value::Str(v) => write!(f, "{v}"),
            Value::List(_, l) => {
                for (i, v) in l.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                Ok(())
            }
            Value::Map(_, m) => {
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                Ok(())
            }
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

    #[test]
    fn collections_compare_and_display() {
        let a = Value::list(ScalarType::Int, [Value::Int(1), Value::Int(2)]);
        let b = Value::list(ScalarType::Int, [Value::Int(1), Value::Int(3)]);
        assert_eq!(a.partial_compare(&b), Some(Ordering::Less));
        assert_eq!(a.partial_compare(&a.clone()), Some(Ordering::Equal));
        // An empty list equals another empty list whatever its element type.
        assert_eq!(
            Value::empty_list(ScalarType::Str).partial_compare(&Value::empty_list(ScalarType::Int)),
            Some(Ordering::Equal)
        );
        let m = Value::map(ScalarType::Int, [("b", Value::Int(2)), ("a", Value::Int(1))]);
        assert_eq!(m.to_string(), "a: 1, b: 2");
        assert_eq!(a.to_string(), "1, 2");
        assert_eq!(m.partial_compare(&Value::empty_map(ScalarType::Int)), None);
        assert_eq!(a.value_type(), ValueType::List(ScalarType::Int));
        assert_eq!(a.len(), Some(2));
        // Ints given to a float list are converted.
        let f = Value::list(ScalarType::Float, [Value::Int(1)]);
        assert!(f.is_well_formed());
        assert_eq!(f, Value::List(ScalarType::Float, Arc::new(vec![Value::Float(1.0)])));
    }

    #[test]
    fn limits() {
        let big = Value::list(ScalarType::Int, (0..=MAX_COLLECTION_LEN as i64).map(Value::Int));
        assert!(!big.within_limits());
        let ok = Value::list(ScalarType::Int, (0..MAX_COLLECTION_LEN as i64).map(Value::Int));
        assert!(ok.within_limits());
        let text = Value::list(ScalarType::Str, [Value::Str("x".repeat(MAX_COLLECTION_TEXT + 1))]);
        assert!(!text.within_limits());
    }

    #[test]
    fn ron_and_old_variants_are_unchanged() {
        let v = Value::list(ScalarType::Str, [Value::from("sword")]);
        let ron = ron::to_string(&v).unwrap();
        assert_eq!(ron, "List(Str,[Str(\"sword\")])");
        assert_eq!(ron::from_str::<Value>(&ron).unwrap(), v);
        assert_eq!(ron::to_string(&Value::Int(3)).unwrap(), "Int(3)");
    }
}
