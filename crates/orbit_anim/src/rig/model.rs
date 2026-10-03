//! The `.rig.ron` data model (ADR 0013 §1–3).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::math::Xform;
use crate::clip::Loop;
use crate::lerp::{Angle, Lerp, Rgba, Step, Vec2};
use crate::track::Track;

/// Current `.rig.ron` format version (independent of the story schema).
pub const RIG_VERSION: u32 = 1;

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// Stable bone id (short ULID, ADR 0004): renaming never breaks clips or weights.
    BoneId
);
id_type!(
    /// Stable slot id.
    SlotId
);
id_type!(
    /// Stable attachment id (key inside a [`Skin`]).
    AttachmentId
);

fn white() -> Rgba {
    Rgba::new(1.0, 1.0, 1.0, 1.0)
}

fn is_white(c: &Rgba) -> bool {
    *c == white()
}

fn first_version() -> u32 {
    1
}

fn default_blend() -> f32 {
    0.2
}

/// A skeletal rig, stored in its own `.rig.ron` asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Rig {
    /// Format version; a missing field reads as 1 (the first format).
    #[serde(default = "first_version")]
    pub rig_version: u32,
    /// Topological order: a parent always comes before its children.
    #[serde(default)]
    pub bones: Vec<Bone>,
    /// Default draw order (first = behind).
    #[serde(default)]
    pub slots: Vec<Slot>,
    /// Appearance variants. The skin named `"default"` (or the first one) is
    /// the fallback for attachments missing in the active skin.
    #[serde(default)]
    pub skins: Vec<Skin>,
    #[serde(default)]
    pub clips: BTreeMap<String, RigClip>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ik: Vec<IkConstraint>,
}

impl Default for Rig {
    fn default() -> Self {
        Self {
            rig_version: RIG_VERSION,
            bones: Vec::new(),
            slots: Vec::new(),
            skins: Vec::new(),
            clips: BTreeMap::new(),
            ik: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Bone {
    pub id: BoneId,
    pub name: String,
    #[serde(default)]
    pub parent: Option<BoneId>,
    /// Setup (rest) pose, relative to the parent.
    #[serde(default)]
    pub rest: Xform,
    /// Length along the local X axis (editor display and IK).
    #[serde(default)]
    pub length: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Slot {
    pub id: SlotId,
    pub name: String,
    pub bone: BoneId,
    /// Attachment shown in the setup pose.
    #[serde(default)]
    pub attachment: Option<AttachmentId>,
    #[serde(default = "white", skip_serializing_if = "is_white")]
    pub color: Rgba,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Skin {
    pub name: String,
    #[serde(default)]
    pub attachments: BTreeMap<AttachmentId, Attachment>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Attachment {
    Region(RegionAttachment),
    Mesh(MeshAttachment),
}

impl Attachment {
    pub fn image(&self) -> &str {
        match self {
            Attachment::Region(r) => &r.image,
            Attachment::Mesh(m) => &m.image,
        }
    }
}

/// A rigid picture attached to the slot's bone, centered on `xform`'s origin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RegionAttachment {
    /// Asset reference of the picture.
    pub image: String,
    /// Size in rig units.
    pub size: [f32; 2],
    /// Placement relative to the slot's bone.
    #[serde(default)]
    pub xform: Xform,
}

/// One bone influence of a vertex.
pub type Influence = (BoneId, f32);

/// A textured triangle mesh deformed by bones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MeshAttachment {
    pub image: String,
    /// Vertex positions in rig space, in the setup pose (bind pose).
    pub vertices: Vec<[f32; 2]>,
    /// Texture coordinates `0..1`, one per vertex.
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u16; 3]>,
    /// Per vertex, 1–4 influences summing to 1. Empty: the whole mesh follows
    /// the slot's bone rigidly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weights: Vec<Vec<Influence>>,
}

/// Per-vertex offsets (rig units, bind space) added before skinning.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct Deform(pub Vec<[f32; 2]>);

impl Lerp for Deform {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        let n = a.0.len().max(b.0.len());
        let get = |v: &Vec<[f32; 2]>, i: usize| v.get(i).copied().unwrap_or([0.0, 0.0]);
        Deform(
            (0..n)
                .map(|i| {
                    let (p, q) = (get(&a.0, i), get(&b.0, i));
                    [f32::lerp(&p[0], &q[0], t), f32::lerp(&p[1], &q[1], t)]
                })
                .collect(),
        )
    }
}

/// An animation track with its rig target. Bone values are *relative to the
/// setup pose* (rotation and translation are added, scale multiplies), so
/// clips stay valid when the rest pose is tweaked and blend additively.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum RigTrack {
    BoneRotate { bone: BoneId, track: Track<Angle> },
    BoneTranslate { bone: BoneId, track: Track<Vec2> },
    BoneScale { bone: BoneId, track: Track<Vec2> },
    /// Discrete.
    SlotAttachment { slot: SlotId, track: Track<Step<Option<AttachmentId>>> },
    SlotColor { slot: SlotId, track: Track<Rgba> },
    /// Vertex offsets of one mesh attachment shown in `slot`.
    Deform { slot: SlotId, attachment: AttachmentId, track: Track<Deform> },
    /// Discrete; full slot order (first = behind). Missing slots keep their
    /// relative setup order after the listed ones.
    DrawOrder { track: Track<Step<Vec<SlotId>>> },
}

impl RigTrack {
    pub fn end_time(&self) -> f32 {
        match self {
            RigTrack::BoneRotate { track, .. } => track.end_time(),
            RigTrack::BoneTranslate { track, .. } | RigTrack::BoneScale { track, .. } => track.end_time(),
            RigTrack::SlotAttachment { track, .. } => track.end_time(),
            RigTrack::SlotColor { track, .. } => track.end_time(),
            RigTrack::Deform { track, .. } => track.end_time(),
            RigTrack::DrawOrder { track } => track.end_time(),
        }
    }

    pub fn validate_keys(&self) -> Result<(), crate::track::TrackError> {
        match self {
            RigTrack::BoneRotate { track, .. } => track.validate(),
            RigTrack::BoneTranslate { track, .. } | RigTrack::BoneScale { track, .. } => track.validate(),
            RigTrack::SlotAttachment { track, .. } => track.validate(),
            RigTrack::SlotColor { track, .. } => track.validate(),
            RigTrack::Deform { track, .. } => track.validate(),
            RigTrack::DrawOrder { track } => track.validate(),
        }
    }
}

/// A skeletal clip: same timing model as [`crate::Clip`] (duration + [`Loop`]),
/// tracks aimed at bones, slots and meshes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RigClip {
    pub duration: f32,
    #[serde(default)]
    pub looping: Loop,
    #[serde(default)]
    pub tracks: Vec<RigTrack>,
}

impl RigClip {
    pub fn new(duration: f32, looping: Loop) -> Self {
        Self { duration, looping, tracks: Vec::new() }
    }

    /// Elapsed time → time inside one pass (delegates to [`crate::Clip`]).
    pub fn local_time(&self, t: f32) -> f32 {
        crate::Clip { duration: self.duration, looping: self.looping, tracks: Vec::new() }.local_time(t)
    }
}

/// Analytic IK on 1 or 2 bones (ADR 0013 §3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct IkConstraint {
    /// `[bone]` aims the bone at the target; `[parent, child]` (child's parent
    /// must be `parent`) bends the pair so the child's tip reaches it.
    pub bones: Vec<BoneId>,
    /// Bone whose world origin is the goal.
    pub target: BoneId,
    /// Two-bone only: the child bends with a positive (clockwise) angle.
    #[serde(default)]
    pub bend_positive: bool,
    /// 0 = no effect, 1 = full IK.
    #[serde(default = "one")]
    pub mix: f32,
}

fn one() -> f32 {
    1.0
}

/// Declarative cross-fade state between two clips (ADR 0013 §2): nothing
/// advances by itself, the caller passes the clock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ClipBlend {
    pub current: ClipPlay,
    #[serde(default)]
    pub previous: Option<ClipPlay>,
    /// Clock time at which `current` replaced `previous`.
    #[serde(default)]
    pub changed_at: f32,
    /// Fade length in seconds.
    #[serde(default = "default_blend")]
    pub blend: f32,
}

/// A clip started at a clock time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ClipPlay {
    pub clip: String,
    pub started_at: f32,
}

impl ClipBlend {
    pub fn new(clip: impl Into<String>, started_at: f32) -> Self {
        Self {
            current: ClipPlay { clip: clip.into(), started_at },
            previous: None,
            changed_at: started_at,
            blend: default_blend(),
        }
    }

    /// Switches to `clip` at `now`, fading from the current one. Same clip:
    /// no change.
    pub fn switch(&mut self, clip: impl Into<String>, now: f32) {
        let clip = clip.into();
        if clip == self.current.clip {
            return;
        }
        let prev = std::mem::replace(&mut self.current, ClipPlay { clip, started_at: now });
        self.previous = Some(prev);
        self.changed_at = now;
    }

    /// Weight of `current` at `now` (0 → only previous, 1 → only current).
    pub fn weight(&self, now: f32) -> f32 {
        if self.previous.is_none() || self.blend <= 0.0 {
            return 1.0;
        }
        ((now - self.changed_at) / self.blend).clamp(0.0, 1.0)
    }
}
