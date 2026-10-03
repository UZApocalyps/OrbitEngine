use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::curve::Curve;
use crate::lerp::{Angle, Rgba, Step, Vec2};
use crate::track::Track;

/// What happens when the clip time passes `duration`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Loop {
    #[default]
    Once,
    /// Plays `n` times in total (`Loop(0)` is already finished).
    Loop(u32),
    Forever,
    /// Forward then backward, forever.
    PingPong,
}

/// A sampled value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Value {
    Scalar(f32),
    Vec2(Vec2),
    Color(Rgba),
    Angle(Angle),
    /// Discrete (staircase) value, e.g. an image or slot name.
    Name(String),
}

/// A typed track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TrackData {
    Scalar(Track<f32>),
    Vec2(Track<Vec2>),
    Color(Track<Rgba>),
    Angle(Track<Angle>),
    Name(Track<Step<String>>),
}

impl TrackData {
    pub fn is_empty(&self) -> bool {
        match self {
            TrackData::Scalar(t) => t.keys.is_empty(),
            TrackData::Vec2(t) => t.keys.is_empty(),
            TrackData::Color(t) => t.keys.is_empty(),
            TrackData::Angle(t) => t.keys.is_empty(),
            TrackData::Name(t) => t.keys.is_empty(),
        }
    }

    pub fn end_time(&self) -> f32 {
        match self {
            TrackData::Scalar(t) => t.end_time(),
            TrackData::Vec2(t) => t.end_time(),
            TrackData::Color(t) => t.end_time(),
            TrackData::Angle(t) => t.end_time(),
            TrackData::Name(t) => t.end_time(),
        }
    }

    pub fn validate(&self) -> Result<(), crate::track::TrackError> {
        match self {
            TrackData::Scalar(t) => t.validate(),
            TrackData::Vec2(t) => t.validate(),
            TrackData::Color(t) => t.validate(),
            TrackData::Angle(t) => t.validate(),
            TrackData::Name(t) => t.validate(),
        }
    }

    /// Samples at a time inside one pass (see [`Clip::local_time`]).
    pub fn sample<'a>(&self, t: f32, r: impl Fn(&str) -> Option<&'a Curve>) -> Option<Value> {
        Some(match self {
            TrackData::Scalar(x) => Value::Scalar(x.sample_with(t, r)?),
            TrackData::Vec2(x) => Value::Vec2(x.sample_with(t, r)?),
            TrackData::Color(x) => Value::Color(x.sample_with(t, r)?),
            TrackData::Angle(x) => Value::Angle(x.sample_with(t, r)?),
            TrackData::Name(x) => Value::Name(x.sample_with(t, r)?.0),
        })
    }
}

/// A channel (key chosen by the caller, e.g. `"offset"`) and its track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ClipTrack {
    pub channel: String,
    pub data: TrackData,
}

/// Sampled values by channel. Two tracks on one channel: the later one wins.
pub type Pose = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Clip {
    /// Seconds.
    pub duration: f32,
    #[serde(default)]
    pub looping: Loop,
    pub tracks: Vec<ClipTrack>,
}

impl Clip {
    pub fn new(duration: f32) -> Self {
        Self { duration, looping: Loop::Once, tracks: Vec::new() }
    }

    /// Total play time, `None` when infinite.
    pub fn total_duration(&self) -> Option<f32> {
        let d = self.duration.max(0.0);
        match self.looping {
            Loop::Once => Some(d),
            Loop::Loop(n) => Some(d * n as f32),
            Loop::Forever | Loop::PingPong => None,
        }
    }

    /// True once a non-infinite clip has played to its end at time `t`.
    pub fn is_finished(&self, t: f32) -> bool {
        self.total_duration().is_some_and(|total| t >= total)
    }

    /// Maps elapsed time to the time inside one pass. Finished clips map to
    /// exactly `duration` (final state).
    pub fn local_time(&self, t: f32) -> f32 {
        let d = self.duration.max(0.0);
        let t = if t.is_nan() || t < 0.0 { 0.0 } else { t };
        if d == 0.0 || self.is_finished(t) {
            return d;
        }
        match self.looping {
            Loop::Once => t,
            Loop::Loop(_) | Loop::Forever => libm::fmodf(t, d),
            Loop::PingPong => {
                let m = libm::fmodf(t, 2.0 * d);
                if m <= d { m } else { 2.0 * d - m }
            }
        }
    }

    /// Samples every track at elapsed time `t`.
    pub fn sample(&self, t: f32) -> Pose {
        self.sample_with(t, |_| None)
    }

    /// Like [`Clip::sample`], resolving named curves.
    pub fn sample_with<'a>(&self, t: f32, resolve: impl Fn(&str) -> Option<&'a Curve> + Copy) -> Pose {
        let lt = self.local_time(t);
        let mut pose = Pose::new();
        for tr in &self.tracks {
            if let Some(v) = tr.data.sample(lt, resolve) {
                pose.insert(tr.channel.clone(), v);
            }
        }
        pose
    }
}
