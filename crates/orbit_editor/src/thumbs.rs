//! Thumbnails of project assets, produced **off the UI thread**.
//!
//! A small pool of worker threads decodes images (or reads the length of an
//! audio file), shrinks them to [`THUMB_SIDE`] and keeps the result in a
//! disk cache (`<project>/.orbit/thumbs/`), keyed by path, size and
//! modification time, so a second session (or a second visit of the same
//! asset) costs one small PNG read. Requests are served newest first: the
//! rows scrolled into view are decoded before the ones left behind.
//! Nothing here knows egui: the UI side (`builtin::thumbs`) turns the
//! decoded pixels into textures.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::SystemTime;

use orbit_core::AssetKind;
use orbit_editor_api::Waker;
use orbit_project::STATE_DIR;

/// Longest side of a thumbnail, in pixels.
pub const THUMB_SIDE: u32 = 192;
/// Disk cache size above which the oldest files are removed at startup.
const DISK_CACHE_MAX_FILES: usize = 3000;
/// Files kept after such a cleanup.
const DISK_CACHE_KEEP: usize = 2000;

/// Identity of one version of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Stamp {
    pub len: u64,
    pub modified_ns: u128,
}

impl Stamp {
    /// `None` when the file does not exist.
    pub fn of(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        let modified_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        Some(Self {
            len: meta.len(),
            modified_ns,
        })
    }
}

/// What to produce a thumbnail for.
#[derive(Clone, Debug)]
pub struct ThumbRequest {
    /// Folder the manifest paths resolve against.
    pub root: PathBuf,
    /// Manifest path (`/` separators).
    pub rel: String,
    pub kind: AssetKind,
    pub stamp: Stamp,
}

impl ThumbRequest {
    /// Cache key: changes with the path and with each version of the file.
    pub fn key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.rel, self.stamp.len, self.stamp.modified_ns, self.kind
        )
    }

    fn cache_dir(&self) -> PathBuf {
        self.root.join(STATE_DIR).join("thumbs")
    }

    fn cache_stem(&self) -> String {
        format!("{:016x}", fnv1a(self.key().as_bytes()))
    }
}

/// FNV-1a, stable across Rust versions (unlike `DefaultHasher`).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A decoded thumbnail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumb {
    /// RGBA pixels (images only).
    pub image: Option<(u32, u32, Vec<u8>)>,
    /// Short facts for the label: `1920×1080` or `0:12`.
    pub info: String,
}

/// `m:ss` for a duration in seconds.
pub fn format_duration(seconds: f32) -> String {
    let total = seconds.max(0.0).round() as u32;
    format!("{}:{:02}", total / 60, total % 60)
}

/// Produces the thumbnail synchronously, through the disk cache. This is
/// what the worker threads run; tests call it directly.
pub fn generate(req: &ThumbRequest) -> Result<Thumb, String> {
    let dir = req.cache_dir();
    let stem = req.cache_stem();
    let info_path = dir.join(format!("{stem}.txt"));
    let png_path = dir.join(format!("{stem}.png"));
    if let Ok(info) = fs::read_to_string(&info_path) {
        match req.kind {
            AssetKind::Image => {
                if let Ok(img) = image::open(&png_path) {
                    let img = img.into_rgba8();
                    return Ok(Thumb {
                        image: Some((img.width(), img.height(), img.into_raw())),
                        info,
                    });
                }
            }
            _ => return Ok(Thumb { image: None, info }),
        }
    }
    let path = req.root.join(&req.rel);
    let thumb = match req.kind {
        AssetKind::Image => {
            let img = image::open(&path).map_err(|e| e.to_string())?;
            let info = format!("{}×{}", img.width(), img.height());
            let small = img.thumbnail(THUMB_SIDE, THUMB_SIDE).into_rgba8();
            if fs::create_dir_all(&dir).is_ok() {
                // Best effort: a read-only project still gets its thumbnails.
                let _ = small.save(&png_path);
                let _ = fs::write(&info_path, &info);
            }
            Thumb {
                image: Some((small.width(), small.height(), small.into_raw())),
                info,
            }
        }
        AssetKind::Music | AssetKind::Sound => {
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            let pcm = orbit_assets::decode_audio(&req.rel, &bytes)?;
            let info = format_duration(pcm.duration_secs());
            if fs::create_dir_all(&dir).is_ok() {
                let _ = fs::write(&info_path, &info);
            }
            Thumb { image: None, info }
        }
    };
    Ok(thumb)
}

/// Removes the oldest cache files when the folder grew past its bound.
pub fn prune_disk_cache(root: &Path) -> usize {
    let dir = root.join(STATE_DIR).join("thumbs");
    let Ok(rd) = fs::read_dir(&dir) else { return 0 };
    let mut files: Vec<(SystemTime, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, e.path()))
        })
        .collect();
    if files.len() <= DISK_CACHE_MAX_FILES {
        return 0;
    }
    files.sort();
    let excess = files.len() - DISK_CACHE_KEEP;
    for (_, path) in files.iter().take(excess) {
        let _ = fs::remove_file(path);
    }
    excess
}

#[derive(Default)]
struct Queue {
    /// Waiting requests; the last one is served first.
    stack: Vec<(String, ThumbRequest)>,
    /// Keys waiting or being generated.
    busy: HashSet<String>,
    done: Vec<(String, Result<Thumb, String>)>,
}

struct Shared {
    queue: Mutex<Queue>,
    wake_worker: Condvar,
    stop: AtomicBool,
}

/// The worker pool. Dropping it stops the threads.
pub struct ThumbWorker {
    shared: Arc<Shared>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl ThumbWorker {
    pub fn new(threads: usize, waker: Waker) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake_worker: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let handles = (0..threads.max(1))
            .filter_map(|n| {
                let shared = shared.clone();
                let waker = waker.clone();
                std::thread::Builder::new()
                    .name(format!("orbit-thumbs-{n}"))
                    .spawn(move || worker_loop(&shared, &waker))
                    .ok()
            })
            .collect();
        Self {
            shared,
            threads: handles,
        }
    }

    /// Queues a request unless the same version is already queued or being
    /// decoded. Returns `true` when it was queued.
    pub fn request(&self, req: ThumbRequest) -> bool {
        let key = req.key();
        let Ok(mut q) = self.shared.queue.lock() else {
            return false;
        };
        if !q.busy.insert(key.clone()) {
            // Already waiting: serve it first again (it is on screen now).
            if let Some(pos) = q.stack.iter().position(|(k, _)| *k == key) {
                let item = q.stack.remove(pos);
                q.stack.push(item);
            }
            return false;
        }
        q.stack.push((key, req));
        drop(q);
        self.shared.wake_worker.notify_one();
        true
    }

    /// Finished thumbnails since the last call.
    pub fn take_done(&self) -> Vec<(String, Result<Thumb, String>)> {
        self.shared
            .queue
            .lock()
            .map(|mut q| std::mem::take(&mut q.done))
            .unwrap_or_default()
    }

    /// Requests not finished yet.
    pub fn pending(&self) -> usize {
        self.shared.queue.lock().map_or(0, |q| q.busy.len())
    }
}

impl Drop for ThumbWorker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake_worker.notify_all();
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
    }
}

fn worker_loop(shared: &Shared, waker: &Waker) {
    loop {
        let job = {
            let Ok(mut q) = shared.queue.lock() else {
                return;
            };
            loop {
                if shared.stop.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(job) = q.stack.pop() {
                    break job;
                }
                q = match shared.wake_worker.wait(q) {
                    Ok(q) => q,
                    Err(_) => return,
                };
            }
        };
        let (key, req) = job;
        let result = generate(&req);
        if let Ok(mut q) = shared.queue.lock() {
            q.busy.remove(&key);
            q.done.push((key, result));
        }
        waker.wake();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orbit_thumbs_{tag}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_png(path: &Path, w: u32, h: u32) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]))
            .save(path)
            .unwrap();
    }

    fn request(root: &Path, rel: &str, kind: AssetKind) -> ThumbRequest {
        ThumbRequest {
            root: root.to_owned(),
            rel: rel.to_owned(),
            kind,
            stamp: Stamp::of(&root.join(rel)).unwrap(),
        }
    }

    #[test]
    fn an_image_is_shrunk_and_cached_on_disk() {
        let root = temp_dir("img");
        write_png(&root.join("assets/images/big.png"), 800, 400);
        let req = request(&root, "assets/images/big.png", AssetKind::Image);
        let first = generate(&req).unwrap();
        let (w, h, pixels) = first.image.clone().unwrap();
        assert_eq!((w, h), (THUMB_SIDE, THUMB_SIDE / 2));
        assert_eq!(pixels.len(), (w * h * 4) as usize);
        assert_eq!(first.info, "800×400");
        assert!(root.join(".orbit/thumbs").is_dir());
        // The second call is served from the cache even if the source is gone.
        fs::remove_file(root.join("assets/images/big.png")).unwrap();
        assert_eq!(generate(&req).unwrap(), first);
        // A new version of the file has another key.
        write_png(&root.join("assets/images/big.png"), 100, 100);
        let newer = request(&root, "assets/images/big.png", AssetKind::Image);
        assert_ne!(newer.key(), req.key());
        assert_eq!(generate(&newer).unwrap().info, "100×100");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_broken_image_is_an_error_not_a_panic() {
        let root = temp_dir("bad");
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join("assets/x.png"), b"not a png").unwrap();
        let req = request(&root, "assets/x.png", AssetKind::Image);
        assert!(generate(&req).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_worker_decodes_off_the_calling_thread_and_reports_done() {
        let root = temp_dir("pool");
        for i in 0..6 {
            write_png(&root.join(format!("assets/images/{i}.png")), 300, 300);
        }
        let worker = ThumbWorker::new(2, Waker::default());
        for i in 0..6 {
            let rel = format!("assets/images/{i}.png");
            assert!(worker.request(request(&root, &rel, AssetKind::Image)));
        }
        // The same version is not queued twice.
        assert!(!worker.request(request(&root, "assets/images/0.png", AssetKind::Image)));
        let started = Instant::now();
        let mut done = Vec::new();
        while done.len() < 6 && started.elapsed() < Duration::from_secs(20) {
            done.extend(worker.take_done());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(done.len(), 6);
        assert!(done.iter().all(|(_, r)| r.is_ok()));
        assert_eq!(worker.pending(), 0);
        drop(worker);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn durations_are_formatted() {
        assert_eq!(format_duration(0.4), "0:00");
        assert_eq!(format_duration(12.0), "0:12");
        assert_eq!(format_duration(125.0), "2:05");
    }

    #[test]
    fn the_disk_cache_is_bounded() {
        let root = temp_dir("prune");
        let dir = root.join(".orbit/thumbs");
        fs::create_dir_all(&dir).unwrap();
        for i in 0..(DISK_CACHE_MAX_FILES + 10) {
            fs::write(dir.join(format!("{i}.txt")), "x").unwrap();
        }
        let removed = prune_disk_cache(&root);
        assert_eq!(removed, DISK_CACHE_MAX_FILES + 10 - DISK_CACHE_KEEP);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), DISK_CACHE_KEEP);
        fs::remove_dir_all(&root).unwrap();
    }
}
