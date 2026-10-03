//! Effects and timelines (ADR 0012, schema v9): the data of the `Effect` and
//! `Timeline` nodes, and the pure composition of the running animations into a
//! [`StageFx`] the renderer applies on top of the staging.
//!
//! Nothing here keeps state or reads a clock: the caller passes the elapsed
//! time of each animation (`scene_clock - started_at`, tracked by the VM).
//! Animated values are **offsets relative to the staging**: offsets add,
//! scales / opacity / zoom multiply, tints are composited in order. A finished
//! animation keeps contributing its final value until the scene changes, so
//! two `Move` nodes in a row accumulate (move by `+0.2`, then by `-0.2` to go
//! back).

use std::collections::BTreeMap;

use orbit_anim::{Angle, Clip, Curve, Key, Loop, Rgba, Track, TrackData, Value, Vec2};
use serde::{Deserialize, Serialize};

use crate::defs::Color;
use crate::ids::CharacterId;
use crate::story::NodeKind;

/// What an animation track moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum AnimTarget {
    Character(CharacterId),
    Background,
    /// The whole stage (background and characters): pan, zoom, shake.
    Camera,
    /// A full-screen overlay (flash, tint).
    Screen,
}

/// The animated property of a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum AnimProp {
    /// `Vec2`, in stage units (1.0 = the screen size). Adds.
    Offset,
    /// `Vec2` multiplier. Multiplies.
    Scale,
    /// `Angle`, degrees. Adds.
    Rotation,
    /// `Scalar` multiplier, 1.0 = unchanged. Multiplies.
    Opacity,
    /// `Color` (rgb 0..1 and alpha = strength), composited in order.
    Tint,
    /// `Scalar` amplitude (stage units) of a deterministic shake.
    Shake,
    /// `Scalar` camera zoom factor, 1.0 = unchanged. Multiplies.
    Zoom,
    /// `Vec2` camera pan. Adds.
    Pan,
}

impl AnimProp {
    pub const ALL: [AnimProp; 8] = [
        AnimProp::Offset,
        AnimProp::Scale,
        AnimProp::Rotation,
        AnimProp::Opacity,
        AnimProp::Tint,
        AnimProp::Shake,
        AnimProp::Zoom,
        AnimProp::Pan,
    ];

    pub fn name(self) -> &'static str {
        match self {
            AnimProp::Offset => "offset",
            AnimProp::Scale => "scale",
            AnimProp::Rotation => "rotation",
            AnimProp::Opacity => "opacity",
            AnimProp::Tint => "tint",
            AnimProp::Shake => "shake",
            AnimProp::Zoom => "zoom",
            AnimProp::Pan => "pan",
        }
    }

    /// Value type of the track this property needs.
    pub fn accepts(self, data: &TrackData) -> bool {
        matches!(
            (self, data),
            (AnimProp::Offset | AnimProp::Scale | AnimProp::Pan, TrackData::Vec2(_))
                | (AnimProp::Rotation, TrackData::Angle(_))
                | (
                    AnimProp::Opacity | AnimProp::Shake | AnimProp::Zoom,
                    TrackData::Scalar(_)
                )
                | (AnimProp::Tint, TrackData::Color(_))
        )
    }

    /// Whether `target` can have this property.
    pub fn applies_to(self, target: &AnimTarget) -> bool {
        match target {
            AnimTarget::Character(_) => matches!(
                self,
                AnimProp::Offset
                    | AnimProp::Scale
                    | AnimProp::Rotation
                    | AnimProp::Opacity
                    | AnimProp::Tint
                    | AnimProp::Shake
            ),
            AnimTarget::Background => matches!(
                self,
                AnimProp::Offset
                    | AnimProp::Scale
                    | AnimProp::Opacity
                    | AnimProp::Tint
                    | AnimProp::Shake
            ),
            AnimTarget::Camera => {
                matches!(self, AnimProp::Pan | AnimProp::Zoom | AnimProp::Shake)
            }
            AnimTarget::Screen => matches!(self, AnimProp::Tint),
        }
    }
}

/// One track of a timeline: (target, property, keys).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StoryTrack {
    pub target: AnimTarget,
    pub prop: AnimProp,
    pub data: TrackData,
}

/// A simple effect, compiled to a one-or-two track timeline by
/// [`Effect::to_clip`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Effect {
    /// Moves a character or the background **by** `(dx, dy)` (stage units).
    Move {
        target: AnimTarget,
        dx: f32,
        dy: f32,
        duration: f32,
        #[serde(default)]
        ease: Option<Curve>,
        #[serde(default)]
        wait: bool,
    },
    /// Shakes the camera, a character or the background; the amplitude
    /// (stage units) decays to zero over `duration`.
    Shake {
        target: AnimTarget,
        amplitude: f32,
        duration: f32,
        #[serde(default)]
        ease: Option<Curve>,
        #[serde(default)]
        wait: bool,
    },
    /// Zooms the camera by `factor` (multiplies the current zoom).
    Zoom {
        factor: f32,
        duration: f32,
        #[serde(default)]
        ease: Option<Curve>,
        #[serde(default)]
        wait: bool,
    },
    /// A full-screen flash of `color` fading out.
    Flash {
        color: Color,
        duration: f32,
        #[serde(default)]
        ease: Option<Curve>,
        #[serde(default)]
        wait: bool,
    },
    /// Fades a tint of `color` (opacity `strength`, 0..=1) onto a character,
    /// the background or the screen. A strength of `0` clears an earlier tint
    /// of the same target.
    Tint {
        target: AnimTarget,
        color: Color,
        strength: f32,
        duration: f32,
        #[serde(default)]
        ease: Option<Curve>,
        #[serde(default)]
        wait: bool,
    },
}

fn rgba(c: Color, a: f32) -> Rgba {
    Rgba::new(
        f32::from(c.r) / 255.0,
        f32::from(c.g) / 255.0,
        f32::from(c.b) / 255.0,
        a,
    )
}

fn two_keys<T>(d: f32, from: T, to: T, ease: &Option<Curve>) -> Track<T> {
    Track::new(vec![
        Key::with_ease(0.0, from, ease.clone().unwrap_or_default()),
        Key::new(d, to),
    ])
}

impl Effect {
    pub fn duration(&self) -> f32 {
        match self {
            Effect::Move { duration, .. }
            | Effect::Shake { duration, .. }
            | Effect::Zoom { duration, .. }
            | Effect::Flash { duration, .. }
            | Effect::Tint { duration, .. } => *duration,
        }
    }

    /// Whether the story waits for the effect to finish.
    pub fn wait(&self) -> bool {
        match self {
            Effect::Move { wait, .. }
            | Effect::Shake { wait, .. }
            | Effect::Zoom { wait, .. }
            | Effect::Flash { wait, .. }
            | Effect::Tint { wait, .. } => *wait,
        }
    }

    pub fn ease(&self) -> Option<&Curve> {
        match self {
            Effect::Move { ease, .. }
            | Effect::Shake { ease, .. }
            | Effect::Zoom { ease, .. }
            | Effect::Flash { ease, .. }
            | Effect::Tint { ease, .. } => ease.as_ref(),
        }
    }

    /// The tracks this effect stands for (the single execution path: the
    /// runtime only knows timelines). Pure.
    pub fn tracks(&self) -> Vec<StoryTrack> {
        let d = self.duration().max(0.0);
        match self {
            Effect::Move { target, dx, dy, ease, .. } => vec![StoryTrack {
                target: *target,
                prop: AnimProp::Offset,
                data: TrackData::Vec2(two_keys(d, Vec2::new(0.0, 0.0), Vec2::new(*dx, *dy), ease)),
            }],
            Effect::Shake { target, amplitude, ease, .. } => vec![StoryTrack {
                target: *target,
                prop: AnimProp::Shake,
                data: TrackData::Scalar(two_keys(d, *amplitude, 0.0, ease)),
            }],
            Effect::Zoom { factor, ease, .. } => vec![StoryTrack {
                target: AnimTarget::Camera,
                prop: AnimProp::Zoom,
                data: TrackData::Scalar(two_keys(d, 1.0, *factor, ease)),
            }],
            Effect::Flash { color, ease, .. } => vec![StoryTrack {
                target: AnimTarget::Screen,
                prop: AnimProp::Tint,
                data: TrackData::Color(two_keys(d, rgba(*color, 1.0), rgba(*color, 0.0), ease)),
            }],
            Effect::Tint { target, color, strength, ease, .. } => vec![StoryTrack {
                target: *target,
                prop: AnimProp::Tint,
                data: TrackData::Color(two_keys(d, rgba(*color, 0.0), rgba(*color, *strength), ease)),
            }],
        }
    }

    /// The equivalent timeline node content: `(duration, looping, wait,
    /// tracks)` as stored by `NodeKind::Timeline`.
    pub fn to_timeline(&self) -> NodeKind {
        NodeKind::Timeline {
            duration: self.duration().max(0.0),
            looping: Loop::Once,
            wait: self.wait(),
            tracks: self.tracks(),
        }
    }

    /// A clip with `channel`s `"0"`, `"1"`... (one per track), for samplers
    /// that want the generic `orbit_anim` model.
    pub fn to_clip(&self) -> Clip {
        let mut clip = Clip::new(self.duration().max(0.0));
        clip.tracks = self
            .tracks()
            .into_iter()
            .enumerate()
            .map(|(i, t)| orbit_anim::ClipTrack { channel: i.to_string(), data: t.data })
            .collect();
        clip
    }
}

/// What an animating node plays: borrowed view of an `Effect` or a
/// `Timeline` node.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimSpec {
    pub duration: f32,
    pub looping: Loop,
    pub wait: bool,
    pub tracks: Vec<StoryTrack>,
}

impl AnimSpec {
    /// Total play time, `None` when infinite.
    pub fn total_duration(&self) -> Option<f32> {
        let mut clip = Clip::new(self.duration);
        clip.looping = self.looping;
        clip.total_duration()
    }

    /// Whether this animation has finished after `elapsed` seconds.
    pub fn is_finished(&self, elapsed: f32) -> bool {
        self.total_duration().is_some_and(|total| elapsed >= total)
    }
}

impl NodeKind {
    /// The animation of an `Effect` / `Timeline` node.
    pub fn animation(&self) -> Option<AnimSpec> {
        match self {
            NodeKind::Effect(e) => Some(AnimSpec {
                duration: e.duration().max(0.0),
                looping: Loop::Once,
                wait: e.wait(),
                tracks: e.tracks(),
            }),
            NodeKind::Timeline {
                duration,
                looping,
                wait,
                tracks,
            } => Some(AnimSpec {
                duration: duration.max(0.0),
                looping: *looping,
                wait: *wait,
                tracks: tracks.clone(),
            }),
            _ => None,
        }
    }

    /// Whether this animating node touches `character`.
    pub fn animates_character(&self, character: CharacterId) -> bool {
        let hit = |t: &AnimTarget| *t == AnimTarget::Character(character);
        match self {
            NodeKind::Effect(e) => e.tracks().iter().any(|t| hit(&t.target)),
            NodeKind::Timeline { tracks, .. } => tracks.iter().any(|t| hit(&t.target)),
            _ => false,
        }
    }
}

/// Composed animated state of one drawable (a character or the background).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityFx {
    /// Stage units.
    pub offset: [f32; 2],
    pub scale: [f32; 2],
    /// Degrees.
    pub rotation: f32,
    pub opacity: f32,
    /// Straight RGBA overlay (alpha 0 = none).
    pub tint: [f32; 4],
}

impl Default for EntityFx {
    fn default() -> Self {
        Self {
            offset: [0.0; 2],
            scale: [1.0; 2],
            rotation: 0.0,
            opacity: 1.0,
            tint: [0.0; 4],
        }
    }
}

impl EntityFx {
    pub fn is_identity(&self) -> bool {
        *self == EntityFx::default()
    }
}

/// Composed camera state. `offset` shifts the whole stage (pan + shake).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraFx {
    pub offset: [f32; 2],
    pub zoom: f32,
}

impl Default for CameraFx {
    fn default() -> Self {
        Self { offset: [0.0; 2], zoom: 1.0 }
    }
}

/// Everything the running animations add on top of the staging, this frame.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StageFx {
    pub characters: BTreeMap<CharacterId, EntityFx>,
    pub background: EntityFx,
    pub camera: CameraFx,
    /// Straight RGBA full-screen overlay.
    pub screen: [f32; 4],
}

/// Deterministic shake direction at `t` seconds, each component in
/// `-1.0..=1.0`. Sum of sines at fixed frequencies (`libm`: identical on
/// native and wasm), so a seek reproduces it exactly.
pub fn shake_wave(t: f32) -> [f32; 2] {
    let tau = core::f32::consts::TAU;
    let x = libm::sinf(tau * 17.0 * t) + 0.5 * libm::sinf(tau * 29.0 * t + 1.3);
    let y = libm::sinf(tau * 23.0 * t + 0.7) + 0.5 * libm::sinf(tau * 37.0 * t + 2.1);
    [x / 1.5, y / 1.5]
}

/// Straight-alpha "source over destination".
fn over(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
    let (sa, da) = (src[3].clamp(0.0, 1.0), dst[3]);
    let a = sa + da * (1.0 - sa);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let c = |i: usize| (src[i] * sa + dst[i] * da * (1.0 - sa)) / a;
    [c(0), c(1), c(2), a]
}

impl StageFx {
    /// Composes `items` (an animating node and the seconds since it
    /// started), in order. Nodes that are not animations are ignored.
    pub fn compose<'a>(
        curves: &BTreeMap<String, Curve>,
        items: impl IntoIterator<Item = (&'a NodeKind, f32)>,
    ) -> StageFx {
        let mut fx = StageFx::default();
        let resolve = |n: &str| curves.get(n);
        for (kind, elapsed) in items {
            let Some(spec) = kind.animation() else { continue };
            let mut clip = Clip::new(spec.duration);
            clip.looping = spec.looping;
            let t = clip.local_time(elapsed);
            for track in &spec.tracks {
                if !track.prop.applies_to(&track.target) {
                    continue;
                }
                let Some(value) = track.data.sample(t, resolve) else { continue };
                fx.apply(track.target, track.prop, value, t);
            }
        }
        fx
    }

    fn entity(&mut self, target: AnimTarget) -> Option<&mut EntityFx> {
        match target {
            AnimTarget::Character(c) => Some(self.characters.entry(c).or_default()),
            AnimTarget::Background => Some(&mut self.background),
            AnimTarget::Camera | AnimTarget::Screen => None,
        }
    }

    fn apply(&mut self, target: AnimTarget, prop: AnimProp, value: Value, t: f32) {
        match (prop, value) {
            (AnimProp::Offset, Value::Vec2(v)) => {
                if let Some(e) = self.entity(target) {
                    e.offset[0] += v.x;
                    e.offset[1] += v.y;
                }
            }
            (AnimProp::Scale, Value::Vec2(v)) => {
                if let Some(e) = self.entity(target) {
                    e.scale[0] *= v.x;
                    e.scale[1] *= v.y;
                }
            }
            (AnimProp::Rotation, Value::Angle(Angle(a))) => {
                if let Some(e) = self.entity(target) {
                    e.rotation += a;
                }
            }
            (AnimProp::Opacity, Value::Scalar(o)) => {
                if let Some(e) = self.entity(target) {
                    e.opacity *= o.max(0.0);
                }
            }
            (AnimProp::Tint, Value::Color(c)) => {
                let src = [c.r, c.g, c.b, c.a];
                match target {
                    AnimTarget::Screen => self.screen = over(self.screen, src),
                    _ => {
                        if let Some(e) = self.entity(target) {
                            e.tint = over(e.tint, src);
                        }
                    }
                }
            }
            (AnimProp::Shake, Value::Scalar(amp)) => {
                let [wx, wy] = shake_wave(t);
                let (dx, dy) = (amp * wx, amp * wy);
                match target {
                    AnimTarget::Camera => {
                        self.camera.offset[0] += dx;
                        self.camera.offset[1] += dy;
                    }
                    _ => {
                        if let Some(e) = self.entity(target) {
                            e.offset[0] += dx;
                            e.offset[1] += dy;
                        }
                    }
                }
            }
            (AnimProp::Zoom, Value::Scalar(z)) => self.camera.zoom *= z.max(0.01),
            (AnimProp::Pan, Value::Vec2(v)) => {
                self.camera.offset[0] += v.x;
                self.camera.offset[1] += v.y;
            }
            _ => {}
        }
    }

    /// True when nothing is animated (the staging is drawn untouched).
    pub fn is_identity(&self) -> bool {
        self.characters.values().all(EntityFx::is_identity)
            && self.background.is_identity()
            && self.camera == CameraFx::default()
            && self.screen[3] <= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::NodeId;

    fn mv(target: AnimTarget, dx: f32, wait: bool) -> NodeKind {
        NodeKind::Effect(Effect::Move { target, dx, dy: 0.0, duration: 2.0, ease: None, wait })
    }

    fn compose(items: &[(&NodeKind, f32)]) -> StageFx {
        StageFx::compose(&BTreeMap::new(), items.iter().copied())
    }

    #[test]
    fn a_move_goes_from_nothing_to_its_delta_and_stays() {
        let c = CharacterId::new();
        let node = mv(AnimTarget::Character(c), 0.4, true);
        let at = |t| compose(&[(&node, t)]).characters[&c].offset[0];
        assert_eq!(at(0.0), 0.0);
        assert!((at(1.0) - 0.2).abs() < 1e-6);
        assert_eq!(at(2.0), 0.4);
        assert_eq!(at(99.0), 0.4);
    }

    #[test]
    fn two_moves_accumulate() {
        let c = CharacterId::new();
        let a = mv(AnimTarget::Character(c), 0.4, false);
        let b = mv(AnimTarget::Character(c), -0.4, false);
        assert_eq!(compose(&[(&a, 9.0), (&b, 9.0)]).characters[&c].offset[0], 0.0);
    }

    #[test]
    fn shake_decays_to_exactly_zero() {
        let n = NodeKind::Effect(Effect::Shake {
            target: AnimTarget::Camera,
            amplitude: 0.05,
            duration: 1.0,
            ease: None,
            wait: false,
        });
        assert_ne!(compose(&[(&n, 0.13)]).camera.offset, [0.0, 0.0]);
        assert_eq!(compose(&[(&n, 1.0)]).camera.offset, [0.0, 0.0]);
        // Same time, same value.
        assert_eq!(compose(&[(&n, 0.3)]), compose(&[(&n, 0.3)]));
    }

    #[test]
    fn flash_fades_out_and_tint_stays() {
        let flash = NodeKind::Effect(Effect::Flash {
            color: Color::WHITE,
            duration: 1.0,
            ease: None,
            wait: false,
        });
        assert_eq!(compose(&[(&flash, 0.0)]).screen[3], 1.0);
        assert_eq!(compose(&[(&flash, 1.0)]).screen[3], 0.0);
        let tint = NodeKind::Effect(Effect::Tint {
            target: AnimTarget::Screen,
            color: Color::rgb(255, 0, 0),
            strength: 0.5,
            duration: 1.0,
            ease: None,
            wait: false,
        });
        let fx = compose(&[(&tint, 5.0)]);
        assert_eq!(fx.screen, [1.0, 0.0, 0.0, 0.5]);
        // A finished flash over a tint leaves the tint.
        assert_eq!(compose(&[(&tint, 5.0), (&flash, 5.0)]).screen, fx.screen);
    }

    #[test]
    fn zoom_multiplies_and_nodes_without_animation_are_ignored() {
        let z = NodeKind::Effect(Effect::Zoom { factor: 2.0, duration: 1.0, ease: None, wait: false });
        let end = NodeKind::End;
        let fx = compose(&[(&z, 1.0), (&z, 1.0), (&end, 3.0)]);
        assert_eq!(fx.camera.zoom, 4.0);
        assert!(StageFx::default().is_identity());
        let _ = NodeId::new();
    }

    #[test]
    fn invalid_target_property_pairs_are_ignored_by_compose() {
        let n = NodeKind::Timeline {
            duration: 1.0,
            looping: Loop::Once,
            wait: false,
            tracks: vec![StoryTrack {
                target: AnimTarget::Screen,
                prop: AnimProp::Offset,
                data: TrackData::Vec2(Track::new(vec![Key::new(0.0, Vec2::new(1.0, 1.0))])),
            }],
        };
        assert!(compose(&[(&n, 1.0)]).is_identity());
    }

    #[test]
    fn looping_timelines_wrap_and_zero_duration_jumps_to_the_end() {
        let tracks = vec![StoryTrack {
            target: AnimTarget::Background,
            prop: AnimProp::Offset,
            data: TrackData::Vec2(Track::new(vec![
                Key::new(0.0, Vec2::new(0.0, 0.0)),
                Key::new(1.0, Vec2::new(1.0, 0.0)),
            ])),
        }];
        let looped = NodeKind::Timeline {
            duration: 1.0,
            looping: Loop::Forever,
            wait: false,
            tracks: tracks.clone(),
        };
        let x = |t| compose(&[(&looped, t)]).background.offset[0];
        assert!((x(1.5) - 0.5).abs() < 1e-5);
        drop(tracks);
        let instant = NodeKind::Effect(Effect::Move {
            target: AnimTarget::Background,
            dx: 1.0,
            dy: 0.0,
            duration: 0.0,
            ease: None,
            wait: false,
        });
        assert_eq!(compose(&[(&instant, 0.0)]).background.offset[0], 1.0);
    }
}
