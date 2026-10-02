//! OrbitEngine audio: a thin wrapper over `kira` 0.12 (MIT/Apache-2.0).
//!
//! * music with looping, fade-in/fade-out and crossfade between two tracks;
//! * one-shot sound effects and voice lines;
//! * per-bus volumes: master / music / sfx / voice;
//! * decoded audio comes from `orbit_assets` ([`orbit_assets::PcmAudio`]: OGG
//!   Vorbis and WAV); kira's own decoders (symphonia, MPL-2.0) are disabled so
//!   the dependency tree stays permissive.
//!
//! # Autoplay policy (web)
//!
//! Browsers block audio until a user gesture. On wasm32 [`AudioEngine::new`]
//! therefore starts in [`AudioState::Locked`]: nothing is created yet, music
//! requests are remembered, effects are dropped. Call
//! [`AudioEngine::resume_after_user_gesture`] from an input handler (first
//! click/key); the engine then starts the audio device and plays the pending
//! music. If no audio device/context is available the engine goes to
//! [`AudioState::Unavailable`], logs once, and every call becomes a silent
//! no-op: it never panics.

use std::sync::Arc;
use std::time::Duration;

use kira::{
    AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Frame, Tween,
    sound::{
        PlaybackState,
        static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings},
    },
    track::{TrackBuilder, TrackHandle},
};
use orbit_assets::PcmAudio;

/// Lifecycle of the audio output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioState {
    /// Waiting for a user gesture ([`AudioEngine::resume_after_user_gesture`]).
    Locked,
    /// Output running.
    Running,
    /// No usable audio output; everything is a no-op.
    Unavailable,
}

/// Volume buses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Master,
    Music,
    Sfx,
    Voice,
}

impl Bus {
    fn index(self) -> usize {
        self as usize
    }
}

/// Errors building a [`Sound`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    #[error("audio has no samples, no channels or a zero sample rate")]
    Empty,
}

/// Decoded audio ready to be played. Cheap to clone (shared samples).
#[derive(Clone)]
pub struct Sound {
    data: StaticSoundData,
}

impl Sound {
    /// Converts decoded PCM. Mono is duplicated to both channels; more than two
    /// channels keep the first two.
    pub fn from_pcm(pcm: &PcmAudio) -> Result<Self, AudioError> {
        let ch = pcm.channels as usize;
        if ch == 0 || pcm.samples.len() < ch || pcm.sample_rate == 0 {
            return Err(AudioError::Empty);
        }
        let frames: Vec<Frame> = pcm
            .samples
            .chunks_exact(ch)
            .map(|c| {
                if ch == 1 {
                    Frame::from_mono(c[0])
                } else {
                    Frame::new(c[0], c[1])
                }
            })
            .collect();
        Ok(Self {
            data: StaticSoundData {
                sample_rate: pcm.sample_rate,
                frames: Arc::from(frames),
                settings: StaticSoundSettings::default(),
                slice: None,
            },
        })
    }

    pub fn duration(&self) -> Duration {
        self.data.duration()
    }

    pub fn frames(&self) -> usize {
        self.data.num_frames()
    }
}

/// How a music track starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MusicOptions {
    pub looping: bool,
    /// Fade-in of the new track (zero = immediate).
    pub fade_in: Duration,
    /// Fade-out applied to the track being replaced (zero = cut). Use the same
    /// value as `fade_in` for a crossfade.
    pub fade_out_previous: Duration,
}

impl Default for MusicOptions {
    fn default() -> Self {
        Self {
            looping: true,
            fade_in: Duration::ZERO,
            fade_out_previous: Duration::ZERO,
        }
    }
}

impl MusicOptions {
    /// Looping track that crossfades with the previous one over `d`.
    pub fn crossfade(d: Duration) -> Self {
        Self {
            looping: true,
            fade_in: d,
            fade_out_previous: d,
        }
    }
}

/// Linear gain (`0.0..`) to kira decibels; (near) zero is silence.
pub fn gain_to_db(gain: f32) -> Decibels {
    if gain.is_nan() || gain <= 0.0005 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * gain.log10()).max_silence()
    }
}

trait ClampDb {
    fn max_silence(self) -> Self;
}

impl ClampDb for Decibels {
    fn max_silence(self) -> Self {
        if self.0 < Decibels::SILENCE.0 {
            Decibels::SILENCE
        } else {
            self
        }
    }
}

fn tween(d: Duration) -> Tween {
    Tween {
        duration: d.max(Duration::from_millis(5)),
        ..Default::default()
    }
}

struct Tracks {
    music: TrackHandle,
    sfx: TrackHandle,
    voice: TrackHandle,
}

pub struct AudioEngine {
    state: AudioState,
    manager: Option<AudioManager<DefaultBackend>>,
    tracks: Option<Tracks>,
    volumes: [f32; 4],
    music: Option<StaticSoundHandle>,
    pending_music: Option<(Sound, MusicOptions)>,
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    /// Native: starts the output immediately. Web: starts [`AudioState::Locked`].
    pub fn new() -> Self {
        let mut engine = Self::new_locked();
        if !cfg!(target_arch = "wasm32") {
            engine.start();
        }
        engine
    }

    /// Always starts locked, on every platform (the same behavior as the web).
    pub fn new_locked() -> Self {
        Self {
            state: AudioState::Locked,
            manager: None,
            tracks: None,
            volumes: [1.0; 4],
            music: None,
            pending_music: None,
        }
    }

    pub fn state(&self) -> AudioState {
        self.state
    }

    /// Call from an input event handler (click, key, touch). Starts the audio
    /// output if it was locked; harmless otherwise.
    pub fn resume_after_user_gesture(&mut self) {
        if self.state == AudioState::Locked {
            self.start();
        }
    }

    fn start(&mut self) {
        let mut manager = match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
        {
            Ok(m) => m,
            Err(err) => {
                log::warn!("audio unavailable, running muted: {err}");
                self.state = AudioState::Unavailable;
                self.pending_music = None;
                return;
            }
        };
        let tracks = (|| {
            Some(Tracks {
                music: manager.add_sub_track(TrackBuilder::default()).ok()?,
                sfx: manager.add_sub_track(TrackBuilder::default()).ok()?,
                voice: manager.add_sub_track(TrackBuilder::default()).ok()?,
            })
        })();
        let Some(tracks) = tracks else {
            log::warn!("audio unavailable: could not create the mixer tracks");
            self.state = AudioState::Unavailable;
            self.pending_music = None;
            return;
        };
        self.manager = Some(manager);
        self.tracks = Some(tracks);
        self.state = AudioState::Running;
        log::info!("audio started");
        for bus in [Bus::Master, Bus::Music, Bus::Sfx, Bus::Voice] {
            self.apply_volume(bus, Duration::ZERO);
        }
        if let Some((sound, options)) = self.pending_music.take() {
            self.play_music(&sound, options);
        }
    }

    fn apply_volume(&mut self, bus: Bus, fade: Duration) {
        let db = gain_to_db(self.volumes[bus.index()]);
        let t = tween(fade);
        match (bus, self.manager.as_mut(), self.tracks.as_mut()) {
            (Bus::Master, Some(m), _) => m.main_track().set_volume(db, t),
            (Bus::Music, _, Some(tr)) => tr.music.set_volume(db, t),
            (Bus::Sfx, _, Some(tr)) => tr.sfx.set_volume(db, t),
            (Bus::Voice, _, Some(tr)) => tr.voice.set_volume(db, t),
            _ => {}
        }
    }

    /// Sets the linear volume (`0.0` silent, `1.0` full) of a bus. Works in
    /// any state: the value is applied when the output starts.
    pub fn set_bus_volume(&mut self, bus: Bus, volume: f32) {
        let v = if volume.is_nan() {
            0.0
        } else {
            volume.clamp(0.0, 4.0)
        };
        self.volumes[bus.index()] = v;
        self.apply_volume(bus, Duration::from_millis(30));
    }

    pub fn bus_volume(&self, bus: Bus) -> f32 {
        self.volumes[bus.index()]
    }

    /// Starts `sound` as the current music, replacing (and fading out) the
    /// previous one. While locked the request is remembered and started on
    /// unlock.
    pub fn play_music(&mut self, sound: &Sound, options: MusicOptions) {
        match self.state {
            AudioState::Locked => {
                self.pending_music = Some((sound.clone(), options));
            }
            AudioState::Unavailable => {}
            AudioState::Running => {
                if let Some(mut prev) = self.music.take() {
                    prev.stop(tween(options.fade_out_previous));
                }
                let mut data = sound.data.clone();
                if options.looping {
                    data = data.loop_region(..);
                }
                if !options.fade_in.is_zero() {
                    data = data.fade_in_tween(tween(options.fade_in));
                }
                if let Some(tracks) = self.tracks.as_mut() {
                    match tracks.music.play(data) {
                        Ok(handle) => self.music = Some(handle),
                        Err(err) => log::warn!("could not play music: {err}"),
                    }
                }
            }
        }
    }

    /// Fades the current music out over `fade` (zero = cut) and forgets it.
    pub fn stop_music(&mut self, fade: Duration) {
        self.pending_music = None;
        if let Some(mut h) = self.music.take() {
            h.stop(tween(fade));
        }
    }

    /// `true` while a music track is audibly playing (or fading).
    pub fn is_music_playing(&self) -> bool {
        self.music
            .as_ref()
            .is_some_and(|h| h.state() != PlaybackState::Stopped)
    }

    fn play_on(&mut self, bus: Bus, sound: &Sound, volume: f32) {
        if self.state != AudioState::Running {
            log::debug!("sound dropped: audio is {:?}", self.state);
            return;
        }
        let data = sound.data.clone().volume(gain_to_db(volume));
        if let Some(tracks) = self.tracks.as_mut() {
            let track = if bus == Bus::Voice {
                &mut tracks.voice
            } else {
                &mut tracks.sfx
            };
            if let Err(err) = track.play(data) {
                log::warn!("could not play sound: {err}");
            }
        }
    }

    /// Plays a one-shot sound effect (dropped while locked/unavailable).
    pub fn play_sfx(&mut self, sound: &Sound) {
        self.play_on(Bus::Sfx, sound, 1.0);
    }

    /// Like [`AudioEngine::play_sfx`] with a per-sound linear gain.
    pub fn play_sfx_with_volume(&mut self, sound: &Sound, volume: f32) {
        self.play_on(Bus::Sfx, sound, volume);
    }

    /// Plays a voice line on the voice bus.
    pub fn play_voice(&mut self, sound: &Sound) {
        self.play_on(Bus::Voice, sound, 1.0);
    }

    /// Housekeeping; call once per frame. Forgets finished music.
    pub fn update(&mut self) {
        if let Some(h) = &self.music
            && h.state() == PlaybackState::Stopped
        {
            self.music = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(channels: u16, frames: usize) -> PcmAudio {
        PcmAudio {
            sample_rate: 22_050,
            channels,
            samples: vec![0.25; frames * channels as usize],
        }
    }

    #[test]
    fn pcm_conversion() {
        assert_eq!(Sound::from_pcm(&pcm(1, 100)).unwrap().frames(), 100);
        assert_eq!(Sound::from_pcm(&pcm(2, 50)).unwrap().frames(), 50);
        assert_eq!(Sound::from_pcm(&pcm(6, 10)).unwrap().frames(), 10);
        assert!(Sound::from_pcm(&pcm(2, 0)).is_err());
        assert!(
            Sound::from_pcm(&PcmAudio {
                sample_rate: 0,
                channels: 1,
                samples: vec![0.0]
            })
            .is_err()
        );
    }

    #[test]
    fn gain_conversion() {
        assert_eq!(gain_to_db(0.0), Decibels::SILENCE);
        assert_eq!(gain_to_db(f32::NAN), Decibels::SILENCE);
        assert!((gain_to_db(1.0).0).abs() < 1e-6);
        assert!((gain_to_db(0.5).0 + 6.02).abs() < 0.01);
    }

    #[test]
    fn locked_engine_remembers_music_and_never_panics() {
        let mut e = AudioEngine::new_locked();
        assert_eq!(e.state(), AudioState::Locked);
        let s = Sound::from_pcm(&pcm(1, 22_050)).unwrap();
        e.play_music(&s, MusicOptions::crossfade(Duration::from_millis(500)));
        e.play_sfx(&s);
        e.set_bus_volume(Bus::Music, 0.5);
        assert_eq!(e.bus_volume(Bus::Music), 0.5);
        assert!(!e.is_music_playing());
        e.stop_music(Duration::ZERO);
        // On a machine without an audio device this ends Unavailable; with one
        // it ends Running. Both are fine, a panic is not.
        e.resume_after_user_gesture();
        assert_ne!(e.state(), AudioState::Locked);
        e.play_music(&s, MusicOptions::default());
        e.play_sfx(&s);
        e.update();
        e.stop_music(Duration::from_millis(10));
    }
}
