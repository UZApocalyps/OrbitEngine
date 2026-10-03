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
}

pub type SharedLink = Rc<RefCell<RigLink>>;

#[derive(Default)]
pub struct RigHost {
    target: Option<(TextureId, [u32; 2])>,
    /// Loaded pictures by project path (`None`: could not be loaded).
    images: HashMap<(PathBuf, String), Option<TextureId>>,
}

impl RigHost {
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
        let Some(req) = link.request.take() else {
            return false;
        };
        let [w, h] = [req.size[0].clamp(16, 4096), req.size[1].clamp(16, 4096)];
        let mut changed = false;
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

        // Load every picture the rig can show (any skin).
        let mut missing = Vec::new();
        for skin in &req.rig.rig().skins {
            for att in skin.attachments.values() {
                let name = att.image();
                let key = (req.root.clone(), name.to_owned());
                if !self.images.contains_key(&key) {
                    let tex = load_image(renderer, &req.root, name);
                    self.images.insert(key.clone(), tex);
                }
                if self.images[&key].is_none() && !missing.iter().any(|m: &String| m == name) {
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
        changed
    }
}

fn load_image(renderer: &mut Renderer2d, root: &std::path::Path, name: &str) -> Option<TextureId> {
    let path = root.join(name);
    let bytes = std::fs::read(&path).ok()?;
    let img = orbit_assets::decode_image(&bytes).ok()?;
    renderer
        .create_texture(&format!("rig:{}", path.display()), img.width, img.height, &img.rgba, TextureOptions::COLOR)
        .ok()
}
