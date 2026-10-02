//! Native storage: one file per key under a per-game data directory.
//!
//! `orbit_save` is (with `orbit_assets`) one of the crates allowed to touch
//! `std::fs`; this module only exists on non-wasm targets.
//!
//! Default location, see [`default_data_dir`]:
//!
//! | OS      | directory                                              |
//! |---------|--------------------------------------------------------|
//! | Windows | `%APPDATA%\OrbitEngine\<game_id>`                      |
//! | macOS   | `~/Library/Application Support/OrbitEngine/<game_id>`  |
//! | Linux   | `$XDG_DATA_HOME` or `~/.local/share`, then `OrbitEngine/<game_id>` |
//!
//! Writes go to `<file>.tmp`, are flushed to disk, then renamed over the
//! target, so a crash never leaves a half-written save.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::storage::{Storage, StorageError, validate_key};

/// The per-user data directory for a game: see the module documentation.
/// `game_id` should be a short stable identifier (the story/project name).
/// Returns `None` if the platform provides no home/data directory.
pub fn default_data_dir(game_id: &str) -> Option<PathBuf> {
    fn var(name: &str) -> Option<PathBuf> {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }
    let base = if cfg!(windows) {
        var("APPDATA")?
    } else if cfg!(target_os = "macos") {
        var("HOME")?.join("Library/Application Support")
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local/share")))?
    };
    Some(base.join("OrbitEngine").join(sanitize_game_id(game_id)))
}

fn sanitize_game_id(id: &str) -> String {
    let s: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_start_matches('.');
    if s.is_empty() {
        "game".into()
    } else {
        s.into()
    }
}

/// Stores each key as a file under `root`.
#[derive(Clone, Debug)]
pub struct FileStorage {
    root: PathBuf,
}

impl FileStorage {
    /// Uses `root` as the data directory (created lazily on first write).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Uses [`default_data_dir`] for `game_id`.
    pub fn for_game(game_id: &str) -> Result<Self, StorageError> {
        default_data_dir(game_id).map(Self::new).ok_or_else(|| {
            StorageError::Unavailable("no user data directory on this platform".into())
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, key: &str) -> Result<PathBuf, StorageError> {
        validate_key(key)?;
        Ok(self.root.join(key))
    }
}

fn io_error(e: std::io::Error) -> StorageError {
    match e.kind() {
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => StorageError::QuotaExceeded,
        _ => StorageError::Io(e.to_string()),
    }
}

impl Storage for FileStorage {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        match fs::read(self.path(key)?) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error(e)),
        }
    }

    fn write(&self, key: &str, data: &[u8]) -> Result<(), StorageError> {
        let path = self.path(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let mut tmp = path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let result = (|| {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(data)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp, &path)
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&tmp);
            return Err(io_error(e));
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        match fs::remove_file(self.path(key)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_error(e)),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, StorageError> {
        // `a/b/c_` -> directory `a/b`, file-name prefix `c_`.
        let (dir, name_prefix) = match prefix.rfind('/') {
            Some(i) => (&prefix[..i], &prefix[i + 1..]),
            None => ("", prefix),
        };
        let dir_path = if dir.is_empty() {
            self.root.clone()
        } else {
            validate_key(dir)?;
            self.root.join(dir)
        };
        let entries = match fs::read_dir(&dir_path) {
            Ok(e) => e,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_error(e)),
        };
        let mut keys = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name.ends_with(".tmp") || !name.starts_with(name_prefix) {
                continue;
            }
            if !entry.file_type().map_err(io_error)?.is_file() {
                continue;
            }
            let key = if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            };
            if validate_key(&key).is_ok() {
                keys.push(key);
            }
        }
        keys.sort();
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "orbit_save_test_{tag}_{}_{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn files_roundtrip_atomically() {
        let root = temp_root("fs");
        let s = FileStorage::new(&root);
        assert_eq!(s.read("saves/quick.sav").unwrap(), None);
        assert!(s.list("saves/").unwrap().is_empty());
        s.write("saves/quick.sav", b"one").unwrap();
        s.write("saves/quick.sav", b"two").unwrap();
        s.write("saves/auto_0.sav", b"a").unwrap();
        s.write("prefs.ron", b"p").unwrap();
        assert_eq!(s.read("saves/quick.sav").unwrap().unwrap(), b"two");
        assert_eq!(
            s.list("saves/").unwrap(),
            ["saves/auto_0.sav", "saves/quick.sav"]
        );
        assert_eq!(s.list("saves/auto").unwrap(), ["saves/auto_0.sav"]);
        assert_eq!(s.list("pre").unwrap(), ["prefs.ron"]);
        // No temporary file is left behind.
        assert!(!root.join("saves/quick.sav.tmp").exists());
        s.delete("saves/quick.sav").unwrap();
        s.delete("saves/quick.sav").unwrap();
        assert_eq!(s.read("saves/quick.sav").unwrap(), None);
        assert!(matches!(s.read("../x"), Err(StorageError::InvalidKey(_))));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn data_dir_is_per_game_and_sanitized() {
        assert_eq!(sanitize_game_id("My Game!"), "My_Game_");
        assert_eq!(sanitize_game_id("../.."), "_..");
        assert_eq!(sanitize_game_id(""), "game");
        if let Some(dir) = default_data_dir("demo") {
            assert!(dir.ends_with("OrbitEngine/demo") || dir.ends_with("OrbitEngine\\demo"));
        }
    }
}
