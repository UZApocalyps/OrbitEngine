//! OrbitEngine plugin API, **editor side** (ADR 0009).
//!
//! An [`EditorPlugin`] extends the project-side `orbit_plugin_api::Plugin`
//! with what needs egui: panels, actions (palette, menus, shortcuts),
//! inspectors per node type. Every built-in tool of the editor is such a
//! plugin (dogfooding), and an experimental plugin can be added the same way.
//!
//! What a plugin sees of the editor is the [`EditorContext`]: it reads the
//! project, **emits commands only** (never a mutable story) and manipulates
//! the editor state (selection, toasts, requests to the host).

pub mod ai;
mod context;
pub mod curve_editor;
pub mod dialogs;
pub mod errors;
pub mod fields;
mod graph_state;
pub mod i18n;
pub mod lazy;
pub mod mcp;

mod preview;
mod registry;
mod scene_view;
pub mod tasks;
pub mod varwidgets;

pub use context::{
    ConsoleEntry, ConsoleLog, Diagnostics, EditorContext, EditorError, EditorRequest, EditorState,
    Level, Location, Problem, ProjectInfo, RecentProject, Selection, ThemeMode, Toast,
    ValidationStatus, Waker,
};
pub use errors::{
    describe_command_error, describe_editor_error, describe_project_error, describe_rejection,
    describe_undo_label,
};
pub use graph_state::GraphState;
pub use lazy::{DocKey, Freshness, Lazy, LazyAsync, PanelVisibility};
pub use i18n::{Lang, Translator, UiText};
pub use orbit_plugin_api::{API_VERSION, Plugin, PluginError, PluginManifest, ProjectRegistry};
pub use preview::{
    PreviewCommand, PreviewInput, PreviewSpeed, PreviewState, PreviewStatus, UiPreviewRequest,
};
pub use registry::{
    ActionDef, DockSlot, EditorRegistry, InspectorDef, MenuDef, MenuSlot, PanelDef, RegisteredAction,
    RegisteredPanel, RegisteredWindow, RegistryMark, SettingsDef, UiEditorLink, WindowDef,
    shortcut_text,
};
pub use orbit_plugin_api::PluginGuard;
pub mod native;

/// The exact egui the editor is built with. A native plugin draws with this
/// one (`orbit_editor_api::egui`), never with its own egui dependency.
pub use egui;
pub use scene_view::{SceneViewActor, SceneViewState};
pub use tasks::{Progress, TaskError, TaskHandle, TaskInfo, Tasks};

/// Glyph constants of the Phosphor icon font (`icons::FLOPPY_DISK`...).
pub use egui_phosphor::regular as icons;

/// The editor-side part of a plugin. It is also a [`Plugin`], so one value
/// carries both halves.
pub trait EditorPlugin: Plugin {
    /// Registers panels, actions, menus and inspectors.
    fn register_ui(&self, registry: &mut EditorRegistry);
}
