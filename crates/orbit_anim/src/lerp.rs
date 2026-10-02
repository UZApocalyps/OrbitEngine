use serde::{Deserialize, Serialize};

use crate::curve::Curve;

/// Linear interpolation. `t` is NOT clamped (curves may overshoot).
pub trait Lerp: Sized {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self;
}

#[inline]
fn lf(a: f32, b: f32, t: f32) -> f32 {
    // Exact at both ends regardless of rounding.
    if t == 0.0 {
        a
    } else if t == 1.0 {
        b
    } else {
        a + (b - a) * t
    }
}

impl Lerp for f32 {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        lf(*a, *b, t)
    }
}

/// A 2D vector (offset, scale, pan...).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Lerp for Vec2 {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        Vec2::new(lf(a.x, b.x, t), lf(a.y, b.y, t))
    }
}

/// Linear RGBA color, components nominally in 0..=1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

impl Lerp for Rgba {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        // Overshooting curves must not push colors out of range.
        let c = |x: f32| x.clamp(0.0, 1.0);
        Rgba::new(
            c(lf(a.r, b.r, t)),
            c(lf(a.g, b.g, t)),
            c(lf(a.b, b.b, t)),
            c(lf(a.a, b.a, t)),
        )
    }
}

/// An angle in degrees, interpolated along the shortest arc.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Angle(pub f32);

impl Lerp for Angle {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        if t == 0.0 {
            return *a;
        }
        if t == 1.0 {
            return *b;
        }
        let mut d = libm::fmodf(b.0 - a.0, 360.0);
        if d > 180.0 {
            d -= 360.0;
        } else if d < -180.0 {
            d += 360.0;
        }
        Angle(a.0 + d * t)
    }
}

/// Discrete value: holds `a` until the end of the segment (staircase).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Step<T>(pub T);

impl<T: Clone> Lerp for Step<T> {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        if t >= 1.0 { b.clone() } else { a.clone() }
    }
}

/// A standalone interpolation between two values over a duration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tween<T> {
    pub from: T,
    pub to: T,
    /// Seconds. A zero (or negative) duration is an instant jump to `to`.
    pub duration: f32,
    pub curve: Curve,
}

impl<T: Lerp> Tween<T> {
    pub fn new(from: T, to: T, duration: f32, curve: Curve) -> Self {
        Self { from, to, duration, curve }
    }

    /// Value at `t` seconds since the start; exact `from` at 0 and `to` from
    /// `duration` on.
    pub fn sample(&self, t: f32) -> T {
        if self.duration <= 0.0 || t >= self.duration {
            return T::lerp(&self.from, &self.to, 1.0);
        }
        let u = if t > 0.0 { t / self.duration } else { 0.0 };
        T::lerp(&self.from, &self.to, self.curve.eval(u))
    }
}
