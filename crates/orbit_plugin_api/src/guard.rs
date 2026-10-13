//! Panic isolation of a plugin loaded from a native library (editor plugins,
//! see `orbit_editor_api::native`).
//!
//! A native plugin carries its own copy of the Rust standard library. A
//! panic raised by that copy cannot be caught by the host's `catch_unwind`
//! (the unwinder sees a *foreign* exception and aborts the process), so the
//! catching must happen in code compiled **into the plugin**. The wrappers
//! made by [`PluginGuard`] are such code as long as they are created by the
//! plugin's copy of this crate: the plugin calls the wrapping functions
//! itself (`ProjectRegistry::guard_since`, called by the editor-side
//! `PluginHost::seal`), so the closures and their `catch_unwind` live in the
//! plugin's binary.
//!
//! Once a guarded call panicked, the guard is *failed*: every later call
//! returns its fallback without entering the plugin, and the host reports
//! the failure (the plugin is disabled with an error notice).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct GuardState {
    plugin: String,
    failed: AtomicBool,
    message: Mutex<Option<String>>,
}

/// Shared failure flag of one plugin (cheap to clone).
#[derive(Clone)]
pub struct PluginGuard(Arc<GuardState>);

impl std::fmt::Debug for PluginGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginGuard")
            .field("plugin", &self.0.plugin)
            .field("failed", &self.is_failed())
            .finish()
    }
}

/// The text of a panic payload.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".to_owned()
    }
}

impl PluginGuard {
    pub fn new(plugin: impl Into<String>) -> Self {
        Self(Arc::new(GuardState {
            plugin: plugin.into(),
            failed: AtomicBool::new(false),
            message: Mutex::new(None),
        }))
    }

    /// Id of the guarded plugin.
    pub fn plugin(&self) -> &str {
        &self.0.plugin
    }

    /// Whether a guarded call panicked (the plugin is not entered any more).
    pub fn is_failed(&self) -> bool {
        self.0.failed.load(Ordering::Acquire)
    }

    /// The message of the first panic, if any.
    pub fn failure(&self) -> Option<String> {
        if !self.is_failed() {
            return None;
        }
        self.0.message.lock().ok().and_then(|m| m.clone())
    }

    /// Marks the plugin failed (first message kept).
    pub fn fail(&self, message: impl Into<String>) {
        if let Ok(mut m) = self.0.message.lock()
            && m.is_none()
        {
            *m = Some(message.into());
        }
        self.0.failed.store(true, Ordering::Release);
    }

    /// Runs `f`, or `fallback` when the plugin already failed or `f` panics.
    pub fn call<R>(&self, fallback: impl FnOnce() -> R, f: impl FnOnce() -> R) -> R {
        if self.is_failed() {
            return fallback();
        }
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(r) => r,
            Err(payload) => {
                self.fail(panic_message(payload.as_ref()));
                fallback()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_fails_the_guard_and_later_calls_use_the_fallback() {
        let guard = PluginGuard::new("acme.test");
        assert_eq!(guard.call(|| 0, || 1), 1);
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let r = guard.call(|| -1, || -> i32 { panic!("boom") });
        std::panic::set_hook(prev);
        assert_eq!(r, -1);
        assert!(guard.is_failed());
        assert_eq!(guard.failure().as_deref(), Some("boom"));
        // Not entered any more.
        assert_eq!(guard.call(|| 7, || unreachable!()), 7);
    }
}
