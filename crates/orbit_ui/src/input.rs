//! Abstract input of the UI. The host maps keyboard, mouse and gamepad to
//! these (the UI knows no concrete device).

/// One logical input. Pointer positions are in virtual canvas pixels
/// (1920x1080, letterbox already removed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiInput {
    Up,
    Down,
    Left,
    Right,
    /// Activate the focused widget (Enter / Space / gamepad A).
    Confirm,
    /// Close / go back (Escape / right click / gamepad B).
    Cancel,
    /// Next / previous widget in reading order (Tab / Shift+Tab).
    Next,
    Prev,
    /// Page Up / Page Down: scroll a list by a page, switch page of a grid.
    PageUp,
    PageDown,
    PointerMove([f32; 2]),
    /// Primary button pressed at a position.
    PointerDown([f32; 2]),
    PointerUp([f32; 2]),
    /// Mouse wheel; positive scrolls up (toward the first item).
    Scroll(f32),
}

pub use orbit_ui_data::UiSound;
