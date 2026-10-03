//! The 3D background layer of the stage (M6-07, ADR 0014 sections 6 and 8).
//!
//! A 3D scene is rendered by `orbit_render3d` into its own offscreen target,
//! copied into a regular 2D render target and drawn as the stage's
//! background sprite. Everything above it (characters, textbox, UI) is the
//! unchanged 2D pipeline, and so are transitions: a 2D <-> 3D fade or
//! dissolve, or a dissolve between two cameras, is the usual blend of the
//! "from" and "to" stages, each with its own 3D view.
//!
//! [`Layer3d`] exists with and without the `3d` feature. Without it (2D
//! builds, the web player of a 2D game) it never produces anything and holds
//! no GPU resource; with it, nothing is created before the first 3D scene is
//! shown. A 2D stage is therefore drawn exactly the same way in both builds.

use orbit_core::Story;
use orbit_render2d::{Renderer2d, TextureId};

use crate::loading::AssetStore;
use crate::stage::StageState;

/// The 3D layer of the runtime: lazily created compositor (feature `3d`).
#[derive(Default)]
pub struct Layer3d {
    #[cfg(feature = "3d")]
    compositor: Option<Compositor3d>,
}

impl Layer3d {
    /// Starts a frame: every view may be (re)used again.
    pub fn begin_frame(&mut self) {
        #[cfg(feature = "3d")]
        if let Some(c) = &mut self.compositor {
            c.begin_frame();
        }
    }

    /// The texture holding the 3D scene of `stage`, rendered if needed.
    /// `None` when the stage shows no 3D scene, when it is not loaded (or
    /// cannot be rendered), and always without the `3d` feature.
    #[cfg_attr(not(feature = "3d"), allow(unused_variables))]
    pub fn draw(
        &mut self,
        renderer: &mut Renderer2d,
        store: &AssetStore,
        story: &Story,
        stage: &StageState,
    ) -> Option<TextureId> {
        let shown = stage.scene3d.as_ref()?;
        #[cfg(feature = "3d")]
        {
            let path = &story.assets.get(shown.scene)?.path;
            let Some(scene) = store.scene3d(path) else {
                log::debug!("3D scene `{path}` is not loaded (yet)");
                return None;
            };
            let compositor = self
                .compositor
                .get_or_insert_with(|| Compositor3d::new(renderer));
            compositor.draw(renderer, path, scene, shown.camera.as_deref())
        }
        #[cfg(not(feature = "3d"))]
        {
            let _ = shown;
            None
        }
    }

    /// The compositor, once a 3D scene was shown (hosts register meshes,
    /// textures and prefabs on its renderer).
    #[cfg(feature = "3d")]
    pub fn compositor_mut(&mut self) -> Option<&mut Compositor3d> {
        self.compositor.as_mut()
    }

    /// Whether GPU resources were created for 3D (never for a 2D game).
    pub fn is_active(&self) -> bool {
        #[cfg(feature = "3d")]
        {
            self.compositor.is_some()
        }
        #[cfg(not(feature = "3d"))]
        false
    }
}

#[cfg(feature = "3d")]
pub use gpu::Compositor3d;

#[cfg(feature = "3d")]
mod gpu {
    use std::collections::HashSet;
    use std::sync::Arc;

    use orbit_render2d::orbit_gfx::wgpu;
    use orbit_render2d::{Renderer2d, TextureId};
    use orbit_render3d::{CameraSelect, QualityProfile, RenderTarget3d, Renderer3d};
    use orbit_scene::Scene3d;

    /// 3D views a frame may need: the two sides of a transition.
    const MAX_VIEWS: usize = 2;

    /// What a view holds: the scene document (by identity, a hot reload
    /// gives a new one) and the camera. Same key = same picture (scenes are
    /// static so far), so it is not rendered again.
    #[derive(Clone, PartialEq, Eq)]
    struct ViewKey {
        scene: usize,
        camera: Option<String>,
    }

    struct View {
        target: RenderTarget3d,
        texture: TextureId,
        key: Option<ViewKey>,
        used: bool,
    }

    /// Renders 3D scenes into 2D-compatible textures.
    pub struct Compositor3d {
        renderer: Renderer3d,
        views: Vec<View>,
        size: (u32, u32),
        warned: HashSet<String>,
    }

    /// The default quality profile of the platform (ADR 0014 section 3).
    fn platform_profile() -> QualityProfile {
        #[cfg(target_arch = "wasm32")]
        {
            QualityProfile::web()
        }
        #[cfg(not(target_arch = "wasm32"))]
        QualityProfile::desktop_low()
    }

    impl Compositor3d {
        /// Creates the 3D renderer on the 2D renderer's device. Views are
        /// sized like the stage's canvas targets.
        pub fn new(r2d: &Renderer2d) -> Self {
            let gfx = r2d.gfx();
            let mut renderer = Renderer3d::new(gfx.device(), gfx.queue());
            renderer.set_quality(platform_profile().adapt_to(gfx.adapter()));
            let [vw, vh] = r2d.camera().virtual_size;
            let max = gfx.max_texture_dimension() as f32;
            let k = (max / vw.max(vh)).min(1.0);
            let size = (
                ((vw * k).round() as u32).max(1),
                ((vh * k).round() as u32).max(1),
            );
            log::info!("3D layer: {}x{} views", size.0, size.1);
            Self {
                renderer,
                views: Vec::new(),
                size,
                warned: HashSet::new(),
            }
        }

        /// The 3D renderer (meshes, textures, prefabs, quality). Cached
        /// views are dropped: what they show may change.
        pub fn renderer_mut(&mut self) -> &mut Renderer3d {
            for view in &mut self.views {
                view.key = None;
            }
            &mut self.renderer
        }

        pub fn begin_frame(&mut self) {
            for view in &mut self.views {
                view.used = false;
            }
        }

        /// Draws `scene` from `camera` (`None`: its first camera) into a
        /// free view and returns its texture.
        pub fn draw(
            &mut self,
            r2d: &mut Renderer2d,
            path: &str,
            scene: &Arc<Scene3d>,
            camera: Option<&str>,
        ) -> Option<TextureId> {
            let key = ViewKey {
                scene: Arc::as_ptr(scene) as usize,
                camera: camera.map(str::to_owned),
            };
            if let Some(view) = self
                .views
                .iter_mut()
                .find(|v| !v.used && v.key.as_ref() == Some(&key))
            {
                view.used = true;
                return Some(view.texture);
            }
            let index = match self.views.iter().position(|v| !v.used) {
                Some(i) => i,
                None if self.views.len() < MAX_VIEWS => {
                    let texture =
                        match r2d.create_render_target("stage 3d", self.size.0, self.size.1) {
                            Ok(t) => t,
                            Err(err) => {
                                self.warn_once(format!("3D layer target: {err}"));
                                return None;
                            }
                        };
                    let target = RenderTarget3d::new(r2d.gfx().device(), self.size.0, self.size.1);
                    self.views.push(View {
                        target,
                        texture,
                        key: None,
                        used: false,
                    });
                    self.views.len() - 1
                }
                None => return None,
            };
            let gfx = r2d.gfx();
            let view = &mut self.views[index];
            view.used = true;
            let select = camera.map_or(CameraSelect::First, CameraSelect::Named);
            if let Err(err) =
                self.renderer
                    .render(gfx.device(), gfx.queue(), scene, select, &view.target)
            {
                view.key = None;
                let message = format!("3D scene `{path}`: {err}");
                if self.warned.insert(message.clone()) {
                    log::warn!("{message}");
                }
                return None;
            }
            let dst = r2d.raw_texture(view.texture)?;
            let (w, h) = view.target.size();
            let mut encoder =
                gfx.device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("3d layer copy"),
                    });
            encoder.copy_texture_to_texture(
                view.target.texture().as_image_copy(),
                dst.as_image_copy(),
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            gfx.queue().submit(Some(encoder.finish()));
            view.key = Some(key);
            Some(view.texture)
        }

        fn warn_once(&mut self, message: String) {
            if self.warned.insert(message.clone()) {
                log::warn!("{message}");
            }
        }
    }
}

/// GPU read-back tests of the 3D layer composited under the 2D stage. Each
/// one is skipped (with a message) when no adapter is available. Run them
/// with and without the `3d` feature: the 2D stage must come out the same.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use orbit_core::{AssetEntry, AssetId, AssetKind, Character, CharacterId, Color, Expression};
    use orbit_render2d::orbit_gfx::Gfx;
    use orbit_render2d::{ReadbackStatus, Renderer2d, Scene, TextureId, TextureOptions};

    use super::*;
    use crate::runtime::stage_scene;
    use crate::stage::StageCharacter;

    const BG: &str = "assets/bg.png";
    const SPRITE: &str = "assets/hero.png";
    const SCENE3D: &str = "assets/3d/room.scene3d.ron";

    fn renderer() -> Option<Renderer2d> {
        let gfx = pollster::block_on(Gfx::headless((64, 64))).ok()?;
        Renderer2d::new(gfx).ok()
    }

    macro_rules! gpu_or_skip {
        () => {
            match renderer() {
                Some(r) => r,
                None => {
                    eprintln!("no GPU adapter: skipping 3D layer read-back test");
                    return;
                }
            }
        };
    }

    struct Fixture {
        story: Story,
        store: AssetStore,
        hero: CharacterId,
        bg: AssetId,
        image: AssetId,
        scene3d: AssetId,
    }

    fn solid(r: &mut Renderer2d, name: &str, rgba: [u8; 4]) -> TextureId {
        let pixels: Vec<u8> = rgba.iter().copied().cycle().take(4 * 4 * 4).collect();
        r.create_texture(name, 4, 4, &pixels, TextureOptions::COLOR)
            .unwrap()
    }

    /// A story with a green 2D background, a red character and a 3D scene;
    /// the store holds their textures (and the scene with the `3d` feature).
    fn fixture(r: &mut Renderer2d) -> Fixture {
        let mut story = Story::new("m6-07");
        let bg = AssetId::new();
        let image = AssetId::new();
        let scene3d = AssetId::new();
        for (id, kind, path) in [
            (bg, AssetKind::Image, BG),
            (image, AssetKind::Image, SPRITE),
            (scene3d, AssetKind::Scene3d, SCENE3D),
        ] {
            story.assets.assets.push(AssetEntry {
                id,
                kind,
                path: path.into(),
            });
        }
        let hero = CharacterId::new();
        let name_id = story.strings.add("Hero");
        story.characters.push(Character {
            id: hero,
            name_id,
            color: Color::rgb(255, 255, 255),
            expressions: vec![Expression {
                name: "neutral".into(),
                image,
                motion: None,
            }],
            default_position: None,
            default_scale: 1.0,
        });
        let mut store = AssetStore::default();
        let green = solid(r, BG, [0, 255, 0, 255]);
        store.insert_texture(BG, green);
        let red = solid(r, SPRITE, [255, 0, 0, 255]);
        store.insert_texture(SPRITE, red);
        #[cfg(feature = "3d")]
        store.insert_scene3d(SCENE3D, blue_room());
        Fixture {
            story,
            store,
            hero,
            bg,
            image,
            scene3d,
        }
    }

    /// A 3D scene that is a plain blue background seen from two cameras.
    #[cfg(feature = "3d")]
    fn blue_room() -> orbit_scene::Scene3d {
        use orbit_scene::{Background, CameraDef, Node3d, Node3dKind, Projection};
        let mut scene = orbit_scene::Scene3d::new("room");
        scene.environment.background = Background::Color([0.0, 0.0, 1.0]);
        for (name, z) in [("front", 5.0), ("back", -5.0)] {
            let mut cam = Node3d::new(
                name,
                None,
                Node3dKind::Camera(CameraDef {
                    name: name.into(),
                    projection: Projection::Perspective { fov_y: 0.8 },
                    near: 0.1,
                    far: 50.0,
                }),
            );
            cam.transform.translation = [0.0, 0.0, z];
            if z < 0.0 {
                // Turned around (180 degrees about Y).
                cam.transform.rotation = [0.0, 1.0, 0.0, 0.0];
            }
            scene.nodes.push(cam);
        }
        scene
    }

    fn stage_2d(f: &Fixture) -> StageState {
        StageState {
            background: Some(f.bg),
            characters: vec![StageCharacter {
                character: f.hero,
                expression: "neutral".into(),
                image: f.image,
                x: 0.5,
                y: 1.0,
                scale: [1.0, 1.0],
                flip_x: false,
                flip_y: false,
            }],
            scene3d: None,
        }
    }

    fn stage_3d(f: &Fixture, camera: Option<&str>) -> StageState {
        let mut stage = stage_2d(f);
        stage.scene3d = Some(crate::stage::Stage3d {
            scene: f.scene3d,
            camera: camera.map(str::to_owned),
        });
        stage
    }

    fn draw_into(
        r: &mut Renderer2d,
        layer: &mut Layer3d,
        f: &Fixture,
        stage: &StageState,
        target: TextureId,
    ) {
        let tex = layer.draw(r, &f.store, &f.story, stage);
        let mut scene = Scene::new();
        stage_scene(r, &f.store, &f.story, stage, None, tex, &mut scene);
        r.render_to_target(target, &scene).unwrap();
    }

    /// Draws `stage` (with the 3D layer) into a canvas target, reads it back.
    fn render(r: &mut Renderer2d, layer: &mut Layer3d, f: &Fixture, stage: &StageState) -> Vec<u8> {
        let target = r.create_canvas_target("test stage").unwrap();
        draw_into(r, layer, f, stage, target);
        read(r, target)
    }

    fn read(r: &Renderer2d, target: TextureId) -> Vec<u8> {
        let mut readback = r.start_readback(target).unwrap();
        for _ in 0..10_000 {
            if let ReadbackStatus::Ready { rgba, .. } = r.poll_readback(&mut readback) {
                return rgba;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("read-back timed out");
    }

    fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
        let i = (y * 1920 + x) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    fn fnv(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    #[test]
    fn a_2d_stage_is_pixel_identical_with_and_without_the_3d_layer() {
        let mut r = gpu_or_skip!();
        let f = fixture(&mut r);
        let stage = stage_2d(&f);
        // Reference: the 2D stage drawn without any 3D layer.
        let target = r.create_canvas_target("reference").unwrap();
        let mut scene = Scene::new();
        stage_scene(&r, &f.store, &f.story, &stage, None, None, &mut scene);
        r.render_to_target(target, &scene).unwrap();
        let reference = read(&r, target);
        assert_eq!(pixel(&reference, 100, 100), [0, 255, 0, 255]);
        assert_eq!(pixel(&reference, 960, 600), [255, 0, 0, 255]);

        let mut layer = Layer3d::default();
        layer.begin_frame();
        assert_eq!(render(&mut r, &mut layer, &f, &stage), reference);
        assert!(!layer.is_active(), "a 2D stage creates no 3D resource");

        // After a 3D scene was drawn (feature `3d`), a 2D stage is unchanged.
        layer.begin_frame();
        let _ = render(&mut r, &mut layer, &f, &stage_3d(&f, None));
        layer.begin_frame();
        assert_eq!(render(&mut r, &mut layer, &f, &stage), reference);

        // Same picture across builds: each build leaves its hash, the other
        // one compares (same adapter only).
        let info = r.gfx().adapter().get_info();
        let mine = format!("{}|{:?}|{:016x}", info.name, info.backend, fnv(&reference));
        let dir = std::env::temp_dir();
        let (feature, other) = if cfg!(feature = "3d") {
            ("3d", "2d")
        } else {
            ("2d", "3d")
        };
        let _ = std::fs::write(dir.join(format!("orbit_m6_07_stage.{feature}.txt")), &mine);
        if let Ok(theirs) =
            std::fs::read_to_string(dir.join(format!("orbit_m6_07_stage.{other}.txt")))
        {
            let adapter = |s: &str| s.rsplit_once('|').map(|(a, _)| a.to_owned());
            if adapter(&theirs) == adapter(&mine) {
                assert_eq!(
                    theirs, mine,
                    "the 2D stage differs between the 2d and 3d builds"
                );
            }
        }
    }

    #[cfg(not(feature = "3d"))]
    #[test]
    fn a_3d_scene_without_the_feature_leaves_the_background_black() {
        let mut r = gpu_or_skip!();
        let f = fixture(&mut r);
        let mut layer = Layer3d::default();
        layer.begin_frame();
        let out = render(&mut r, &mut layer, &f, &stage_3d(&f, None));
        assert_eq!(pixel(&out, 100, 100), [0, 0, 0, 255]);
        // The character is still drawn on top.
        assert_eq!(pixel(&out, 960, 600), [255, 0, 0, 255]);
        assert!(!layer.is_active());
    }

    #[cfg(feature = "3d")]
    fn blueish(p: [u8; 4]) -> bool {
        p[2] > 128 && p[0] < 64 && p[1] < 64
    }

    #[cfg(feature = "3d")]
    #[test]
    fn the_3d_scene_replaces_the_background_under_the_sprites() {
        let mut r = gpu_or_skip!();
        let f = fixture(&mut r);
        let mut layer = Layer3d::default();
        layer.begin_frame();
        let out = render(&mut r, &mut layer, &f, &stage_3d(&f, Some("front")));
        assert!(layer.is_active());
        assert!(
            blueish(pixel(&out, 100, 100)),
            "{:?}",
            pixel(&out, 100, 100)
        );
        assert_eq!(pixel(&out, 960, 600), [255, 0, 0, 255], "sprite on top");
        // An unknown camera draws nothing (black), it never panics.
        layer.begin_frame();
        let out = render(&mut r, &mut layer, &f, &stage_3d(&f, Some("nope")));
        assert_eq!(pixel(&out, 100, 100), [0, 0, 0, 255]);
    }

    #[cfg(feature = "3d")]
    #[test]
    fn a_dissolve_blends_the_2d_background_into_the_3d_scene() {
        let mut r = gpu_or_skip!();
        let f = fixture(&mut r);
        let mut layer = Layer3d::default();
        layer.begin_frame();
        let from = r.create_canvas_target("from").unwrap();
        let to = r.create_canvas_target("to").unwrap();
        let out = r.create_canvas_target("out").unwrap();
        draw_into(&mut r, &mut layer, &f, &stage_2d(&f), from);
        draw_into(&mut r, &mut layer, &f, &stage_3d(&f, None), to);
        r.render_to_target(out, &Scene::new()).unwrap();
        r.draw_transition_to_target(out, from, to, &orbit_render2d::Transition::Dissolve, 0.5)
            .unwrap();
        let rgba = read(&r, out);
        let mixed = pixel(&rgba, 100, 100);
        assert!(mixed[1] > 64 && mixed[2] > 64 && mixed[0] < 64, "{mixed:?}");
        // The character is on both sides: unchanged by the blend.
        assert_eq!(pixel(&rgba, 960, 600), [255, 0, 0, 255]);
    }

    #[cfg(feature = "3d")]
    #[test]
    fn two_cameras_get_two_views_in_one_frame_and_views_are_cached() {
        let mut r = gpu_or_skip!();
        let f = fixture(&mut r);
        let mut layer = Layer3d::default();
        layer.begin_frame();
        let a = layer.draw(&mut r, &f.store, &f.story, &stage_3d(&f, Some("front")));
        let b = layer.draw(&mut r, &f.store, &f.story, &stage_3d(&f, Some("back")));
        assert!(a.is_some() && b.is_some());
        assert_ne!(a, b, "a camera transition needs both views");
        // Next frame, same cameras: the same (cached) textures.
        layer.begin_frame();
        let a2 = layer.draw(&mut r, &f.store, &f.story, &stage_3d(&f, Some("front")));
        let b2 = layer.draw(&mut r, &f.store, &f.story, &stage_3d(&f, Some("back")));
        assert_eq!((a2, b2), (a, b));
    }
}
