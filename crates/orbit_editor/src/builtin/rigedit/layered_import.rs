//! "Import layered file…": a PSD, Krita (.kra) or OpenRaster (.ora) file
//! becomes a new rig, or is added to the open one (one undo step). The
//! parsing and the rig building live in `orbit_layers` (shared with the MCP
//! tool `rig_import_layers`); this module only asks for the file, writes the
//! trimmed layer PNGs into the project and reports what happened.

use std::path::PathBuf;

use orbit_editor_api::EditorContext;
use orbit_layers::{ImportOptions, LayeredDoc};

use super::model::RigDoc;
use super::{State, Tool};

/// File name safe in a project asset path.
fn file_part(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).collect();
    let s = s.trim_matches('_');
    if s.is_empty() { "layer".to_owned() } else { s.to_owned() }
}

/// Writes the layers that will be imported (trimmed, in import order) as PNG
/// files under `dir` and declares them as project images in one go. Returns
/// the asset paths, one per kept layer.
fn write_layers(
    ctx: &mut EditorContext<'_>,
    doc: &LayeredDoc,
    opts: &ImportOptions,
    dir: &std::path::Path,
) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut files: Vec<PathBuf> = Vec::new();
    for l in &doc.layers {
        if !opts.include_hidden && !doc.is_visible(l) {
            continue;
        }
        let mut l = l.clone();
        if !orbit_layers::trim(&mut l) {
            continue;
        }
        let abs = dir.join(format!("{:02}_{}.png", files.len() + 1, file_part(&l.name)));
        std::fs::write(&abs, orbit_layers::encode_png(l.width, l.height, &l.rgba)?).map_err(|e| e.to_string())?;
        files.push(abs);
    }
    let report = crate::builtin::import_paths(ctx, &files);
    if report.imported.len() != files.len() {
        return Err(report.failed.first().map_or_else(|| "import failed".to_owned(), |(_, e)| format!("{e:?}")));
    }
    Ok(report.imported.into_iter().map(|i| i.path).collect())
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

    pub fn import_layers(&mut self, ctx: &mut EditorContext<'_>, path: &std::path::Path, new_rig: bool) {
        let shown = path.display().to_string();
        let doc = match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| orbit_layers::parse(&b)) {
            Ok(d) => d,
            Err(e) => {
                ctx.error(ctx.tf("rig.layers.err.read", &[("path", &shown), ("error", &e)]));
                return;
            }
        };
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
        let assets = match write_layers(ctx, &doc, &opts, &dir) {
            Ok(a) => a,
            Err(e) => {
                ctx.error(ctx.tf("rig.layers.err.write", &[("error", &e)]));
                return;
            }
        };
        let mut next = assets.into_iter();
        let mut save = |_: &str, _: u32, _: u32, _: &[u8]| next.next().ok_or_else(|| "missing layer picture".to_owned());
        let mut rig = match (&self.doc, new_rig) {
            (Some(d), false) => d.rig().clone(),
            _ => orbit_anim::rig::Rig::default(),
        };
        let report = match orbit_layers::import_into_rig(&mut rig, &doc, &opts, &mut save) {
            Ok(r) => r,
            Err(e) => {
                ctx.error(ctx.tf("rig.layers.err.import", &[("path", &shown), ("error", &e)]));
                return;
            }
        };
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
