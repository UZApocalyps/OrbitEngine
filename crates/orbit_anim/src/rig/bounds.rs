//! Bounds of what a rig draws in its setup pose, and the "origin far from the
//! drawing" check (the rig origin `(0, 0)` is the character's bottom-center,
//! where it is placed on the stage). Editor and tooling side: the runtime
//! never calls this.

use super::math::Affine;
use super::model::{Attachment, Rig, Skin};

/// Horizontal tolerance of the origin, as a fraction of the drawing's width.
pub const ORIGIN_TOLERANCE_X: f32 = 0.25;
/// Vertical tolerance of the origin (either side of the bottom edge), as a
/// fraction of the drawing's height.
pub const ORIGIN_TOLERANCE_Y: f32 = 0.10;

/// Axis aligned bounds `[min_x, min_y, max_x, max_y]` (Y down: `max_y` is the
/// bottom).
pub type Bounds = [f32; 4];

impl Rig {
    /// The skin used when none is named: `"default"`, else the first.
    pub fn default_skin(&self) -> Option<&Skin> {
        self.skins.iter().find(|s| s.name == "default").or_else(|| self.skins.first())
    }

    /// Real bounds of the visible attachments (the slot's setup attachment)
    /// in the setup pose. Mesh vertices and region corners count. `skin`:
    /// the active skin (None: the default one) with the default skin as
    /// fallback for what it lacks; `all_skins`: every skin's pictures. `None`
    /// when nothing is drawn.
    pub fn rest_bounds(&self, skin: Option<&str>, all_skins: bool) -> Option<Bounds> {
        let locals: Vec<Affine> = self.bones.iter().map(|b| b.rest.matrix()).collect();
        let index: std::collections::HashMap<&str, usize> =
            self.bones.iter().enumerate().map(|(i, b)| (b.id.as_str(), i)).collect();
        let mut world: Vec<Affine> = Vec::with_capacity(locals.len());
        for (i, b) in self.bones.iter().enumerate() {
            let w = match b.parent.as_ref().and_then(|p| index.get(p.as_str())) {
                Some(&p) if p < world.len() => world[p].mul(&locals[i]),
                _ => locals[i],
            };
            world.push(w);
        }
        let active = skin.and_then(|n| self.skins.iter().find(|s| s.name == n)).or_else(|| self.default_skin());
        let fallback = self.default_skin();
        let mut b: Option<Bounds> = None;
        let mut grow = |p: [f32; 2]| {
            if !(p[0].is_finite() && p[1].is_finite()) {
                return;
            }
            let r = b.get_or_insert([p[0], p[1], p[0], p[1]]);
            r[0] = r[0].min(p[0]);
            r[1] = r[1].min(p[1]);
            r[2] = r[2].max(p[0]);
            r[3] = r[3].max(p[1]);
        };
        for slot in &self.slots {
            let Some(att_id) = &slot.attachment else { continue };
            let Some(&bi) = index.get(slot.bone.as_str()) else { continue };
            let found: Vec<&Attachment> = if all_skins {
                self.skins.iter().filter_map(|s| s.attachments.get(att_id)).collect()
            } else {
                active
                    .and_then(|s| s.attachments.get(att_id))
                    .or_else(|| fallback.and_then(|s| s.attachments.get(att_id)))
                    .into_iter()
                    .collect()
            };
            for att in found {
                match att {
                    Attachment::Mesh(m) => m.vertices.iter().for_each(|v| grow(*v)),
                    Attachment::Region(r) => {
                        let m = world[bi].mul(&r.xform.matrix());
                        let (hx, hy) = (r.size[0] * 0.5, r.size[1] * 0.5);
                        for c in [[-hx, -hy], [hx, -hy], [hx, hy], [-hx, hy]] {
                            grow(m.apply(c));
                        }
                    }
                }
            }
        }
        b
    }
}

/// How far the origin is from the bottom-center of `bounds`: `(dx, dy)` with
/// `dx` = center X, `dy` = bottom Y (both 0 for a perfectly placed rig), and
/// whether that is beyond the tolerances.
pub fn origin_offset(bounds: &Bounds) -> ([f32; 2], bool) {
    let (w, h) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    let d = [(bounds[0] + bounds[2]) * 0.5, bounds[3]];
    let far = d[0].abs() > ORIGIN_TOLERANCE_X * w.max(1.0) || d[1].abs() > ORIGIN_TOLERANCE_Y * h.max(1.0);
    (d, far)
}
