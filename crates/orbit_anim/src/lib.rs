//! Animation primitives (ADR 0012): curves, tweens, keyframe tracks and clips.
//!
//! Everything here is pure: time is always an argument, nothing advances by
//! itself, and every transcendental function goes through `libm` so results are
//! identical on native and wasm. This crate depends on neither `orbit_core` nor
//! any GPU/platform crate.

mod clip;
mod curve;
mod lerp;
mod track;

pub use clip::{Clip, ClipTrack, Loop, Pose, TrackData, Value};
pub use curve::{Curve, Ease};
pub use lerp::{Angle, Lerp, Rgba, Step, Tween, Vec2};
pub use track::{Key, Track, TrackError};

#[cfg(test)]
mod tests;
