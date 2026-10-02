//! Small image previews of project assets (sprites, backgrounds) for the
//! inspector and the character editor. Decoded once per file version and
//! cached as egui textures. (Asynchronous, size-bounded thumbnails with a
//! shared cache are part of M3e; this is the minimal synchronous version.)

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use egui::{ColorImage, Context, Id, TextureHandle, TextureOptions};
use orbit_core::{AssetId, Story};
use orbit_editor_api::EditorContext;

/// Longest side of a stored thumbnail, in pixels.
const MAX_SIDE: u32 = 192;

type Cache = Arc<Mutex<HashMap<String, Option<TextureHandle>>>>;

fn cache(ctx: &Context) -> Cache {
    let id = Id::new("orbit_thumbnail_cache");
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with::<Cache>(id, Cache::default)
            .clone()
    })
}

fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Folder the manifest's relative paths resolve against, for the open project.
pub fn assets_root(ctx: &EditorContext<'_>, story: &Story) -> Option<std::path::PathBuf> {
    let info = ctx.project_info()?;
    Some(crate::preview_host::find_assets_root(&info.root, story))
}

/// A texture for the image asset, or `None` (not an image, missing file,
/// undecodable).
pub fn thumbnail(
    egui_ctx: &Context,
    root: &Path,
    story: &Story,
    asset: AssetId,
) -> Option<TextureHandle> {
    let entry = story.assets.get(asset)?;
    let path = root.join(&entry.path);
    let key = format!("{}@{:?}", entry.path, stamp(&path));
    let cache = cache(egui_ctx);
    let mut cache = cache.lock().ok()?;
    if let Some(found) = cache.get(&key) {
        return found.clone();
    }
    let texture = std::fs::read(&path).ok().and_then(|bytes| {
        let img = image::load_from_memory(&bytes).ok()?;
        let img = img.thumbnail(MAX_SIDE, MAX_SIDE).into_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        Some(egui_ctx.load_texture(
            format!("thumb:{}", entry.path),
            ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
            TextureOptions::LINEAR,
        ))
    });
    // Old versions of the same file are dropped with the cache entry.
    cache.retain(|k, _| !k.starts_with(&format!("{}@", entry.path)));
    cache.insert(key, texture.clone());
    texture
}

/// Draws a thumbnail fitted in a `max` box, or a neutral placeholder.
pub fn show(
    ui: &mut egui::Ui,
    ctx: &EditorContext<'_>,
    story: &Story,
    asset: AssetId,
    max: egui::Vec2,
) {
    let texture = assets_root(ctx, story).and_then(|root| thumbnail(ui.ctx(), &root, story, asset));
    match texture {
        Some(t) => {
            ui.add(egui::Image::new(&t).fit_to_exact_size(max).corner_radius(6));
        }
        None => {
            let (rect, _) = ui.allocate_exact_size(max, egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(6),
                ui.visuals().widgets.inactive.bg_fill,
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                orbit_editor_api::icons::IMAGE_BROKEN,
                egui::FontId::proportional(18.0),
                ui.visuals().weak_text_color(),
            );
        }
    }
}
