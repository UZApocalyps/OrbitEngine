//! Data-driven screens: a screen is a RON document ([`ScreenAsset`]) holding
//! a tree of [`WidgetDef`]s instead of Rust code.
//!
//! The document model (widgets, actions, bindings, migration, validation,
//! [`UiBundle`]) lives in the GPU-free `orbit_ui_data` crate and is
//! re-exported here. What stays in this module needs widgets or the renderer:
//!
//! * `screen`: [`DataScreen`], the [`crate::Screen`] that builds a widget
//!   [`crate::Ui`] from a document and maps actions to [`crate::UiEvent`]s;
//! * `preview`: simulated navigation for the editor preview.
//!
//! Nothing here touches the GPU: documents are parsed, validated and
//! turned into widgets in plain unit tests.

mod preview;
mod screen;
mod slots;

pub use orbit_ui_data::{
    AchievementField, BindPath, Cond, Issue, LayoutDef, OverGame, SYSTEM_SCREENS, ScreenAsset, ScreenMusic,
    SettingKey, Severity, SlotField, SlotsLook, SlotsMode, SplashContent, SplashDef, TextRef, UI_VERSION, UiAction,
    UiBundle, WidgetDef, WidgetKind, default_slot_card, from_ron, migrate, scene_issues, to_ron, validate,
    validate_with_vars, variable_issues,
};
pub use preview::{NavStep, PreviewNav, sample_saves};
pub use screen::{AchievementRow, DataContext, DataScreen};
pub use slots::{SlotsPage, slots_page};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stock_screen_builds_into_widgets() {
        let b = UiBundle::builtin();
        let ctx = DataContext { show_quit: true, ..Default::default() };
        for s in &b.screens {
            let mut screen = DataScreen::new(
                s.clone(),
                b.theme.as_ref().unwrap(),
                &crate::Texts::builtin("en"),
                ctx.clone(),
            );
            assert!(screen.ui().focused().is_some(), "{} has a focusable widget", s.id);
        }
    }
}
