//! Data-driven screens: a screen is a RON document ([`ScreenAsset`]) holding
//! a tree of [`WidgetDef`]s instead of Rust code.
//!
//! * [`widget`]: the document model;
//! * [`action`]: what a button does ([`UiAction`]) and what a control edits
//!   ([`BindPath`]);
//! * [`migrate`] / [`validate`]: versioning and checking of a document;
//! * [`screen`]: [`DataScreen`], the [`crate::Screen`] that builds a widget
//!   [`crate::Ui`] from a document and maps actions to [`crate::UiEvent`]s;
//! * [`bundle`]: [`UiBundle`], the theme + screens + splashes of a project.
//!
//! Nothing here touches the GPU: documents are parsed, validated and
//! turned into widgets in plain unit tests.

mod action;
mod bundle;
pub mod migrate;
mod preview;
mod screen;
mod slots;
pub mod validate;
mod widget;

pub use action::{BindPath, SettingKey, UiAction};
pub use bundle::{SYSTEM_SCREENS, SplashContent, SplashDef, UiBundle};
pub use screen::{DataContext, DataScreen};
pub use preview::{NavStep, PreviewNav, sample_saves};
pub use slots::{SlotsPage, slots_page};
pub use validate::{Issue, Severity, validate};
pub use widget::{
    Cond, LayoutDef, ScreenAsset, SlotsMode, TextRef, UI_VERSION, WidgetDef, WidgetKind, from_ron, to_ron,
};
