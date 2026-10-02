//! Thumbnails of project assets for the inspector, the character editor and
//! the asset library: textures cached per file version, filled by the
//! worker threads of [`crate::thumbs`] (nothing is decoded in a frame).
//!
//! Lazy: a thumbnail is requested the first time a row that shows it is
//! drawn (lists are virtualized, so off-screen assets cost nothing). The
//! texture cache is bounded (least recently drawn first out); the disk
//! cache lives in `.orbit/thumbs/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{ColorImage, Context, Id, TextureHandle, TextureOptions};
use orbit_core::{AssetEntry, AssetId, Story};
use orbit_editor_api::{EditorContext, Waker};

use crate::thumbs::{Stamp, Thumb, ThumbRequest, ThumbWorker, prune_disk_cache};

/// Textures kept; above it the least recently drawn are dropped.
const MAX_TEXTURES: usize = 400;
const KEEP_TEXTURES: usize = 300;
/// File stamps are looked at again after this long (or on a watch event).
const STAMP_TTL: Duration = Duration::from_millis(1500);
const WORKERS: usize = 2;

/// What the UI can show for an asset right now.
#[derive(Clone)]
pub enum ThumbState {
    /// Being decoded.
    Loading,
    Ready {
        texture: Option<TextureHandle>,
        /// `1920×1080`, `0:12`...
        info: String,
    },
    /// Missing or undecodable file.
    Failed(String),
}

struct Entry {
    state: ThumbState,
    last_used: u64,
}

struct Service {
    worker: ThumbWorker,
    entries: HashMap<String, Entry>,
    stamps: HashMap<(PathBuf, String), (Instant, Option<Stamp>)>,
    pruned: Vec<PathBuf>,
    /// Pass number of the last poll of the worker.
    polled_pass: u64,
    tick: u64,
}

type Shared = Arc<Mutex<Service>>;

fn service(ctx: &Context, waker: &Waker) -> Shared {
    let id = Id::new("orbit_thumb_service");
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with::<Shared>(id, || {
            Arc::new(Mutex::new(Service {
                worker: ThumbWorker::new(WORKERS, waker.clone()),
                entries: HashMap::new(),
                stamps: HashMap::new(),
                pruned: Vec::new(),
                polled_pass: u64::MAX,
                tick: 0,
            }))
        })
        .clone()
    })
}

impl Service {
    fn poll(&mut self, ctx: &Context) {
        for (key, result) in self.worker.take_done() {
            let state = match result {
                Ok(Thumb { image, info }) => ThumbState::Ready {
                    texture: image.map(|(w, h, rgba)| {
                        ctx.load_texture(
                            format!("thumb:{key}"),
                            ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
                            TextureOptions::LINEAR,
                        )
                    }),
                    info,
                },
                Err(e) => ThumbState::Failed(e),
            };
            let last_used = self.tick;
            self.entries.insert(key, Entry { state, last_used });
        }
        if self.entries.len() > MAX_TEXTURES {
            let mut by_use: Vec<(u64, String)> = self
                .entries
                .iter()
                .filter(|(_, e)| !matches!(e.state, ThumbState::Loading))
                .map(|(k, e)| (e.last_used, k.clone()))
                .collect();
            by_use.sort();
            let excess = self.entries.len().saturating_sub(KEEP_TEXTURES);
            for (_, key) in by_use.into_iter().take(excess) {
                self.entries.remove(&key);
            }
        }
    }

    fn stamp(&mut self, root: &Path, rel: &str) -> Option<Stamp> {
        let key = (root.to_owned(), rel.to_owned());
        let now = Instant::now();
        if let Some((at, stamp)) = self.stamps.get(&key)
            && now.duration_since(*at) < STAMP_TTL
        {
            return *stamp;
        }
        let stamp = Stamp::of(&root.join(rel));
        self.stamps.insert(key, (now, stamp));
        stamp
    }
}

/// The thumbnail state of an asset; asks for it when it is not known yet.
pub fn state(egui_ctx: &Context, waker: &Waker, root: &Path, entry: &AssetEntry) -> ThumbState {
    let shared = service(egui_ctx, waker);
    let Ok(mut svc) = shared.lock() else {
        return ThumbState::Loading;
    };
    let pass = egui_ctx.cumulative_pass_nr();
    if svc.polled_pass != pass {
        svc.polled_pass = pass;
        svc.tick += 1;
        svc.poll(egui_ctx);
    }
    if !svc.pruned.iter().any(|r| r == root) {
        svc.pruned.push(root.to_owned());
        let root = root.to_owned();
        let _ = std::thread::Builder::new()
            .name("orbit-thumbs-prune".to_owned())
            .spawn(move || {
                prune_disk_cache(&root);
            });
    }
    let Some(stamp) = svc.stamp(root, &entry.path) else {
        return ThumbState::Failed("file not found".to_owned());
    };
    let req = ThumbRequest {
        root: root.to_owned(),
        rel: entry.path.clone(),
        kind: entry.kind,
        stamp,
    };
    let key = req.key();
    let tick = svc.tick;
    if let Some(found) = svc.entries.get_mut(&key) {
        found.last_used = tick;
        return found.state.clone();
    }
    svc.worker.request(req);
    ThumbState::Loading
}

/// Forgets what is known about files that changed on disk (watcher).
pub fn invalidate(egui_ctx: &Context, waker: &Waker, rels: &[String]) {
    let shared = service(egui_ctx, waker);
    if let Ok(mut svc) = shared.lock() {
        svc.stamps.retain(|(_, rel), _| !rels.contains(rel));
    }
}

/// Thumbnails still being produced (a frame must follow when this is not
/// zero and nothing woke the loop).
pub fn pending(egui_ctx: &Context, waker: &Waker) -> usize {
    service(egui_ctx, waker)
        .lock()
        .map_or(0, |svc| svc.worker.pending())
}

/// The texture of an image asset, when it is ready (asks for it otherwise).
pub fn thumbnail(
    egui_ctx: &Context,
    waker: &Waker,
    root: &Path,
    story: &Story,
    asset: AssetId,
) -> Option<TextureHandle> {
    let entry = story.assets.get(asset)?;
    match state(egui_ctx, waker, root, entry) {
        ThumbState::Ready { texture, .. } => texture,
        _ => None,
    }
}

/// Folder the manifest's relative paths resolve against, for the open project.
pub fn assets_root(ctx: &EditorContext<'_>, story: &Story) -> Option<PathBuf> {
    let info = ctx.project_info()?;
    Some(crate::preview_host::find_assets_root(&info.root, story))
}

fn placeholder(ui: &mut egui::Ui, max: egui::Vec2, glyph: &str) {
    let (rect, _) = ui.allocate_exact_size(max, egui::Sense::hover());
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(6),
        ui.visuals().widgets.inactive.bg_fill,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(18.0),
        ui.visuals().weak_text_color(),
    );
}

/// Draws a thumbnail fitted in a `max` box, or a neutral placeholder while
/// it loads (or when the file is missing).
pub fn show(
    ui: &mut egui::Ui,
    ctx: &EditorContext<'_>,
    story: &Story,
    asset: AssetId,
    max: egui::Vec2,
) {
    let waker = ctx.state().waker.clone();
    let state = story
        .assets
        .get(asset)
        .zip(assets_root(ctx, story))
        .map(|(entry, root)| state(ui.ctx(), &waker, &root, entry));
    match state {
        Some(ThumbState::Ready {
            texture: Some(t), ..
        }) => {
            ui.add(egui::Image::new(&t).fit_to_exact_size(max).corner_radius(6));
        }
        Some(ThumbState::Loading) => placeholder(ui, max, orbit_editor_api::icons::IMAGE),
        _ => placeholder(ui, max, orbit_editor_api::icons::IMAGE_BROKEN),
    }
}
