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

/// Background layer + character layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageState {
    pub background: Option<AssetId>,
    /// Back to front.
    pub characters: Vec<StageCharacter>,
}

/// What applying a presentation event did to the stage.
#[derive(Clone, Copy, Debug, PartialEq)]
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
                StageEffect::Visual(*transition)
            }
            PresentationEvent::Show {
                character,
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
                StageEffect::Visual(*transition)
            }
            PresentationEvent::Hide {
                character,
                transition,
            } => {
                self.characters.retain(|c| c.character != *character);
                StageEffect::Visual(*transition)
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
            transition: t,
        });
        assert_eq!(stage.background, Some(asset(1)));
        assert_eq!(effect, StageEffect::Visual(t));
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
}
