//! The rig editor's background jobs: the heavy rig operations (cut + fill +
//! re-trace, re-trace, new rig from a picture, bind, layered import) run on
//! the editor's worker pool (`orbit_editor_api::tasks`) on a **copy** of the
//! rig, and their result is applied here, on the UI thread, as one undo step.
//!
//! One job at a time per rig document: other heavy rig actions are refused
//! while it runs (status bar: progress and cancel). A result is applied only
//! if the document did not change meanwhile (same document, revision and
//! pixels); otherwise nothing happens and the user is told to run it again.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use egui::{Align2, Color32, FontId, Rect, Ui};
use orbit_anim::rig::{BoneId, Rig, SlotId};
use orbit_editor_api::{EditorContext, Progress, TaskError, TaskHandle};

use super::layered_import::LayersOut;
use super::model::RigDoc;
use super::pixeltools::CutOut;
use super::State;

/// What the jobs hold (status bar, `Tasks::busy`).
pub const DOC: &str = "orbit.rig";

/// What a job computed, applied by [`poll`].
pub enum Out {
    Cut(Box<CutOut>),
    /// "Re-trace mesh": the rig with the meshes traced again.
    Retraced { rig: Rig, n: usize },
    /// "Bind to bones": the rig with new weights, meshes bound.
    Bound { rig: Rig, n: usize },
    /// "New rig from image": the new document, its root bone and slot.
    NewDoc { doc: Box<RigDoc>, root: BoneId, slot: SlotId, path: String, size: [f32; 2] },
    /// Layered file read and parsed: asks the questions, then imports.
    LayersParsed { doc: Box<orbit_layers::LayeredDoc>, path: PathBuf, new_rig: bool },
    /// Layered import done: pictures written, rig built.
    Layers(Box<LayersOut>),
}

pub struct RigJob {
    pub title: String,
    pub handle: TaskHandle<Out>,
    /// Files the job writes (it may add to them while it runs): removed when
    /// its result is not applied.
    pub files: Files,
}

/// Identity of the document's content: another document, a rig edit or a
/// pixel edit all change it.
pub fn stamp(st: &State) -> u64 {
    st.doc.as_ref().map_or(0, |d| {
        (st.doc_serial << 48) ^ (d.revision << 24) ^ d.pixel_rev
    })
}

/// The files a job writes, shared with it.
pub type Files = Arc<Mutex<Vec<PathBuf>>>;

pub fn files(list: Vec<PathBuf>) -> Files {
    Arc::new(Mutex::new(list))
}

/// A rig job runs: tells the user and returns `true` (the caller stops).
pub fn busy(ctx: &mut EditorContext<'_>, st: &State) -> bool {
    match &st.job {
        Some(job) => {
            ctx.warn(ctx.tf("rig.task.busy", &[("task", &job.title)]));
            true
        }
        None => false,
    }
}

/// Starts `work` on the worker pool on owned inputs. `files` are the files it
/// may write (cleaned up when its result is dropped).
pub fn start(
    ctx: &mut EditorContext<'_>,
    st: &mut State,
    title_key: &str,
    files: Files,
    work: impl FnOnce(&Progress) -> Result<Out, TaskError> + Send + 'static,
) {
    let title = ctx.t(title_key);
    let handle = ctx.state().spawn_task(title.clone(), Some(DOC), work).with_base(stamp(st));
    st.job = Some(RigJob { title, handle, files });
}

/// Applies the finished job, if any (called at the top of every rig panel).
pub fn poll(ctx: &mut EditorContext<'_>, st: &mut State) {
    let current = stamp(st);
    let Some(outcome) = st.job.as_mut().and_then(|j| j.handle.poll_fresh(current)) else { return };
    let Some(job) = st.job.take() else { return };
    let out = match outcome {
        Ok(out) => out,
        Err(e) => {
            for f in job.files.lock().map(|f| f.clone()).unwrap_or_default() {
                let _ = std::fs::remove_file(f);
            }
            match e {
                TaskError::Cancelled => ctx.info(ctx.tf("rig.task.cancelled", &[("task", &job.title)])),
                TaskError::Stale => ctx.warn(ctx.tf("rig.task.stale", &[("task", &job.title)])),
                TaskError::Failed(m) => {
                    // Jobs may fail with a text key (`rig.err.*`).
                    let m = if m.starts_with("rig.") { ctx.t(&m) } else { m };
                    ctx.warn(ctx.tf("rig.task.failed", &[("task", &job.title), ("error", &m)]))
                }
                TaskError::Panicked => {
                    ctx.error(ctx.tf("rig.task.failed", &[("task", &job.title), ("error", &ctx.t("rig.err.unknown"))]))
                }
            }
            return;
        }
    };
    match out {
        Out::Cut(cut) => super::pixeltools::apply_cut(ctx, st, *cut),
        Out::Retraced { rig, n } => {
            if let Some(doc) = st.doc.as_mut() {
                let _ = doc.edit(|r| {
                    *r = rig;
                    Ok(())
                });
                ctx.info(ctx.tn("rig.paint.retraced", n, &[]));
            }
        }
        Out::Bound { rig, n } => st.finish_bind(ctx, rig, n),
        Out::NewDoc { doc, root, slot, path, size } => st.finish_new_from_image(ctx, *doc, root, slot, path, size),
        Out::LayersParsed { doc, path, new_rig } => st.import_parsed_layers(ctx, *doc, &path, new_rig),
        Out::Layers(out) => st.finish_import_layers(ctx, *out),
    }
}

/// The job running, as an overlay on `rect` (spinner, title, progress): the
/// rest of the editor stays usable.
pub fn overlay(ui: &Ui, st: &State, rect: Rect) {
    let Some(job) = &st.job else { return };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(90));
    let center = rect.center();
    let spinner = Rect::from_center_size(center - egui::vec2(0.0, 18.0), egui::vec2(24.0, 24.0));
    egui::Spinner::new().size(24.0).paint_at(ui, spinner);
    let text = match job.handle.fraction() {
        Some(f) => format!("{} — {:.0} %", job.title, f * 100.0),
        None => job.title.clone(),
    };
    painter.text(center + egui::vec2(0.0, 12.0), Align2::CENTER_CENTER, text, FontId::proportional(14.0), Color32::WHITE);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
}
