//! 2D particles and parallax background layers (ADR 0012 addendum, M5A-07 /
//! M5A-08, schema v13).
//!
//! Particles are **stateless**: particle `i` of an emitter is born at
//! `i / rate`, every one of its parameters comes from an integer hash of
//! `(seed, i, salt)` and its position at age `a` is analytic
//! (`p0 + v*a + g*a^2/2` plus a sinusoidal sway). Nothing is accumulated from
//! frame to frame and no random generator of the VM is consumed, so rollback,
//! saves, seeking and skipping only have to know *when the emitter started*
//! (`ActiveAnim::started_at`): the particles are recomputed from that.
//!
//! Everything is deterministic and identical on native and wasm: integer
//! hashing, `f64` for the birth times and `libm` for the transcendental
//! functions.

use serde::{Deserialize, Serialize};

use crate::defs::Color;
use crate::ids::AssetId;

/// Upper bound of live particles per emitter (the WebGL2 budget: a few
/// thousand instanced quads per frame, shared by every emitter).
pub const MAX_PARTICLES_PER_EMITTER: usize = 1500;

/// Upper bound of live particles drawn in one frame, all emitters together.
pub const MAX_PARTICLES_TOTAL: usize = 4000;

/// One layer of a parallax background (`NodeKind::BackgroundLayers`).
///
/// `depth` is how much the layer follows the camera: `1.0` moves exactly
/// like the stage (a plain background), `0.0` is glued to the screen (the
/// sky), values in between give the parallax, and above `1.0` a foreground
/// that moves faster than the stage. A layer is shifted by
/// `camera pan * depth` and zoomed by `1 + (zoom - 1) * depth`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BgLayer {
    pub asset: AssetId,
    pub depth: f32,
    /// Tiles the layer horizontally (it wraps when the camera pans).
    #[serde(default)]
    pub repeat_x: bool,
}

impl BgLayer {
    pub fn new(asset: AssetId, depth: f32) -> Self {
        Self { asset, depth, repeat_x: false }
    }
}

/// The preset an emitter was made from. It is a label for the editor (and
/// the DSL): the emitter's own parameters are what plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ParticlePreset {
    Rain,
    Snow,
    Petals,
    Dust,
    Sparks,
    #[default]
    Custom,
}

impl ParticlePreset {
    pub const ALL: [ParticlePreset; 6] = [
        ParticlePreset::Rain,
        ParticlePreset::Snow,
        ParticlePreset::Petals,
        ParticlePreset::Dust,
        ParticlePreset::Sparks,
        ParticlePreset::Custom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ParticlePreset::Rain => "rain",
            ParticlePreset::Snow => "snow",
            ParticlePreset::Petals => "petals",
            ParticlePreset::Dust => "dust",
            ParticlePreset::Sparks => "sparks",
            ParticlePreset::Custom => "custom",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// A particle emitter: the data of the `Particles` node.
///
/// Lengths are in *stage units* (1.0 = the screen width horizontally, the
/// screen height vertically), times in seconds. Angles are degrees on screen
/// (`0` = right, `90` = down, `270` = up).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Emitter {
    /// Names the emitter so a `StopParticles` node (or a later `Particles`
    /// node with the same name, which replaces it) can find it.
    pub name: String,
    #[serde(default)]
    pub preset: ParticlePreset,
    /// Makes two emitters of the same preset look different.
    #[serde(default)]
    pub seed: u32,
    /// Particles born per second.
    pub rate: f32,
    /// Seconds a particle lives.
    pub lifetime: f32,
    /// Random variation of the lifetime, `0.0..=1.0`.
    #[serde(default)]
    pub lifetime_jitter: f32,
    /// Where particles are born: `[x0, y0, x1, y1]` in stage units.
    pub spawn: [f32; 4],
    /// Direction of the initial velocity, degrees.
    pub direction: f32,
    /// Random deviation from `direction` (plus or minus), degrees.
    #[serde(default)]
    pub spread: f32,
    /// Initial speed range `[min, max]`, stage units per second.
    pub speed: [f32; 2],
    /// Constant acceleration, stage units per second squared.
    #[serde(default)]
    pub gravity: [f32; 2],
    /// Sideways sway `[amplitude (stage units), frequency (Hz)]`.
    #[serde(default)]
    pub sway: [f32; 2],
    /// Size range `[min, max]` of the particle width, as a fraction of the
    /// screen height.
    pub size: [f32; 2],
    /// Length of the particle along its movement, in widths (`1.0` is a
    /// square, rain streaks are longer).
    #[serde(default = "one")]
    pub stretch: f32,
    /// Random spin, plus or minus degrees per second (ignored when
    /// `stretch` aligns the particle with its movement).
    #[serde(default)]
    pub spin: f32,
    /// Color at birth and at death.
    pub color: Color,
    pub color_end: Color,
    /// Peak opacity, `0.0..=1.0`.
    #[serde(default = "one")]
    pub opacity: f32,
    /// Fraction of the lifetime spent fading in, and again fading out
    /// (`0.0..=0.5`).
    #[serde(default)]
    pub fade: f32,
    /// A picture for the particles (a plain quad when `None`).
    #[serde(default)]
    pub sprite: Option<AssetId>,
    /// Drawn over the characters instead of behind them.
    #[serde(default)]
    pub front: bool,
    /// Starts already running (as if it had been emitting for a full
    /// lifetime) instead of filling the screen progressively.
    #[serde(default)]
    pub prewarm: bool,
}

fn one() -> f32 {
    1.0
}

/// One live particle, ready to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    /// Center, stage units.
    pub pos: [f32; 2],
    /// Width as a fraction of the screen height.
    pub size: f32,
    /// Length / width.
    pub stretch: f32,
    /// Degrees, clockwise on screen.
    pub rotation: f32,
    /// Straight sRGB RGBA, alpha included.
    pub color: [f32; 4],
}

/// Hash of `(seed, index, salt)` to `0.0..1.0` (24 bits, exact in `f32`).
pub fn hash01(seed: u32, index: i64, salt: u32) -> f32 {
    let mut z = (u64::from(seed) << 32)
        ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ u64::from(salt).wrapping_mul(0xD6E8_FEB8_6659_FD93);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / 16_777_216.0
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn channel(c: u8) -> f32 {
    f32::from(c) / 255.0
}

impl Emitter {
    /// An emitter with the parameters of `preset`.
    pub fn preset(preset: ParticlePreset) -> Emitter {
        let base = Emitter {
            name: preset.name().to_owned(),
            preset,
            seed: 1,
            rate: 20.0,
            lifetime: 4.0,
            lifetime_jitter: 0.2,
            spawn: [0.0, 0.0, 1.0, 1.0],
            direction: 90.0,
            spread: 10.0,
            speed: [0.05, 0.1],
            gravity: [0.0, 0.0],
            sway: [0.0, 0.0],
            size: [0.006, 0.012],
            stretch: 1.0,
            spin: 0.0,
            color: Color::WHITE,
            color_end: Color::WHITE,
            opacity: 0.8,
            fade: 0.15,
            sprite: None,
            front: true,
            prewarm: true,
        };
        match preset {
            ParticlePreset::Custom => base,
            ParticlePreset::Rain => Emitter {
                rate: 170.0,
                lifetime: 0.9,
                lifetime_jitter: 0.25,
                spawn: [-0.1, -0.12, 1.1, -0.04],
                direction: 98.0,
                spread: 2.0,
                speed: [1.1, 1.5],
                size: [0.0035, 0.006],
                stretch: 7.0,
                color: Color { r: 190, g: 210, b: 255 },
                color_end: Color { r: 170, g: 190, b: 240 },
                opacity: 0.45,
                fade: 0.05,
                ..base
            },
            ParticlePreset::Snow => Emitter {
                rate: 60.0,
                lifetime: 9.0,
                lifetime_jitter: 0.3,
                spawn: [-0.05, -0.05, 1.05, -0.02],
                direction: 90.0,
                spread: 15.0,
                speed: [0.06, 0.14],
                sway: [0.03, 0.3],
                size: [0.006, 0.014],
                opacity: 0.85,
                fade: 0.08,
                ..base
            },
            ParticlePreset::Petals => Emitter {
                rate: 12.0,
                lifetime: 10.0,
                lifetime_jitter: 0.3,
                spawn: [-0.1, -0.05, 0.9, -0.02],
                direction: 60.0,
                spread: 20.0,
                speed: [0.05, 0.12],
                sway: [0.06, 0.25],
                size: [0.01, 0.018],
                spin: 90.0,
                color: Color { r: 255, g: 183, b: 197 },
                color_end: Color { r: 255, g: 150, b: 170 },
                opacity: 0.9,
                fade: 0.08,
                ..base
            },
            ParticlePreset::Dust => Emitter {
                rate: 20.0,
                lifetime: 8.0,
                lifetime_jitter: 0.4,
                direction: 270.0,
                spread: 180.0,
                speed: [0.005, 0.02],
                sway: [0.01, 0.15],
                size: [0.002, 0.005],
                color: Color { r: 255, g: 244, b: 214 },
                color_end: Color { r: 255, g: 244, b: 214 },
                opacity: 0.5,
                fade: 0.4,
                front: false,
                ..base
            },
            ParticlePreset::Sparks => Emitter {
                rate: 40.0,
                lifetime: 1.2,
                lifetime_jitter: 0.5,
                spawn: [0.45, 0.85, 0.55, 0.9],
                direction: 270.0,
                spread: 40.0,
                speed: [0.3, 0.7],
                gravity: [0.0, 0.8],
                size: [0.004, 0.008],
                stretch: 3.0,
                color: Color { r: 255, g: 220, b: 120 },
                color_end: Color { r: 255, g: 80, b: 20 },
                opacity: 1.0,
                fade: 0.3,
                prewarm: false,
                ..base
            },
        }
    }

    /// The longest a particle lives, seconds.
    pub fn max_lifetime(&self) -> f32 {
        self.lifetime * (1.0 + self.lifetime_jitter.clamp(0.0, 1.0))
    }

    /// How many particles can be alive at once (rate x longest lifetime).
    pub fn peak_count(&self) -> f32 {
        self.rate.max(0.0) * self.max_lifetime().max(0.0)
    }

    /// Why the emitter cannot be played, `None` when it is fine.
    pub fn problem(&self) -> Option<String> {
        let finite = |xs: &[f32]| xs.iter().all(|x| x.is_finite());
        if !(finite(&self.spawn)
            && finite(&self.speed)
            && finite(&self.gravity)
            && finite(&self.sway)
            && finite(&self.size)
            && finite(&[
                self.rate,
                self.lifetime,
                self.lifetime_jitter,
                self.direction,
                self.spread,
                self.stretch,
                self.spin,
                self.opacity,
                self.fade,
            ]))
        {
            return Some("a parameter is not a finite number".to_owned());
        }
        if self.rate <= 0.0 {
            return Some("the rate must be above 0".to_owned());
        }
        if self.lifetime <= 0.0 {
            return Some("the lifetime must be above 0".to_owned());
        }
        if self.size[0] <= 0.0 || self.size[1] < self.size[0] {
            return Some("the size range must be positive and ordered".to_owned());
        }
        if self.speed[1] < self.speed[0] {
            return Some("the speed range must be ordered".to_owned());
        }
        if self.peak_count() > MAX_PARTICLES_PER_EMITTER as f32 {
            return Some(format!(
                "about {} particles alive at once exceeds the limit of {MAX_PARTICLES_PER_EMITTER} (lower the rate or the lifetime)",
                self.peak_count().ceil()
            ));
        }
        None
    }

    /// The live particles `elapsed` seconds after the emitter started, pushed
    /// to `out`. `ends_at` is the emitter time at which a `StopParticles`
    /// stopped the births (live particles finish their life).
    ///
    /// A pure function of its arguments: calling it twice, or after a
    /// rollback, gives the same particles bit for bit.
    pub fn sample(&self, elapsed: f32, ends_at: Option<f32>, out: &mut Vec<Particle>) {
        if self.problem().is_some() {
            return;
        }
        let life_max = f64::from(self.max_lifetime());
        let rate = f64::from(self.rate);
        let shift = if self.prewarm { life_max } else { 0.0 };
        let t = f64::from(elapsed.max(0.0)) + shift;
        let birth_limit = ends_at.map_or(t, |e| t.min(f64::from(e.max(0.0)) + shift));
        let last = (birth_limit * rate).floor() as i64;
        let first = (((t - life_max) * rate).ceil() as i64).max(0);
        let first = first.max(last - MAX_PARTICLES_PER_EMITTER as i64 + 1);
        let tau = core::f32::consts::TAU;
        for i in first..=last {
            let age = (t - i as f64 / rate) as f32;
            let jitter = self.lifetime_jitter.clamp(0.0, 1.0);
            let life = self.lifetime * (1.0 + jitter * (2.0 * hash01(self.seed, i, 7) - 1.0));
            if age < 0.0 || age >= life {
                continue;
            }
            let h = |salt: u32| hash01(self.seed, i, salt);
            let x0 = lerp(self.spawn[0], self.spawn[2], h(1));
            let y0 = lerp(self.spawn[1], self.spawn[3], h(2));
            let angle = (self.direction + self.spread * (2.0 * h(3) - 1.0)).to_radians();
            let speed = lerp(self.speed[0], self.speed[1], h(4));
            let (vx, vy) = (libm::cosf(angle) * speed, libm::sinf(angle) * speed);
            let phase = h(8) * tau;
            let sway = self.sway[0] * libm::sinf(tau * self.sway[1] * age + phase);
            let pos = [
                x0 + vx * age + 0.5 * self.gravity[0] * age * age + sway,
                y0 + vy * age + 0.5 * self.gravity[1] * age * age,
            ];
            let size = lerp(self.size[0], self.size[1], h(5));
            let rotation = if self.stretch > 1.0 {
                // The long axis follows the velocity at this age.
                let (ux, uy) = (vx + self.gravity[0] * age, vy + self.gravity[1] * age);
                libm::atan2f(-ux, uy).to_degrees()
            } else {
                self.spin * (2.0 * h(6) - 1.0) * age + h(9) * 360.0 * f32::from(self.spin != 0.0)
            };
            let u = (age / life).clamp(0.0, 1.0);
            let fade = self.fade.clamp(0.0, 0.5);
            let envelope = if fade > 0.0 { (u / fade).min((1.0 - u) / fade).min(1.0) } else { 1.0 };
            let mix = |a: u8, b: u8| lerp(channel(a), channel(b), u);
            out.push(Particle {
                pos,
                size,
                stretch: self.stretch.max(0.01),
                rotation,
                color: [
                    mix(self.color.r, self.color_end.r),
                    mix(self.color.g, self.color_end.g),
                    mix(self.color.b, self.color_end.b),
                    (self.opacity.clamp(0.0, 1.0) * envelope).clamp(0.0, 1.0),
                ],
            });
        }
    }
}

/// A running emitter this frame: the data of [`StageFx::emitters`](crate::StageFx).
#[derive(Clone, Debug, PartialEq)]
pub struct EmitterInstance {
    pub emitter: Emitter,
    /// Seconds since the `Particles` node ran.
    pub elapsed: f32,
    /// Emitter time at which a `StopParticles` stopped the births.
    pub ends_at: Option<f32>,
}

impl EmitterInstance {
    pub fn sample(&self, out: &mut Vec<Particle>) {
        self.emitter.sample(self.elapsed, self.ends_at, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(e: &Emitter, t: f32) -> Vec<Particle> {
        let mut out = Vec::new();
        e.sample(t, None, &mut out);
        out
    }

    #[test]
    fn the_hash_is_stable_and_in_range() {
        assert_eq!(hash01(1, 2, 3), hash01(1, 2, 3));
        assert_ne!(hash01(1, 2, 3), hash01(2, 2, 3));
        assert_ne!(hash01(1, 2, 3), hash01(1, 3, 3));
        for i in -50..50 {
            let h = hash01(9, i, 4);
            assert!((0.0..1.0).contains(&h));
        }
        // Frozen: a change here would change every saved scene's particles.
        assert_eq!(hash01(1, 0, 0).to_bits(), hash01(1, 0, 0).to_bits());
        assert_eq!(hash01(0, 0, 0), 0.8833108);
    }

    #[test]
    fn sampling_is_a_pure_function_of_time() {
        let e = Emitter::preset(ParticlePreset::Snow);
        let a = sample(&e, 3.7);
        let b = sample(&e, 3.7);
        assert_eq!(a, b);
        assert!(!a.is_empty());
        // Going back in time gives the same frame as the first time.
        let _later = sample(&e, 20.0);
        assert_eq!(sample(&e, 3.7), a);
    }

    #[test]
    fn a_particle_follows_its_closed_form_path() {
        let mut e = Emitter::preset(ParticlePreset::Custom);
        e.prewarm = false;
        e.rate = 1.0;
        e.lifetime = 100.0;
        e.lifetime_jitter = 0.0;
        e.spawn = [0.5, 0.0, 0.5, 0.0];
        e.direction = 90.0;
        e.spread = 0.0;
        e.speed = [0.1, 0.1];
        e.gravity = [0.0, 0.2];
        e.sway = [0.0, 0.0];
        e.fade = 0.0;
        // Particle 0 is born at t=0: at t=2 it is at y = 0.1*2 + 0.5*0.2*4.
        let p = sample(&e, 2.0);
        let first = p[0];
        assert!((first.pos[0] - 0.5).abs() < 1e-5);
        assert!((first.pos[1] - 0.6).abs() < 1e-5, "{}", first.pos[1]);
        assert_eq!(p.len(), 3, "births at 0, 1 and 2 s");
    }

    #[test]
    fn prewarm_starts_full_and_a_cold_start_fills_up() {
        let mut e = Emitter::preset(ParticlePreset::Snow);
        e.prewarm = true;
        let warm = sample(&e, 0.0).len();
        e.prewarm = false;
        let cold = sample(&e, 0.0).len();
        assert!(warm > 100, "{warm}");
        assert!(cold <= 1);
        // Both reach the same steady state.
        e.prewarm = false;
        let later = sample(&e, 60.0).len();
        assert!((later as i64 - warm as i64).abs() < 90);
    }

    #[test]
    fn stopping_ends_the_births_and_lets_the_rest_finish() {
        let mut e = Emitter::preset(ParticlePreset::Snow);
        e.prewarm = false;
        let mut running = Vec::new();
        e.sample(20.0, None, &mut running);
        let mut stopped = Vec::new();
        e.sample(20.0, Some(5.0), &mut stopped);
        assert!(stopped.len() < running.len());
        // Long after the stop everything has died.
        let mut gone = Vec::new();
        e.sample(40.0, Some(5.0), &mut gone);
        assert!(gone.is_empty());
    }

    #[test]
    fn presets_are_valid_and_within_budget() {
        for p in ParticlePreset::ALL {
            let e = Emitter::preset(p);
            assert_eq!(e.problem(), None, "{p:?}");
            assert!(e.peak_count() <= MAX_PARTICLES_PER_EMITTER as f32);
            assert_eq!(ParticlePreset::from_name(p.name()), Some(p));
        }
        let mut e = Emitter::preset(ParticlePreset::Rain);
        e.rate = 5000.0;
        assert!(e.problem().is_some());
        e.rate = f32::NAN;
        assert!(e.problem().is_some());
        // An invalid emitter draws nothing instead of panicking.
        assert!(sample(&e, 1.0).is_empty());
    }

    #[test]
    fn colors_and_opacity_stay_in_range() {
        for p in ParticlePreset::ALL {
            let e = Emitter::preset(p);
            for t in [0.0, 0.3, 2.0, 9.0] {
                for q in sample(&e, t) {
                    assert!(q.color.iter().all(|c| (0.0..=1.0).contains(c)), "{p:?}");
                    assert!(q.pos.iter().all(|c| c.is_finite()));
                }
            }
        }
    }
}
