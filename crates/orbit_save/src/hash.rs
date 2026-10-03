//! Stable hash of a story's *structure*.

use orbit_core::{NodeKind, Story};

/// Hash of the structure of `story`: the ordered scene ids, and for every node
/// its id and its kind (`Say`, `Choice`, ...). Texts, string tables, asset
/// paths, expressions' contents, jump targets and variable values are left
/// out on purpose, so fixing a typo or retiming a transition does not
/// invalidate saves, while adding, removing or retyping nodes does.
///
/// The algorithm is FNV-1a 64 over a fixed byte encoding; it never changes
/// between platforms or Rust versions (saves depend on it).
pub fn story_hash(story: &Story) -> u64 {
    let mut h = Fnv::new();
    h.write(&(story.scenes.len() as u64).to_le_bytes());
    for scene in &story.scenes {
        h.write(&scene.id.as_u128().to_le_bytes());
        h.write(&(scene.nodes.len() as u64).to_le_bytes());
        for node in &scene.nodes {
            h.write(&node.id.as_u128().to_le_bytes());
            h.write(&[kind_tag(&node.kind)]);
        }
    }
    h.finish()
}

/// Stable tag per node kind. Append new kinds with new numbers; never reuse.
fn kind_tag(kind: &NodeKind) -> u8 {
    match kind {
        NodeKind::Label { .. } => 1,
        NodeKind::Say { .. } => 2,
        NodeKind::Choice { .. } => 3,
        NodeKind::Jump { .. } => 4,
        NodeKind::SetVar { .. } => 5,
        NodeKind::If { .. } => 6,
        NodeKind::Background { .. } => 7,
        NodeKind::Show { .. } => 8,
        NodeKind::Hide { .. } => 9,
        NodeKind::PlayMusic { .. } => 10,
        NodeKind::StopMusic { .. } => 11,
        NodeKind::PlaySound { .. } => 12,
        NodeKind::Wait { .. } => 13,
        NodeKind::Call { .. } => 14,
        NodeKind::Return => 15,
        NodeKind::End => 16,
        NodeKind::Custom { .. } => 17,
        NodeKind::JumpScene { .. } => 18,
        NodeKind::ShowScene3d { .. } => 19,
        NodeKind::Camera3d { .. } => 20,
        NodeKind::HideScene3d { .. } => 21,
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}
