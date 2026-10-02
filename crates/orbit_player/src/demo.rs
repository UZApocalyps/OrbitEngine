//! M1 presentation demo: background + two characters, a textbox with a
//! typewriter, transitions on click (mask dissolve, crossfade, fade through
//! black, in rotation) and music that starts after the first click.
//!
//! Native: `orbit_player --demo`. Web: `index.html?demo`.

use std::collections::HashMap;
use std::time::Duration;

use orbit_assets::{AssetHandle, AssetLoader, AssetPoll, ImageData, PcmAudio};
use orbit_audio::{AudioEngine, MusicOptions, Sound};
use orbit_gfx::Gfx;
use orbit_platform::{App, AppConfig, InputEvent, KeyCode, MouseButton, PhysicalSize};
use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId, TextureOptions, Transition};
use orbit_text::{DrawParams, TextBlock, TextEngine, TextRenderer, TextStyle};

use crate::PlayerError;

/// Duration of a transition in seconds.
const TRANSITION_SECS: f32 = 1.4;
const CHARS_PER_SECOND: f32 = 38.0;

const IMAGES: &[&str] = &[
    "demo/bg_room.png",
    "demo/bg_street.png",
    "demo/alice_sad.png",
    "demo/alice_happy.png",
    "demo/bob_sad.png",
    "demo/bob_mad.png",
    "demo/mask_wipe.png",
];
const FONTS: &[&str] = &[
    "fonts/Inter-Regular.ttf",
    "fonts/Inter-Bold.ttf",
    "fonts/Inter-Italic.ttf",
];

/// Root folder of the assets: `ORBIT_ASSETS` if set, else the workspace
/// `assets/` folder (native); the `assets/` URL prefix (web).
pub(crate) fn asset_root() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        "assets".to_owned()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("ORBIT_ASSETS")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").to_owned())
    }
}

pub fn run() -> Result<(), PlayerError> {
    orbit_platform::run(
        AppConfig {
            title: "OrbitEngine M1 demo".to_owned(),
            ..Default::default()
        },
        |window| async move {
            let size = window.inner_size();
            let gfx = Gfx::new(window, (size.width, size.height))
                .await
                .expect("GPU initialization failed");
            let renderer = Renderer2d::new(gfx).expect("renderer initialization failed");
            Demo::new(renderer)
        },
    )?;
    Ok(())
}

struct Pending {
    images: Vec<(&'static str, AssetHandle<ImageData>)>,
    fonts: Vec<AssetHandle<Vec<u8>>>,
    music: AssetHandle<PcmAudio>,
    door: AssetHandle<PcmAudio>,
}

struct ActiveTransition {
    from: usize,
    to: usize,
    elapsed: f32,
    kind: Transition,
}

struct Ready {
    textures: HashMap<&'static str, TextureId>,
    white: TextureId,
    target_a: TextureId,
    target_b: TextureId,
    text_engine: TextEngine,
    text_renderer: TextRenderer,
    dialogue: [TextBlock; 2],
    speaker: [TextBlock; 2],
    music: Option<Sound>,
    door: Option<Sound>,
    audio: AudioEngine,
    music_started: bool,
    current: usize,
    transitions_done: usize,
    transition: Option<ActiveTransition>,
    time: f32,
}

enum Stage {
    Loading(Pending),
    Ready(Box<Ready>),
    /// Initialization failed (logged); keep showing the clear color.
    Failed,
}

struct Demo {
    renderer: Renderer2d,
    stage: Stage,
}

impl Demo {
    fn new(renderer: Renderer2d) -> Self {
        let loader = AssetLoader::new(asset_root());
        let pending = Pending {
            images: IMAGES.iter().map(|p| (*p, loader.load_image(p))).collect(),
            fonts: FONTS.iter().map(|p| loader.load_bytes(p)).collect(),
            music: loader.load_audio("demo/music_theme.wav"),
            door: loader.load_audio("demo/sfx_door.wav"),
        };
        log::info!("demo: loading assets from `{}`", loader.root());
        Self {
            renderer,
            stage: Stage::Loading(pending),
        }
    }
}

fn dialogue_style() -> TextStyle {
    TextStyle {
        font_size: 40.0,
        ..Default::default()
    }
}

fn speaker_style(color: [u8; 4]) -> TextStyle {
    TextStyle {
        font_size: 44.0,
        color,
        ..Default::default()
    }
}

/// Builds the live state once every asset finished loading (or failed).
fn finish_loading(renderer: &mut Renderer2d, pending: &Pending) -> Option<Ready> {
    let mut textures = HashMap::new();
    for (name, handle) in &pending.images {
        let options = if name.contains("mask") {
            TextureOptions::DATA
        } else {
            TextureOptions::COLOR
        };
        let id = match handle.poll() {
            AssetPoll::Ready(img) => {
                renderer.create_texture(name, img.width, img.height, &img.rgba, options)
            }
            _ => renderer.create_texture(name, 1, 1, &[255, 0, 255, 255], options),
        };
        match id {
            Ok(id) => {
                textures.insert(*name, id);
            }
            Err(err) => log::error!("texture `{name}`: {err}"),
        }
    }

    let mut text_engine = TextEngine::new();
    for font in &pending.fonts {
        if let AssetPoll::Ready(bytes) = font.poll() {
            text_engine.add_font_data(bytes.to_vec());
        }
    }
    text_engine.set_default_family("Inter");
    let text_renderer = TextRenderer::new(renderer).ok()?;

    let sound = |h: &AssetHandle<PcmAudio>| match h.poll() {
        AssetPoll::Ready(pcm) => Sound::from_pcm(&pcm).ok(),
        _ => None,
    };

    let (target_a, target_b) = (
        renderer.create_canvas_target("scene A").ok()?,
        renderer.create_canvas_target("scene B").ok()?,
    );
    let infinite = f32::INFINITY;
    let dialogue = [
        TextBlock::new(
            "Welcome to {color=#ffd24a}OrbitEngine{/color}!{w=0.6} This is the {b}M1{/b} demo: \
             {i}sprites{/i}, layers and {color=#6cf}typewriter{/color} text.{w=0.4} \
             Click to speed up, click again for the next scene. Caf\u{e9}, na\u{ef}ve, \u{dc}b\u{e9}rtr\u{e4}ger.",
            dialogue_style(),
            Some(1580.0),
            CHARS_PER_SECOND,
        ),
        TextBlock::new(
            "Nice transition, wasn't it?{w=0.5} Masks, crossfades and fades all run on \
             {b}render targets{/b}.{w=0.4} {color=#ff7a7a}Click{/color} to go back.",
            dialogue_style(),
            Some(1580.0),
            CHARS_PER_SECOND,
        ),
    ];
    let speaker = [
        TextBlock::new("Alice", speaker_style([255, 150, 190, 255]), None, infinite),
        TextBlock::new("Bob", speaker_style([130, 190, 255, 255]), None, infinite),
    ];
    let mut speaker = speaker;
    for block in &mut speaker {
        block.skip_to_end(); // names are shown at once
    }
    Some(Ready {
        white: renderer.white_texture(),
        textures,
        target_a,
        target_b,
        text_engine,
        text_renderer,
        dialogue,
        speaker,
        music: sound(&pending.music),
        door: sound(&pending.door),
        audio: AudioEngine::new_locked(),
        music_started: false,
        current: 0,
        transitions_done: 0,
        transition: None,
        time: 0.0,
    })
}

impl Ready {
    fn tex(&self, name: &str) -> TextureId {
        self.textures.get(name).copied().unwrap_or(self.white)
    }

    /// Fills `scene` with scene number `idx` (0 = room, 1 = street).
    fn build_scene(&mut self, renderer: &Renderer2d, idx: usize, scene: &mut Scene) {
        scene.clear_sprites();
        scene.clear = Some([0.0, 0.0, 0.0, 1.0]);
        let (bg, left, right) = if idx == 0 {
            ("demo/bg_room.png", "demo/alice_sad.png", "demo/bob_sad.png")
        } else {
            (
                "demo/bg_street.png",
                "demo/alice_happy.png",
                "demo/bob_mad.png",
            )
        };
        scene.push(
            Sprite2d::new(self.tex(bg), [0.0, 0.0], [1920.0, 1080.0])
                .anchor([0.0, 0.0])
                .layer(Layer::BACKGROUND),
        );
        let bob = 5.0 * (self.time * 2.0).sin();
        scene.push(
            Sprite2d::new(self.tex(left), [520.0, 1080.0 + bob], [600.0, 900.0])
                .anchor([0.5, 1.0])
                .layer(Layer::CHARACTERS),
        );
        scene.push(
            Sprite2d::new(self.tex(right), [1400.0, 1080.0 - bob], [600.0, 900.0])
                .anchor([0.5, 1.0])
                .layer(Layer::CHARACTERS),
        );
        // Textbox.
        scene.push(
            Sprite2d::new(self.white, [960.0, 905.0], [1740.0, 240.0])
                .layer(Layer::UI)
                .z(0.0)
                .tint([0.0, 0.0, 0.01, 0.72]),
        );
        let name_params = DrawParams {
            origin: [140.0, 800.0],
            ..Default::default()
        };
        let text_params = DrawParams {
            origin: [140.0, 860.0],
            ..Default::default()
        };
        self.speaker[idx].draw(
            &mut self.text_engine,
            &mut self.text_renderer,
            renderer,
            scene,
            &name_params,
        );
        self.dialogue[idx].draw(
            &mut self.text_engine,
            &mut self.text_renderer,
            renderer,
            scene,
            &text_params,
        );
        // Blinking "continue" marker once the text is complete.
        if self.dialogue[idx].is_complete() && self.transition.is_none() {
            let a = 0.5 + 0.5 * (self.time * 5.0).sin();
            scene.push(
                Sprite2d::new(self.white, [1760.0, 1000.0], [22.0, 22.0])
                    .rotation(std::f32::consts::FRAC_PI_4)
                    .layer(Layer::UI)
                    .z(2.0)
                    .alpha(a),
            );
        }
    }

    fn click(&mut self) {
        self.audio.resume_after_user_gesture();
        if !self.music_started {
            self.music_started = true;
            if let Some(music) = &self.music {
                self.audio
                    .play_music(music, MusicOptions::crossfade(Duration::from_millis(1500)));
            }
        }
        if self.transition.is_some() {
            return;
        }
        let block = &mut self.dialogue[self.current];
        if !block.is_complete() {
            block.skip_to_end();
            return;
        }
        let to = 1 - self.current;
        self.dialogue[to].typewriter_mut().reset();
        let kind = match self.transitions_done % 3 {
            0 => Transition::MaskDissolve {
                mask: self.tex("demo/mask_wipe.png"),
                softness: 0.18,
            },
            1 => Transition::Dissolve,
            _ => Transition::Fade {
                color: [0.0, 0.0, 0.0, 1.0],
            },
        };
        self.transitions_done += 1;
        log::info!(
            "transition {} -> {} ({:?})",
            self.current,
            to,
            kind_name(&kind)
        );
        self.transition = Some(ActiveTransition {
            from: self.current,
            to,
            elapsed: 0.0,
            kind,
        });
        if let Some(door) = &self.door {
            self.audio.play_sfx(door);
        }
    }
}

fn kind_name(kind: &Transition) -> &'static str {
    match kind {
        Transition::Fade { .. } => "fade",
        Transition::Dissolve => "dissolve",
        Transition::MaskDissolve { .. } => "mask dissolve",
    }
}

impl App for Demo {
    fn update(&mut self, dt: Duration) {
        let dt = dt.as_secs_f32().min(0.1);
        if let Stage::Loading(pending) = &self.stage {
            let handles_done = pending
                .images
                .iter()
                .map(|(_, h)| !matches!(h.poll(), AssetPoll::Loading))
                .chain(
                    pending
                        .fonts
                        .iter()
                        .map(|h| !matches!(h.poll(), AssetPoll::Loading)),
                )
                .chain([
                    !matches!(pending.music.poll(), AssetPoll::Loading),
                    !matches!(pending.door.poll(), AssetPoll::Loading),
                ])
                .all(|d| d);
            if handles_done {
                self.stage = match finish_loading(&mut self.renderer, pending) {
                    Some(ready) => {
                        log::info!("demo: assets ready");
                        Stage::Ready(Box::new(ready))
                    }
                    None => {
                        log::error!("demo: initialization failed");
                        Stage::Failed
                    }
                };
            }
        }
        if let Stage::Ready(ready) = &mut self.stage {
            ready.time += dt;
            ready.audio.update();
            let current = ready.current;
            ready.dialogue[current].update(dt);
            if let Some(tr) = &mut ready.transition {
                tr.elapsed += dt;
                if tr.elapsed >= TRANSITION_SECS {
                    ready.current = tr.to;
                    ready.transition = None;
                }
            }
        }
    }

    fn render(&mut self) {
        let Stage::Ready(ready) = &mut self.stage else {
            // Loading / failed: just clear.
            if let Ok(Some(frame)) = self.renderer.begin_frame() {
                self.renderer.end_frame(frame);
            }
            return;
        };
        let renderer = &mut self.renderer;
        let frame = match renderer.begin_frame() {
            Ok(Some(f)) => f,
            Ok(None) => return,
            Err(err) => {
                log::error!("begin_frame: {err}");
                return;
            }
        };
        let mut frame = frame;
        let mut scene = Scene::new();
        let result = if let Some(tr) = &ready.transition {
            let (from, to, progress) = (tr.from, tr.to, tr.elapsed / TRANSITION_SECS);
            let kind = tr.kind.clone();
            ready.build_scene(renderer, from, &mut scene);
            let a = renderer.render_to_target(ready.target_a, &scene);
            ready.build_scene(renderer, to, &mut scene);
            let b = renderer.render_to_target(ready.target_b, &scene);
            a.and(b).and_then(|_| {
                renderer.draw_transition(
                    &mut frame,
                    ready.target_a,
                    ready.target_b,
                    &kind,
                    progress,
                )
            })
        } else {
            let current = ready.current;
            ready.build_scene(renderer, current, &mut scene);
            renderer.draw_scene(&mut frame, &scene)
        };
        if let Err(err) = result {
            log::error!("render error: {err}");
        }
        renderer.end_frame(frame);
    }

    fn resize(&mut self, size: PhysicalSize<u32>, _scale_factor: f64) {
        self.renderer.resize(size.width, size.height);
    }

    fn input(&mut self, event: &InputEvent) {
        let clicked = matches!(
            event,
            InputEvent::MouseButton {
                button: MouseButton::Left,
                pressed: true,
            } | InputEvent::Key {
                code: KeyCode::Space | KeyCode::Enter,
                pressed: true,
            }
        );
        if clicked && let Stage::Ready(ready) = &mut self.stage {
            ready.click();
        }
    }
}
