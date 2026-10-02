use core::f32::consts::PI;

use serde::{Deserialize, Serialize};

/// Named easing presets. `eval(0) == 0` and `eval(1) == 1` for all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ease {
    Linear,
    InQuad,
    OutQuad,
    InOutQuad,
    InCubic,
    OutCubic,
    InOutCubic,
    InQuart,
    OutQuart,
    InOutQuart,
    InQuint,
    OutQuint,
    InOutQuint,
    InSine,
    OutSine,
    InOutSine,
    InExpo,
    OutExpo,
    InOutExpo,
    InCirc,
    OutCirc,
    InOutCirc,
    InBack,
    OutBack,
    InOutBack,
    InElastic,
    OutElastic,
    InOutElastic,
    InBounce,
    OutBounce,
    InOutBounce,
    /// Holds 0 until the very end, then jumps to 1.
    Step,
}

const BACK_C1: f32 = 1.70158;
const BACK_C2: f32 = BACK_C1 * 1.525;
const BACK_C3: f32 = BACK_C1 + 1.0;

fn pw(t: f32, n: i32) -> f32 {
    libm::powf(t, n as f32)
}

fn out_bounce(t: f32) -> f32 {
    const N: f32 = 7.5625;
    const D: f32 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let t = t - 1.5 / D;
        N * t * t + 0.75
    } else if t < 2.5 / D {
        let t = t - 2.25 / D;
        N * t * t + 0.9375
    } else {
        let t = t - 2.625 / D;
        N * t * t + 0.984375
    }
}

fn in_elastic(t: f32) -> f32 {
    let c4 = 2.0 * PI / 3.0;
    -libm::powf(2.0, 10.0 * t - 10.0) * libm::sinf((t * 10.0 - 10.75) * c4)
}

fn out_elastic(t: f32) -> f32 {
    let c4 = 2.0 * PI / 3.0;
    libm::powf(2.0, -10.0 * t) * libm::sinf((t * 10.0 - 0.75) * c4) + 1.0
}

/// Builds the in-out variant from an "in" function.
fn in_out(t: f32, f: impl Fn(f32) -> f32) -> f32 {
    if t < 0.5 {
        f(2.0 * t) / 2.0
    } else {
        1.0 - f(2.0 * (1.0 - t)) / 2.0
    }
}

impl Ease {
    /// `t` clamped to 0..=1; exact 0 and 1 at the ends.
    pub fn eval(self, t: f32) -> f32 {
        let t = if t.is_nan() { 0.0 } else { t };
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        use Ease::*;
        match self {
            Linear => t,
            Step => 0.0,
            InQuad => pw(t, 2),
            OutQuad => 1.0 - pw(1.0 - t, 2),
            InOutQuad => in_out(t, |x| pw(x, 2)),
            InCubic => pw(t, 3),
            OutCubic => 1.0 - pw(1.0 - t, 3),
            InOutCubic => in_out(t, |x| pw(x, 3)),
            InQuart => pw(t, 4),
            OutQuart => 1.0 - pw(1.0 - t, 4),
            InOutQuart => in_out(t, |x| pw(x, 4)),
            InQuint => pw(t, 5),
            OutQuint => 1.0 - pw(1.0 - t, 5),
            InOutQuint => in_out(t, |x| pw(x, 5)),
            InSine => 1.0 - libm::cosf(t * PI / 2.0),
            OutSine => libm::sinf(t * PI / 2.0),
            InOutSine => -(libm::cosf(PI * t) - 1.0) / 2.0,
            InExpo => libm::powf(2.0, 10.0 * t - 10.0),
            OutExpo => 1.0 - libm::powf(2.0, -10.0 * t),
            InOutExpo => in_out(t, |x| libm::powf(2.0, 10.0 * x - 10.0)),
            InCirc => 1.0 - libm::sqrtf(1.0 - t * t),
            OutCirc => libm::sqrtf(1.0 - pw(t - 1.0, 2)),
            InOutCirc => in_out(t, |x| 1.0 - libm::sqrtf(1.0 - x * x)),
            InBack => BACK_C3 * t * t * t - BACK_C1 * t * t,
            OutBack => 1.0 + BACK_C3 * pw(t - 1.0, 3) + BACK_C1 * pw(t - 1.0, 2),
            InOutBack => {
                if t < 0.5 {
                    (pw(2.0 * t, 2) * ((BACK_C2 + 1.0) * 2.0 * t - BACK_C2)) / 2.0
                } else {
                    (pw(2.0 * t - 2.0, 2) * ((BACK_C2 + 1.0) * (t * 2.0 - 2.0) + BACK_C2) + 2.0) / 2.0
                }
            }
            InElastic => in_elastic(t),
            OutElastic => out_elastic(t),
            InOutElastic => in_out(t, in_elastic),
            InBounce => 1.0 - out_bounce(1.0 - t),
            OutBounce => out_bounce(t),
            InOutBounce => in_out(t, |x| 1.0 - out_bounce(1.0 - x)),
        }
    }
}

/// An easing curve mapping progress 0..=1 to 0..=1 (may overshoot in between).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Curve {
    Preset(Ease),
    /// CSS `cubic-bezier(x1, y1, x2, y2)`: P0 = (0,0), P3 = (1,1); x are
    /// clamped to [0,1], y are free (overshoot).
    Bezier { p1: [f32; 2], p2: [f32; 2] },
    /// Reference to a story-level named curve; resolve with [`Curve::eval_with`].
    /// Evaluated as linear when unresolved.
    Named(String),
}

impl Default for Curve {
    fn default() -> Self {
        Curve::Preset(Ease::Linear)
    }
}

impl Curve {
    pub const LINEAR: Curve = Curve::Preset(Ease::Linear);

    pub const fn preset(e: Ease) -> Self {
        Curve::Preset(e)
    }

    pub const fn bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Curve::Bezier { p1: [x1, y1], p2: [x2, y2] }
    }

    /// CSS `ease`.
    pub const fn css_ease() -> Self {
        Self::bezier(0.25, 0.1, 0.25, 1.0)
    }

    /// CSS `ease-in`.
    pub const fn css_ease_in() -> Self {
        Self::bezier(0.42, 0.0, 1.0, 1.0)
    }

    /// CSS `ease-out`.
    pub const fn css_ease_out() -> Self {
        Self::bezier(0.0, 0.0, 0.58, 1.0)
    }

    /// CSS `ease-in-out`.
    pub const fn css_ease_in_out() -> Self {
        Self::bezier(0.42, 0.0, 0.58, 1.0)
    }

    /// Evaluates with no named-curve table (`Named` is linear).
    pub fn eval(&self, t: f32) -> f32 {
        self.eval_with(t, |_| None)
    }

    /// Evaluates resolving `Named` through `resolve`. A named curve that
    /// resolves to another `Named` (or to nothing) is linear: no cycles.
    pub fn eval_with<'a>(&self, t: f32, resolve: impl Fn(&str) -> Option<&'a Curve>) -> f32 {
        match self {
            Curve::Preset(e) => e.eval(t),
            Curve::Bezier { p1, p2 } => bezier_eval(*p1, *p2, t),
            Curve::Named(n) => match resolve(n) {
                Some(Curve::Named(_)) | None => Ease::Linear.eval(t),
                Some(c) => c.eval(t),
            },
        }
    }
}

/// Bezier with endpoints 0 and 1 and control values a, b.
fn sample_axis(a: f32, b: f32, s: f32) -> f32 {
    let u = 1.0 - s;
    3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
}

fn sample_axis_deriv(a: f32, b: f32, s: f32) -> f32 {
    let u = 1.0 - s;
    3.0 * u * u * a + 6.0 * u * s * (b - a) + 3.0 * s * s * (1.0 - b)
}

fn bezier_eval(p1: [f32; 2], p2: [f32; 2], t: f32) -> f32 {
    let t = if t.is_nan() { 0.0 } else { t };
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let x1 = p1[0].clamp(0.0, 1.0);
    let x2 = p2[0].clamp(0.0, 1.0);
    let (y1, y2) = (p1[1], p2[1]);
    // Newton, 4 iterations.
    let mut s = t;
    for _ in 0..4 {
        let err = sample_axis(x1, x2, s) - t;
        if libm::fabsf(err) < 1e-6 {
            break;
        }
        let d = sample_axis_deriv(x1, x2, s);
        if libm::fabsf(d) < 1e-6 {
            break;
        }
        s -= err / d;
    }
    // Fall back to bisection when Newton did not converge or left [0,1].
    if !(0.0..=1.0).contains(&s) || libm::fabsf(sample_axis(x1, x2, s) - t) > 1e-6 {
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        s = t;
        for _ in 0..40 {
            let x = sample_axis(x1, x2, s);
            if libm::fabsf(x - t) <= 1e-6 {
                break;
            }
            if x < t {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) / 2.0;
        }
    }
    sample_axis(y1, y2, s)
}
