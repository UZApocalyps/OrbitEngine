//! "Hello": the template of a native editor plugin (plugin SDK).
//!
//! * a floating window (Tools > Hello window) drawn with the editor's egui;
//! * a Tools menu action "Add a note" that appends a narrator line to the
//!   current scene as **one undo entry** (`ctx.group`), and a button that does
//!   the same through the MCP tool `add_dialogue` (`ctx.call_tool`);
//! * a settings page (plugin manager > Settings) saved in the plugin's own
//!   data folder;
//! * its texts in `locales/<lang>/hello.ron` (shipped in the package);
//! * a "crash test" action (palette only) that panics, to see the editor
//!   disable a crashing plugin instead of closing.
//!
//! Build: `cargo xtask plugins` (official plugins) or
//! `cargo build --release -p orbit_hello_plugin`, then install the `.dll` /
//! `.so` / `.dylib` (or a `.orbitplugin` package) from Tools > Extensions >
//! Plugins. A plugin must be rebuilt for every editor version.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use orbit_core::{Node, NodeId, NodeKind, StringId};
use orbit_editor_api::egui;
use orbit_editor_api::native::{NativePlugin, PluginHost};
use orbit_editor_api::{ActionDef, EditorContext, SettingsDef, WindowDef, icons};
use orbit_project::Command;

/// The plugin id (also in `manifest.ron`).
pub const ID: &str = "acme.hello";

#[derive(Default)]
struct State {
    window_open: Cell<bool>,
    /// Notes added this session.
    added: Cell<u32>,
    /// Text of the notes (settings page), saved in `data/note.txt`.
    note: RefCell<String>,
    settings_file: RefCell<Option<PathBuf>>,
    /// Pending MCP call and its last result.
    ticket: Cell<Option<u64>>,
    tool_result: RefCell<Option<String>>,
}

impl State {
    fn note_text(&self, ctx: &EditorContext<'_>) -> String {
        let note = self.note.borrow();
        let base = if note.trim().is_empty() { ctx.t("hello.default_note") } else { note.clone() };
        format!("{base} #{}", self.added.get() + 1)
    }

    fn save(&self) {
        if let Some(file) = self.settings_file.borrow().as_ref() {
            let _ = std::fs::write(file, self.note.borrow().as_bytes());
        }
    }
}

#[derive(Default)]
pub struct Hello {
    state: Rc<State>,
}

/// Appends a narrator line to the scene shown (or the first one, or a new
/// "Notes" scene) as one undo entry. Returns the new node.
pub fn add_note(ctx: &mut EditorContext<'_>, text: &str) -> Option<NodeId> {
    let Some(story) = ctx.story() else {
        let message = ctx.t("hello.no_project");
        ctx.warn(message);
        return None;
    };
    let scene = ctx
        .state()
        .current_scene
        .filter(|s| story.scene(*s).is_some())
        .or_else(|| story.scenes.first().map(|s| s.id));
    let label = ctx.t("hello.undo_label");
    let result = ctx.group(&label, |p| {
        let scene = match scene {
            Some(s) => s,
            None => orbit_project::ops::add_scene(p, "Notes")?,
        };
        let index = p.story().scene(scene).map_or(0, |s| s.nodes.len());
        let text_id = StringId::new();
        p.execute(Command::SetString { id: text_id, text: Some(text.to_owned()) })?;
        let id = NodeId::new();
        p.execute(Command::InsertNode {
            scene,
            index,
            node: Node { id, kind: NodeKind::Say { speaker: None, text_id, voice: None } },
        })?;
        Ok(id)
    });
    match result {
        Ok(id) => {
            let message = ctx.tf("hello.added", &[("text", &text)]);
            ctx.info(message);
            Some(id)
        }
        Err(e) => {
            let message = ctx.describe(&e);
            ctx.error(message);
            None
        }
    }
}

fn window(ctx: &egui::Context, ectx: &mut EditorContext<'_>, s: &State) {
    // Result of the MCP call started from the window.
    if let Some(ticket) = s.ticket.get()
        && let Some(result) = ectx.take_tool_result(ticket)
    {
        s.ticket.set(None);
        *s.tool_result.borrow_mut() = Some(match result {
            Ok(text) => text,
            Err(e) => format!("error: {e}"),
        });
    }
    let mut open = s.window_open.get();
    if !open {
        return;
    }
    egui::Window::new(ectx.t("hello.window_title"))
        .id(egui::Id::new("acme.hello.window"))
        .open(&mut open)
        .default_width(320.0)
        .show(ctx, |ui| {
            ui.label(ectx.t("hello.greeting"));
            ui.label(ectx.tn("hello.added_count", s.added.get() as usize, &[]));
            ui.add_space(6.0);
            let has_project = ectx.has_project();
            if ui
                .add_enabled(has_project, egui::Button::new(format!("{} {}", icons::NOTE_PENCIL, ectx.t("hello.add_note"))))
                .clicked()
            {
                let text = s.note_text(ectx);
                if add_note(ectx, &text).is_some() {
                    s.added.set(s.added.get() + 1);
                }
            }
            if ui
                .add_enabled(has_project && s.ticket.get().is_none(), egui::Button::new(ectx.t("hello.add_note_mcp")))
                .clicked()
            {
                let scene = ectx
                    .state()
                    .current_scene
                    .and_then(|id| ectx.story().and_then(|st| st.scene(id)).map(|sc| sc.name.clone()))
                    .or_else(|| ectx.story().and_then(|st| st.scenes.first()).map(|sc| sc.name.clone()))
                    .unwrap_or_default();
                let args = format!(
                    "{{\"scene\": {:?}, \"text\": {:?}}}",
                    scene,
                    s.note_text(ectx)
                );
                s.ticket.set(Some(ectx.call_tool("add_dialogue", args)));
                s.added.set(s.added.get() + 1);
            }
            if let Some(result) = s.tool_result.borrow().as_ref() {
                ui.small(result);
            }
        });
    s.window_open.set(open);
}

impl NativePlugin for Hello {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        let s = self.state.clone();
        let file = host.data_dir().join("note.txt");
        *s.note.borrow_mut() = std::fs::read_to_string(&file).unwrap_or_default();
        *s.settings_file.borrow_mut() = Some(file);

        let st = s.clone();
        host.window(WindowDef::new("window", move |ctx, ectx| window(ctx, ectx, &st)));

        let st = s.clone();
        host.action(
            ActionDef::new("open", "hello.open", move |_| st.window_open.set(!st.window_open.get()))
                .icon(icons::HAND_WAVING)
                .category("category.tools")
                .menu("tools", 900),
        );
        let st = s.clone();
        host.action(
            ActionDef::new("add_note", "hello.add_note", move |ctx| {
                let text = st.note_text(ctx);
                if add_note(ctx, &text).is_some() {
                    st.added.set(st.added.get() + 1);
                }
            })
            .icon(icons::NOTE_PENCIL)
            .category("category.tools")
            .menu("tools", 901)
            .enabled(|ctx| ctx.has_project()),
        );
        // Palette only: shows how a crashing plugin is disabled.
        host.action(
            ActionDef::new("crash_test", "hello.crash_test", |_| panic!("hello: crash test"))
                .category("category.tools"),
        );

        let st = s.clone();
        host.settings_page(SettingsDef::new("hello.settings_title", move |ui, ctx| {
            ui.label(ctx.t("hello.note_text"));
            let mut note = st.note.borrow().clone();
            if ui
                .add(egui::TextEdit::singleline(&mut note).hint_text(ctx.t("hello.default_note")))
                .changed()
            {
                *st.note.borrow_mut() = note;
                st.save();
            }
        }));
        Ok(())
    }
}

orbit_editor_api::export_plugin!(
    Hello,
    id = "acme.hello",
    name = "Hello",
    description = "Example plugin: a window and a Tools menu action that adds a note to the project.",
    author = "OrbitEngine contributors",
);
