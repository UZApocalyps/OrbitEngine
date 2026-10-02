//! GPU-free data model of the OrbitEngine UI (ADR 0011, addendum UI-01).
//!
//! Screen documents ([`ScreenAsset`], [`WidgetDef`]), actions and bindings,
//! the serializable [`Theme`] and styles, the project [`UiBundle`],
//! validation and `ui_version` migration. Nothing here touches the GPU or a
//! font, so export, pak and MCP tooling can use it without the render
//! crates. `orbit_ui` re-exports everything under its historical paths.

mod action;
mod bundle;
mod layout;
pub mod migrate;
mod sound;
mod theme;
pub mod validate;
mod widget;

pub use action::{BindPath, SettingKey, UiAction};
pub use bundle::{SYSTEM_SCREENS, SplashContent, SplashDef, UiBundle};
pub use layout::{Align, Anchor, Dir, PanelKind, Size};
pub use sound::UiSound;
pub use theme::{
    Border, ButtonTheme, ChoiceTheme, ControlTheme, Fill, ListTheme, PanelTheme, ScreenTheme,
    StateColors, Style, StyleSounds, TextBoxTheme, Theme, UiSounds, WidgetTheme,
    resolve_style,
};
pub use validate::{Issue, Severity, validate, validate_with_vars, variable_issues};
pub use widget::{
    Cond, LayoutDef, ScreenAsset, SlotsMode, TextRef, UI_VERSION, WidgetDef, WidgetKind, from_ron,
    to_ron,
};
