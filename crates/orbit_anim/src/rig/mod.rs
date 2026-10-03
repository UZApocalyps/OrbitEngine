//! 2D skeletal rigs (ADR 0013): the `.rig.ron` model, pose evaluation
//! (clips, cross-fades, 1–2 bone IK, world matrices), CPU skinning behind
//! [`Skinner`], validation and web budget checks. GPU-free.
//!
//! ```ignore
//! let rig = PreparedRig::new(load_rig(text)?.rig)?;
//! let pose = rig.sample_blend(&blend_state, now);
//! let world = rig.world(&pose);
//! let list = rig.draw_list(&pose, &world, None, &CpuSkinner);
//! ```

mod eval;
mod format;
mod math;
mod model;
mod validate;

pub use eval::{
    CpuSkinner, PreparedMesh, PreparedRig, RigDrawItem, RigDrawList, RigPose, SkinJob, Skinner, SlotPose,
    WorldPose,
};
pub use format::{LoadedRig, RigLoadError, load_rig, migrate_rig_text, rig_text_version, save_rig};
pub use math::{Affine, Xform};
pub use model::{
    Attachment, AttachmentId, Bone, BoneId, ClipBlend, ClipPlay, Deform, IkConstraint, Influence, MeshAttachment,
    RIG_VERSION, RegionAttachment, Rig, RigClip, RigTrack, Skin, Slot, SlotId,
};
pub use validate::{MAX_INFLUENCES, RigIssue, RigIssueKind, Severity, WEB_MAX_BONES, WEB_MAX_VERTICES};

#[cfg(test)]
mod tests;
