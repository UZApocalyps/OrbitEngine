//! Host side of the 3D editor (`orbit.scene3d`): renders the viewport and
//! the material preview with `orbit_render3d` into render targets shown to
//! egui as textures, on demand.
//!
//! The plugin (a panel, which has no GPU access) leaves a
//! [`Viewport3dRequest`] in the shared [`Viewport3dLink`] whenever the
//! picture is out of date; the app calls [`Scene3dHost::update`] after the UI
//! pass and before egui paints, so the frame already shows the result. No
//! request means no render: nothing is drawn while the scene is idle.

use std::cell::RefCell;
use std::rc::Rc;

use orbit_gfx::Gfx;
use orbit_render3d::{Camera3d, CameraSelect, Renderer3d, RenderTarget3d};
use orbit_scene::{MeshAsset, Prefab, Scene3d, TextureAsset};

/// Slot of the main viewport.
pub const MAIN: usize = 0;
/// Slot of the material preview sphere.
pub const MATERIAL: usize = 1;
const SLOTS: usize = 2;

/// Imported assets the scene refers to.
#[derive(Default)]
pub struct ModelAssets {
    pub prefabs: Vec<Prefab>,
    pub meshes: Vec<MeshAsset>,
    pub textures: Vec<TextureAsset>,
}

pub struct Viewport3dRequest {
    pub scene: Scene3d,
    pub camera: Camera3d,
    /// Target size in physical pixels.
    pub size: [u32; 2],
    /// Identifies `assets`: uploaded again when it differs from the last
    /// upload of this slot.
    pub assets_key: u64,
    pub assets: Rc<ModelAssets>,
}

#[derive(Default)]
pub struct Slot {
    /// Set by the plugin, taken by the host.
    pub request: Option<Viewport3dRequest>,
    /// Published by the host.
    pub texture: Option<egui::TextureId>,
    pub size: [u32; 2],
}

#[derive(Default)]
pub struct Viewport3dLink {
    /// A GPU renderer exists (false in headless tests).
    pub available: bool,
    pub slots: [Slot; SLOTS],
    /// Last render error, if any.
    pub error: Option<String>,
    /// Renders done (tests, diagnostics).
    pub renders: u64,
}

pub type SharedLink = Rc<RefCell<Viewport3dLink>>;

#[derive(Default)]
pub struct Scene3dHost {
    renderer: Option<Renderer3d>,
    targets: [Option<RenderTarget3d>; SLOTS],
    uploaded: [Option<u64>; SLOTS],
}

impl Scene3dHost {
    /// Renders the pending requests. Returns true when a texture id or size
    /// changed, so the UI needs another frame to pick it up.
    pub fn update(
        &mut self,
        link: &SharedLink,
        gfx: &Gfx,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) -> bool {
        let mut link = link.borrow_mut();
        link.available = true;
        let mut changed = false;
        for slot in 0..SLOTS {
            let Some(req) = link.slots[slot].request.take() else {
                continue;
            };
            let device = gfx.device();
            let queue = gfx.queue();
            let renderer = self
                .renderer
                .get_or_insert_with(|| Renderer3d::new(device, queue));
            if self.uploaded[slot] != Some(req.assets_key) {
                for p in &req.assets.prefabs {
                    renderer.add_prefab(p.clone());
                }
                for t in &req.assets.textures {
                    renderer.add_texture(t.clone());
                }
                for m in &req.assets.meshes {
                    renderer.upload_mesh(device, m);
                }
                self.uploaded[slot] = Some(req.assets_key);
            }
            let [w, h] = [req.size[0].clamp(16, 4096), req.size[1].clamp(16, 4096)];
            let resized = self.targets[slot]
                .as_ref()
                .is_none_or(|t| t.size() != (w, h));
            if resized {
                let target = RenderTarget3d::new_viewable(device, w, h);
                let view = target.gamma_view();
                match link.slots[slot].texture {
                    Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(
                        device,
                        &view,
                        egui_wgpu::wgpu::FilterMode::Linear,
                        id,
                    ),
                    None => {
                        link.slots[slot].texture = Some(egui_renderer.register_native_texture(
                            device,
                            &view,
                            egui_wgpu::wgpu::FilterMode::Linear,
                        ));
                    }
                }
                link.slots[slot].size = [w, h];
                self.targets[slot] = Some(target);
                changed = true;
            }
            if let Some(target) = &self.targets[slot] {
                match renderer.render(device, queue, &req.scene, CameraSelect::View(req.camera), target)
                {
                    Ok(()) => {
                        link.error = None;
                        link.renders += 1;
                    }
                    Err(e) => link.error = Some(e.to_string()),
                }
            }
        }
        changed
    }
}

/// Renders `scene` once, seen from the named `camera` (the first one when
/// `None`), into a fresh `size` target and returns it as PNG bytes. Used by
/// the live MCP `capture_scene3d` tool; independent of the viewport and of
/// the host's cached renderer.
pub fn capture_png(
    device: &orbit_gfx::wgpu::Device,
    queue: &orbit_gfx::wgpu::Queue,
    scene: &Scene3d,
    assets: &ModelAssets,
    camera: Option<&str>,
    size: [u32; 2],
) -> Result<Vec<u8>, String> {
    let [w, h] = [size[0].clamp(16, 4096), size[1].clamp(16, 4096)];
    let mut renderer = Renderer3d::new(device, queue);
    for p in &assets.prefabs {
        renderer.add_prefab(p.clone());
    }
    for t in &assets.textures {
        renderer.add_texture(t.clone());
    }
    for m in &assets.meshes {
        renderer.upload_mesh(device, m);
    }
    let target = RenderTarget3d::new(device, w, h);
    let select = match camera {
        Some(name) => CameraSelect::Named(name),
        None => CameraSelect::First,
    };
    renderer
        .render(device, queue, scene, select, &target)
        .map_err(|e| e.to_string())?;
    let rgba = target
        .read_rgba_blocking(device, queue)
        .ok_or_else(|| "cannot read the picture back".to_owned())?;
    let mut png = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut png),
        &rgba,
        w,
        h,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| format!("cannot encode the PNG: {e}"))?;
    Ok(png)
}
