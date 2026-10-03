use serde::{Deserialize, Serialize};

use crate::curve::Curve;
use crate::lerp::Lerp;

/// A keyframe. `ease` shapes the segment going from this key to the next.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Key<T> {
    /// Seconds, relative to the clip start.
    pub t: f32,
    pub value: T,
    #[serde(default)]
    pub ease: Curve,
}

/// Written by hand (instead of `skip_serializing_if`): the linear `ease` is
/// only left out of *human readable* formats. A binary, non self-describing
/// format such as the postcard game pack must always write it, or the reader
/// would misalign.
impl<T: Serialize> Serialize for Key<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let write_ease = !serializer.is_human_readable() || self.ease != Curve::default();
        let mut st = serializer.serialize_struct("Key", 2 + usize::from(write_ease))?;
        st.serialize_field("t", &self.t)?;
        st.serialize_field("value", &self.value)?;
        if write_ease {
            st.serialize_field("ease", &self.ease)?;
        } else {
            st.skip_field("ease")?;
        }
        st.end()
    }
}

impl<T> Key<T> {
    pub fn new(t: f32, value: T) -> Self {
        Self { t, value, ease: Curve::default() }
    }

    pub fn with_ease(t: f32, value: T, ease: Curve) -> Self {
        Self { t, value, ease }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackError {
    /// Key `i` has a NaN or negative time.
    BadTime(usize),
    /// Key `i` is not strictly after key `i - 1`.
    NotSorted(usize),
}

impl core::fmt::Display for TrackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TrackError::BadTime(i) => write!(f, "key {i} has an invalid time"),
            TrackError::NotSorted(i) => write!(f, "key {i} is not after the previous key"),
        }
    }
}

impl core::error::Error for TrackError {}

/// Keys sorted by strictly increasing time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct Track<T> {
    pub keys: Vec<Key<T>>,
}

impl<T> Default for Track<T> {
    fn default() -> Self {
        Self { keys: Vec::new() }
    }
}

impl<T> Track<T> {
    pub fn new(keys: Vec<Key<T>>) -> Self {
        Self { keys }
    }

    /// Time of the last key (0 if empty).
    pub fn end_time(&self) -> f32 {
        self.keys.last().map_or(0.0, |k| k.t)
    }

    pub fn validate(&self) -> Result<(), TrackError> {
        for (i, k) in self.keys.iter().enumerate() {
            if k.t.is_nan() || k.t < 0.0 {
                return Err(TrackError::BadTime(i));
            }
            if i > 0 && k.t <= self.keys[i - 1].t {
                return Err(TrackError::NotSorted(i));
            }
        }
        Ok(())
    }

    /// Sorts by time; on equal times the later key wins. Drops NaN times.
    pub fn normalize(&mut self) {
        self.keys.retain(|k| !k.t.is_nan());
        self.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        let mut out: Vec<Key<T>> = Vec::with_capacity(self.keys.len());
        for k in self.keys.drain(..) {
            match out.last_mut() {
                Some(last) if last.t == k.t => *last = k,
                _ => out.push(k),
            }
        }
        self.keys = out;
    }
}

impl<T: Lerp> Track<T> {
    /// Value at `t`: first value before the first key, last value after the
    /// last, `lerp(a, b, ease(u))` in between. `None` for an empty track.
    /// Assumes sorted keys (see [`Track::validate`]).
    pub fn sample(&self, t: f32) -> Option<T> {
        self.sample_with(t, |_| None)
    }

    /// Like [`Track::sample`], resolving `Curve::Named` through `resolve`.
    pub fn sample_with<'a>(&self, t: f32, resolve: impl Fn(&str) -> Option<&'a Curve>) -> Option<T> {
        let first = self.keys.first()?;
        let t = if t.is_nan() { 0.0 } else { t };
        // Number of keys at or before t.
        let idx = self.keys.partition_point(|k| k.t <= t);
        if idx == 0 {
            return Some(T::lerp(&first.value, &first.value, 0.0));
        }
        let a = &self.keys[idx - 1];
        let Some(b) = self.keys.get(idx) else {
            return Some(T::lerp(&a.value, &a.value, 0.0));
        };
        let u = (t - a.t) / (b.t - a.t);
        Some(T::lerp(&a.value, &b.value, a.ease.eval_with(u, resolve)))
    }
}
