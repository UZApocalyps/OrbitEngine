//! Copy-on-write of shared pictures. A picture file can be drawn by several
//! slots, rigs, expressions or UI files. Before the first pixel edit of a
//! picture (paint, erase, clone, fill, cut with a filled hole, edit in another
//! application) the rig editor asks [`orbit_project::picture_users`] whether
//! anything else uses it; if so a non-blocking dialog offers to make a copy for
//! this slot (default) or to edit the shared file. The choice is remembered
//! per picture for the session.
//!
//! Editor only and idle when unused: the users index is built once per
//! project revision / watcher epoch (in the background while the inspector
//! shows a picture, or on the spot at the first edit) and a stroke only does
//! a hash lookup.

use std::collections::HashMap;

use egui::{Align2, RichText, Ui, vec2};
use orbit_anim::rig::SlotId;
use orbit_editor_api::{EditorContext, TaskHandle};
use orbit_project::picture_users::{PictureUsers, norm, repoint_slot};

use super::model;
use super::pixeltools::{free_path, register, sanitize, write_png};
use super::State;
use crate::builtin::ui_util::{accent, faint, weak};

/// What the user chose for a shared picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// A copy for the edited slot.
    Copy,
    /// The shared file itself.
    Shared,
}

/// What an edit of a picture must do first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Not shared: the usual rules apply (copy of a foreign original...).
    Edit,
    /// Make a copy for the slot, edit that.
    Copy,
    /// Edit the shared file itself.
    InPlace,
    /// A dialog was opened: nothing is edited now.
    Ask,
}

/// The dialog asking what to do with a shared picture.
pub struct Prompt {
    pub path: String,
    pub slot: SlotId,
    pub users: Vec<String>,
}

/// What the index was built for.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    epoch: u64,
    revision: u64,
    doc: u64,
}

#[derive(Default)]
pub struct SharedState {
    index: Option<(Key, PictureUsers)>,
    job: Option<(Key, TaskHandle<PictureUsers>)>,
    choice: HashMap<String, Choice>,
    pub prompt: Option<Prompt>,
}

impl SharedState {
    /// Remembers what to do with a picture for the session.
    pub fn set_choice(&mut self, path: &str, choice: Choice) {
        self.choice.insert(norm(path), choice);
    }
}

fn key(ctx: &EditorContext<'_>, st: &State) -> Key {
    Key { epoch: ctx.state().asset_epoch, revision: ctx.project().map_or(0, |p| p.revision()), doc: st.doc_serial }
}

/// Builds the index now when it is stale (first edit of a picture).
fn ensure(ctx: &EditorContext<'_>, st: &mut State) {
    let k = key(ctx, st);
    if st.px.shared.index.as_ref().is_some_and(|(c, _)| *c == k) {
        return;
    }
    let (root, rel) = (st.project_root(ctx), st.project_rel(ctx));
    let index = ctx.story().map(|s| PictureUsers::build(&root, s, rel.as_deref())).unwrap_or_default();
    st.px.shared.index = Some((k, index));
    st.px.shared.job = None;
}

/// Starts (or collects) the background scan: the inspector asks for it while
/// it shows a picture, so the badge is ready without a pause.
fn prefetch(ctx: &mut EditorContext<'_>, st: &mut State) {
    let k = key(ctx, st);
    let mut done = None;
    {
        let sh = &mut st.px.shared;
        if sh.index.as_ref().is_some_and(|(c, _)| *c == k) {
            return;
        }
        if let Some((jk, h)) = sh.job.as_mut() {
            match h.poll() {
                Some(r) => done = Some((*jk, r)),
                None if *jk == k => return,
                None => {}
            }
        }
    }
    if let Some((jk, r)) = done {
        st.px.shared.job = None;
        if let Ok(index) = r
            && jk == k
        {
            st.px.shared.index = Some((k, index));
            return;
        }
    }
    let (root, rel) = (st.project_root(ctx), st.project_rel(ctx));
    let Some(story) = ctx.story().cloned() else { return };
    let handle = ctx.state().spawn_task(ctx.t("rig.task.users"), None, move |_| {
        Ok(PictureUsers::build(&root, &story, rel.as_deref()))
    });
    st.px.shared.job = Some((k, handle));
}

/// The users of `path` other than `slot` of the open rig, worded; `None`
/// while the index is not ready (`wait` false).
fn users(ctx: &mut EditorContext<'_>, st: &mut State, path: &str, slot: &SlotId, wait: bool) -> Option<Vec<String>> {
    if wait {
        ensure(ctx, st);
    } else {
        prefetch(ctx, st);
    }
    let rel = st.project_rel(ctx).unwrap_or_default();
    let (_, index) = st.px.shared.index.as_ref()?;
    // Other rig files (the open rig is not in the index) ...
    let mut out: Vec<String> = index.users(path).iter().map(ToString::to_string).collect();
    // ... and the open rig as it is in memory (unsaved edits included).
    if let Some(doc) = st.doc.as_ref() {
        let rig = doc.rig();
        let path = norm(path);
        for s in rig.slots.iter().filter(|s| &s.id != slot) {
            let draws = s.attachment.as_ref().is_some_and(|a| {
                rig.skins.iter().any(|k| k.attachments.get(a).is_some_and(|att| norm(att.image()) == path))
            });
            if draws {
                out.push(format!("{rel} / {}", s.name));
            }
        }
    }
    Some(out)
}

/// Decides how an edit of `path` by `slot` goes on; opens the dialog when the
/// picture is shared and the user has not chosen yet.
pub fn decide(ctx: &mut EditorContext<'_>, st: &mut State, path: &str, slot: &SlotId) -> Decision {
    let others = users(ctx, st, path, slot, true).unwrap_or_default();
    if others.is_empty() {
        return Decision::Edit;
    }
    match st.px.shared.choice.get(&norm(path)) {
        Some(Choice::Copy) => Decision::Copy,
        Some(Choice::Shared) => Decision::InPlace,
        None => {
            st.px.shared.prompt = Some(Prompt { path: path.to_owned(), slot: slot.clone(), users: others });
            Decision::Ask
        }
    }
}

/// `<rigname>_<slot>.png` next to `beside`, not on disk yet.
pub fn copy_path(ctx: &EditorContext<'_>, st: &State, beside: &str, slot_name: &str) -> String {
    let rel = st.project_rel(ctx).unwrap_or_default();
    let file = rel.rsplit('/').next().unwrap_or("");
    let rig = file.strip_suffix(".rig.ron").or_else(|| file.rsplit_once('.').map(|(s, _)| s)).unwrap_or("rig");
    let stem = format!("{}_{}", sanitize(rig), sanitize(slot_name));
    free_path(&st.project_root(ctx), beside, &stem)
}

/// Writes a copy of `path` for `slot`, declares it and points the slot to it.
/// Part of the open undo step (the caller opens and closes the gesture).
pub fn make_copy(ctx: &mut EditorContext<'_>, st: &mut State, path: &str, slot: &SlotId) -> Option<String> {
    let doc = st.doc.as_ref()?;
    let c = doc.canvas(path)?.clone();
    let slot_name = model::slot(doc.rig(), slot).map(|s| s.name.clone())?;
    let copy = copy_path(ctx, st, path, &slot_name);
    let root = st.project_root(ctx);
    if let Err(e) = write_png(&root.join(&copy), c.w, c.h, &c.rgba) {
        ctx.error(ctx.tf("rig.pixels.write_failed", &[("path", &copy), ("error", &e)]));
        return None;
    }
    register(ctx, &copy);
    let doc = st.doc.as_mut()?;
    doc.set_canvas(&copy, c.w, c.h, c.rgba);
    let (id, from) = (slot.as_str().to_owned(), path.to_owned());
    let to = copy.clone();
    let _ = doc.edit(|rig| {
        repoint_slot(rig, &id, &from, &to);
        Ok(())
    });
    st.px.owned.insert(copy.clone());
    if let Some(s) = st.image_sizes.get(path).copied() {
        st.image_sizes.insert(copy.clone(), s);
    }
    ctx.info(ctx.tf("rig.shared.copied", &[("from", &path), ("path", &copy)]));
    Some(copy)
}

/// "Make unique": the copy as one undo step of its own.
pub fn make_unique(ctx: &mut EditorContext<'_>, st: &mut State, path: &str, slot: &SlotId) -> Option<String> {
    let doc = st.doc.as_mut()?;
    doc.begin_gesture();
    let copy = make_copy(ctx, st, path, slot);
    if let Some(d) = st.doc.as_mut() {
        d.end_gesture();
    }
    copy
}

/// The "Shared by N" badge, its tooltip and the "Make unique" button, under
/// the picture picker of the inspector.
pub fn badge(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State, slot: &SlotId, path: Option<&str>) {
    let Some(path) = path else { return };
    let Some(others) = users(ctx, st, path, slot, false).filter(|u| !u.is_empty()) else { return };
    let tip = format!("{}\n{}", ctx.t("rig.shared.tip"), others.join("\n"));
    let mut unique = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(ctx.tf("rig.shared.badge", &[("n", &(others.len() + 1))])).small().strong().color(accent(ui)))
            .on_hover_text(tip);
        unique = ui.small_button(ctx.t("rig.shared.make_unique")).on_hover_text(ctx.t("rig.shared.make_unique_hint")).clicked();
    });
    if unique {
        make_unique(ctx, st, path, slot);
    }
}

/// The dialog of a shared picture (non-blocking: the editor stays usable).
pub fn dialog(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(prompt) = st.px.shared.prompt.take() else { return };
    let (mut copy, mut shared, mut cancel) = (false, false, false);
    egui::Window::new(ctx.t("rig.shared.title"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label(format!("{} {}", ctx.t("rig.shared.intro"), prompt.path));
            for u in prompt.users.iter().take(8) {
                ui.label(format!("  - {u}"));
            }
            if prompt.users.len() > 8 {
                ui.label(RichText::new(ctx.tf("rig.shared.more", &[("n", &(prompt.users.len() - 8))])).color(weak(ui)));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                copy = ui
                    .add(egui::Button::new(RichText::new(ctx.t("rig.shared.copy")).strong()))
                    .on_hover_text(ctx.t("rig.shared.copy_hint"))
                    .clicked();
                shared = ui.button(ctx.t("rig.shared.edit")).on_hover_text(ctx.t("rig.shared.edit_hint")).clicked();
                cancel = ui.button(ctx.t("rig.cancel")).clicked();
            });
            ui.label(RichText::new(ctx.t("rig.shared.remember")).small().color(faint(ui)));
        });
    if copy {
        st.px.shared.set_choice(&prompt.path, Choice::Copy);
        make_unique(ctx, st, &prompt.path, &prompt.slot);
    } else if shared {
        st.px.shared.set_choice(&prompt.path, Choice::Shared);
    } else if !cancel {
        st.px.shared.prompt = Some(prompt);
    }
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
