//! The stage: what is on screen, as plain data (no GPU).

use orbit_core::{
    AssetId, BgLayer, CameraFx, CharacterId, EntityFx, MAX_PARTICLES_TOTAL, Particle, StageFx,
    Transition,
};
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
    /// Expression changes of a character, for the cross-fade of rig clips
    /// (M5B-07): keyed by the character and its *new* expression.
    fades: std::collections::HashMap<(CharacterId, String), ExpressionFade>,
    /// Reduced motion: rigs hold their first pose and never cross-fade.
    frozen: bool,
}

/// The expression a character just left, kept while a rig cross-fades.
#[derive(Clone, Debug, PartialEq)]
struct ExpressionFade {
    previous: String,
    previous_started: f32,
    changed_at: f32,
}

/// A rig cross-fade in progress: the expression being left, its own clip
/// time, and the weight of the new one (`0..1`).
#[derive(Clone, Debug, PartialEq)]
pub struct RigFade {
    pub previous: String,
    pub previous_elapsed: f32,
    pub weight: f32,
}

/// Length (seconds) of the cross-fade between the clips of two expressions
/// of the same rig (ADR 0013 §2: the default blend).
pub const RIG_BLEND: f32 = 0.2;

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
                let key = (c.character, c.expression.clone());
                if self.starts.contains_key(&key) {
                    continue;
                }
                // A new expression for a character already standing there:
                // remember what it left (the other entry of the character
                // that is not on screen any more).
                let left = self
                    .starts
                    .iter()
                    .filter(|((who, name), _)| *who == c.character && *name != c.expression)
                    .filter(|((who, name), _)| {
                        !alive.iter().any(|(w, n)| w == who && *n == name.as_str())
                    })
                    .map(|((_, name), start)| (name.clone(), *start))
                    .next();
                if let Some((previous, previous_started)) = left {
                    self.fades.insert(
                        key.clone(),
                        ExpressionFade {
                            previous,
                            previous_started,
                            changed_at: self.now,
                        },
                    );
                }
                self.starts.insert(key, self.now);
            }
        }
        self.starts
            .retain(|(who, name), _| alive.iter().any(|(w, n)| w == who && *n == name.as_str()));
        self.fades
            .retain(|(who, name), _| alive.iter().any(|(w, n)| w == who && *n == name.as_str()));
    }

    /// Seconds since the animation of `character` started (0 when unknown).
    pub fn elapsed(&self, character: CharacterId, expression: &str) -> f32 {
        self.starts
            .get(&(character, expression.to_owned()))
            .map_or(0.0, |start| (self.now - start).max(0.0))
    }

    /// [`AnimClock::elapsed`] for a rig clip: 0 under reduced motion (the
    /// first pose is held).
    pub fn rig_elapsed(&self, character: CharacterId, expression: &str) -> f32 {
        if self.frozen {
            0.0
        } else {
            self.elapsed(character, expression)
        }
    }

    /// Reduced motion on / off. Frame animations are not affected (they are
    /// the author's choice); rigs hold their first pose and do not fade.
    pub fn set_reduced_motion(&mut self, on: bool) {
        self.frozen = on;
    }

    /// The cross-fade of `character` towards `expression`, while it lasts.
    pub fn rig_fade(&self, character: CharacterId, expression: &str) -> Option<RigFade> {
        if self.frozen {
            return None;
        }
        let f = self.fades.get(&(character, expression.to_owned()))?;
        let weight = (self.now - f.changed_at) / RIG_BLEND;
        (weight < 1.0).then(|| RigFade {
            previous: f.previous.clone(),
            previous_elapsed: (self.now - f.previous_started).max(0.0),
            weight: weight.clamp(0.0, 1.0),
        })
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
    /// Parallax layers of the background, back to front (M5A-07). When not
    /// empty they are drawn instead of `background` (which is the first
    /// layer, kept for the code that only knows a single background).
    pub bg_layers: Vec<BgLayer>,
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
                self.bg_layers.clear();
                // A 2D background replaces a 3D scene (ADR 0014).
                self.scene3d = None;
                StageEffect::Visual(transition.clone())
            }
            PresentationEvent::BackgroundLayers { layers, transition } => {
                self.background = layers.first().map(|l| l.asset);
                self.bg_layers = layers.clone();
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
            | PresentationEvent::AchievementUnlocked { .. }
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

// ------------------------------------------------------------ effects

/// A point of the stage seen through the camera: scaled around the canvas
/// center by the zoom, then shifted by the camera offset (stage units).
pub fn camera_point(p: [f32; 2], camera: &CameraFx) -> [f32; 2] {
    let c = [CANVAS[0] * 0.5, CANVAS[1] * 0.5];
    [
        c[0] + (p[0] - c[0]) * camera.zoom + camera.offset[0] * CANVAS[0],
        c[1] + (p[1] - c[1]) * camera.zoom + camera.offset[1] * CANVAS[1],
    ]
}

fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear sprite tint for an entity: the opacity in alpha, the overlay as a
/// multiply that moves white towards the tint color by its strength.
pub fn entity_tint(fx: &EntityFx) -> [f32; 4] {
    let [r, g, b, a] = fx.tint;
    let mix = |c: f32| srgb_to_linear(1.0 + (c - 1.0) * a.clamp(0.0, 1.0));
    [mix(r), mix(g), mix(b), fx.opacity.clamp(0.0, 1.0)]
}

/// Linear RGBA of the full-screen overlay (flash, tint), `None` when it is
/// invisible.
pub fn screen_overlay(fx: &StageFx) -> Option<[f32; 4]> {
    let [r, g, b, a] = fx.screen;
    (a > 0.0).then(|| {
        [
            srgb_to_linear(r),
            srgb_to_linear(g),
            srgb_to_linear(b),
            a.clamp(0.0, 1.0),
        ]
    })
}

/// A character placement after the animations: camera, offset, scale,
/// rotation (radians) and tint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimatedPlacement {
    pub anchor: [f32; 2],
    pub scale: [f32; 2],
    pub rotation: f32,
    pub tint: [f32; 4],
}

pub fn animate_placement(
    placement: &CharacterPlacement,
    character: CharacterId,
    fx: &StageFx,
) -> AnimatedPlacement {
    let e = fx.characters.get(&character).copied().unwrap_or_default();
    let anchor = camera_point(
        [
            placement.anchor[0] + e.offset[0] * CANVAS[0],
            placement.anchor[1] + e.offset[1] * CANVAS[1],
        ],
        &fx.camera,
    );
    AnimatedPlacement {
        anchor,
        scale: [
            placement.scale[0] * e.scale[0] * fx.camera.zoom,
            placement.scale[1] * e.scale[1] * fx.camera.zoom,
        ],
        rotation: e.rotation.to_radians(),
        tint: entity_tint(&e),
    }
}

/// The background after the animations: center position, size, tint.
pub fn animate_background(size: [f32; 2], fx: &StageFx) -> ([f32; 2], [f32; 2], [f32; 4]) {
    let e = fx.background;
    let center = camera_point(
        [
            CANVAS[0] * 0.5 + e.offset[0] * CANVAS[0],
            CANVAS[1] * 0.5 + e.offset[1] * CANVAS[1],
        ],
        &fx.camera,
    );
    let z = fx.camera.zoom;
    (
        center,
        [size[0] * e.scale[0] * z, size[1] * e.scale[1] * z],
        entity_tint(&e),
    )
}

/// Parallax layers are drawn this much larger than the screen (a cover fit
/// times this), so a gentle pan never shows their edge.
pub const LAYER_OVERSCAN: f32 = 1.08;

/// One layer of a parallax background after the animations: it follows the
/// camera by `depth` (`1` = like the stage, `0` = fixed on the screen), and
/// otherwise takes the animations of the background. Center, size, tint.
pub fn animate_layer(
    size: [f32; 2],
    depth: f32,
    fx: &StageFx,
) -> ([f32; 2], [f32; 2], [f32; 4]) {
    let camera = CameraFx {
        offset: [fx.camera.offset[0] * depth, fx.camera.offset[1] * depth],
        zoom: (1.0 + (fx.camera.zoom - 1.0) * depth).max(0.01),
    };
    let e = fx.background;
    let center = camera_point(
        [
            CANVAS[0] * 0.5 + e.offset[0] * CANVAS[0],
            CANVAS[1] * 0.5 + e.offset[1] * CANVAS[1],
        ],
        &camera,
    );
    (
        center,
        [size[0] * e.scale[0] * camera.zoom, size[1] * e.scale[1] * camera.zoom],
        entity_tint(&e),
    )
}

/// The horizontal centers a repeating layer of width `width` is drawn at, so
/// that the canvas is covered: the tile that wraps around the center, and
/// its neighbours.
pub fn tiled_centers(center_x: f32, width: f32) -> Vec<f32> {
    if !(width > 1.0) {
        return vec![center_x];
    }
    let canvas_center = CANVAS[0] * 0.5;
    let wrapped = center_x - ((center_x - canvas_center) / width).round() * width;
    [-1.0, 0.0, 1.0].iter().map(|k| wrapped + k * width).collect()
}

/// A particle ready for the sprite batch: virtual pixels, radians, linear tint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleQuad {
    pub position: [f32; 2],
    /// Unrotated width and height.
    pub size: [f32; 2],
    pub rotation: f32,
    pub tint: [f32; 4],
}

/// Every live particle of the running emitters, in drawing order, split by
/// layer: `(behind the characters, over them)` plus the sprite of each emitter
/// (`None`: a plain quad). Capped at [`MAX_PARTICLES_TOTAL`] in all; screen
/// space (the camera does not move weather).
pub fn particle_quads(fx: &StageFx) -> Vec<(Option<AssetId>, bool, Vec<ParticleQuad>)> {
    let mut budget = MAX_PARTICLES_TOTAL;
    let mut out = Vec::new();
    let mut live: Vec<Particle> = Vec::new();
    for inst in &fx.emitters {
        if budget == 0 {
            break;
        }
        live.clear();
        inst.sample(&mut live);
        live.truncate(budget);
        budget -= live.len();
        let quads: Vec<ParticleQuad> = live
            .iter()
            .map(|p| {
                let width = p.size * CANVAS[1];
                let [r, g, b, a] = p.color;
                ParticleQuad {
                    position: [p.pos[0] * CANVAS[0], p.pos[1] * CANVAS[1]],
                    size: [width, width * p.stretch],
                    rotation: p.rotation.to_radians(),
                    tint: [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), a],
                }
            })
            .collect();
        if !quads.is_empty() {
            out.push((inst.emitter.sprite, inst.emitter.front, quads));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::Position;

    #[test]
    fn no_animation_leaves_the_placement_untouched() {
        let c = StageCharacter {
            character: CharacterId::from_u128(1),
            expression: "a".into(),
            image: AssetId::from_u128(1),
            x: 0.25,
            y: 1.0,
            scale: [1.0, 1.0],
            flip_x: false,
            flip_y: false,
        };
        let p = character_placement(&c, [100, 200]);
        let a = animate_placement(&p, c.character, &StageFx::default());
        assert_eq!(a.anchor, p.anchor);
        assert_eq!(a.scale, p.scale);
        assert_eq!(a.rotation, 0.0);
        assert_eq!(a.tint, [1.0, 1.0, 1.0, 1.0]);
        let (pos, size, tint) = animate_background([2000.0, 1100.0], &StageFx::default());
        assert_eq!((pos, size, tint), ([960.0, 540.0], [2000.0, 1100.0], [1.0; 4]));
        assert_eq!(screen_overlay(&StageFx::default()), None);
    }

    #[test]
    fn the_camera_zooms_around_the_center_and_pans() {
        let cam = CameraFx { offset: [0.1, 0.0], zoom: 2.0 };
        assert_eq!(camera_point([960.0, 540.0], &cam), [960.0 + 192.0, 540.0]);
        assert_eq!(camera_point([0.0, 0.0], &cam), [-960.0 + 192.0, -540.0]);
    }

    #[test]
    fn effects_offset_scale_rotate_and_tint_a_character() {
        let id = CharacterId::from_u128(7);
        let mut fx = StageFx::default();
        fx.characters.insert(
            id,
            EntityFx {
                offset: [0.1, -0.1],
                scale: [2.0, 1.0],
                rotation: 90.0,
                opacity: 0.5,
                tint: [1.0, 0.0, 0.0, 1.0],
            },
        );
        let p = CharacterPlacement {
            anchor: [100.0, 1000.0],
            size: [10.0, 20.0],
            scale: [1.5, 1.5],
            flip_x: false,
            flip_y: false,
        };
        let a = animate_placement(&p, id, &fx);
        assert_eq!(a.anchor, [292.0, 892.0]);
        assert_eq!(a.scale, [3.0, 1.5]);
        assert!((a.rotation - core::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(a.tint, [1.0, 0.0, 0.0, 0.5]);
        // Another character is untouched.
        let other = animate_placement(&p, CharacterId::from_u128(8), &fx);
        assert_eq!(other.anchor, p.anchor);
    }

    #[test]
    fn the_overlay_is_linear_and_hidden_at_zero_alpha() {
        let mut fx = StageFx::default();
        fx.screen = [1.0, 1.0, 1.0, 0.0];
        assert_eq!(screen_overlay(&fx), None);
        fx.screen = [1.0, 0.0, 0.0, 0.25];
        assert_eq!(screen_overlay(&fx), Some([1.0, 0.0, 0.0, 0.25]));
    }

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

    #[test]
    fn layers_replace_the_background_and_a_plain_background_replaces_the_layers() {
        let mut stage = StageState::default();
        let layers = vec![BgLayer::new(asset(1), 0.0), BgLayer::new(asset(2), 0.5)];
        let effect = stage.apply(&PresentationEvent::BackgroundLayers {
            layers: layers.clone(),
            transition: Some(Transition::dissolve(0.5)),
        });
        assert!(matches!(effect, StageEffect::Visual(Some(_))));
        assert_eq!(stage.bg_layers, layers);
        assert_eq!(stage.background, Some(asset(1)));
        stage.apply(&PresentationEvent::Background { asset: asset(3), transition: None });
        assert!(stage.bg_layers.is_empty());
        assert_eq!(stage.background, Some(asset(3)));
        // A 3D scene hides the layers' slot, then a 2D one gives it back.
        stage.apply(&PresentationEvent::BackgroundLayers { layers, transition: None });
        stage.apply(&PresentationEvent::Scene3d { scene: asset(9), camera: None, transition: None });
        assert!(stage.scene3d.is_some());
        stage.apply(&PresentationEvent::BackgroundLayers {
            layers: vec![BgLayer::new(asset(4), 1.0)],
            transition: None,
        });
        assert!(stage.scene3d.is_none());
    }

    #[test]
    fn a_layer_follows_the_camera_by_its_depth() {
        let mut fx = StageFx::default();
        fx.camera.offset = [0.1, 0.0];
        fx.camera.zoom = 1.5;
        let size = [1920.0, 1080.0];
        let (full, full_size, _) = animate_layer(size, 1.0, &fx);
        let (_, plain, _) = animate_background(size, &fx);
        assert_eq!(full_size, plain, "depth 1 is the plain background");
        assert_eq!(full[0], animate_background(size, &fx).0[0]);
        let (far, far_size, _) = animate_layer(size, 0.0, &fx);
        assert_eq!(far, [960.0, 540.0], "depth 0 stays glued to the screen");
        assert_eq!(far_size, size);
        let (mid, mid_size, _) = animate_layer(size, 0.5, &fx);
        assert!((mid[0] - (960.0 + 0.05 * 1920.0)).abs() < 1e-3);
        assert!((mid_size[0] - 1920.0 * 1.25).abs() < 1e-3);
        // A foreground layer moves faster than the stage.
        assert!(animate_layer(size, 2.0, &fx).0[0] > full[0]);
    }

    #[test]
    fn a_repeating_layer_is_tiled_around_the_canvas() {
        let centers = tiled_centers(960.0, 2000.0);
        assert_eq!(centers, vec![-1040.0, 960.0, 2960.0]);
        // Panned far to the right it wraps back: still one tile on the canvas center.
        let panned = tiled_centers(960.0 + 2000.0 * 7.0 + 300.0, 2000.0);
        assert!((panned[1] - 1260.0).abs() < 1e-2, "{panned:?}");
        assert_eq!(tiled_centers(100.0, 0.0), vec![100.0]);
    }

    #[test]
    fn particles_become_quads_in_virtual_pixels_within_the_budget() {
        use orbit_core::{Emitter, EmitterInstance, ParticlePreset};
        let mut fx = StageFx::default();
        assert!(particle_quads(&fx).is_empty());
        let rain = Emitter::preset(ParticlePreset::Rain);
        fx.emitters.push(EmitterInstance { emitter: rain.clone(), elapsed: 2.0, ends_at: None });
        let groups = particle_quads(&fx);
        assert_eq!(groups.len(), 1);
        let (sprite, front, quads) = &groups[0];
        assert!(sprite.is_none() && *front);
        assert!(quads.len() > 100);
        for q in quads {
            assert!(q.tint.iter().all(|c| (0.0..=1.0).contains(c)));
            assert!(q.size[1] > q.size[0], "rain streaks are long");
        }
        // The total is capped whatever the emitters ask for.
        let mut snow = Emitter::preset(ParticlePreset::Snow);
        snow.rate = 100.0;
        snow.lifetime = 9.0;
        for i in 0..6 {
            let mut e = snow.clone();
            e.seed = i;
            fx.emitters.push(EmitterInstance { emitter: e, elapsed: 30.0, ends_at: None });
        }
        let total: usize = particle_quads(&fx).iter().map(|(_, _, q)| q.len()).sum();
        assert_eq!(total, MAX_PARTICLES_TOTAL);
    }
}
