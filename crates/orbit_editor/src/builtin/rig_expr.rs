//! Rigged expressions in the characters panel (M5B-07): the choice of the
//! rig, skin, clip and loop mode, and the animated thumbnail.
//!
//! The first half is the pure model (reading a rig file into the lists the
//! pickers show, building the [`RiggedMotion`], finding what is wrong with
//! it), unit tested without a GPU; the second half is the egui widget. The
//! thumbnail is drawn by the host ([`crate::rig_host`]), which this module
//! only talks to through the shared [`ThumbLink`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::SystemTime;

use egui::{ComboBox, RichText, Ui, vec2};
use orbit_anim::rig::{PreparedRig, Rig};
use orbit_core::{AssetId, AssetKind, CharacterId, Expression, LoopMode, Motion, RiggedMotion, Story};
use orbit_editor_api::fields::asset_picker;
use orbit_editor_api::{EditorContext, UiText};

use super::anim_edit::loop_label;
use super::ui_util::{danger, weak};
use crate::rig_host::{SharedThumbs, ThumbRequest};

/// What the pickers need to know about a usable rig.
pub struct RigInfo {
    pub prepared: Rc<PreparedRig>,
    /// `(name, duration in seconds)`, by name.
    pub clips: Vec<(String, f32)>,
    pub skins: Vec<String>,
}

impl RigInfo {
    /// Resolves `rig` like the game does; the error is worded.
    pub fn from_rig(rig: Rig) -> Result<Self, String> {
        let clips = rig.clips.iter().map(|(n, c)| (n.clone(), c.duration)).collect();
        let skins = rig.skins.iter().map(|s| s.name.clone()).collect();
        let prepared = PreparedRig::new(rig).map_err(|issues| {
            issues.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
        })?;
        Ok(Self {
            prepared: Rc::new(prepared),
            clips,
            skins,
        })
    }

    pub fn has_clip(&self, name: &str) -> bool {
        self.clips.iter().any(|(n, _)| n == name)
    }

    pub fn has_skin(&self, name: &str) -> bool {
        self.skins.iter().any(|s| s == name)
    }
}

/// What is wrong with the choices made for a rigged expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RigIssue {
    Unreadable(String),
    NoClips,
    UnknownClip(String),
    UnknownSkin(String),
}

/// The problems of `motion` given what its rig file turned out to be.
pub fn issues(info: &Result<Rc<RigInfo>, String>, motion: &RiggedMotion) -> Vec<RigIssue> {
    let info = match info {
        Ok(info) => info,
        Err(e) => return vec![RigIssue::Unreadable(e.clone())],
    };
    let mut out = Vec::new();
    if info.clips.is_empty() {
        out.push(RigIssue::NoClips);
    } else if !info.has_clip(&motion.clip) {
        out.push(RigIssue::UnknownClip(motion.clip.clone()));
    }
    if let Some(skin) = &motion.skin
        && !info.has_skin(skin)
    {
        out.push(RigIssue::UnknownSkin(skin.clone()));
    }
    out
}

/// The motion for `rig`: what `previous` chose when the rig still offers it
/// (skin, clip), else the rig's first clip and the default skin. An
/// unreadable rig (`info` is `None`) keeps the previous choices.
pub fn make_motion(rig: AssetId, info: Option<&RigInfo>, previous: Option<&RiggedMotion>) -> RiggedMotion {
    let loop_mode = previous.map_or(LoopMode::Loop, |p| p.loop_mode);
    let prev_clip = previous.map(|p| p.clip.clone());
    let prev_skin = previous.and_then(|p| p.skin.clone());
    let Some(info) = info else {
        return RiggedMotion {
            rig,
            skin: prev_skin,
            clip: prev_clip.unwrap_or_default(),
            loop_mode,
        };
    };
    let clip = prev_clip
        .filter(|c| info.has_clip(c))
        .or_else(|| info.clips.first().map(|(n, _)| n.clone()))
        .unwrap_or_default();
    RiggedMotion {
        rig,
        skin: prev_skin.filter(|s| info.has_skin(s)),
        clip,
        loop_mode,
    }
}

/// Makes the expression play `motion` (the poster `image` is kept: it is the
/// fallback and the thumbnail while the rig cannot be drawn).
pub fn set_motion(e: &mut Expression, motion: RiggedMotion) {
    e.motion = Some(Motion::Rigged(motion));
}

fn motion_mut(e: &mut Expression) -> Option<&mut RiggedMotion> {
    match &mut e.motion {
        Some(Motion::Rigged(m)) => Some(m),
        _ => None,
    }
}

/// Points the expression at another rig, keeping skin and clip when it has them.
pub fn set_rig(e: &mut Expression, rig: AssetId, info: Option<&RigInfo>) {
    let motion = make_motion(rig, info, e.rigged());
    set_motion(e, motion);
}

pub fn set_skin(e: &mut Expression, skin: Option<String>) {
    if let Some(m) = motion_mut(e) {
        m.skin = skin;
    }
}

pub fn set_clip(e: &mut Expression, clip: String) {
    if let Some(m) = motion_mut(e) {
        m.clip = clip;
    }
}

pub fn set_loop_mode(e: &mut Expression, mode: LoopMode) {
    if let Some(m) = motion_mut(e) {
        m.loop_mode = mode;
    }
}

// ------------------------------------------------------------------ cache

struct Entry {
    path: String,
    stamp: Option<(SystemTime, u64)>,
    info: Result<Rc<RigInfo>, String>,
}

/// The rig files the panel shows, read once and again when they change on
/// disk (the rig editor saves them while the panel is open).
#[derive(Default)]
pub struct RigCache {
    entries: RefCell<HashMap<AssetId, Entry>>,
}

fn stamp_of(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

impl RigCache {
    /// The rig of asset `id`: `None` when it is not a rig asset of `story`.
    pub fn get(&self, root: &Path, story: &Story, id: AssetId) -> Option<Result<Rc<RigInfo>, String>> {
        let entry = story.assets.get(id).filter(|a| a.kind == AssetKind::Rig)?;
        let stamp = stamp_of(&root.join(&entry.path));
        let mut map = self.entries.borrow_mut();
        if let Some(cached) = map.get(&id)
            && cached.path == entry.path
            && cached.stamp == stamp
        {
            return Some(cached.info.clone());
        }
        let info = orbit_project::rigs::read_rig(root, &entry.path)
            .and_then(RigInfo::from_rig)
            .map(Rc::new);
        map.insert(
            id,
            Entry {
                path: entry.path.clone(),
                stamp,
                info: info.clone(),
            },
        );
        Some(info)
    }
}

/// What the characters panel keeps for rigged expressions.
#[derive(Default)]
pub struct RigUi {
    pub cache: RigCache,
    pub thumbs: SharedThumbs,
}

impl RigUi {
    pub fn new(thumbs: SharedThumbs) -> Self {
        Self {
            cache: RigCache::default(),
            thumbs,
        }
    }
}

/// "name (1.25 s)" from the translated template `{name} ({seconds} s)`.
pub fn clip_label(template: &str, name: &str, seconds: f32) -> String {
    template
        .replace("{name}", name)
        .replace("{seconds}", &format!("{seconds:.2}"))
}

// ----------------------------------------------------------------- widget

fn first_rig(story: &Story) -> Option<AssetId> {
    story
        .assets
        .assets
        .iter()
        .find(|a| a.kind == AssetKind::Rig)
        .map(|a| a.id)
}

/// The expression with the "Rig" type chosen: plays the first rig of the
/// project. `None` when the project has no rig asset.
pub fn switch_to_rig(story: &Story, root: &Path, ui: &RigUi, e: &Expression) -> Option<Expression> {
    let rig = first_rig(story)?;
    let info = ui.cache.get(root, story, rig).and_then(Result::ok);
    let mut new = e.clone();
    set_rig(&mut new, rig, info.as_deref());
    Some(new)
}

/// Whether the project has a rig asset to switch to.
pub fn has_rigs(story: &Story) -> bool {
    first_rig(story).is_some()
}

/// The controls of a rigged expression: rig, skin, clip, loop mode, and the
/// reason the rig cannot play. Returns the changed expression.
pub fn editor(
    ui: &mut Ui,
    story: &Story,
    root: &Path,
    rig_ui: &RigUi,
    salt: (CharacterId, usize),
    e: &Expression,
) -> Option<Expression> {
    let motion = e.rigged()?.clone();
    let mut new = e.clone();
    let info = rig_ui.cache.get(root, story, motion.rig);
    let readable = info.as_ref().and_then(|i| i.as_ref().ok()).cloned();

    if let Some(rig) = asset_picker(ui, ("rig_asset", salt.0, salt.1), story, AssetKind::Rig, motion.rig) {
        let info = rig_ui.cache.get(root, story, rig).and_then(Result::ok);
        set_rig(&mut new, rig, info.as_deref());
    }

    ui.horizontal(|ui| {
        ui.label(RichText::new(ui.t("defs.anim.rig.skin")).small().color(weak(ui)));
        let default_label = ui.t("defs.anim.rig.skin_default");
        let current = motion.skin.clone();
        let shown = current.clone().unwrap_or_else(|| default_label.clone());
        let mut chosen = current.clone();
        ComboBox::from_id_salt(("rig_skin", salt.0, salt.1))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, &default_label);
                if let Some(info) = &readable {
                    for s in &info.skins {
                        ui.selectable_value(&mut chosen, Some(s.clone()), s);
                    }
                }
            });
        if chosen != current {
            set_skin(&mut new, chosen);
        }
    });

    ui.horizontal(|ui| {
        ui.label(RichText::new(ui.t("defs.anim.rig.clip")).small().color(weak(ui)));
        let item = ui.t("defs.anim.rig.clip_item");
        let label = |name: &str, info: &Option<Rc<RigInfo>>| match info
            .as_ref()
            .and_then(|i| i.clips.iter().find(|(n, _)| n == name))
        {
            Some((_, d)) => clip_label(&item, name, *d),
            None => name.to_owned(),
        };
        let mut chosen = motion.clip.clone();
        ComboBox::from_id_salt(("rig_clip", salt.0, salt.1))
            .selected_text(label(&motion.clip, &readable))
            .show_ui(ui, |ui| {
                if let Some(info) = &readable {
                    for (name, _) in &info.clips {
                        let text = label(name, &readable);
                        ui.selectable_value(&mut chosen, name.clone(), text);
                    }
                }
            });
        if chosen != motion.clip {
            set_clip(&mut new, chosen);
        }
        let mut mode = motion.loop_mode;
        ComboBox::from_id_salt(("rig_loop", salt.0, salt.1))
            .selected_text(loop_label(ui, mode))
            .show_ui(ui, |ui| {
                for m in [LoopMode::Loop, LoopMode::Once, LoopMode::PingPong] {
                    let text = loop_label(ui, m);
                    ui.selectable_value(&mut mode, m, text);
                }
            });
        if mode != motion.loop_mode {
            set_loop_mode(&mut new, mode);
        }
    });

    if let Some(info) = &info {
        for issue in issues(info, &motion) {
            let text = match &issue {
                RigIssue::Unreadable(err) => ui.tf("defs.anim.rig.unreadable", &[("error", err)]),
                RigIssue::NoClips => ui.t("defs.anim.rig.no_clips"),
                RigIssue::UnknownClip(c) => ui.tf("defs.anim.rig.unknown_clip", &[("clip", c)]),
                RigIssue::UnknownSkin(s) => ui.tf("defs.anim.rig.unknown_skin", &[("skin", s)]),
            };
            ui.label(RichText::new(text).small().color(danger(ui)));
        }
    }
    (new != *e).then_some(new)
}

/// Draws the animated thumbnail of rigged expression `index` of `character`:
/// the clip's pose at the current time, drawn by the host into a texture.
/// Returns false (nothing drawn) when the rig cannot be played or there is no
/// GPU yet; the caller then shows the poster.
pub fn preview(
    ui: &mut Ui,
    ctx: &EditorContext<'_>,
    story: &Story,
    rig_ui: &RigUi,
    key: (CharacterId, usize),
    e: &Expression,
    max: egui::Vec2,
) -> bool {
    let (Some(motion), Some(project)) = (e.rigged(), ctx.project_info()) else {
        return false;
    };
    let root = project.root.clone();
    let Some(Ok(info)) = rig_ui.cache.get(&root, story, motion.rig) else {
        return false;
    };
    if !issues(&Ok(info.clone()), motion).is_empty() {
        return false;
    }
    let time = ui.input(|i| i.time) as f32;
    let pose = orbit_runtime::rigs::clip_pose(&info.prepared, story, motion, time);
    let skin = motion.skin.clone().filter(|s| info.has_skin(s));
    let texture = {
        let mut link = rig_ui.thumbs.borrow_mut();
        link.requests.push(ThumbRequest {
            key,
            rig: info.prepared.clone(),
            pose,
            skin,
            root,
        });
        link.available.then(|| link.textures.get(&key).copied()).flatten()
    };
    // Redraw continuously while visible (the host draws after the UI pass).
    ui.ctx().request_repaint();
    let Some(texture) = texture else {
        // First frame: the host has not made the texture yet.
        return false;
    };
    let size = vec2(max.x, max.y);
    ui.allocate_ui(max, |ui| {
        ui.add(egui::Image::new(egui::load::SizedTexture::new(texture, size)).fit_to_exact_size(size));
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIG: &str = r#"(
        rig_version: 1,
        bones: [(id: "root", name: "root")],
        slots: [(id: "s", name: "body", bone: "root", attachment: Some("body"))],
        skins: [
            (name: "default", attachments: {"body": Region((image: "assets/body.png", size: (10.0, 20.0)))}),
            (name: "winter", attachments: {"body": Region((image: "assets/body.png", size: (10.0, 20.0)))}),
        ],
        clips: {
            "idle": (duration: 1.0, looping: Forever, tracks: []),
            "wave": (duration: 2.5, looping: Once, tracks: []),
        },
    )"#;

    fn info() -> RigInfo {
        let rig = orbit_anim::rig::load_rig(RIG).unwrap().rig;
        RigInfo::from_rig(rig).unwrap()
    }

    fn still() -> Expression {
        Expression::still("happy", AssetId::new())
    }

    #[test]
    fn the_lists_come_from_the_rig_file() {
        let i = info();
        assert_eq!(i.clips, vec![("idle".to_owned(), 1.0), ("wave".to_owned(), 2.5)]);
        assert!(i.has_skin("winter") && !i.has_skin("summer"));
        assert!(i.has_clip("wave") && !i.has_clip("run"));
    }

    #[test]
    fn a_rig_that_cannot_be_resolved_is_an_error_with_a_reason() {
        let mut rig = orbit_anim::rig::load_rig(RIG).unwrap().rig;
        rig.slots[0].bone = orbit_anim::rig::BoneId("ghost".into());
        let Err(reason) = RigInfo::from_rig(rig) else { panic!("must fail") };
        assert!(!reason.is_empty());
    }

    #[test]
    fn switching_to_a_rig_takes_the_first_clip_and_keeps_the_poster() {
        let mut e = still();
        let poster = e.image;
        let rig = AssetId::new();
        set_rig(&mut e, rig, Some(&info()));
        let m = e.rigged().unwrap();
        assert_eq!((m.rig, m.clip.as_str(), m.skin.clone(), m.loop_mode), (rig, "idle", None, LoopMode::Loop));
        assert_eq!(e.image, poster);
    }

    #[test]
    fn changing_the_rig_keeps_the_choices_the_new_rig_has() {
        let mut e = still();
        let (a, b) = (AssetId::new(), AssetId::new());
        set_rig(&mut e, a, Some(&info()));
        set_clip(&mut e, "wave".into());
        set_skin(&mut e, Some("winter".into()));
        set_loop_mode(&mut e, LoopMode::PingPong);
        set_rig(&mut e, b, Some(&info()));
        let m = e.rigged().unwrap();
        assert_eq!((m.rig, m.clip.as_str(), m.skin.as_deref(), m.loop_mode), (b, "wave", Some("winter"), LoopMode::PingPong));
        // A rig without that clip / skin falls back to its first clip and the default skin.
        let mut small = orbit_anim::rig::load_rig(RIG).unwrap().rig;
        small.clips.remove("wave");
        small.skins.pop();
        set_rig(&mut e, a, Some(&RigInfo::from_rig(small).unwrap()));
        let m = e.rigged().unwrap();
        assert_eq!((m.clip.as_str(), m.skin.clone()), ("idle", None));
        // An unreadable rig keeps the choices.
        set_skin(&mut e, Some("winter".into()));
        set_rig(&mut e, b, None);
        assert_eq!(e.rigged().unwrap().skin.as_deref(), Some("winter"));
    }

    #[test]
    fn setters_do_nothing_on_a_still() {
        let mut e = still();
        let before = e.clone();
        set_clip(&mut e, "idle".into());
        set_skin(&mut e, Some("x".into()));
        set_loop_mode(&mut e, LoopMode::Once);
        assert_eq!(e, before);
    }

    #[test]
    fn issues_name_what_is_wrong() {
        let ok: Result<Rc<RigInfo>, String> = Ok(Rc::new(info()));
        let mut m = make_motion(AssetId::new(), Some(&info()), None);
        assert_eq!(issues(&ok, &m), vec![]);
        m.clip = "run".into();
        m.skin = Some("summer".into());
        assert_eq!(
            issues(&ok, &m),
            vec![RigIssue::UnknownClip("run".into()), RigIssue::UnknownSkin("summer".into())]
        );
        let bad: Result<Rc<RigInfo>, String> = Err("no such file".into());
        assert_eq!(issues(&bad, &m), vec![RigIssue::Unreadable("no such file".into())]);
    }

    #[test]
    fn the_cache_reads_a_project_rig_and_follows_the_file() {
        let dir = std::env::temp_dir().join(format!("orbit_rig_expr_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets/rigs")).unwrap();
        std::fs::write(dir.join("assets/rigs/hero.rig.ron"), RIG).unwrap();
        let mut story = Story::new("t");
        let id = AssetId::new();
        story.assets.assets.push(orbit_core::AssetEntry {
            id,
            kind: AssetKind::Rig,
            path: "assets/rigs/hero.rig.ron".into(),
        });
        let cache = RigCache::default();
        let info = cache.get(&dir, &story, id).unwrap().unwrap();
        assert_eq!(info.clips.len(), 2);
        // The file changes (different size): the cache follows.
        std::fs::write(dir.join("assets/rigs/hero.rig.ron"), "not a rig at all, really").unwrap();
        assert!(cache.get(&dir, &story, id).unwrap().is_err());
        // Not a rig asset: nothing.
        assert!(cache.get(&dir, &story, AssetId::new()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
