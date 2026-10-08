//! The music of the screens (`ui_version` 9, `ScreenAsset::music`): a small
//! state machine that turns "which screen is on top, is a game running" into
//! the audio operations to carry out. It owns no audio: the shell and the
//! editor preview apply the [`MusicOp`]s it returns, which keeps every
//! transition testable.
//!
//! Rules:
//! * the topmost open screen that declares music is the one that counts (a
//!   screen without music lets the one below keep playing);
//! * the same track on the next screen is not restarted (unless that screen
//!   turns `keep_if_same` off);
//! * over a running game a screen either replaces the game's music, which
//!   comes back when the screens close (`resume_game_music`), or only ducks
//!   it ([`OverGame::Duck`]);
//! * a `PlayMusic` / `StopMusic` button wins until the screen on top changes.

use orbit_ui::{OverGame, ScreenMusic};

/// Fade of a music started or stopped by a button.
const MANUAL_FADE_MS: u32 = 500;

/// One audio operation to carry out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MusicOp {
    /// Starts the music file `path`, fading the music being replaced out.
    Play {
        path: String,
        looping: bool,
        volume: f32,
        fade_in_ms: u32,
        fade_out_prev_ms: u32,
    },
    /// Fades the music out.
    Stop { fade_out_ms: u32 },
    /// Puts the game's own music back, fading it in (it was replaced by the
    /// music of a screen). The shell turns it into a play or a stop.
    RestoreGame { fade_in_ms: u32 },
    /// Plays the music at this fraction of its volume (`1.0` = normal).
    Duck(f32),
}

/// The screen whose music counts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Wanted<'a> {
    pub screen: &'a str,
    pub music: &'a ScreenMusic,
}

#[derive(Clone, Debug, PartialEq)]
struct Playing {
    path: String,
    screen: String,
    fade_out_ms: u32,
    resume: bool,
}

#[derive(Debug)]
pub(crate) struct UiMusic {
    playing: Option<Playing>,
    /// The track playing replaced the music of the running game.
    displaced: bool,
    duck: f32,
    /// A button started or stopped the music: it stays so until the screen
    /// on top changes.
    manual: bool,
    last_screen: Option<String>,
}

impl Default for UiMusic {
    fn default() -> Self {
        Self {
            playing: None,
            displaced: false,
            duck: 1.0,
            manual: false,
            last_screen: None,
        }
    }
}

impl UiMusic {
    /// The path of the track playing on behalf of the screens, if any.
    pub fn playing(&self) -> Option<&str> {
        self.playing.as_ref().map(|p| p.path.as_str())
    }

    /// Whether the game's music is currently replaced by a screen's.
    #[cfg(test)]
    pub fn displaced(&self) -> bool {
        self.displaced
    }

    /// Brings the audio in line with `wanted` (the topmost screen that
    /// declares music, if any) and whether a `game` is running; returns what
    /// to do (nothing when nothing changed).
    pub fn sync(&mut self, wanted: Option<Wanted<'_>>, game: bool) -> Vec<MusicOp> {
        let screen = wanted.map(|w| w.screen);
        if !game {
            // No game: there is no music to put back.
            self.displaced = false;
        }
        if self.manual {
            if self.last_screen.as_deref() == screen {
                return Vec::new();
            }
            self.manual = false;
        }
        self.last_screen = screen.map(str::to_owned);
        let mut ops = Vec::new();

        // Ducking: a Duck screen over a running game lowers its music.
        let duck_wanted = match wanted {
            Some(w) if game && w.music.over_game == OverGame::Duck => w.music.duck_volume.clamp(0.0, 1.0),
            _ => 1.0,
        };
        if (duck_wanted - self.duck).abs() > 1e-4 {
            self.duck = duck_wanted;
            ops.push(MusicOp::Duck(duck_wanted));
        }

        // The track.
        let track = wanted.filter(|w| !(w.music.asset.trim().is_empty() || game && w.music.over_game == OverGame::Duck));
        match (track, self.playing.as_mut()) {
            (Some(w), Some(p)) if p.path == w.music.asset && (p.screen == w.screen || w.music.keep_if_same) => {
                p.screen = w.screen.to_owned();
                p.fade_out_ms = w.music.fade_out_ms;
                p.resume = w.music.resume_game_music;
            }
            (Some(w), current) => {
                let m = w.music;
                let fade_out_prev_ms = current.as_ref().map_or(m.fade_in_ms, |p| p.fade_out_ms);
                if game && current.is_none() {
                    self.displaced = true;
                }
                ops.push(MusicOp::Play {
                    path: m.asset.clone(),
                    looping: m.looping,
                    volume: m.volume.clamp(0.0, 1.0),
                    fade_in_ms: m.fade_in_ms,
                    fade_out_prev_ms,
                });
                self.playing = Some(Playing {
                    path: m.asset.clone(),
                    screen: w.screen.to_owned(),
                    fade_out_ms: m.fade_out_ms,
                    resume: m.resume_game_music,
                });
            }
            (None, Some(_)) => {
                let p = self.playing.take().expect("checked above");
                if self.displaced && game && p.resume {
                    ops.push(MusicOp::RestoreGame {
                        fade_in_ms: p.fade_out_ms,
                    });
                } else {
                    ops.push(MusicOp::Stop {
                        fade_out_ms: p.fade_out_ms,
                    });
                }
                self.displaced = false;
            }
            (None, None) => {}
        }
        ops
    }

    /// A button asked for `path` to play (looping): it replaces whatever
    /// plays and stays until the screen on top changes.
    pub fn manual_play(&mut self, path: &str, game: bool) -> MusicOp {
        if game && self.playing.is_none() {
            self.displaced = true;
        }
        self.manual = true;
        self.playing = Some(Playing {
            path: path.to_owned(),
            screen: String::new(),
            fade_out_ms: MANUAL_FADE_MS,
            resume: true,
        });
        MusicOp::Play {
            path: path.to_owned(),
            looping: true,
            volume: 1.0,
            fade_in_ms: MANUAL_FADE_MS,
            fade_out_prev_ms: MANUAL_FADE_MS,
        }
    }

    /// A button asked for silence.
    pub fn manual_stop(&mut self) -> MusicOp {
        self.manual = true;
        self.playing = None;
        self.displaced = false;
        MusicOp::Stop {
            fade_out_ms: MANUAL_FADE_MS,
        }
    }

    /// The game's own music is about to be stopped or replaced (a game
    /// starts, loads, ends): forgets the screens' track. Returns `true` when
    /// one was playing (the caller stops the audio).
    pub fn forget_for_game_change(&mut self) -> bool {
        let was_playing = self.playing.take().is_some();
        self.displaced = false;
        self.manual = false;
        self.last_screen = None;
        was_playing
    }

    /// The ducking factor to restore after [`UiMusic::forget_for_game_change`]
    /// (`Some` when the music was ducked).
    pub fn take_duck(&mut self) -> Option<f32> {
        if (self.duck - 1.0).abs() > 1e-4 {
            self.duck = 1.0;
            Some(1.0)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn music(asset: &str) -> ScreenMusic {
        ScreenMusic {
            asset: asset.to_owned(),
            fade_in_ms: 300,
            fade_out_ms: 700,
            ..ScreenMusic::default()
        }
    }

    fn want<'a>(screen: &'a str, m: &'a ScreenMusic) -> Option<Wanted<'a>> {
        Some(Wanted { screen, music: m })
    }

    fn play(path: &str, fade_in: u32, prev: u32) -> MusicOp {
        MusicOp::Play {
            path: path.to_owned(),
            looping: true,
            volume: 1.0,
            fade_in_ms: fade_in,
            fade_out_prev_ms: prev,
        }
    }

    #[test]
    fn the_title_music_starts_once_and_stops_with_the_screen() {
        let mut u = UiMusic::default();
        let title = music("title.ogg");
        assert_eq!(u.sync(want("main_menu", &title), false), vec![play("title.ogg", 300, 300)]);
        // Nothing changes while the screen stays.
        assert!(u.sync(want("main_menu", &title), false).is_empty());
        assert_eq!(u.playing(), Some("title.ogg"));
        // The screen goes away: fade out with its own time.
        assert_eq!(u.sync(None, false), vec![MusicOp::Stop { fade_out_ms: 700 }]);
        assert_eq!(u.playing(), None);
        assert!(u.sync(None, false).is_empty());
    }

    #[test]
    fn a_screen_without_music_lets_the_one_below_keep_playing() {
        let mut u = UiMusic::default();
        let title = music("title.ogg");
        u.sync(want("main_menu", &title), false);
        // Preferences opened over the title declare nothing: the wanted
        // screen stays the main menu.
        assert!(u.sync(want("main_menu", &title), false).is_empty());
    }

    #[test]
    fn the_same_track_on_the_next_screen_is_kept_unless_the_screen_restarts_it() {
        let mut u = UiMusic::default();
        let a = music("theme.ogg");
        u.sync(want("main_menu", &a), false);
        // Same file, other screen: kept.
        assert!(u.sync(want("options", &a), false).is_empty());
        // That screen refuses to keep it: restarted.
        let restart = ScreenMusic {
            keep_if_same: false,
            ..music("theme.ogg")
        };
        assert_eq!(u.sync(want("credits", &restart), false), vec![play("theme.ogg", 300, 700)]);
        // Another file crossfades from the old track's fade-out time.
        let b = music("other.ogg");
        assert_eq!(u.sync(want("extras", &b), false), vec![play("other.ogg", 300, 700)]);
    }

    #[test]
    fn a_menu_over_the_game_replaces_the_music_and_the_game_music_comes_back() {
        let mut u = UiMusic::default();
        let pause = music("pause.ogg");
        let ops = u.sync(want("pause", &pause), true);
        assert_eq!(ops, vec![play("pause.ogg", 300, 300)]);
        assert!(u.displaced());
        // Closing: the game's own music is restored with the fade.
        assert_eq!(u.sync(None, true), vec![MusicOp::RestoreGame { fade_in_ms: 700 }]);
        assert!(!u.displaced() && u.playing().is_none());
        // Opening again works the same.
        assert_eq!(u.sync(want("pause", &pause), true).len(), 1);
    }

    #[test]
    fn without_resume_the_music_just_fades_out() {
        let mut u = UiMusic::default();
        let pause = ScreenMusic {
            resume_game_music: false,
            ..music("pause.ogg")
        };
        u.sync(want("pause", &pause), true);
        assert_eq!(u.sync(None, true), vec![MusicOp::Stop { fade_out_ms: 700 }]);
    }

    #[test]
    fn a_stacked_menu_restores_the_game_music_only_when_all_close() {
        let mut u = UiMusic::default();
        let pause = music("pause.ogg");
        let prefs = music("prefs.ogg");
        u.sync(want("pause", &pause), true);
        // Preferences on top have their own track: a crossfade, still
        // displaced.
        assert_eq!(u.sync(want("prefs", &prefs), true), vec![play("prefs.ogg", 300, 700)]);
        // Back to the pause menu: its track returns.
        assert_eq!(u.sync(want("pause", &pause), true), vec![play("pause.ogg", 300, 700)]);
        assert_eq!(u.sync(None, true), vec![MusicOp::RestoreGame { fade_in_ms: 700 }]);
    }

    #[test]
    fn a_duck_screen_lowers_the_game_music_and_plays_nothing() {
        let mut u = UiMusic::default();
        let pause = ScreenMusic {
            over_game: OverGame::Duck,
            duck_volume: 0.25,
            ..music("ignored.ogg")
        };
        assert_eq!(u.sync(want("pause", &pause), true), vec![MusicOp::Duck(0.25)]);
        assert!(u.playing().is_none());
        assert!(u.sync(want("pause", &pause), true).is_empty());
        assert_eq!(u.sync(None, true), vec![MusicOp::Duck(1.0)]);
        // On the title the same screen is a plain replacement.
        let ops = u.sync(want("pause", &pause), false);
        assert_eq!(ops, vec![play("ignored.ogg", 300, 300)]);
        assert!(!u.displaced());
    }

    #[test]
    fn leaving_the_game_does_not_restore_anything_and_the_title_music_takes_over() {
        let mut u = UiMusic::default();
        let pause = music("pause.ogg");
        u.sync(want("pause", &pause), true);
        // The player returns to the title: the shell forgets the track
        // (it stops the audio itself) ...
        assert!(u.forget_for_game_change());
        // ... and the title screen starts its own music.
        let title = music("title.ogg");
        assert_eq!(u.sync(want("main_menu", &title), false), vec![play("title.ogg", 300, 300)]);
        // Starting a game from the title: the track is forgotten too.
        assert!(u.forget_for_game_change());
        assert!(u.sync(None, true).is_empty(), "nothing to restore, nothing to stop");
    }

    #[test]
    fn a_button_decides_until_the_screen_on_top_changes() {
        let mut u = UiMusic::default();
        let title = music("title.ogg");
        u.sync(want("main_menu", &title), false);
        assert!(matches!(u.manual_play("jingle.ogg", false), MusicOp::Play { .. }));
        assert_eq!(u.playing(), Some("jingle.ogg"));
        // The screen did not change: left alone.
        assert!(u.sync(want("main_menu", &title), false).is_empty());
        assert_eq!(u.manual_stop(), MusicOp::Stop { fade_out_ms: 500 });
        assert!(u.sync(want("main_menu", &title), false).is_empty());
        // Another screen on top: the normal rules apply again.
        let other = music("other.ogg");
        assert_eq!(u.sync(want("extras", &other), false), vec![play("other.ogg", 300, 300)]);
    }

    #[test]
    fn the_ducking_is_cleared_when_the_game_changes() {
        let mut u = UiMusic::default();
        let pause = ScreenMusic {
            over_game: OverGame::Duck,
            ..ScreenMusic::default()
        };
        u.sync(want("pause", &pause), true);
        assert_eq!(u.take_duck(), Some(1.0));
        assert_eq!(u.take_duck(), None);
    }
}
