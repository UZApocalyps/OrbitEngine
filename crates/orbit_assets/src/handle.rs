use std::sync::{Arc, Mutex};

use crate::AssetError;

enum State<T> {
    Loading,
    Ready(Arc<T>),
    Failed(AssetError),
}

/// Result of polling an [`AssetHandle`].
#[derive(Debug)]
pub enum AssetPoll<T> {
    Loading,
    Ready(Arc<T>),
    Failed(AssetError),
}

impl<T> AssetPoll<T> {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }
}

/// Handle to an asset being loaded in the background. Cheap to clone.
pub struct AssetHandle<T> {
    shared: Arc<Mutex<State<T>>>,
}

impl<T> Clone for AssetHandle<T> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

/// Write side of a handle (used by the loader).
pub(crate) struct Slot<T> {
    shared: Arc<Mutex<State<T>>>,
}

impl<T> AssetHandle<T> {
    pub(crate) fn new_pair() -> (Self, Slot<T>) {
        let shared = Arc::new(Mutex::new(State::Loading));
        (
            Self {
                shared: shared.clone(),
            },
            Slot { shared },
        )
    }

    /// Non-blocking state check; call it every frame.
    pub fn poll(&self) -> AssetPoll<T> {
        match &*self.shared.lock().unwrap_or_else(|e| e.into_inner()) {
            State::Loading => AssetPoll::Loading,
            State::Ready(v) => AssetPoll::Ready(v.clone()),
            State::Failed(e) => AssetPoll::Failed(e.clone()),
        }
    }

    /// Blocks until loaded. Native tests/tools only: it would deadlock on the
    /// web, where loading needs the browser event loop.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_blocking(&self) -> AssetPoll<T> {
        loop {
            match self.poll() {
                AssetPoll::Loading => std::thread::sleep(std::time::Duration::from_millis(2)),
                other => return other,
            }
        }
    }
}

impl<T> Drop for Slot<T> {
    /// A loader that disappears without a result (thread could not start,
    /// panic in a decoder) must not leave the handle `Loading` forever.
    fn drop(&mut self) {
        let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*state, State::Loading) {
            *state = State::Failed(AssetError::Io {
                path: String::new(),
                message: "loader terminated without a result".to_owned(),
            });
        }
    }
}

impl<T> Slot<T> {
    /// Every handle was dropped: nobody wants the result any more (the
    /// loader skips the work).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn is_abandoned(&self) -> bool {
        Arc::strong_count(&self.shared) == 1
    }

    pub(crate) fn ready(&self, value: T) {
        *self.shared.lock().unwrap_or_else(|e| e.into_inner()) = State::Ready(Arc::new(value));
    }

    pub(crate) fn fail(&self, error: AssetError) {
        log::warn!("asset load failed: {error}");
        *self.shared.lock().unwrap_or_else(|e| e.into_inner()) = State::Failed(error);
    }
}
