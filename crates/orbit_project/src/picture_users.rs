//! Who uses a picture: the index behind copy-on-write painting in the rig
//! editor. Before the first pixel edit of a picture the editor asks this
//! index whether anything else shows the same file (another rig, another
//! slot of the same rig, a character expression, a background, the UI), so
//! a stroke never changes a picture behind the back of another user.
//!
//! Pure and editor-only: [`PictureUsers::build`] reads the rig files and the
//! UI files of a project once; the editor keeps the result until the
//! project or its files change.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::Path;

use orbit_anim::rig::{Attachment, Rig};
use orbit_core::{AssetKind, Story};

use crate::ext::{AssetUser, asset_users};

/// One user of a picture.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PictureUser {
    /// A slot of a rig file (`slot` is the attachment id when no slot shows it).
    Rig { rig: String, slot: String },
    /// Something of the story (a background node, an expression, an icon...), worded.
    Story(String),
    /// A file of the project's `ui/` folder.
    Ui(String),
}

impl fmt::Display for PictureUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PictureUser::Rig { rig, slot } => write!(f, "{rig} / {slot}"),
            PictureUser::Story(what) => f.write_str(what),
            PictureUser::Ui(file) => f.write_str(file),
        }
    }
}

/// Picture path as keys use it (forward slashes).
pub fn norm(path: &str) -> String {
    path.replace('\\', "/")
}

/// Every picture of a project and its users.
#[derive(Clone, Debug, Default)]
pub struct PictureUsers {
    by_picture: HashMap<String, BTreeSet<PictureUser>>,
}

impl PictureUsers {
    /// Reads the rig files (`assets/rigs` and the story's rig assets), the
    /// story and the `ui/` folder under `root`. `skip_rig`: a rig file
    /// (relative path) to leave out because the caller adds its in-memory
    /// version with [`PictureUsers::add_rig`].
    pub fn build(root: &Path, story: &Story, skip_rig: Option<&str>) -> Self {
        let mut me = Self::default();
        let mut rigs: BTreeSet<String> =
            story.assets.assets.iter().filter(|a| a.kind == AssetKind::Rig).map(|a| norm(&a.path)).collect();
        if let Ok(dir) = std::fs::read_dir(root.join(crate::rigs::RIG_DIR)) {
            for e in dir.flatten() {
                if let Ok(n) = e.file_name().into_string()
                    && n.ends_with(".rig.ron")
                {
                    rigs.insert(format!("{}/{n}", crate::rigs::RIG_DIR));
                }
            }
        }
        for path in rigs {
            if skip_rig.is_some_and(|s| norm(s) == path) {
                continue;
            }
            if let Ok(rig) = crate::rigs::read_rig(root, &path) {
                me.add_rig(&path, &rig);
            }
        }
        me.add_story(story);
        me.add_ui(&root.join("ui"), "ui", 0);
        me
    }

    fn add(&mut self, picture: &str, user: PictureUser) {
        self.by_picture.entry(norm(picture)).or_default().insert(user);
    }

    /// Adds the pictures a rig draws (one user per slot).
    pub fn add_rig(&mut self, rig_path: &str, rig: &Rig) {
        let rig_path = norm(rig_path);
        for skin in &rig.skins {
            for (id, att) in &skin.attachments {
                let slot = rig
                    .slots
                    .iter()
                    .find(|s| s.attachment.as_ref() == Some(id))
                    .map_or_else(|| id.to_string(), |s| s.name.clone());
                self.add(att.image(), PictureUser::Rig { rig: rig_path.clone(), slot });
            }
        }
    }

    fn add_story(&mut self, story: &Story) {
        for entry in story.assets.assets.iter().filter(|a| a.kind == AssetKind::Image) {
            for u in asset_users(story, entry.id) {
                let what = match u {
                    AssetUser::Node(n) => format!("story node {n}"),
                    AssetUser::Character(c) => format!("character {c}"),
                    AssetUser::Achievement(a) => format!("achievement {a}"),
                };
                self.add(&entry.path, PictureUser::Story(what));
            }
        }
    }

    /// Picture paths quoted in the `.ron` files of the UI folder.
    fn add_ui(&mut self, dir: &Path, rel: &str, depth: u32) {
        let Ok(read) = std::fs::read_dir(dir) else { return };
        for e in read.flatten() {
            let Ok(name) = e.file_name().into_string() else { continue };
            let (path, rel) = (e.path(), format!("{rel}/{name}"));
            if path.is_dir() {
                if depth < 3 {
                    self.add_ui(&path, &rel, depth + 1);
                }
            } else if name.ends_with(".ron")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                for quoted in text.split('"').skip(1).step_by(2) {
                    let lower = quoted.to_ascii_lowercase();
                    if [".png", ".jpg", ".jpeg", ".webp"].iter().any(|x| lower.ends_with(x)) {
                        self.add(quoted, PictureUser::Ui(rel.clone()));
                    }
                }
            }
        }
    }

    /// Every user of `picture`.
    pub fn users(&self, picture: &str) -> Vec<PictureUser> {
        self.by_picture.get(&norm(picture)).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }

    /// The users of `picture` other than slot `slot` of rig `rig`.
    pub fn others(&self, picture: &str, rig: &str, slot: &str) -> Vec<PictureUser> {
        let me = PictureUser::Rig { rig: norm(rig), slot: slot.to_owned() };
        self.users(picture).into_iter().filter(|u| *u != me).collect()
    }
}

/// Every attachment shown by slot `slot` (id or name) that draws `from`
/// draws `to` instead, in all skins. Other slots keep `from`. Returns how
/// many attachments changed.
pub fn repoint_slot(rig: &mut Rig, slot: &str, from: &str, to: &str) -> usize {
    let Some(att) =
        rig.slots.iter().find(|s| s.id.as_str() == slot || s.name == slot).and_then(|s| s.attachment.clone())
    else {
        return 0;
    };
    let mut n = 0;
    for k in &mut rig.skins {
        match k.attachments.get_mut(&att) {
            Some(Attachment::Mesh(m)) if m.image == from => {
                m.image = to.to_owned();
                n += 1;
            }
            Some(Attachment::Region(r)) if r.image == from => {
                r.image = to.to_owned();
                n += 1;
            }
            _ => {}
        }
    }
    n
}

#[cfg(test)]
#[path = "picture_users_tests.rs"]
mod tests;
