//! Ren'Py import as a native editor plugin: File > Import from Ren'Py picks a
//! Ren'Py game folder and a parent folder, converts the game into a new
//! project in the background (`orbit_renpy`), opens it and shows the import
//! report. The editor ships without it; build it with `cargo xtask plugins`
//! and install `target/plugins/orbit.renpy_import-*.orbitplugin` from Tools >
//! Extensions > Plugins. The command line keeps its own `orbit import-renpy`.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use orbit_editor_sdk::egui;
use orbit_editor_sdk::native::{NativePlugin, PluginHost};
use orbit_editor_sdk::{
    ActionDef, EditorContext, EditorRequest, Level, TaskError, TaskHandle, WindowDef, icons,
};

#[derive(Default)]
struct State {
    /// The import running in the background.
    importing: RefCell<Option<TaskHandle<orbit_renpy::ImportReport>>>,
    /// Markdown of the last report, while its window is open.
    report: RefCell<Option<String>>,
    report_open: Cell<bool>,
}

/// A folder name safe on every platform (same rule as the editor's new
/// project folders).
fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '-' } else { c })
        .collect();
    cleaned.trim_matches(|c| c == '.' || c == ' ').to_owned()
}

/// Where the project of the game in `source` goes: a folder of `parent`
/// named after the game, numbered when taken.
fn destination(source: &Path, parent: &Path) -> PathBuf {
    let game_name = source
        .file_name()
        .filter(|n| *n != "game")
        .or_else(|| source.parent().and_then(Path::file_name))
        .map_or_else(|| "renpy".to_owned(), |n| n.to_string_lossy().into_owned());
    let base = folder_name(&game_name);
    let base = if base.is_empty() { "renpy".to_owned() } else { base };
    let free = |p: &Path| !p.exists() || p.read_dir().is_ok_and(|mut d| d.next().is_none());
    let mut dest = parent.join(&base);
    for n in 2..100 {
        if free(&dest) {
            break;
        }
        dest = parent.join(format!("{base}-{n}"));
    }
    dest
}

/// The action: two folder pickers, then the background import.
fn start(ctx: &mut EditorContext<'_>, s: &State) {
    if s.importing.borrow().is_some() {
        return;
    }
    let source = ctx.pick_folder(&ctx.t("renpy.pick_source"));
    let parent = source.as_ref().and_then(|_| ctx.pick_folder(&ctx.t("renpy.pick_parent")));
    let (Some(source), Some(parent)) = (source, parent) else {
        return;
    };
    let dest = destination(&source, &parent);
    let title = ctx.t("renpy.task");
    let label = title.clone();
    let job = ctx.state().spawn_task(title, None, move |p| {
        // The plugin has its own copy of std: a panic must not unwind into
        // the editor's worker.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            orbit_renpy::import(&source, &dest, &|fraction, _| p.set(fraction, label.clone()))
        }))
        .unwrap_or_else(|_| Err("the Ren'Py import crashed".to_owned()))
        .map_err(TaskError::Failed)
    });
    *s.importing.borrow_mut() = Some(job);
}

/// Opens the project of a finished import and shows its report. Drawn every
/// frame: also where the running import is polled.
fn window(ctx: &egui::Context, ectx: &mut EditorContext<'_>, s: &State) {
    let outcome = s.importing.borrow_mut().as_mut().and_then(TaskHandle::poll);
    if let Some(outcome) = outcome {
        *s.importing.borrow_mut() = None;
        match outcome {
            Ok(report) => {
                let summary = report.summary();
                let level = if report.validation_errors.is_empty() { Level::Info } else { Level::Warn };
                let message = ectx.tf("renpy.done", &[("summary", &summary)]);
                ectx.state_mut().push_toast(level, message);
                ectx.request(EditorRequest::OpenProject(report.project.clone()));
                *s.report.borrow_mut() = Some(report.to_markdown());
                s.report_open.set(true);
            }
            Err(TaskError::Cancelled) => {}
            Err(TaskError::Failed(e)) => ectx.error(e),
            Err(e) => ectx.error(format!("{e:?}")),
        }
    }

    let mut open = s.report_open.get();
    if !open {
        return;
    }
    egui::Window::new(ectx.t("renpy.report_title"))
        .id(egui::Id::new("orbit.renpy_import.report"))
        .open(&mut open)
        .default_width(620.0)
        .show(ctx, |ui| {
            ui.small(ectx.t("renpy.report_note"));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                if let Some(text) = s.report.borrow().as_ref() {
                    ui.add(egui::Label::new(egui::RichText::new(text.as_str()).monospace().size(12.0)).wrap());
                }
            });
        });
    s.report_open.set(open);
    if !open {
        *s.report.borrow_mut() = None;
    }
}

#[derive(Default)]
pub struct RenpyImport {
    state: Rc<State>,
}

impl NativePlugin for RenpyImport {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        let st = self.state.clone();
        host.window(WindowDef::new("report", move |ctx, ectx| window(ctx, ectx, &st)));
        let st = self.state.clone();
        host.action(
            ActionDef::new("import", "renpy.action", move |ctx| start(ctx, &st))
                .icon(icons::DOWNLOAD_SIMPLE)
                .category("category.file")
                .menu("file", 105)
                .menu_group("menu.exchange"),
        );
        Ok(())
    }
}

orbit_editor_sdk::export_plugin!(
    RenpyImport,
    id = "orbit.renpy_import",
    name = "Ren'Py import",
    description = "File > Import from Ren'Py: converts a Ren'Py game into a new project and shows the import report.",
    author = "OrbitEngine contributors",
);
