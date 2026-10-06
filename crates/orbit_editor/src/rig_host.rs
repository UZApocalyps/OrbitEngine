//! Host side of the rig editor (`orbit.rig`): draws the posed rig with
//! `orbit_render2d` (the same `push_rig` the game uses) into a render target
//! shown to egui as a texture, on demand.
//!
//! The plugin (a panel, which has no GPU access) leaves a [`RigRequest`] in
//! the shared [`RigLink`] whenever the picture is out of date; the app calls
//! [`RigHost::update`] after the UI pass and before egui paints. No request,
//! no render. The renderer is the preview's: its camera is borrowed for the
//! draw and restored.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use orbit_anim::rig::{CpuSkinner, PreparedRig, RigPose};
use orbit_gfx::wgpu;
use orbit_render2d::{Camera2d, Renderer2d, RigPlacement, Scene, TextureId, TextureOptions, push_rig};

/// Linear RGBA of the viewport background.
const BACKGROUND: [f32; 4] = [0.012, 0.014, 0.018, 1.0];

pub struct RigRequest {
    pub rig: Rc<PreparedRig>,
    pub pose: RigPose,
    pub skin: Option<String>,
    /// Target size in physical pixels.
    pub size: [u32; 2],
    /// Physical pixels per rig unit, and the offset of the rig origin from the
    /// target's center, in physical pixels.
    pub zoom: f32,
    pub pan: [f32; 2],
    /// Project root the image references are relative to.
    pub root: PathBuf,
    /// Pictures edited in memory (paint, cut) whose pixels changed since the
    /// last request: they replace the file's texture.
    pub pixels: Vec<PixelUpload>,
    /// Pictures whose edited pixels were dropped (another rig opened): their
    /// texture is loaded from the file again.
    pub forget: Vec<String>,
}

/// The pixels of a picture held by the rig editor.
pub struct PixelUpload {
    /// Project path, as the rig names it.
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Default)]
pub struct RigLink {
    /// A GPU renderer exists (false in headless tests).
    pub available: bool,
    /// Set by the plugin, taken by the host.
    pub request: Option<RigRequest>,
    /// Published by the host.
    pub texture: Option<egui::TextureId>,
    pub size: [u32; 2],
    pub error: Option<String>,
    /// Images the last render could not load (project paths).
    pub missing: Vec<String>,
    pub renders: u64,
    /// Bumped when pictures finished loading in the background: the picture
    /// is out of date, the plugin asks again.
    pub images_loaded: u64,
}

pub type SharedLink = Rc<RefCell<RigLink>>;

#[derive(Default)]
pub struct RigHost {
    target: Option<(TextureId, [u32; 2])>,
    /// Loaded pictures by project path (`None`: could not be loaded).
    images: HashMap<(PathBuf, String), Option<TextureId>>,
    /// Textures made for edited pixels (not shared by name: released when
    /// replaced).
    edited: std::collections::HashSet<TextureId>,
    loader: ImageLoader,
    /// Pictures being read again after their edits were dropped.
    reload: std::collections::HashSet<(PathBuf, String)>,
    /// Last look for pictures that could not be loaded (their files may
    /// appear later): at most once a second, not a `stat` per frame.
    last_retry: Option<std::time::Instant>,
}

impl RigHost {
    /// Uploads edited pixels into a texture of their own (reused while the
    /// size holds).
    fn upload(&mut self, renderer: &mut Renderer2d, root: &std::path::Path, up: &PixelUpload) {
        let key = (root.to_owned(), up.name.clone());
        let current = self.images.get(&key).copied().flatten();
        if up.rgba.len() != up.width as usize * up.height as usize * 4 || up.width == 0 || up.height == 0 {
            return;
        }
        // Only a texture of our own is written over: the file's texture is
        // shared by name (thumbnails) and must keep the file's pixels.
        if let Some(id) = current
            && self.edited.contains(&id)
            && renderer.texture_size(id) == Some([up.width, up.height])
            && renderer.update_texture(id, 0, 0, up.width, up.height, &up.rgba).is_ok()
        {
            return;
        }
        let label = format!("rig edit:{}", up.name);
        if let Ok(id) = renderer.create_empty_texture(&label, up.width, up.height, TextureOptions::COLOR)
            && renderer.update_texture(id, 0, 0, up.width, up.height, &up.rgba).is_ok()
        {
            if let Some(old) = current
                && self.edited.remove(&old)
            {
                renderer.release_texture(old);
            }
            self.edited.insert(id);
            self.images.insert(key, Some(id));
        }
    }

    /// Renders the pending request. Returns true when the texture id or size
    /// changed, so the UI needs another frame to pick it up.
    pub fn update(
        &mut self,
        link: &SharedLink,
        renderer: &mut Renderer2d,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) -> bool {
        let mut link = link.borrow_mut();
        link.available = true;
        let mut changed = false;
        let loaded = self.loader.take();
        if !loaded.is_empty() {
            for (key, img) in loaded {
                if self.reload.remove(&key) {
                    // Dropped edits: the file's current pixels in a texture
                    // of our own (the one shared by name may hold what the
                    // file had before a save).
                    match img {
                        Some(img) => {
                            let up = PixelUpload { name: key.1.clone(), width: img.width, height: img.height, rgba: img.rgba };
                            self.upload(renderer, &key.0, &up);
                        }
                        None => {
                            if let Some(Some(id)) = self.images.remove(&key)
                                && self.edited.remove(&id)
                            {
                                renderer.release_texture(id);
                            }
                        }
                    }
                    continue;
                }
                let tex = upload_image(renderer, &key, img);
                if let Some(Some(old)) = self.images.insert(key, tex)
                    && self.edited.remove(&old)
                {
                    renderer.release_texture(old);
                }
            }
            link.images_loaded += 1;
            changed = true;
        }
        let Some(req) = link.request.take() else {
            return changed || self.loader.busy();
        };
        let [w, h] = [req.size[0].clamp(16, 4096), req.size[1].clamp(16, 4096)];
        if self.target.is_none_or(|(_, s)| s != [w, h]) {
            if let Some((old, _)) = self.target.take() {
                renderer.release_texture(old);
            }
            let id = match renderer.create_render_target("rig editor", w, h) {
                Ok(id) => id,
                Err(e) => {
                    link.error = Some(e.to_string());
                    return false;
                }
            };
            let Some(view) = renderer.target_gamma_view(id) else {
                link.error = Some("no gamma view".to_owned());
                return false;
            };
            let device = renderer.gfx().device().clone();
            match link.texture {
                Some(t) => egui_renderer.update_egui_texture_from_wgpu_texture(&device, &view, wgpu::FilterMode::Linear, t),
                None => {
                    link.texture =
                        Some(egui_renderer.register_native_texture(&device, &view, wgpu::FilterMode::Linear));
                }
            }
            link.size = [w, h];
            self.target = Some((id, [w, h]));
            changed = true;
        }
        let Some((target, _)) = self.target else {
            return changed;
        };

        // Dropped edits: the file's current pixels again (read now: the
        // texture shared by name may hold what the file had before a save).
        // The edited texture stays on screen until the file's pixels are
        // decoded (in the background) and replace it.
        for name in &req.forget {
            let key = (req.root.clone(), name.clone());
            self.loader.request(&key);
            self.reload.insert(key);
        }
        for up in &req.pixels {
            self.upload(renderer, &req.root, up);
        }
        // Load every picture the rig can show (any skin), in the background.
        let retry = self
            .last_retry
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1));
        if retry {
            self.last_retry = Some(std::time::Instant::now());
        }
        let mut missing = Vec::new();
        for skin in &req.rig.rig().skins {
            for att in skin.attachments.values() {
                let name = att.image();
                let key = (req.root.clone(), name.to_owned());
                // A picture that could not be loaded is retried once its
                // file exists (imported or copied after the first draw).
                if retry && self.images.get(&key).is_some_and(Option::is_none) && req.root.join(name).is_file() {
                    self.images.remove(&key);
                }
                if !self.images.contains_key(&key) {
                    self.loader.request(&key);
                } else if self.images[&key].is_none()
                    && !self.loader.is_loading(&key)
                    && !missing.iter().any(|m: &String| m == name)
                {
                    missing.push(name.to_owned());
                }
            }
        }
        link.missing = missing;

        let world = req.rig.world(&req.pose);
        let list = req.rig.draw_list(&req.pose, &world, req.skin.as_deref(), &CpuSkinner);
        let mut scene = Scene::new();
        scene.clear = Some(BACKGROUND);
        let placement = RigPlacement {
            position: [w as f32 * 0.5 + req.pan[0], h as f32 * 0.5 + req.pan[1]],
            scale: req.zoom,
            ..RigPlacement::default()
        };
        let root = req.root.clone();
        push_rig(&mut scene, &list, &placement, |name| {
            self.images.get(&(root.clone(), name.to_owned())).copied().flatten()
        });
        let saved = *renderer.camera();
        *renderer.camera_mut() = Camera2d::new(w as f32, h as f32);
        let result = renderer.render_to_target(target, &scene);
        *renderer.camera_mut() = saved;
        match result {
            Ok(()) => {
                link.error = None;
                link.renders += 1;
            }
            Err(e) => link.error = Some(e.to_string()),
        }
        changed || self.loader.busy()
    }
}

// ------------------------------------------------------------- thumbnails

/// Size, in physical pixels, of the animated thumbnail of a rigged expression
/// (the characters panel draws it at half this size in points).
pub const THUMB_PX: [u32; 2] = [108, 136];
/// Thumbnails kept at once; past this the unused ones are released.
const MAX_THUMBS: usize = 48;

/// Which expression a thumbnail is of.
pub type ThumbKey = (orbit_core::CharacterId, usize);

/// One thumbnail to draw this frame: the clip's pose, framed on the rig's
/// bounding box.
pub struct ThumbRequest {
    pub key: ThumbKey,
    pub rig: Rc<PreparedRig>,
    pub pose: RigPose,
    pub skin: Option<String>,
    pub root: PathBuf,
}

/// Link with the host for the thumbnails: the panel leaves requests, the host
/// publishes one egui texture per key.
#[derive(Default)]
pub struct ThumbLink {
    /// A GPU renderer exists (false in headless tests).
    pub available: bool,
    pub requests: Vec<ThumbRequest>,
    pub textures: HashMap<ThumbKey, egui::TextureId>,
}

pub type SharedThumbs = Rc<RefCell<ThumbLink>>;

/// Bounding box (rig space) of what a draw list shows.
pub fn draw_bounds(list: &orbit_anim::rig::RigDrawList<'_>) -> Option<([f32; 2], [f32; 2])> {
    use orbit_anim::rig::RigDrawItem;
    let mut out: Option<([f32; 2], [f32; 2])> = None;
    let mut add = |p: [f32; 2]| {
        out = Some(match out {
            None => (p, p),
            Some((lo, hi)) => ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]),
        });
    };
    for item in &list.items {
        match item {
            RigDrawItem::Region { size, transform, .. } => {
                let (w, h) = (size[0] * 0.5, size[1] * 0.5);
                for c in [[-w, -h], [w, -h], [-w, h], [w, h]] {
                    add(transform.apply(c));
                }
            }
            RigDrawItem::Mesh { range, .. } => {
                for p in list.positions.get(range.clone()).unwrap_or(&[]) {
                    add(*p);
                }
            }
        }
    }
    out.filter(|(lo, hi)| lo.iter().chain(hi.iter()).all(|v| v.is_finite()))
}

/// Scale and target position of the rig origin that fit `bounds` in a target
/// of `size` pixels with a margin. A degenerate box gives scale 1 centered on
/// the origin.
pub fn fit_bounds(bounds: Option<([f32; 2], [f32; 2])>, size: [u32; 2]) -> (f32, [f32; 2]) {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let Some((lo, hi)) = bounds else {
        return (1.0, [w * 0.5, h * 0.5]);
    };
    let (bw, bh) = (hi[0] - lo[0], hi[1] - lo[1]);
    if bw <= 0.0 && bh <= 0.0 {
        return (1.0, [w * 0.5, h * 0.5]);
    }
    let zoom = (w * 0.92 / bw.max(1e-3)).min(h * 0.92 / bh.max(1e-3));
    let center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
    (zoom, [w * 0.5 - center[0] * zoom, h * 0.5 - center[1] * zoom])
}

#[derive(Default)]
pub struct ThumbHost {
    targets: HashMap<ThumbKey, TextureId>,
    registered: HashMap<ThumbKey, egui::TextureId>,
    images: HashMap<(PathBuf, String), Option<TextureId>>,
    loader: ImageLoader,
}

impl ThumbHost {
    /// Draws the pending thumbnails. Returns true when a texture was created
    /// (the UI needs another frame to pick it up).
    pub fn update(
        &mut self,
        link: &Rc<RefCell<ThumbLink>>,
        renderer: &mut Renderer2d,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) -> bool {
        let mut link = link.borrow_mut();
        link.available = true;
        let requests = std::mem::take(&mut link.requests);
        let mut changed = false;
        for (key, img) in self.loader.take() {
            let tex = upload_image(renderer, &key, img);
            self.images.insert(key, tex);
            changed = true;
        }
        if requests.is_empty() {
            return changed;
        }
        let [w, h] = THUMB_PX;
        if self.targets.len() >= MAX_THUMBS {
            for (_, id) in self.targets.drain() {
                renderer.release_texture(id);
            }
            for (_, tex) in self.registered.drain() {
                egui_renderer.free_texture(&tex);
            }
            link.textures.clear();
        }
        for req in requests {
            if !self.targets.contains_key(&req.key) {
                let Ok(id) = renderer.create_render_target("rig thumbnail", w, h) else { continue };
                let Some(view) = renderer.target_gamma_view(id) else { continue };
                let device = renderer.gfx().device().clone();
                let tex = egui_renderer.register_native_texture(&device, &view, wgpu::FilterMode::Linear);
                self.targets.insert(req.key, id);
                self.registered.insert(req.key, tex);
                link.textures.insert(req.key, tex);
                changed = true;
            }
            let target = self.targets[&req.key];
            for skin in &req.rig.rig().skins {
                for att in skin.attachments.values() {
                    let key = (req.root.clone(), att.image().to_owned());
                    if !self.images.contains_key(&key) {
                        self.loader.request(&key);
                    }
                }
            }
            let (zoom, position) = {
                let setup = req.rig.setup_pose();
                let world = req.rig.world(&setup);
                let list = req.rig.draw_list(&setup, &world, req.skin.as_deref(), &CpuSkinner);
                fit_bounds(draw_bounds(&list), THUMB_PX)
            };
            let world = req.rig.world(&req.pose);
            let list = req.rig.draw_list(&req.pose, &world, req.skin.as_deref(), &CpuSkinner);
            let mut scene = Scene::new();
            scene.clear = Some([0.0, 0.0, 0.0, 0.0]);
            let placement = RigPlacement { position, scale: zoom, ..RigPlacement::default() };
            let root = req.root.clone();
            push_rig(&mut scene, &list, &placement, |name| {
                self.images.get(&(root.clone(), name.to_owned())).copied().flatten()
            });
            let saved = *renderer.camera();
            *renderer.camera_mut() = Camera2d::new(w as f32, h as f32);
            let _ = renderer.render_to_target(target, &scene);
            *renderer.camera_mut() = saved;
        }
        changed || self.loader.busy()
    }
}

/// A picture key: project root and project path.
type ImageKey = (PathBuf, String);

/// Decodes rig pictures on a worker thread: reading and decoding a 2048 px
/// PNG takes from tens of milliseconds (release) to over a second (debug),
/// which froze the first frame of the rig editor and of the characters
/// panel. Until a picture arrives the rig is drawn without it.
#[derive(Default)]
pub struct ImageLoader {
    tx: Option<std::sync::mpsc::Sender<ImageKey>>,
    rx: Option<std::sync::mpsc::Receiver<(ImageKey, Option<orbit_assets::ImageData>)>>,
    in_flight: std::collections::HashSet<ImageKey>,
}

impl ImageLoader {
    /// Queues `name` (relative to `root`) unless it is already on its way.
    pub fn request(&mut self, key: &ImageKey) {
        if self.in_flight.contains(key) {
            return;
        }
        if self.tx.is_none() {
            let (req_tx, req_rx) = std::sync::mpsc::channel::<ImageKey>();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("orbit-rig-images".into())
                .spawn(move || {
                    while let Ok(key) = req_rx.recv() {
                        let image = {
                            orbit_profile::scope!("rig picture decode");
                            std::fs::read(key.0.join(&key.1))
                                .ok()
                                .and_then(|b| orbit_assets::decode_image(&b).ok())
                        };
                        if done_tx.send((key, image)).is_err() {
                            break;
                        }
                    }
                });
            if spawned.is_err() {
                return;
            }
            self.tx = Some(req_tx);
            self.rx = Some(done_rx);
        }
        if let Some(tx) = &self.tx
            && tx.send(key.clone()).is_ok()
        {
            self.in_flight.insert(key.clone());
        }
    }

    /// Pictures decoded since the last call (`None`: unreadable).
    pub fn take(&mut self) -> Vec<(ImageKey, Option<orbit_assets::ImageData>)> {
        let Some(rx) = &self.rx else {
            return Vec::new();
        };
        let done: Vec<_> = rx.try_iter().collect();
        for (key, _) in &done {
            self.in_flight.remove(key);
        }
        done
    }

    pub fn is_loading(&self, key: &ImageKey) -> bool {
        self.in_flight.contains(key)
    }

    pub fn busy(&self) -> bool {
        !self.in_flight.is_empty()
    }
}

/// Uploads a decoded picture (the GPU copy is cheap next to the decode).
fn upload_image(renderer: &mut Renderer2d, key: &ImageKey, img: Option<orbit_assets::ImageData>) -> Option<TextureId> {
    let img = img?;
    orbit_profile::scope!("rig picture upload");
    renderer
        .create_texture(
            &format!("rig:{}", key.0.join(&key.1).display()),
            img.width,
            img.height,
            &img.rgba,
            TextureOptions::COLOR,
        )
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumbnail_frames_the_whole_rig() {
        let (zoom, pos) = fit_bounds(Some(([-50.0, -200.0], [50.0, 0.0])), [100, 100]);
        // Height is the limit: 200 units in 92 px.
        assert!((zoom - 0.46).abs() < 1e-3, "{zoom}");
        // The middle of the box lands in the middle of the target.
        assert!((pos[0] - 50.0).abs() < 1e-3);
        assert!((pos[1] - (50.0 + 100.0 * zoom)).abs() < 1e-3);
    }

    #[test]
    fn an_empty_or_degenerate_box_does_not_blow_up() {
        assert_eq!(fit_bounds(None, [100, 100]), (1.0, [50.0, 50.0]));
        assert_eq!(fit_bounds(Some(([3.0, 3.0], [3.0, 3.0])), [100, 100]), (1.0, [50.0, 50.0]));
        let (zoom, _) = fit_bounds(Some(([0.0, 0.0], [0.0, 10.0])), [100, 100]);
        assert!(zoom.is_finite() && zoom > 0.0);
    }

    #[test]
    fn the_draw_bounds_cover_regions() {
        use orbit_anim::rig::{PreparedRig, load_rig};
        let rig = load_rig(
            r#"(rig_version: 1, bones: [(id: "root", name: "root")],
            slots: [(id: "s", name: "body", bone: "root", attachment: Some("b"))],
            skins: [(name: "default", attachments: {"b": Region((image: "x.png", size: (10.0, 20.0)))})],
            clips: {"idle": (duration: 1.0, looping: Forever, tracks: [])})"#,
        )
        .unwrap()
        .rig;
        let prepared = PreparedRig::new(rig).unwrap();
        let pose = prepared.setup_pose();
        let world = prepared.world(&pose);
        let list = prepared.draw_list(&pose, &world, None, &CpuSkinner);
        let (lo, hi) = draw_bounds(&list).unwrap();
        assert!((hi[0] - lo[0] - 10.0).abs() < 1e-3 && (hi[1] - lo[1] - 20.0).abs() < 1e-3, "{lo:?} {hi:?}");
    }
}
