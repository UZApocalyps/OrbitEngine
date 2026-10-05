//! Experimental feature switches. The 3D features are hidden from the editor
//! (menus, palettes, panels, MCP tool list, manual) unless this is on; nothing
//! is removed, and projects that already use 3D keep loading and playing.
//!
//! On when the environment variable `ORBIT_EXPERIMENTAL_3D` is set to a value
//! other than `0` / empty / `false`, or when the editor preference is on.

use std::sync::atomic::{AtomicBool, Ordering};

/// Editor preference (set by the editor from its config and its menu).
static PREFERENCE: AtomicBool = AtomicBool::new(false);

/// Name of the environment override.
pub const ENV_3D: &str = "ORBIT_EXPERIMENTAL_3D";

fn env_on() -> bool {
    std::env::var(ENV_3D).is_ok_and(|v| {
        let v = v.trim();
        !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false"))
    })
}

/// Are the experimental 3D features visible?
pub fn enabled_3d() -> bool {
    env_on() || PREFERENCE.load(Ordering::Relaxed)
}

/// Sets the preference part of the switch.
pub fn set_preference_3d(on: bool) {
    PREFERENCE.store(on, Ordering::Relaxed);
}

/// The preference alone (what the settings checkbox shows).
pub fn preference_3d() -> bool {
    PREFERENCE.load(Ordering::Relaxed)
}
