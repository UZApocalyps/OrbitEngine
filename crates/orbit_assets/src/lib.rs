//! Asynchronous asset loading.
//!
//! [`AssetLoader`] resolves paths relative to a root (a directory natively, a
//! URL prefix on the web) and returns an [`AssetHandle`] immediately. The game
//! loop polls the handle each frame and never blocks:
//!
//! * native: bytes are read **and decoded** on a worker thread;
//! * web: bytes are fetched with `fetch` as a browser task; decoding then runs
//!   on the main thread (no threads in plain wasm32).
//!
//! A loader can also read from a **pak** ([`PakSource`], see `orbit_pak`): the
//! same calls, but the bytes come from the game's pak file, natively or, on
//! the web, lazily with HTTP range requests.
//!
//! [`FileWatcher`] polls a file for changes (hot reload; inert on the web).
//!
//! This crate is the only place allowed to touch `std::fs` / `std::thread`.
//! It knows nothing about asset IDs or manifests: callers pass relative paths.

mod audio;
mod handle;
mod image_data;
mod io;
#[cfg(not(target_arch = "wasm32"))]
pub use io::loading_threads;
mod pak;
mod watch;

pub use audio::{PcmAudio, decode_audio};
pub use handle::{AssetHandle, AssetPoll};
pub use image_data::{ImageData, decode_image};
pub use pak::{PakLocation, PakSource};
#[cfg(not(target_arch = "wasm32"))]
pub use pak::{PakOpener, PakRead, register_pak_scheme};
pub use watch::FileWatcher;

use std::sync::Arc;

use thiserror::Error;

/// Errors produced while loading or decoding an asset.
#[derive(Debug, Clone, Error)]
pub enum AssetError {
    #[error("invalid asset path `{0}` (must be relative, without `..`)")]
    InvalidPath(String),
    #[error("io error for `{path}`: {message}")]
    Io { path: String, message: String },
    #[error("decode error for `{path}`: {message}")]
    Decode { path: String, message: String },
}

/// Where an [`AssetLoader`] reads from.
#[derive(Debug, Clone)]
pub(crate) enum Source {
    /// A directory (native) or a URL prefix (web).
    Root(Arc<str>),
    /// A pak; `prefix` is prepended to every requested path.
    Pak { pak: PakSource, prefix: Arc<str> },
}

/// Loads assets relative to a root (or from a pak). Cheap to clone.
#[derive(Debug, Clone)]
pub struct AssetLoader {
    source: Source,
}

/// Validates a relative asset path: non-empty, no absolute prefix, no `..`, no
/// backslashes or drive letters. Returns the normalized `/`-separated path.
pub fn normalize_path(path: &str) -> Result<String, AssetError> {
    let invalid = || AssetError::InvalidPath(path.to_owned());
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Err(invalid());
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(invalid()),
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(invalid());
    }
    Ok(parts.join("/"))
}

impl AssetLoader {
    /// `root` is a directory path (native) or a URL prefix such as `"assets"`
    /// (web; resolved against the page URL).
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            source: Source::Root(root.into().into()),
        }
    }

    /// A loader reading from `pak`: a requested path `p` is the entry
    /// `<prefix>/<p>` (`prefix` empty means `p` itself).
    pub fn from_pak(pak: PakSource, prefix: impl Into<String>) -> Self {
        let prefix: String = prefix.into();
        Self {
            source: Source::Pak {
                pak,
                prefix: prefix.trim_matches('/').into(),
            },
        }
    }

    /// The root (directory or URL prefix), or `<pak>` for a pak loader.
    pub fn root(&self) -> &str {
        match &self.source {
            Source::Root(root) => root,
            Source::Pak { .. } => "<pak>",
        }
    }

    /// Loads raw bytes.
    pub fn load_bytes(&self, path: &str) -> AssetHandle<Vec<u8>> {
        self.load_with(path, |_, bytes| Ok(bytes))
    }

    /// Loads and decodes a PNG/JPEG image to RGBA8.
    pub fn load_image(&self, path: &str) -> AssetHandle<ImageData> {
        #[cfg(target_arch = "wasm32")]
        {
            // On the web the browser decodes JPEG (smaller bundle).
            let (handle, slot) = AssetHandle::new_pair();
            match normalize_path(path) {
                Ok(p) => io::spawn_load_image(self.source.clone(), p, slot),
                Err(err) => slot.fail(err),
            }
            handle
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.load_with(path, |_, bytes| decode_image(&bytes))
    }

    /// Loads and decodes OGG Vorbis or WAV audio to PCM.
    pub fn load_audio(&self, path: &str) -> AssetHandle<PcmAudio> {
        self.load_with(path, |name, bytes| decode_audio(name, &bytes))
    }

    /// Loads bytes then runs `decode(path, bytes)` (on a worker thread when
    /// possible). Failures are reported through the handle, never panics.
    pub fn load_with<T, F>(&self, path: &str, decode: F) -> AssetHandle<T>
    where
        T: Send + Sync + 'static,
        F: FnOnce(&str, Vec<u8>) -> Result<T, String> + Send + 'static,
    {
        let (handle, slot) = AssetHandle::new_pair();
        let normalized = match normalize_path(path) {
            Ok(p) => p,
            Err(err) => {
                slot.fail(err);
                return handle;
            }
        };
        io::spawn_load(self.source.clone(), normalized, decode, slot);
        handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_normalization() {
        assert_eq!(normalize_path("a/b.png").unwrap(), "a/b.png");
        assert_eq!(normalize_path("./a//b.png").unwrap(), "a/b.png");
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "C:/x", "a\\b", "."] {
            assert!(normalize_path(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn missing_file_fails_without_panic() {
        let root = std::env::temp_dir().join("orbit_missing_root");
        let loader = AssetLoader::new(root.to_string_lossy());
        let h = loader.load_bytes("nope.bin");
        assert!(matches!(
            h.wait_blocking(),
            AssetPoll::Failed(AssetError::Io { .. })
        ));
    }

    /// Many requests at once run on the bounded pool: all complete, never
    /// more at a time than it has threads.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn loads_run_on_a_bounded_pool() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = std::env::temp_dir().join(format!("orbit_assets_pool_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.bin"), b"abc").unwrap();
        let loader = AssetLoader::new(dir.to_string_lossy());
        let (now, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let handles: Vec<_> = (0..48)
            .map(|_| {
                let (now, peak) = (now.clone(), peak.clone());
                loader.load_with("a.bin", move |_, bytes| {
                    let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(n, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    now.fetch_sub(1, Ordering::SeqCst);
                    Ok(bytes.len())
                })
            })
            .collect();
        for h in &handles {
            assert!(matches!(h.wait_blocking(), AssetPoll::Ready(n) if *n == 3));
        }
        assert!(peak.load(Ordering::SeqCst) <= loading_threads(), "{peak:?}");
        // A request nobody waits for any more is skipped.
        drop(loader.load_bytes("a.bin"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn loads_and_decodes_png_in_background() {
        use image::{ImageEncoder, codecs::png::PngEncoder};
        let dir = std::env::temp_dir().join("orbit_assets_test");
        std::fs::create_dir_all(&dir).unwrap();
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(
                &[255, 0, 0, 255, 0, 255, 0, 128],
                2,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        std::fs::write(dir.join("t.png"), &png).unwrap();
        let loader = AssetLoader::new(dir.to_string_lossy());
        match loader.load_image("t.png").wait_blocking() {
            AssetPoll::Ready(img) => {
                assert_eq!((img.width, img.height), (2, 1));
                assert_eq!(img.rgba.len(), 8);
            }
            _ => panic!("expected ready"),
        }
        // A bad decode is a Failed state, not a panic.
        std::fs::write(dir.join("bad.png"), b"not a png").unwrap();
        assert!(matches!(
            loader.load_image("bad.png").wait_blocking(),
            AssetPoll::Failed(AssetError::Decode { .. })
        ));
    }
}
