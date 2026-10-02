//! Web storage: `window.localStorage`, values base64-encoded.
//!
//! Why `localStorage` and not IndexedDB: it is synchronous (matching the
//! [`Storage`] trait), needs no JS glue, and our payloads are small. Cost: the
//! browser grants about 5 MB per origin and strings are UTF-16, and base64 adds
//! 33 %. A save is typically 5-30 KB without thumbnail; a 320x180 PNG
//! thumbnail adds 30-60 KB, so ~20 slots fit comfortably. When the quota is
//! hit, [`Storage::write`] fails with [`StorageError::QuotaExceeded`] and the
//! previous value is untouched (`setItem` is atomic); the caller can ask the
//! player to delete a slot. If the budget ever becomes a problem, switch the
//! thumbnails (the bulk of the data) to an IndexedDB-backed implementation.
//!
//! Keys are stored as `orbit/<game_id>/<key>`, so several games on the same
//! origin do not collide.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

use crate::storage::{Storage, StorageError, validate_key};

pub struct LocalStorage {
    prefix: String,
    storage: web_sys::Storage,
}

impl LocalStorage {
    /// Opens `localStorage` for `game_id`. Fails with
    /// [`StorageError::Unavailable`] when the browser blocks it (e.g. some
    /// private modes, disabled site data).
    pub fn new(game_id: &str) -> Result<Self, StorageError> {
        let unavailable = |what: &str| StorageError::Unavailable(what.to_owned());
        let storage = web_sys::window()
            .ok_or_else(|| unavailable("no window"))?
            .local_storage()
            .map_err(|_| unavailable("localStorage access denied"))?
            .ok_or_else(|| unavailable("localStorage is not available"))?;
        let id: String = game_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Ok(Self {
            prefix: format!("orbit/{id}/"),
            storage,
        })
    }

    fn full_key(&self, key: &str) -> Result<String, StorageError> {
        validate_key(key)?;
        Ok(format!("{}{key}", self.prefix))
    }
}

fn js_error(e: JsValue) -> StorageError {
    if let Some(ex) = e.dyn_ref::<web_sys::DomException>() {
        // QuotaExceededError (code 22), or NS_ERROR_DOM_QUOTA_REACHED (Firefox).
        if ex.code() == 22 || ex.name().contains("Quota") {
            return StorageError::QuotaExceeded;
        }
        return StorageError::Io(format!("{}: {}", ex.name(), ex.message()));
    }
    StorageError::Io(format!("{e:?}"))
}

impl Storage for LocalStorage {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let text = self
            .storage
            .get_item(&self.full_key(key)?)
            .map_err(js_error)?;
        match text {
            None => Ok(None),
            Some(t) => STANDARD
                .decode(t.as_bytes())
                .map(Some)
                .map_err(|e| StorageError::Io(format!("corrupt base64 for `{key}`: {e}"))),
        }
    }

    fn write(&self, key: &str, data: &[u8]) -> Result<(), StorageError> {
        self.storage
            .set_item(&self.full_key(key)?, &STANDARD.encode(data))
            .map_err(js_error)
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.storage
            .remove_item(&self.full_key(key)?)
            .map_err(js_error)
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        let len = self.storage.length().map_err(js_error)?;
        let mut keys = Vec::new();
        for i in 0..len {
            if let Some(full) = self.storage.key(i).map_err(js_error)?
                && let Some(key) = full.strip_prefix(&self.prefix)
                && key.starts_with(prefix)
            {
                keys.push(key.to_owned());
            }
        }
        keys.sort();
        Ok(keys)
    }
}
