//! 2D affine math for rigs. Rig space is Y down (like images and the stage);
//! a positive rotation turns clockwise on screen. Angles are in degrees in the
//! data model and go through `libm` so results match on native and wasm.

use serde::{Deserialize, Serialize};

use crate::lerp::{Angle, Lerp};

fn one() -> f32 {
    1.0
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

fn is_one(v: &f32) -> bool {
    *v == 1.0
}

/// A local transform: scale, then rotate, then translate (no shear).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Xform {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub x: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub y: f32,
    /// Degrees, clockwise on screen.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub sx: f32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub sy: f32,
}

impl Default for Xform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Xform {
    pub const IDENTITY: Xform = Xform { x: 0.0, y: 0.0, rotation: 0.0, sx: 1.0, sy: 1.0 };

    pub fn translate(x: f32, y: f32) -> Self {
        Self { x, y, ..Self::IDENTITY }
    }

    pub fn matrix(&self) -> Affine {
        let r = self.rotation.to_radians();
        let (s, c) = (libm::sinf(r), libm::cosf(r));
        Affine { a: c * self.sx, b: s * self.sx, c: -s * self.sy, d: c * self.sy, tx: self.x, ty: self.y }
    }
}

impl Lerp for Xform {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        Xform {
            x: f32::lerp(&a.x, &b.x, t),
            y: f32::lerp(&a.y, &b.y, t),
            rotation: Angle::lerp(&Angle(a.rotation), &Angle(b.rotation), t).0,
            sx: f32::lerp(&a.sx, &b.sx, t),
            sy: f32::lerp(&a.sy, &b.sy, t),
        }
    }
}

/// A 2x3 affine matrix: `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    /// `self * rhs`: applies `rhs` first, then `self`.
    pub fn mul(&self, rhs: &Affine) -> Affine {
        Affine {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            tx: self.a * rhs.tx + self.c * rhs.ty + self.tx,
            ty: self.b * rhs.tx + self.d * rhs.ty + self.ty,
        }
    }

    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        [self.a * p[0] + self.c * p[1] + self.tx, self.b * p[0] + self.d * p[1] + self.ty]
    }

    pub fn det(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    /// `None` for a degenerate (zero scale) matrix.
    pub fn inverse(&self) -> Option<Affine> {
        let det = self.det();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let k = 1.0 / det;
        let (a, b, c, d) = (self.d * k, -self.b * k, -self.c * k, self.a * k);
        Some(Affine { a, b, c, d, tx: -(a * self.tx + c * self.ty), ty: -(b * self.tx + d * self.ty) })
    }

    /// `w * self` accumulated into `acc` (linear blend skinning).
    pub fn add_scaled(acc: &mut Affine, m: &Affine, w: f32) {
        acc.a += m.a * w;
        acc.b += m.b * w;
        acc.c += m.c * w;
        acc.d += m.d * w;
        acc.tx += m.tx * w;
        acc.ty += m.ty * w;
    }

    pub const ZERO: Affine = Affine { a: 0.0, b: 0.0, c: 0.0, d: 0.0, tx: 0.0, ty: 0.0 };

    /// Origin of the local frame.
    pub fn origin(&self) -> [f32; 2] {
        [self.tx, self.ty]
    }

    /// Direction angle of the local X axis, degrees.
    pub fn x_angle(&self) -> f32 {
        libm::atan2f(self.b, self.a).to_degrees()
    }
}
