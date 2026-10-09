//! GPU-free data model of the OrbitEngine UI (ADR 0011, addendum UI-01).
//!
//! Screen documents ([`ScreenAsset`], [`WidgetDef`]), actions and bindings,
//! the serializable [`Theme`] and styles, the project [`UiBundle`],
//! validation and `ui_version` migration. Nothing here touches the GPU or a
//! font, so export, pak and MCP tooling can use it without the render
//! crates. `orbit_ui` re-exports everything under its historical paths.

mod access;
mod action;
mod bundle;
mod coverage;
mod layout;
pub mod migrate;
mod slots;
mod sound;
mod theme;
mod transition;
pub mod validate;
mod widget;

pub use access::{
    AA_COMPONENT, AA_TEXT, ContrastIssue, TEXT_SCALE_RANGE, clamp_text_scale, contrast_ratio, luminance,
};
pub use action::{AchievementField, BindPath, SettingKey, SlotField, UiAction};
pub use bundle::{SYSTEM_SCREENS, SplashContent, SplashDef, UiBundle};
pub use layout::{Align, Anchor, Dir, PanelKind, Size};
pub use coverage::{describe_chars, language_missing_chars, missing_chars};
pub use slots::default_slot_card;
pub use sound::UiSound;
pub use transition::{ScreenFx, ScreenTransition, ScreenTransitionKind};
pub use theme::{
    BubbleCharacter, BubbleKind, BubbleTheme, DialogueMode, NarratorStyle, OffscreenStyle,
    Border, ButtonTheme, CANVAS, ChoiceTheme, ControlTheme, DialogueLayout, Fill, FontRef, ImageFit, ListTheme,
    NamePlate, PanelTheme, QuickMenuTheme, STYLE_FIELDS, ScreenTheme, Shadow, StateColors, StateFills, Style,
    StyleSounds, TextAlignment, TextBoxTheme, Theme, UiSounds, WidgetState, WidgetTheme, anchored, resolve_style,
};
pub use validate::{Issue, Severity, scene_issues, validate, validate_with_vars, variable_issues};
pub use widget::{
    Cond, LayoutDef, MAX_REPEAT_ITEMS, OverGame, Overflow, ScreenAsset, ScreenMusic, SlotsLook, SlotsMode, TextRef, UI_VERSION,
    WidgetDef, WidgetKind, from_ron, to_ron,
};
