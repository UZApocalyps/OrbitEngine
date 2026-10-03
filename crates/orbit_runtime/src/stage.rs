//! The stage: what is on screen, as plain data (no GPU).

use orbit_core::{AssetId, CharacterId, Transition};
use orbit_vm::PresentationEvent;

/// Virtual canvas the stage is laid out in.
pub const CANVAS: [f32; 2] = [1920.0, 1080.0];
/// Height characters are scaled to (virtual pixels), whatever their texture.
pub const CHARACTER_HEIGHT: f32 = 920.0;

/// A character standing on the stage.
#[derive(Clone, Debug, PartialEq)]
pub struct StageCharacter {
    pub character: CharacterId,
    /// Name of the current expression (looks up its animation, if any).
    pub expression: String,
    /// Image of the current expression.
    pub image: AssetId,
    /// Normalized position of the bottom-center anchor (`0..=1`, origin
    /// top-left).
    pub x: f32,
    pub y: f32,
    /// Scale `[x, y]` applied on top of [`CHARACTER_HEIGHT`], around the
    /// bottom-center anchor (1.0 = standard size).
    pub scale: [f32; 2],
    /// Mirrored on the X axis (the picture is flipped in place).
    pub flip_x: bool,
    /// Mirrored on the Y axis (the picture is flipped in place).
    pub flip_y: bool,
}


/// When each character's animation started, on a clock that only runs while
/// the game does (so animations freeze with the game and a pause screen).
/// Not part of the game state: saves and rollbacks simply restart animations.
#[derive(Clone, Debug, Default)]
pub struct AnimClock {
    now: f32,
    starts: std::collections::HashMap<(CharacterId, String), f32>,
}

impl AnimClock {
    /// A clock where every character of `stage` started `time` seconds ago
    /// (a still picture of the stage at that moment of its animations: the
    /// editor's scene view).
    pub fn at(stage: &StageState, time: f32) -> Self {
        let mut clock = Self::default();
        clock.sync([stage]);
        clock.advance(time);
        clock
    }

    /// Advances the clock by `dt` seconds.
    pub fn advance(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.now += dt;
        }
    }

    /// Starts the animation of every character of `stages` that is new, and
    /// forgets the ones that left every one of them.
    pub fn sync<'a>(&mut self, stages: impl IntoIterator<Item = &'a StageState>) {
        let mut alive: Vec<(CharacterId, &str)> = Vec::new();
        for stage in stages {
            for c in &stage.characters {
                alive.push((c.character, c.expression.as_str()));
                self.starts
                    .entry((c.character, c.expression.clone()))
                    .or_insert(self.now);
            }
        }
        self.starts
            .retain(|(who, name), _| alive.iter().any(|(w, n)| w == who && *n == name.as_str()));
    }

    /// Seconds since the animation of `character` started (0 when unknown).
    pub fn elapsed(&self, character: CharacterId, expression: &str) -> f32 {
        self.starts
            .get(&(character, expression.to_owned()))
            .map_or(0.0, |start| (self.now - start).max(0.0))
    }
}

/// A 3D scene shown as the background layer (ADR 0014): which
/// `.scene3d.ron` asset, from which named camera (`None`: the first one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage3d {
    pub scene: AssetId,
    pub camera: Option<String>,
}

/// Background layer + character layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageState {
    pub background: Option<AssetId>,
    /// Back to front.
    pub characters: Vec<StageCharacter>,
    /// When set, the 3D scene takes the background layer's place (the 2D
    /// background is kept and shows again once the scene is hidden).
    /// Sprites, textbox and UI stay on top, unchanged.
    pub scene3d: Option<Stage3d>,
}

/// What applying a presentation event did to the stage.
#[derive(Clone, Debug, PartialEq)]
pub enum StageEffect {
    /// Not a visual event (audio...): the stage is untouched.
    None,
    /// The stage changed, optionally with a transition requested.
    Visual(Option<Transition>),
}

impl StageState {
    /// Applies one VM presentation event.
    pub fn apply(&mut self, event: &PresentationEvent) -> StageEffect {
        match event {
            PresentationEvent::Background { asset, transition } => {
                self.background = Some(*asset);
                // A 2D background replaces a 3D scene (ADR 0014).
                self.scene3d = None;
                StageEffect::Visual(transition.clone())
            }
            PresentationEvent::Scene3d {
                scene,
                camera,
                transition,
            } => {
                self.scene3d = Some(Stage3d {
                    scene: *scene,
                    camera: camera.clone(),
                });
                StageEffect::Visual(transition.clone())
            }
            PresentationEvent::HideScene3d { transition } => {
                self.scene3d = None;
                StageEffect::Visual(transition.clone())
            }
            PresentationEvent::Show {
                character,
                expression,
                image,
                x,
                y,
                scale,
                flip_x,
                flip_y,
                transition,
                ..
            } => {
                let entry = StageCharacter {
                    character: *character,
                    expression: expression.clone(),
                    image: *image,
                    x: *x,
                    y: *y,
                    scale: *scale,
                    flip_x: *flip_x,
                    flip_y: *flip_y,
                };
                match self
                    .characters
                    .iter_mut()
                    .find(|c| c.character == *character)
                {
                    Some(existing) => *existing = entry,
                    None => self.characters.push(entry),
                }
                StageEffect::Visual(transition.clone())
            }
            PresentationEvent::Hide {
                character,
                transition,
            } => {
                self.characters.retain(|c| c.character != *character);
                StageEffect::Visual(transition.clone())
            }
            PresentationEvent::PlayMusic { .. }
            | PresentationEvent::StopMusic { .. }
            | PresentationEvent::PlaySound { .. }
            | PresentationEvent::Custom { .. } => StageEffect::None,
        }
    }
}

/// Normalized stage coordinates to virtual pixels.
pub fn to_virtual(x: f32, y: f32) -> [f32; 2] {
    [x * CANVAS[0], y * CANVAS[1]]
}

/// Size of a character sprite: scaled to [`CHARACTER_HEIGHT`], aspect ratio
/// kept. Degenerate textures give a zero size.
pub fn character_size(texture: [u32; 2]) -> [f32; 2] {
    let (w, h) = (texture[0] as f32, texture[1] as f32);
    if w <= 0.0 || h <= 0.0 {
        return [0.0, 0.0];
    }
    let scale = CHARACTER_HEIGHT / h;
    [w * scale, CHARACTER_HEIGHT]
}

/// Where and how big a character sprite is drawn, in virtual pixels. This is
/// the single mapping from stage data to a sprite: the renderer and the
/// editor's scene view (gizmos, hit tests) both use it, so they cannot
/// disagree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterPlacement {
    /// Bottom-center anchor.
    pub anchor: [f32; 2],
    /// Unscaled size (aspect ratio of the texture at [`CHARACTER_HEIGHT`]).
    pub size: [f32; 2],
    /// Scale applied around the anchor.
    pub scale: [f32; 2],
    pub flip_x: bool,
    pub flip_y: bool,
}

impl CharacterPlacement {
    /// Drawn size in virtual pixels.
    pub fn drawn_size(&self) -> [f32; 2] {
        [self.size[0] * self.scale[0], self.size[1] * self.scale[1]]
    }

    /// Bounding box `[left, top, right, bottom]` in virtual pixels.
    pub fn rect(&self) -> [f32; 4] {
        let [w, h] = self.drawn_size();
        [
            self.anchor[0] - w * 0.5,
            self.anchor[1] - h,
            self.anchor[0] + w * 0.5,
            self.anchor[1],
        ]
    }
}

/// Size factor of an animation frame of `size` pixels relative to the
/// `reference` frame (the first one) of the same animation. Frames of a
/// sequence may differ in size: they keep the pixel scale of the reference
/// instead of being stretched to the same height, and stay anchored at the
/// bottom center. `[1, 1]` for a degenerate size.
pub fn frame_scale(size: [u32; 2], reference: [u32; 2]) -> [f32; 2] {
    if size[0] == 0 || size[1] == 0 || reference[0] == 0 || reference[1] == 0 {
        return [1.0, 1.0];
    }
    [
        size[0] as f32 / reference[0] as f32,
        size[1] as f32 / reference[1] as f32,
    ]
}

/// Maps a stage character and its texture size to its placement.
pub fn character_placement(character: &StageCharacter, texture: [u32; 2]) -> CharacterPlacement {
    CharacterPlacement {
        anchor: to_virtual(character.x, character.y),
        size: character_size(texture),
        scale: character.scale,
        flip_x: character.flip_x,
        flip_y: character.flip_y,
    }
}

/// Size of a background scaled to cover the whole canvas (centered, aspect
/// ratio kept, the overflow is cropped by the viewport).
pub fn cover_size(texture: [u32; 2]) -> [f32; 2] {
    let (w, h) = (texture[0] as f32, texture[1] as f32);
    if w <= 0.0 || h <= 0.0 {
        return CANVAS;
    }
    let scale = (CANVAS[0] / w).max(CANVAS[1] / h);
    [w * scale, h * scale]
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::Position;

    fn asset(n: u128) -> AssetId {
        AssetId::from_u128(n)
    }

    #[test]
    fn frames_keep_the_pixel_scale_of_the_first_frame() {
        assert_eq!(frame_scale([100, 200], [100, 200]), [1.0, 1.0]);
        // A wider and shorter frame is not stretched to the same height.
        assert_eq!(frame_scale([150, 100], [100, 200]), [1.5, 0.5]);
        assert_eq!(frame_scale([0, 100], [100, 200]), [1.0, 1.0]);
        assert_eq!(frame_scale([10, 10], [0, 0]), [1.0, 1.0]);
    }

    #[test]
    fn a_clock_at_a_time_gives_that_elapsed_time() {
        let mut stage = StageState::default();
        stage.characters.push(StageCharacter {
            character: who(1),
            expression: "a".into(),
            image: asset(1),
            x: 0.5,
            y: 1.0,
            scale: [1.0, 1.0],
            flip_x: false,
            flip_y: false,
        });
        let clock = AnimClock::at(&stage, 1.5);
        assert!((clock.elapsed(who(1), "a") - 1.5).abs() < 1e-6);
        assert_eq!(clock.elapsed(who(2), "a"), 0.0);
    }

    fn who(n: u128) -> CharacterId {
        CharacterId::from_u128(n)
    }

    fn show(c: u128, image: u128, x: f32, t: Option<Transition>) -> PresentationEvent {
        PresentationEvent::Show {
            character: who(c),
            expression: "neutral".into(),
            image: asset(image),
            position: Position::Left,
            x,
            y: 1.0,
            scale: [1.0, 1.0],
            flip_x: false,
            flip_y: false,
            transition: t,
        }
    }

    #[test]
    fn positions_map_to_virtual_coordinates() {
        assert_eq!(to_virtual(0.25, 1.0), [480.0, 1080.0]);
        assert_eq!(to_virtual(0.5, 1.0), [960.0, 1080.0]);
        assert_eq!(to_virtual(0.75, 0.5), [1440.0, 540.0]);
    }

    #[test]
    fn character_is_scaled_to_a_fixed_height() {
        let s = character_size([600, 900]);
        assert_eq!(s[1], CHARACTER_HEIGHT);
        assert!((s[0] / s[1] - 600.0 / 900.0).abs() < 1e-5);
        assert_eq!(character_size([0, 10]), [0.0, 0.0]);
    }

    #[test]
    fn placement_combines_position_scale_and_mirror() {
        let mut stage = StageState::default();
        stage.apply(&PresentationEvent::Show {
            character: who(1),
            expression: "neutral".into(),
            image: asset(10),
            position: Position::Free { x: 0.4, y: 0.9 },
            x: 0.4,
            y: 0.9,
            scale: [0.5, 0.25],
            flip_x: true,
            flip_y: false,
            transition: None,
        });
        let p = character_placement(&stage.characters[0], [600, 920]);
        assert_eq!(p.anchor, [768.0, 972.0]);
        assert_eq!(p.scale, [0.5, 0.25]);
        assert!(p.flip_x && !p.flip_y);
        // 600x920 texture is drawn at 920 px high: 600 wide, then scaled.
        assert_eq!(p.drawn_size(), [300.0, 230.0]);
        // Bottom-center anchored: the box hangs from the anchor.
        assert_eq!(p.rect(), [618.0, 742.0, 918.0, 972.0]);
        // Defaults give the standard size.
        let d = character_placement(&show_default(), [600, 920]);
        assert_eq!(d.drawn_size(), [600.0, CHARACTER_HEIGHT]);
    }

    fn show_default() -> StageCharacter {
        let mut stage = StageState::default();
        stage.apply(&show(1, 10, 0.5, None));
        stage.characters.remove(0)
    }

    #[test]
    fn show_replaces_scale_and_mirror_in_place() {
        let mut stage = StageState::default();
        stage.apply(&show(1, 10, 0.25, None));
        stage.apply(&PresentationEvent::Show {
            character: who(1),
            expression: "neutral".into(),
            image: asset(10),
            position: Position::Left,
            x: 0.25,
            y: 1.0,
            scale: [2.0, 2.0],
            flip_x: true,
            flip_y: true,
            transition: None,
        });
        assert_eq!(stage.characters.len(), 1);
        let c = &stage.characters[0];
        assert_eq!(c.scale, [2.0, 2.0]);
        assert!(c.flip_x && c.flip_y);
    }

    #[test]
    fn background_covers_the_canvas() {
        // Wider than 16:9: height-limited, overflows horizontally.
        let wide = cover_size([4000, 1000]);
        assert_eq!(wide[1], 1080.0);
        assert!(wide[0] > 1920.0);
        // Taller than 16:9: width-limited.
        let tall = cover_size([1000, 4000]);
        assert_eq!(tall[0], 1920.0);
        assert!(tall[1] > 1080.0);
        // Exactly 16:9.
        let exact = cover_size([1280, 720]);
        assert!((exact[0] - 1920.0).abs() < 1e-3 && (exact[1] - 1080.0).abs() < 1e-3);
        assert_eq!(cover_size([0, 0]), CANVAS);
    }

    #[test]
    fn background_event_sets_the_layer_and_reports_transition() {
        let mut stage = StageState::default();
        let t = Some(Transition::dissolve(0.5));
        let effect = stage.apply(&PresentationEvent::Background {
            asset: asset(1),
            transition: t.clone(),
        });
        assert_eq!(stage.background, Some(asset(1)));
        assert_eq!(effect, StageEffect::Visual(t.clone()));
    }

    #[test]
    fn the_3d_scene_takes_the_background_layer_and_gives_it_back() {
        let mut stage = StageState::default();
        stage.apply(&PresentationEvent::Background {
            asset: asset(1),
            transition: None,
        });
        stage.apply(&show(1, 10, 0.5, None));
        let t = Some(Transition::dissolve(0.5));
        let effect = stage.apply(&PresentationEvent::Scene3d {
            scene: asset(7),
            camera: Some("wide".into()),
            transition: t.clone(),
        });
        assert_eq!(effect, StageEffect::Visual(t));
        assert_eq!(
            stage.scene3d,
            Some(Stage3d {
                scene: asset(7),
                camera: Some("wide".into()),
            })
        );
        // Characters and the (hidden) 2D background are untouched.
        assert_eq!(stage.background, Some(asset(1)));
        assert_eq!(stage.characters.len(), 1);
        stage.apply(&PresentationEvent::HideScene3d { transition: None });
        assert_eq!(stage.scene3d, None);
        assert_eq!(stage.background, Some(asset(1)));
        // A background replaces a 3D scene.
        stage.apply(&PresentationEvent::Scene3d {
            scene: asset(7),
            camera: None,
            transition: None,
        });
        stage.apply(&PresentationEvent::Background {
            asset: asset(2),
            transition: None,
        });
        assert_eq!(stage.scene3d, None);
        assert_eq!(stage.background, Some(asset(2)));
    }

    #[test]
    fn show_adds_then_updates_in_place() {
        let mut stage = StageState::default();
        stage.apply(&show(1, 10, 0.25, None));
        stage.apply(&show(2, 20, 0.75, None));
        // Expression change of character 1 keeps its z-order.
        stage.apply(&show(1, 11, 0.5, None));
        assert_eq!(stage.characters.len(), 2);
        assert_eq!(stage.characters[0].character, who(1));
        assert_eq!(stage.characters[0].image, asset(11));
        assert_eq!(stage.characters[0].x, 0.5);
        assert_eq!(stage.characters[1].image, asset(20));
    }

    #[test]
    fn hide_removes_and_unknown_hide_is_harmless() {
        let mut stage = StageState::default();
        stage.apply(&show(1, 10, 0.25, None));
        let effect = stage.apply(&PresentationEvent::Hide {
            character: who(1),
            transition: None,
        });
        assert!(stage.characters.is_empty());
        assert_eq!(effect, StageEffect::Visual(None));
        stage.apply(&PresentationEvent::Hide {
            character: who(9),
            transition: None,
        });
        assert!(stage.characters.is_empty());
    }

    #[test]
    fn audio_events_do_not_touch_the_stage() {
        let mut stage = StageState::default();
        let before = stage.clone();
        let effect = stage.apply(&PresentationEvent::PlaySound { asset: asset(3) });
        assert_eq!(effect, StageEffect::None);
        assert_eq!(stage, before);
    }

    #[test]
    fn anim_clock_runs_per_character_and_forgets_the_gone() {
        let mut stage = StageState::default();
        stage.apply(&show(1, 10, 0.5, None));
        let mut clock = AnimClock::default();
        clock.sync([&stage]);
        clock.advance(0.5);
        assert_eq!(clock.elapsed(who(1), "neutral"), 0.5);
        // A character arriving later starts at 0 while the first keeps going.
        stage.apply(&show(2, 20, 0.7, None));
        clock.sync([&stage]);
        clock.advance(0.25);
        assert_eq!(clock.elapsed(who(1), "neutral"), 0.75);
        assert_eq!(clock.elapsed(who(2), "neutral"), 0.25);
        // Unknown = frame 0; bad dt is ignored.
        assert_eq!(clock.elapsed(who(3), "neutral"), 0.0);
        clock.advance(f32::NAN);
        clock.advance(-1.0);
        assert_eq!(clock.elapsed(who(1), "neutral"), 0.75);
        // Hidden characters are forgotten, so showing them again restarts.
        stage.apply(&PresentationEvent::Hide {
            character: who(1),
            transition: None,
        });
        clock.sync([&stage]);
        stage.apply(&show(1, 10, 0.5, None));
        clock.sync([&stage]);
        assert_eq!(clock.elapsed(who(1), "neutral"), 0.0);
    }
}
