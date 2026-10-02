//! Key/value storage abstraction.
//!
//! The trait is **synchronous** on purpose: native files and the web
//! `localStorage` are both synchronous APIs, saves are small (tens of KB, a
//! PNG thumbnail at most) and happen at human speed (a click, a scene change).
//! An asynchronous backend (IndexedDB) can be wrapped later behind a
//! write-through in-memory cache without changing this trait or its callers.
//!
//! Keys are `/`-separated lowercase paths such as `saves/quick.sav`; see
//! [`validate_key`]. Writes are atomic: a failed write (quota, disk full,
//! crash) leaves the previous value intact.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StorageError {
    /// The storage is full (browser quota, disk full). Nothing was written.
    #[error("storage quota exceeded")]
    QuotaExceeded,
    /// The storage cannot be used at all (private browsing, no data dir...).
    #[error("storage unavailable: {0}")]
    Unavailable(String),
    #[error("invalid storage key `{0}`")]
    InvalidKey(String),
    #[error("storage I/O error: {0}")]
    Io(String),
}

/// A place to keep named blobs.
pub trait Storage {
    /// The value of `key`, or `None` if it does not exist.
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError>;
    /// Creates or atomically replaces `key`.
    fn write(&self, key: &str, data: &[u8]) -> Result<(), StorageError>;
    /// Removes `key`; a missing key is not an error.
    fn delete(&self, key: &str) -> Result<(), StorageError>;
    /// All keys starting with `prefix`, sorted.
    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError>;
}

/// Checks a key: 1..=128 characters among `a-z 0-9 _ - . /`, no empty,
/// leading-dot or `..` segment, no leading or trailing `/`.
pub fn validate_key(key: &str) -> Result<(), StorageError> {
    let ok = !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-./".contains(c))
        && key
            .split('/')
            .all(|seg| !seg.is_empty() && !seg.starts_with('.'));
    if ok {
        Ok(())
    } else {
        Err(StorageError::InvalidKey(key.to_owned()))
    }
}

/// In-memory storage, for tests and tools. Clones share the same data, which
/// lets a test "restart" a game against the same storage.
#[derive(Clone, Debug, Default)]
pub struct MemoryStorage {
    inner: Arc<Mutex<MemoryInner>>,
}

#[derive(Debug, Default)]
struct MemoryInner {
    data: BTreeMap<String, Vec<u8>>,
    quota: Option<usize>,
    fail_writes: bool,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fails writes with [`StorageError::QuotaExceeded`] once the total size
    /// would exceed `bytes` (to test quota handling).
    pub fn with_quota(bytes: usize) -> Self {
        let s = Self::default();
        s.lock().quota = Some(bytes);
        s
    }

    /// Makes every write fail with an I/O error (to test failure paths).
    pub fn set_fail_writes(&self, fail: bool) {
        self.lock().fail_writes = fail;
    }

    /// Overwrites a value bypassing key validation and quota (to plant
    /// corrupt data in tests).
    pub fn put_raw(&self, key: &str, data: Vec<u8>) {
        self.lock().data.insert(key.to_owned(), data);
    }

    /// Total number of stored bytes.
    pub fn total_size(&self) -> usize {
        self.lock().data.values().map(Vec::len).sum()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Storage for MemoryStorage {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        validate_key(key)?;
        Ok(self.lock().data.get(key).cloned())
    }

    fn write(&self, key: &str, data: &[u8]) -> Result<(), StorageError> {
        validate_key(key)?;
        let mut inner = self.lock();
        if inner.fail_writes {
            return Err(StorageError::Io("simulated write failure".into()));
        }
        if let Some(quota) = inner.quota {
            let others: usize = inner
                .data
                .iter()
                .filter(|(k, _)| k.as_str() != key)
                .map(|(_, v)| v.len())
                .sum();
            if others + data.len() > quota {
                return Err(StorageError::QuotaExceeded);
            }
        }
        inner.data.insert(key.to_owned(), data.to_vec());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        validate_key(key)?;
        self.lock().data.remove(key);
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        Ok(self
            .lock()
            .data
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_validation() {
        for ok in ["a", "saves/quick.sav", "saves/auto_0.sav", "prefs.ron"] {
            assert!(validate_key(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "/a", "a/", "a//b", "../x", "a/../b", ".hidden", "A", "a b", "a\\b", "C:/x",
        ] {
            assert!(validate_key(bad).is_err(), "{bad}");
        }
        assert!(validate_key(&"a".repeat(129)).is_err());
    }

    #[test]
    fn memory_storage_basics_and_quota() {
        let s = MemoryStorage::with_quota(10);
        assert_eq!(s.read("k").unwrap(), None);
        s.write("k", b"123456").unwrap();
        // Replacing does not count the old value twice.
        s.write("k", b"1234567890").unwrap();
        assert_eq!(s.write("j", b"1"), Err(StorageError::QuotaExceeded));
        assert_eq!(s.read("k").unwrap().unwrap(), b"1234567890");
        assert_eq!(s.list("").unwrap(), ["k"]);
        s.delete("k").unwrap();
        s.delete("k").unwrap();
        assert!(s.list("").unwrap().is_empty());
        assert!(matches!(
            s.write("../x", b""),
            Err(StorageError::InvalidKey(_))
        ));
    }
}
