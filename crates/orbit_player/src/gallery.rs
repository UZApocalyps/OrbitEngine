//! M2 UI gallery: every `orbit_ui` screen with fake view-models, to check
//! the look and the navigation by eye (native and web).
//!
//! Native: `orbit_player --ui-gallery`. Web: `index.html?ui-gallery`.
//!
//! Keys: `1`..`7` jump to a screen (main menu, pause, save, load,
//! preferences, history, play), `L` switches the language, `T` pushes a toast,
//! Escape / right click goes back. Arrows, Tab, Enter, mouse and wheel drive
//! the widgets like in the game.

use std::collections::HashMap;
use std::time::Duration;

use orbit_assets::{AssetHandle, AssetLoader, AssetPoll, ImageData};
use orbit_gfx::Gfx;
use orbit_platform::{App, AppConfig, InputEvent, KeyCode, MouseButton, PhysicalSize};
use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId, TextureOptions};
use orbit_text::{TextEngine, TextRenderer};
use orbit_ui::{
    Anchor, CategoryView, DialogueBox, DrawCtx, HistoryEntryView, HistoryScreen, HistoryView,
    LanguageView, MainMenu, MainMenuView, Node, PauseMenu, PauseView, PreferencesScreen, PrefsView,
    QuickMenu, QuickMenuView, SaveLoadMode, SaveLoadScreen, SaveLoadView, Screen, SlotCategory,
    SlotId, SlotState, SlotView, Texts, Theme, ToastStack, Ui, UiEvent, UiInput,
};

use crate::PlayerError;
use crate::demo::asset_root;

const IMAGES: &[&str] = &[
    "demo/bg_room.png",
    "demo/bg_street.png",
    "demo/alice_neutral.png",
    "demo/alice_happy.png",
    "demo/bob_neutral.png",
    "demo/bob_angry.png",
];
const FONTS: &[&str] = &[
    "fonts/Inter-Regular.ttf",
    "fonts/Inter-Bold.ttf",
    "fonts/Inter-Italic.ttf",
];

pub fn run() -> Result<(), PlayerError> {
    orbit_platform::run(
        AppConfig {
            title: "OrbitEngine UI gallery".to_owned(),
            ..Default::default()
        },
        |window| async move {
            let size = window.inner_size();
            let gfx = Gfx::new(window, (size.width, size.height))
                .await
                .expect("GPU initialization failed");
            let renderer = Renderer2d::new(gfx).expect("renderer initialization failed");
            Gallery::new(renderer)
        },
    )?;
    Ok(())
}

struct Pending {
    images: Vec<(&'static str, AssetHandle<ImageData>)>,
    fonts: Vec<AssetHandle<Vec<u8>>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    Main,
    Pause,
    Save,
    Load,
    Prefs,
    History,
    Play,
}

struct Ready {
    white: TextureId,
    textures: HashMap<&'static str, TextureId>,
    text_engine: TextEngine,
    text_renderer: TextRenderer,
    texts: Texts,
    language: &'static str,
    current: Which,
    back: Vec<Which>,
    main: MainMenu,
    pause: PauseMenu,
    save: SaveLoadScreen,
    load: SaveLoadScreen,
    prefs: PreferencesScreen,
    prefs_view: PrefsView,
    history: HistoryScreen,
    quick: QuickMenu,
    quick_view: QuickMenuView,
    toasts: ToastStack,
    hint: Ui,
    dialogue: DialogueBox,
    slots: Vec<CategoryView>,
    pointer: [f32; 2],
    shift: bool,
    time: f32,
}

enum Stage {
    Loading(Pending),
    Ready(Box<Ready>),
    Failed,
}

struct Gallery {
    renderer: Renderer2d,
    stage: Stage,
}

impl Gallery {
    fn new(renderer: Renderer2d) -> Self {
        let loader = AssetLoader::new(asset_root());
        let pending = Pending {
            images: IMAGES.iter().map(|p| (*p, loader.load_image(p))).collect(),
            fonts: FONTS.iter().map(|p| loader.load_bytes(p)).collect(),
        };
        Self {
            renderer,
            stage: Stage::Loading(pending),
        }
    }
}

fn fake_slots(textures: &HashMap<&'static str, TextureId>) -> Vec<CategoryView> {
    // Backgrounds only: they have the 16:9 ratio of real thumbnails.
    let thumbs: Vec<Option<TextureId>> = IMAGES[..2]
        .iter()
        .map(|n| textures.get(n).copied())
        .collect();
    let previews = [
        "Welcome to OrbitEngine! This is the first line of the story.",
        "Nice transition, wasn't it? Masks, crossfades and fades all run on render targets.",
        "I told you not to open that door.",
        "...",
        "Caf\u{e9}, na\u{ef}ve, \u{dc}b\u{e9}rtr\u{e4}ger: accents work too, and a very long line gets cut with an ellipsis at the end of the second line.",
    ];
    let make = |category: SlotCategory, index: u32, state: SlotState| SlotView {
        id: SlotId { category, index },
        state,
        thumbnail: (state != SlotState::Empty)
            .then(|| thumbs[index as usize % thumbs.len()])
            .flatten(),
        timestamp: format!("2026-10-{:02} 18:{:02}", 1 + index % 28, (index * 7) % 60),
        preview: previews[index as usize % previews.len()].to_owned(),
    };
    let manual = (0..30)
        .map(|i| {
            let state = match i {
                0..=4 => SlotState::Used,
                7 => SlotState::Incompatible,
                10..=12 => SlotState::Used,
                _ => SlotState::Empty,
            };
            if state == SlotState::Empty {
                SlotView::empty(SlotId {
                    category: SlotCategory::Manual,
                    index: i,
                })
            } else {
                make(SlotCategory::Manual, i, state)
            }
        })
        .collect();
    let auto = (0..3)
        .map(|i| make(SlotCategory::Auto, i, SlotState::Used))
        .collect();
    vec![
        CategoryView {
            category: SlotCategory::Manual,
            slots: manual,
        },
        CategoryView {
            category: SlotCategory::Auto,
            slots: auto,
        },
        CategoryView {
            category: SlotCategory::Quick,
            slots: vec![make(SlotCategory::Quick, 0, SlotState::Used)],
        },
    ]
}

fn fake_history() -> HistoryView {
    let lines = [
        (
            Some(("Alice", [255, 150, 190])),
            "Welcome to the history screen. Each line is listed with its speaker.",
        ),
        (
            Some(("Bob", [130, 190, 255])),
            "Scroll with the wheel, the arrows or Page Up / Page Down.",
        ),
        (
            None,
            "The narrator speaks without a name, and the text of a long entry wraps on several lines so rows have different heights.",
        ),
        (
            Some(("Alice", [255, 150, 190])),
            "Rewind buttons only appear where the view-model allows it.",
        ),
    ];
    HistoryView {
        entries: (0..26)
            .map(|i| {
                let (speaker, text) = lines[i % lines.len()];
                HistoryEntryView {
                    id: i,
                    speaker: speaker.map(|(n, c)| (n.to_owned(), c)),
                    text: format!("{} ({})", text, i + 1),
                    can_rollback: i % 3 != 1,
                }
            })
            .collect(),
    }
}

fn languages() -> Vec<LanguageView> {
    vec![
        LanguageView {
            code: "en".into(),
            name: "English".into(),
        },
        LanguageView {
            code: "fr".into(),
            name: "Fran\u{e7}ais".into(),
        },
    ]
}

fn build_hint(theme: &Theme, texts: &Texts) -> Ui {
    let _ = texts;
    let mut ui = Ui::new(theme.clone());
    ui.set_layer(Layer(50));
    ui.add_root(
        Node::label(
            "[1] Main  [2] Pause  [3] Save  [4] Load  [5] Prefs  [6] History  [7] Play  \
             [L] Language  [T] Toast  [Esc] Back",
        )
        .font(20.0)
        .dimmed()
        .anchor(Anchor::top_left(14.0, 8.0)),
    );
    ui.layout();
    ui
}

impl Ready {
    fn new(renderer: &mut Renderer2d, pending: &Pending) -> Option<Self> {
        let mut textures = HashMap::new();
        for (name, handle) in &pending.images {
            let id = match handle.poll() {
                AssetPoll::Ready(img) => renderer.create_texture(
                    name,
                    img.width,
                    img.height,
                    &img.rgba,
                    TextureOptions::COLOR,
                ),
                _ => {
                    renderer.create_texture(name, 1, 1, &[255, 0, 255, 255], TextureOptions::COLOR)
                }
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
        let theme = Theme::default();
        let texts = Texts::builtin("en");
        let slots = fake_slots(&textures);
        let prefs_view = PrefsView {
            languages: languages(),
            language: "en".into(),
            fullscreen: if cfg!(target_arch = "wasm32") {
                None
            } else {
                Some(false)
            },
            ..Default::default()
        };
        let quick_view = QuickMenuView {
            can_rollback: true,
            can_quick_load: true,
            ..Default::default()
        };
        let mut dialogue = DialogueBox::new(theme.textbox.clone());
        dialogue.set_line(
            Some(("Alice", [255, 150, 190])),
            "This is the {color=#ffd24a}play{/color} view: the quick menu sits under the \
             textbox. Click the buttons, or press Escape for the pause menu.",
        );
        Some(Self {
            white: renderer.white_texture(),
            main: MainMenu::new(&theme, &texts, main_view(&textures)),
            pause: PauseMenu::new(
                &theme,
                &texts,
                PauseView {
                    can_save: true,
                    show_quit: !cfg!(target_arch = "wasm32"),
                },
            ),
            save: SaveLoadScreen::new(&theme, &texts, save_view(SaveLoadMode::Save, &slots)),
            load: SaveLoadScreen::new(&theme, &texts, save_view(SaveLoadMode::Load, &slots)),
            prefs: PreferencesScreen::new(&theme, &texts, prefs_view.clone()),
            history: HistoryScreen::new(&theme, &texts, fake_history()),
            quick: QuickMenu::new(&theme, &texts, quick_view),
            toasts: ToastStack::new(&theme),
            hint: build_hint(&theme, &texts),
            textures,
            text_engine,
            text_renderer,
            texts,
            language: "en",
            current: Which::Main,
            back: Vec::new(),
            prefs_view,
            quick_view,
            dialogue,
            slots,
            pointer: [-1.0, -1.0],
            shift: false,
            time: 0.0,
        })
    }

    fn tex(&self, name: &str) -> TextureId {
        self.textures.get(name).copied().unwrap_or(self.white)
    }

    fn go(&mut self, which: Which) {
        if which != self.current {
            self.back.push(self.current);
            self.current = which;
        }
    }

    fn jump(&mut self, which: Which) {
        self.back.clear();
        self.current = which;
    }

    fn pop(&mut self) {
        self.current = self.back.pop().unwrap_or(Which::Main);
    }

    fn screen_mut(&mut self) -> Option<&mut dyn Screen> {
        match self.current {
            Which::Main => Some(&mut self.main),
            Which::Pause => Some(&mut self.pause),
            Which::Save => Some(&mut self.save),
            Which::Load => Some(&mut self.load),
            Which::Prefs => Some(&mut self.prefs),
            Which::History => Some(&mut self.history),
            Which::Play => None,
        }
    }

    fn refresh_slots(&mut self) {
        self.save
            .set_view(save_view(SaveLoadMode::Save, &self.slots));
        self.load
            .set_view(save_view(SaveLoadMode::Load, &self.slots));
    }

    fn set_language(&mut self, code: &str) {
        self.language = if code == "fr" { "fr" } else { "en" };
        self.texts = Texts::builtin(self.language);
        self.prefs_view.language = self.language.to_owned();
        let texts = self.texts.clone();
        self.main.set_texts(&texts);
        self.pause.set_texts(&texts);
        self.save.set_texts(&texts);
        self.load.set_texts(&texts);
        self.prefs.set_texts(&texts);
        self.history.set_texts(&texts);
        self.quick.set_texts(&texts);
    }

    fn apply(&mut self, events: Vec<UiEvent>) {
        for ev in events {
            match ev {
                UiEvent::Sound(_) => {}
                UiEvent::StartGame | UiEvent::Continue => self.jump(Which::Play),
                UiEvent::OpenLoad => self.go(Which::Load),
                UiEvent::OpenSave => self.go(Which::Save),
                UiEvent::OpenPreferences => self.go(Which::Prefs),
                UiEvent::OpenHistory => self.go(Which::History),
                UiEvent::OpenPause => self.go(Which::Pause),
                UiEvent::Resume => self.jump(Which::Play),
                UiEvent::Back => self.pop(),
                UiEvent::ReturnToMainMenu => self.jump(Which::Main),
                UiEvent::Quit => self.toasts.push("Quit requested (ignored in the gallery)"),
                UiEvent::Toast(text) => self.toasts.push(text),
                UiEvent::LoadSlot(id) => {
                    log::info!("gallery: load {id:?}");
                    self.toasts.push(self.texts.get("ui.toast.loaded"));
                    self.jump(Which::Play);
                }
                UiEvent::SaveSlot(id) => {
                    log::info!("gallery: save {id:?}");
                    self.mark_used(id);
                    self.toasts.push(self.texts.get("ui.toast.saved"));
                }
                UiEvent::DeleteSlot(id) => {
                    log::info!("gallery: delete {id:?}");
                    self.mark_empty(id);
                    self.toasts.push(self.texts.get("ui.toast.deleted"));
                }
                UiEvent::SetLanguage(code) => self.set_language(&code),
                UiEvent::ResetPreferences => {
                    self.prefs_view = PrefsView {
                        language: self.language.to_owned(),
                        ..PrefsView {
                            languages: languages(),
                            fullscreen: self.prefs_view.fullscreen.map(|_| false),
                            ..Default::default()
                        }
                    };
                    self.prefs.set_view(self.prefs_view.clone());
                }
                UiEvent::SetSkipMode(mode) => {
                    self.prefs_view.skip_mode = mode;
                    log::info!("gallery: skip mode {mode:?}");
                }
                UiEvent::SetVolume(bus, v) => log::debug!("gallery: volume {bus:?} = {v:.2}"),
                UiEvent::SetTextSpeed(v) => log::debug!("gallery: text speed {v:.0}"),
                UiEvent::SetAutoDelay(v) => log::debug!("gallery: auto delay {v:.1}"),
                UiEvent::SetFullscreen(on) => log::info!("gallery: fullscreen {on}"),
                UiEvent::ToggleSkip => {
                    self.quick_view.skip_active = !self.quick_view.skip_active;
                    self.quick.set_view(self.quick_view);
                }
                UiEvent::ToggleAuto => {
                    self.quick_view.auto_active = !self.quick_view.auto_active;
                    self.quick.set_view(self.quick_view);
                }
                UiEvent::QuickSave => self.toasts.push(self.texts.get("ui.toast.quick_saved")),
                UiEvent::QuickLoad => self.toasts.push(self.texts.get("ui.toast.loaded")),
                UiEvent::Rollback => self.toasts.push("Rollback"),
                UiEvent::RollbackTo(id) => {
                    self.toasts.push(format!("Rollback to entry {id}"));
                    self.jump(Which::Play);
                }
            }
        }
    }

    fn slot_mut(&mut self, id: SlotId) -> Option<&mut SlotView> {
        self.slots
            .iter_mut()
            .find(|c| c.category == id.category)
            .and_then(|c| c.slots.iter_mut().find(|s| s.id == id))
    }

    fn mark_used(&mut self, id: SlotId) {
        let thumb = self.textures.get("demo/bg_room.png").copied();
        if let Some(slot) = self.slot_mut(id) {
            slot.state = SlotState::Used;
            slot.thumbnail = thumb;
            slot.timestamp = "2026-10-02 19:00".into();
            slot.preview = "Saved from the gallery.".into();
        }
        self.refresh_slots();
    }

    fn mark_empty(&mut self, id: SlotId) {
        if let Some(slot) = self.slot_mut(id) {
            *slot = SlotView::empty(id);
        }
        self.refresh_slots();
    }

    fn handle_input(&mut self, input: UiInput) {
        // Play view: keys advance the dialogue, the quick menu has the
        // pointer, Cancel opens the pause menu.
        if self.current == Which::Play {
            match input {
                UiInput::PointerMove(_) | UiInput::PointerDown(_) | UiInput::PointerUp(_) => {
                    let events = self.quick.handle(input);
                    let over_bar = match input {
                        UiInput::PointerDown(p) => self.quick.wants_pointer(p),
                        _ => false,
                    };
                    self.apply(events);
                    if let UiInput::PointerDown(_) = input
                        && !over_bar
                    {
                        self.advance_dialogue();
                    }
                }
                UiInput::Confirm => self.advance_dialogue(),
                UiInput::Cancel => self.go(Which::Pause),
                UiInput::PageUp => self.go(Which::History),
                _ => {}
            }
            return;
        }
        let Some(screen) = self.screen_mut() else {
            return;
        };
        let events = screen.handle(input);
        self.apply(events);
    }

    fn advance_dialogue(&mut self) {
        if self.dialogue.advance() == orbit_ui::Advance::Next {
            self.dialogue.set_line(
                Some(("Bob", [130, 190, 255])),
                "Next line: it types out again, click to complete it.",
            );
        }
    }

    fn key(&mut self, code: KeyCode, pressed: bool) {
        match code {
            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.shift = pressed,
            _ if !pressed => {}
            KeyCode::Digit1 => self.jump(Which::Main),
            KeyCode::Digit2 => self.jump(Which::Pause),
            KeyCode::Digit3 => self.jump(Which::Save),
            KeyCode::Digit4 => self.jump(Which::Load),
            KeyCode::Digit5 => self.jump(Which::Prefs),
            KeyCode::Digit6 => self.jump(Which::History),
            KeyCode::Digit7 => self.jump(Which::Play),
            KeyCode::KeyL => {
                let next = if self.language == "en" { "fr" } else { "en" };
                self.set_language(next);
                self.prefs.set_view(self.prefs_view.clone());
            }
            KeyCode::KeyT => {
                let text = self.texts.get("ui.toast.saved");
                self.toasts.push(text);
            }
            other => {
                if let Some(input) = map_key(other, self.shift) {
                    self.handle_input(input);
                }
            }
        }
    }

    fn update(&mut self, dt: f32) {
        self.time += dt;
        self.dialogue.update(dt);
        self.toasts.update(dt);
        self.quick.update(dt);
        if let Some(screen) = self.screen_mut() {
            screen.update(dt);
        }
    }

    fn build_play_scene(&self, scene: &mut Scene) {
        scene.clear = Some([0.0, 0.0, 0.0, 1.0]);
        scene.push(
            Sprite2d::new(self.tex("demo/bg_room.png"), [0.0, 0.0], [1920.0, 1080.0])
                .anchor([0.0, 0.0])
                .layer(Layer::BACKGROUND),
        );
        scene.push(
            Sprite2d::new(
                self.tex("demo/alice_neutral.png"),
                [520.0, 1080.0],
                [600.0, 900.0],
            )
            .anchor([0.5, 1.0])
            .layer(Layer::CHARACTERS),
        );
        scene.push(
            Sprite2d::new(
                self.tex("demo/bob_neutral.png"),
                [1400.0, 1080.0],
                [600.0, 900.0],
            )
            .anchor([0.5, 1.0])
            .layer(Layer::CHARACTERS),
        );
    }

    fn render(&mut self, renderer: &Renderer2d) -> Scene {
        let mut scene = Scene::new();
        scene.clear = Some([0.0, 0.0, 0.0, 1.0]);
        if self.current != Which::Main {
            self.build_play_scene(&mut scene);
        }
        let mut ctx = DrawCtx {
            engine: &mut self.text_engine,
            text_renderer: &mut self.text_renderer,
            renderer,
            scene: &mut scene,
            white: self.white,
        };
        if matches!(self.current, Which::Play | Which::Pause) {
            self.dialogue.draw(
                ctx.engine,
                ctx.text_renderer,
                ctx.renderer,
                ctx.scene,
                ctx.white,
            );
            self.quick.draw(&mut ctx);
        }
        match self.current {
            Which::Main => self.main.draw(&mut ctx),
            Which::Pause => self.pause.draw(&mut ctx),
            Which::Save => self.save.draw(&mut ctx),
            Which::Load => self.load.draw(&mut ctx),
            Which::Prefs => self.prefs.draw(&mut ctx),
            Which::History => self.history.draw(&mut ctx),
            Which::Play => {}
        }
        self.toasts.draw(&mut ctx);
        self.hint.draw(&mut ctx);
        scene
    }
}

fn main_view(textures: &HashMap<&'static str, TextureId>) -> MainMenuView {
    MainMenuView {
        title: "Orbit Demo".into(),
        subtitle: Some("A visual novel made with OrbitEngine".into()),
        background: textures.get("demo/bg_street.png").copied(),
        can_continue: true,
        show_quit: !cfg!(target_arch = "wasm32"),
    }
}

fn save_view(mode: SaveLoadMode, slots: &[CategoryView]) -> SaveLoadView {
    SaveLoadView {
        mode,
        categories: slots.to_vec(),
    }
}

fn map_key(code: KeyCode, shift: bool) -> Option<UiInput> {
    Some(match code {
        KeyCode::ArrowUp => UiInput::Up,
        KeyCode::ArrowDown => UiInput::Down,
        KeyCode::ArrowLeft => UiInput::Left,
        KeyCode::ArrowRight => UiInput::Right,
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => UiInput::Confirm,
        KeyCode::Escape => UiInput::Cancel,
        KeyCode::Tab if shift => UiInput::Prev,
        KeyCode::Tab => UiInput::Next,
        KeyCode::PageUp => UiInput::PageUp,
        KeyCode::PageDown => UiInput::PageDown,
        _ => return None,
    })
}

impl App for Gallery {
    fn update(&mut self, dt: Duration) {
        let dt = dt.as_secs_f32().min(0.1);
        if let Stage::Loading(pending) = &self.stage {
            let done = pending
                .images
                .iter()
                .map(|(_, h)| !matches!(h.poll(), AssetPoll::Loading))
                .chain(
                    pending
                        .fonts
                        .iter()
                        .map(|h| !matches!(h.poll(), AssetPoll::Loading)),
                )
                .all(|d| d);
            if done {
                self.stage = match Ready::new(&mut self.renderer, pending) {
                    Some(ready) => {
                        log::info!("ui gallery: ready");
                        Stage::Ready(Box::new(ready))
                    }
                    None => {
                        log::error!("ui gallery: initialization failed");
                        Stage::Failed
                    }
                };
            }
        }
        if let Stage::Ready(ready) = &mut self.stage {
            ready.update(dt);
        }
    }

    fn render(&mut self) {
        let Some(mut frame) = (match self.renderer.begin_frame() {
            Ok(frame) => frame,
            Err(err) => {
                log::error!("begin_frame: {err}");
                return;
            }
        }) else {
            return;
        };
        if let Stage::Ready(ready) = &mut self.stage {
            let scene = ready.render(&self.renderer);
            if let Err(err) = self.renderer.draw_scene(&mut frame, &scene) {
                log::error!("render error: {err}");
            }
        }
        self.renderer.end_frame(frame);
    }

    fn resize(&mut self, size: PhysicalSize<u32>, _scale_factor: f64) {
        self.renderer.resize(size.width, size.height);
    }

    fn input(&mut self, event: &InputEvent) {
        let Stage::Ready(ready) = &mut self.stage else {
            return;
        };
        match event {
            InputEvent::CursorMoved { x, y } => {
                ready.pointer = self.renderer.screen_to_world([*x as f32, *y as f32]);
                ready.handle_input(UiInput::PointerMove(ready.pointer));
            }
            InputEvent::MouseButton { button, pressed } => match (button, pressed) {
                (MouseButton::Left, true) => {
                    ready.handle_input(UiInput::PointerDown(ready.pointer));
                }
                (MouseButton::Left, false) => {
                    ready.handle_input(UiInput::PointerUp(ready.pointer));
                }
                (MouseButton::Right, true) => ready.handle_input(UiInput::Cancel),
                _ => {}
            },
            InputEvent::Scroll { dy, .. } => ready.handle_input(UiInput::Scroll(*dy)),
            InputEvent::Key { code, pressed } => ready.key(*code, *pressed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_abstract_inputs() {
        assert_eq!(map_key(KeyCode::ArrowLeft, false), Some(UiInput::Left));
        assert_eq!(map_key(KeyCode::Tab, false), Some(UiInput::Next));
        assert_eq!(map_key(KeyCode::Tab, true), Some(UiInput::Prev));
        assert_eq!(map_key(KeyCode::Escape, false), Some(UiInput::Cancel));
        assert_eq!(map_key(KeyCode::KeyA, false), None);
    }

    #[test]
    fn fake_data_is_consistent() {
        let slots = fake_slots(&HashMap::new());
        assert_eq!(slots[0].slots.len(), 30);
        assert!(
            slots[0]
                .slots
                .iter()
                .any(|s| s.state == SlotState::Incompatible)
        );
        assert_eq!(fake_history().entries.len(), 26);
    }
}
