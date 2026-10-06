//! "Import layered file…": a PSD, Krita (.kra) or OpenRaster (.ora) file
//! becomes a new rig, or is added to the open one (one undo step). The
//! parsing and the rig building live in `orbit_layers` (shared with the MCP
//! tool `rig_import_layers`); this module only asks for the file, writes the
//! trimmed layer PNGs into the project and reports what happened.

use std::path::PathBuf;

use orbit_anim::rig::Rig;
use orbit_editor_api::{EditorContext, Progress, TaskError};
use orbit_layers::{ImportOptions, ImportReport, LayeredDoc};

use super::jobs;
use super::model::RigDoc;
use super::{State, Tool};

/// File name safe in a project asset path.
fn file_part(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).collect();
    let s = s.trim_matches('_');
    if s.is_empty() { "layer".to_owned() } else { s.to_owned() }
}

/// Writes the layers that will be imported (trimmed, in import order) as PNG
/// files under `dir` (in the background: `written` follows, for the clean up
/// of a dropped result). Returns the files, one per kept layer.
fn write_layers(
    doc: &LayeredDoc,
    opts: &ImportOptions,
    dir: &std::path::Path,
    p: &Progress,
    written: &jobs::Files,
) -> Result<Vec<PathBuf>, TaskError> {
    let fail = |e: String| TaskError::Failed(e);
    std::fs::create_dir_all(dir).map_err(|e| fail(e.to_string()))?;
    let mut files: Vec<PathBuf> = Vec::new();
    let n = doc.layers.len().max(1) as f32;
    for (i, l) in doc.layers.iter().enumerate() {
        p.check()?;
        p.set_fraction(0.8 * i as f32 / n);
        if !opts.include_hidden && !doc.is_visible(l) {
            continue;
        }
        let mut l = l.clone();
        if !orbit_layers::trim(&mut l) {
            continue;
        }
        let abs = dir.join(format!("{:02}_{}.png", files.len() + 1, file_part(&l.name)));
        let png = orbit_layers::encode_png(l.width, l.height, &l.rgba).map_err(fail)?;
        if let Ok(mut w) = written.lock() {
            w.push(abs.clone());
        }
        std::fs::write(&abs, png).map_err(|e| fail(e.to_string()))?;
        files.push(abs);
    }
    Ok(files)
}

/// A layered import computed by a job: the rig built on placeholder picture
/// paths (the files written), to point at the project images once declared.
pub struct LayersOut {
    rig: Rig,
    report: ImportReport,
    files: Vec<PathBuf>,
    new_rig: bool,
}

/// Placeholder picture path of a written layer file, until it is declared.
fn placeholder(file: &std::path::Path) -> String {
    format!("\u{1}{}", file.display())
}

impl State {
    /// "Import layered file…": `new_rig` makes a new rig (unsaved changes of
    /// the open one must be saved first), else the layers are added to the
    /// open rig.
    pub fn request_import_layers(&mut self, ctx: &mut EditorContext<'_>, new_rig: bool) {
        if !ctx.has_project() {
            ctx.error(ctx.t("rig.layers.no_project"));
            return;
        }
        if !new_rig && self.doc.is_none() {
            ctx.warn(ctx.t("rig.layers.need_rig"));
            return;
        }
        if new_rig && self.doc.as_ref().is_some_and(RigDoc::dirty) {
            ctx.warn(ctx.t("rig.layers.unsaved"));
            return;
        }
        let mut dlg = rfd::FileDialog::new().add_filter(ctx.t("rig.layers.filter"), &["psd", "psb", "kra", "ora"]);
        if let Some(root) = ctx.project_info().map(|p| p.root.clone()) {
            dlg = dlg.set_directory(root);
        }
        if let Some(path) = dlg.pick_file() {
            self.import_layers(ctx, &path, new_rig);
        }
    }

    /// Reads and parses the file in the background, then
    /// [`State::import_parsed_layers`].
    pub fn import_layers(&mut self, ctx: &mut EditorContext<'_>, path: &std::path::Path, new_rig: bool) {
        if jobs::busy(ctx, self) {
            return;
        }
        let path = path.to_owned();
        jobs::start(ctx, self, "rig.task.layers", jobs::files(Vec::new()), move |p| {
            let fail = |e: String| TaskError::Failed(format!("{}: {e}", path.display()));
            let bytes = std::fs::read(&path).map_err(|e| fail(e.to_string()))?;
            p.check()?;
            let doc = orbit_layers::parse(&bytes).map_err(fail)?;
            Ok(jobs::Out::LayersParsed { doc: Box::new(doc), path, new_rig })
        });
    }

    /// The file is parsed: asks how to import the groups, then writes the
    /// layer pictures and builds the rig in the background.
    pub(super) fn import_parsed_layers(
        &mut self,
        ctx: &mut EditorContext<'_>,
        doc: LayeredDoc,
        path: &std::path::Path,
        new_rig: bool,
    ) {
        let shown = path.display().to_string();
        let mut opts = ImportOptions::default();
        if !doc.groups.is_empty() {
            let answer = rfd::MessageDialog::new()
                .set_title(ctx.t("rig.layers.groups_title"))
                .set_description(ctx.t("rig.layers.groups_ask"))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            opts.groups_as_bones = answer == rfd::MessageDialogResult::Yes;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).map(file_part).unwrap_or_else(|| "layers".to_owned());
        let dir = self.project_root(ctx).join("images").join(&stem);
        let mut rig = match (&self.doc, new_rig) {
            (Some(d), false) => d.rig().clone(),
            _ => Rig::default(),
        };
        let written = jobs::files(Vec::new());
        let files_of_job = written.clone();
        jobs::start(ctx, self, "rig.task.layers", written, move |p| {
            let files = write_layers(&doc, &opts, &dir, p, &files_of_job)?;
            p.check()?;
            p.set_fraction(0.85);
            let mut next = files.iter();
            let mut save = |_: &str, _: u32, _: u32, _: &[u8]| {
                next.next().map(|f| placeholder(f)).ok_or_else(|| "missing layer picture".to_owned())
            };
            let report = orbit_layers::import_into_rig(&mut rig, &doc, &opts, &mut save)
                .map_err(|e| TaskError::Failed(format!("{shown}: {e}")))?;
            Ok(jobs::Out::Layers(Box::new(LayersOut { rig, report, files, new_rig })))
        });
    }

    /// Declares the written pictures as project images and applies the rig
    /// (a new document, or one undo step on the open one).
    pub(super) fn finish_import_layers(&mut self, ctx: &mut EditorContext<'_>, out: LayersOut) {
        let LayersOut { mut rig, report, files, new_rig } = out;
        let imported = crate::builtin::import_paths(ctx, &files);
        if imported.imported.len() != files.len() {
            let e = imported.failed.first().map_or_else(|| "import failed".to_owned(), |(_, e)| format!("{e:?}"));
            ctx.error(ctx.tf("rig.layers.err.write", &[("error", &e)]));
            return;
        }
        for (file, asset) in files.iter().zip(imported.imported) {
            super::pixeltools::repoint(&mut rig, &placeholder(file), &asset.path);
        }
        let root = rig.bones.iter().find(|b| b.parent.is_none()).map(|b| b.id.clone());
        let last_slot = rig.slots.last().map(|s| s.id.clone());
        if new_rig || self.doc.is_none() {
            self.set_doc(RigDoc::from_rig(rig, None));
            self.selected = root;
            self.tool = Tool::Create;
        } else if let Some(d) = self.doc.as_mut() {
            d.replace(rig);
        }
        self.selected_slot = last_slot;
        self.sync();
        ctx.info(ctx.tf(
            "rig.layers.done",
            &[("slots", &report.slots.len().to_string()), ("hidden", &report.hidden.len().to_string())],
        ));
        for w in report.warnings.iter().take(5) {
            ctx.warn(w.clone());
        }
        if report.warnings.len() > 5 {
            ctx.warn(ctx.tf("rig.layers.more_warnings", &[("n", &(report.warnings.len() - 5).to_string())]));
        }
    }
}
