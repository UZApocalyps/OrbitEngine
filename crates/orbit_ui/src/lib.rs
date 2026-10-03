//! OrbitEngine in-game UI.
//!
//! Everything is data-driven through a serializable [`Theme`] whose defaults
//! are usable as is. Logic (layout, hit testing, focus, input handling,
//! typewriter advance) is free of any GPU access so it can be unit tested;
//! only the `draw` methods touch the renderer.
//!
//! Dialogue (M1):
//! * [`DialogueBox`]: translucent panel, colored speaker name, typewriter
//!   text, blinking "continue" marker;
//! * [`ChoiceMenu`]: stacked buttons with mouse hover, keyboard and digit
//!   selection;
//! * [`Banner`] and [`draw_progress_bar`]: building blocks of the loading,
//!   start, end and error screens.
//!
//! Menus (M2):
//! * [`Ui`] / [`Node`]: a small retained widget toolkit (panels, labels,
//!   buttons, sliders, toggles, selectors, scroll lists, images; vertical /
//!   horizontal / stack layout with anchors on the 1920x1080 canvas; spatial
//!   focus navigation; modal dialogs). It emits [`WidgetEvent`]s.
//! * Screens ([`MainMenu`], [`PauseMenu`], [`SaveLoadScreen`],
//!   [`PreferencesScreen`], [`HistoryScreen`], [`QuickMenu`], [`ToastStack`]):
//!   a view-model in ([`MainMenuView`]...), [`UiEvent`]s out. They know
//!   nothing about saves, the VM or audio.
//! * Input is abstract ([`UiInput`]: directions, confirm/cancel, pointer,
//!   wheel); the host maps keyboard, mouse and gamepad to it.
//! * Texts come from a key table ([`Texts`], [`TextProvider`]) with built-in
//!   English and French.
//!
//! Known limit: no clipping, so scroll lists show whole items only and
//! scroll item by item.

pub mod a11y;
pub mod asset;
mod banner;
mod choice;
mod dialogue;
mod input;
mod screens;
mod texts;
mod theme;
mod view;
mod widgets;

pub use banner::{Banner, draw_progress_bar};
pub use choice::{ChoiceMenu, MenuInput, button_rects, hit_test};
pub use dialogue::{Advance, DialogueBox};
pub use input::{UiInput, UiSound};
pub use screens::{
    HistoryScreen, MainMenu, PauseMenu, PreferencesScreen, QuickMenu, SaveLoadScreen, Screen,
    ToastStack, page_count, page_range,
};
pub use texts::{EN, FR, TextProvider, TextTable, Texts};
pub use theme::{
    Border, ButtonTheme, ChoiceTheme, ControlTheme, ListTheme, PanelTheme, ScreenTheme, TextBoxTheme,
    Fill, FontRef, StateColors, Style, StyleSounds, Theme, UiSounds, WidgetTheme, resolve_style, tint,
};
pub use view::*;
pub use widgets::{
    Align, Anchor, ButtonData, ButtonKind, CANVAS, Dir, DrawCtx, Kind, LabelData, NO_TAG, Node,
    NodeId, PanelKind, Size, Tag, Ui, WidgetEvent, ellipsize, escape_markup, estimate_lines,
    estimate_text_width,
};
pub use orbit_ui_data::{AA_COMPONENT, AA_TEXT, ContrastIssue, TEXT_SCALE_RANGE, clamp_text_scale, contrast_ratio};
pub use orbit_ui_data::{describe_chars, language_missing_chars};
pub use asset::{
    BindPath, Cond, DataContext, DataScreen, Issue, LayoutDef, ScreenAsset, Severity, SlotsMode, NavStep, PreviewNav,
    SettingKey, SplashContent, SplashDef, TextRef, UI_VERSION, UiAction, UiBundle, WidgetDef,
    WidgetKind, validate, validate_with_vars,
};
