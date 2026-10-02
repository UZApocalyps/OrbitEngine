//! Reading a pak (see `orbit_pak`): from a file natively, with HTTP range
//! requests on the web.
//!
//! [`PakSource::open`] reads the header and the index only (a few KiB); the
//! entries are then read one at a time, when an [`crate::AssetLoader`] asks
//! for them. On the web that is the lazy loading: nothing but the index and
//! the requested files crosses the network. A server that ignores `Range`
//! (answers `200` with the whole file) still works: the whole pak is then
//! downloaded once and served from memory.

use std::sync::Arc;

use orbit_pak::{HEADER_LEN, Header, PakIndex};

use crate::AssetError;
use crate::handle::AssetHandle;

/// Where a pak is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PakLocation {
    /// A file path (native) or a URL (web).
    pub target: String,
    /// Native only: the pak is glued to the end of `target` (an executable)
    /// and found through its trailer; otherwise it starts at the first byte.
    pub appended: bool,
}

impl PakLocation {
    pub fn file(path: impl Into<String>) -> Self {
        Self {
            target: path.into(),
            appended: false,
        }
    }

    pub fn url(url: impl Into<String>) -> Self {
        Self::file(url)
    }

    /// Native: the pak of this executable. Looks for a pak glued to the end
    /// of the running executable, then for a `game.pak` file next to it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn beside_executable() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        let here = Self {
            target: exe.to_string_lossy().into_owned(),
            appended: true,
        };
        if native::locate(&here).is_ok() {
            return Some(here);
        }
        let file = exe.parent()?.join("game.pak");
        file.is_file().then(|| Self::file(file.to_string_lossy()))
    }
}

enum Backend {
    /// Whole pak in memory (tests, servers that ignore `Range`).
    Memory(Vec<u8>),
    #[cfg(not(target_arch = "wasm32"))]
    File {
        path: std::path::PathBuf,
        /// Offset of the pak in the file (non zero when glued to an
        /// executable).
        base: u64,
    },
    #[cfg(target_arch = "wasm32")]
    Web { url: String },
}

struct Inner {
    header: Header,
    index: PakIndex,
    backend: Backend,
}

/// An opened pak. Cheap to clone.
#[derive(Clone)]
pub struct PakSource {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PakSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PakSource({} entries)", self.inner.index.entries().len())
    }
}

fn pak_error(err: orbit_pak::PakError) -> AssetError {
    AssetError::Decode {
        path: "pak".to_owned(),
        message: err.to_string(),
    }
}

impl PakSource {
    /// A pak held in memory.
    pub fn from_memory(bytes: Vec<u8>) -> Result<Self, AssetError> {
        let (header, index) = orbit_pak::container::parse_prefix(&bytes).map_err(pak_error)?;
        Ok(Self {
            inner: Arc::new(Inner {
                header,
                index,
                backend: Backend::Memory(bytes),
            }),
        })
    }

    pub fn index(&self) -> &PakIndex {
        &self.inner.index
    }

    /// A metadata value of the pak (`name`, `id`...).
    pub fn meta(&self, key: &str) -> Option<&str> {
        self.inner.index.meta(key)
    }

    pub fn contains(&self, path: &str) -> bool {
        self.inner.index.contains(path)
    }

    /// Reads and decodes an entry, blocking. Native and in-memory paks only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_blocking(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        let inner = &self.inner;
        let entry = inner.index.entry(path).ok_or_else(|| AssetError::Io {
            path: path.to_owned(),
            message: "not in the pak".to_owned(),
        })?;
        let (start, len) = inner.index.stored_range(&inner.header, entry);
        let stored = match &inner.backend {
            Backend::Memory(bytes) => bytes
                .get(start as usize..(start + len) as usize)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| AssetError::Io {
                    path: path.to_owned(),
                    message: "the pak is truncated".to_owned(),
                })?,
            Backend::File { path: file, base } => native::read_at(file, base + start, len as usize)
                .map_err(|e| AssetError::Io {
                    path: path.to_owned(),
                    message: e.to_string(),
                })?,
        };
        entry.decode(&stored).map_err(|e| AssetError::Decode {
            path: path.to_owned(),
            message: e.to_string(),
        })
    }

    /// Opens a pak in the background (a worker thread natively, `fetch` on
    /// the web): only the header and the index are read.
    pub fn open(location: PakLocation) -> AssetHandle<PakSource> {
        let (handle, slot) = AssetHandle::new_pair();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let spawned = std::thread::Builder::new()
                .name("orbit-pak-open".to_owned())
                .spawn(move || match Self::open_blocking(&location) {
                    Ok(pak) => slot.ready(pak),
                    Err(err) => slot.fail(err),
                });
            if let Err(err) = spawned {
                log::error!("could not spawn the pak reader thread: {err}");
            }
        }
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            match web::open(&location.target).await {
                Ok(pak) => slot.ready(pak),
                Err(err) => slot.fail(err),
            }
        });
        handle
    }

    /// Opens a pak file now (the player reads the game name and icon before
    /// the window exists). Native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_blocking(location: &PakLocation) -> Result<Self, AssetError> {
        let io_error = |e: std::io::Error| AssetError::Io {
            path: location.target.clone(),
            message: e.to_string(),
        };
        let (path, base) = native::locate(location).map_err(io_error)?;
        let header_bytes = native::read_at(&path, base, HEADER_LEN).map_err(io_error)?;
        let header = Header::parse(&header_bytes).map_err(pak_error)?;
        let index_bytes =
            native::read_at(&path, base + HEADER_LEN as u64, header.index_len as usize)
                .map_err(io_error)?;
        let index = PakIndex::from_bytes(&index_bytes).map_err(pak_error)?;
        Ok(Self {
            inner: Arc::new(Inner {
                header,
                index,
                backend: Backend::File { path, base },
            }),
        })
    }

    /// Reads and decodes an entry on the web.
    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn read_async(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        let inner = &self.inner;
        let entry = inner.index.entry(path).ok_or_else(|| AssetError::Io {
            path: path.to_owned(),
            message: "not in the pak".to_owned(),
        })?;
        let (start, len) = inner.index.stored_range(&inner.header, entry);
        let stored = match &inner.backend {
            Backend::Memory(bytes) => bytes
                .get(start as usize..(start + len) as usize)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| AssetError::Io {
                    path: path.to_owned(),
                    message: "the pak is truncated".to_owned(),
                })?,
            Backend::Web { url } => {
                if len == 0 {
                    Vec::new()
                } else {
                    web::read_range(url, start, len)
                        .await
                        .map_err(|message| AssetError::Io {
                            path: path.to_owned(),
                            message,
                        })?
                }
            }
        };
        entry.decode(&stored).map_err(|e| AssetError::Decode {
            path: path.to_owned(),
            message: e.to_string(),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::{Read, Seek, SeekFrom};
    use std::path::{Path, PathBuf};

    use orbit_pak::trailer::{self, TRAILER_LEN};

    use super::PakLocation;

    /// Reads `len` bytes at `offset`. The file is opened for each read: the
    /// loader threads never share a cursor.
    pub(super) fn read_at(path: &Path, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// The file and the offset of the pak described by `location`.
    pub(super) fn locate(location: &PakLocation) -> std::io::Result<(PathBuf, u64)> {
        let path = PathBuf::from(&location.target);
        if !location.appended {
            return Ok((path, 0));
        }
        let size = std::fs::metadata(&path)?.len();
        let not_found = || std::io::Error::other("no pak is attached to this file");
        if size < TRAILER_LEN as u64 {
            return Err(not_found());
        }
        let tail = read_at(&path, size - TRAILER_LEN as u64, TRAILER_LEN)?;
        let pak_len = trailer::decode(&tail).ok_or_else(not_found)?;
        let base = (size - TRAILER_LEN as u64)
            .checked_sub(pak_len)
            .ok_or_else(not_found)?;
        Ok((path, base))
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    use super::*;

    /// How much is asked for with the first request: the header and, in
    /// practice, the whole index.
    const FIRST_READ: u64 = 64 * 1024;

    fn js_err(e: wasm_bindgen::JsValue) -> String {
        format!("{e:?}")
    }

    /// `(status, body)` of a request for bytes `start..start + len`.
    async fn fetch_range(url: &str, start: u64, len: u64) -> Result<(u16, Vec<u8>), String> {
        let headers = web_sys::Headers::new().map_err(js_err)?;
        headers
            .set("Range", &format!("bytes={start}-{}", start + len - 1))
            .map_err(js_err)?;
        let init = web_sys::RequestInit::new();
        init.set_headers(&headers);
        let window = web_sys::window().ok_or("no window")?;
        let resp = JsFuture::from(window.fetch_with_str_and_init(url, &init))
            .await
            .map_err(js_err)?;
        let resp: web_sys::Response = resp.dyn_into().map_err(js_err)?;
        if !resp.ok() {
            return Err(format!("HTTP {} for {url}", resp.status()));
        }
        let buf = JsFuture::from(resp.array_buffer().map_err(js_err)?)
            .await
            .map_err(js_err)?;
        Ok((resp.status(), js_sys::Uint8Array::new(&buf).to_vec()))
    }

    /// Reads a range; a server that ignores `Range` sends the whole file,
    /// the wanted part is cut out of it.
    pub(super) async fn read_range(url: &str, start: u64, len: u64) -> Result<Vec<u8>, String> {
        let (status, body) = fetch_range(url, start, len).await?;
        if status == 206 {
            return Ok(body);
        }
        body.get(start as usize..(start + len) as usize)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "the server answered with a short body".to_owned())
    }

    pub(super) async fn open(url: &str) -> Result<PakSource, AssetError> {
        let error = |message: String| AssetError::Io {
            path: url.to_owned(),
            message,
        };
        let (status, first) = fetch_range(url, 0, FIRST_READ).await.map_err(error)?;
        if status != 206 {
            // No range support: keep the whole file.
            log::warn!("`{url}`: the server ignores range requests, downloading the whole pak");
            return PakSource::from_memory(first);
        }
        let header = Header::parse(&first).map_err(pak_error)?;
        let index_end = header.index_end();
        let index_bytes = if (first.len() as u64) >= index_end {
            first[HEADER_LEN..index_end as usize].to_vec()
        } else {
            read_range(url, HEADER_LEN as u64, u64::from(header.index_len))
                .await
                .map_err(error)?
        };
        let index = PakIndex::from_bytes(&index_bytes).map_err(pak_error)?;
        Ok(PakSource {
            inner: Arc::new(Inner {
                header,
                index,
                backend: Backend::Web {
                    url: url.to_owned(),
                },
            }),
        })
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use orbit_pak::{Compression, PakWriter};

    use super::*;
    use crate::AssetLoader;

    fn sample_pak() -> Vec<u8> {
        let mut w = PakWriter::new(Compression::Lz4);
        w.meta("name", "Demo");
        w.add("assets/a.txt", &b"alpha ".repeat(50)).unwrap();
        w.add("b.bin", &[1, 2, 3]).unwrap();
        w.to_bytes().0
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("orbit_pak_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn memory_pak_serves_entries_through_a_loader() {
        let pak = PakSource::from_memory(sample_pak()).unwrap();
        assert_eq!(pak.meta("name"), Some("Demo"));
        let loader = AssetLoader::from_pak(pak, "");
        let bytes = loader.load_bytes("assets/a.txt").wait_blocking();
        assert!(matches!(&bytes, crate::AssetPoll::Ready(b) if **b == b"alpha ".repeat(50)));
        assert!(matches!(
            loader.load_bytes("missing").wait_blocking(),
            crate::AssetPoll::Failed(AssetError::Io { .. })
        ));
    }

    #[test]
    fn prefix_maps_requests_into_a_subtree() {
        let pak = PakSource::from_memory(sample_pak()).unwrap();
        let loader = AssetLoader::from_pak(pak, "assets");
        assert!(loader.load_bytes("a.txt").wait_blocking().is_ready());
    }

    #[test]
    fn file_pak_and_pak_glued_to_an_executable() {
        let dir = temp_dir("file");
        let pak = sample_pak();
        let plain = dir.join("game.pak");
        std::fs::write(&plain, &pak).unwrap();
        let opened = PakSource::open(PakLocation::file(plain.to_string_lossy())).wait_blocking();
        let crate::AssetPoll::Ready(opened) = opened else {
            panic!("pak did not open");
        };
        assert_eq!(opened.read_blocking("b.bin").unwrap(), vec![1, 2, 3]);

        // [fake executable][pak][trailer]
        let exe = dir.join("player.exe");
        let mut glued = vec![0xAAu8; 1000];
        glued.extend_from_slice(&pak);
        glued.extend_from_slice(&orbit_pak::trailer::encode(pak.len() as u64));
        std::fs::write(&exe, &glued).unwrap();
        let location = PakLocation {
            target: exe.to_string_lossy().into_owned(),
            appended: true,
        };
        let pak = PakSource::open_blocking(&location).unwrap();
        assert_eq!(pak.read_blocking("b.bin").unwrap(), vec![1, 2, 3]);
        assert_eq!(pak.read_blocking("assets/a.txt").unwrap().len(), 300);

        // An executable without trailer, and a file that is no pak.
        std::fs::write(&exe, vec![0u8; 100]).unwrap();
        assert!(PakSource::open_blocking(&location).is_err());
        assert!(PakSource::open_blocking(&PakLocation::file(exe.to_string_lossy())).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
