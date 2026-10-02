//! Typewriter effect: reveals graphemes over time, with pauses.

use crate::markup::Pause;

/// Time-driven reveal of `total` graphemes. Deterministic: it only advances
/// through [`Typewriter::update`].
#[derive(Debug, Clone)]
pub struct Typewriter {
    total: usize,
    pauses: Vec<Pause>,
    next_pause: usize,
    pause_left: f32,
    shown: f32,
    chars_per_second: f32,
}

impl Typewriter {
    /// `pauses` need not be sorted. A non-positive `chars_per_second` freezes
    /// the text; infinity (or any huge value) reveals everything at once.
    pub fn new(total: usize, mut pauses: Vec<Pause>, chars_per_second: f32) -> Self {
        pauses.sort_by_key(|p| p.grapheme);
        pauses.retain(|p| p.grapheme <= total);
        Self {
            total,
            pauses,
            next_pause: 0,
            pause_left: 0.0,
            shown: 0.0,
            chars_per_second: if chars_per_second.is_nan() {
                0.0
            } else {
                chars_per_second
            },
        }
    }

    pub fn set_speed(&mut self, chars_per_second: f32) {
        self.chars_per_second = if chars_per_second.is_nan() {
            0.0
        } else {
            chars_per_second
        };
    }

    pub fn speed(&self) -> f32 {
        self.chars_per_second
    }

    /// Number of graphemes currently visible.
    pub fn revealed(&self) -> usize {
        (self.shown.floor().max(0.0) as usize).min(self.total)
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn is_complete(&self) -> bool {
        self.shown >= self.total as f32
    }

    /// Reveals everything immediately and drops pending pauses.
    pub fn skip_to_end(&mut self) {
        self.shown = self.total as f32;
        self.next_pause = self.pauses.len();
        self.pause_left = 0.0;
    }

    pub fn reset(&mut self) {
        self.shown = 0.0;
        self.next_pause = 0;
        self.pause_left = 0.0;
    }

    /// Advances by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        if self.is_complete() || !dt.is_finite() || dt <= 0.0 {
            return;
        }
        if self.chars_per_second.is_infinite() {
            self.skip_to_end();
            return;
        }
        let mut remaining = dt;
        while remaining > 0.0 && !self.is_complete() {
            if self.pause_left > 0.0 {
                let take = self.pause_left.min(remaining);
                self.pause_left -= take;
                remaining -= take;
                continue;
            }
            if self.chars_per_second <= 0.0 {
                return;
            }
            let stop = self
                .pauses
                .get(self.next_pause)
                .map_or(self.total, |p| p.grapheme) as f32;
            let needed = ((stop - self.shown) / self.chars_per_second).max(0.0);
            if needed <= remaining {
                self.shown = self.shown.max(stop);
                remaining -= needed;
                if let Some(p) = self.pauses.get(self.next_pause) {
                    self.pause_left = p.seconds;
                    self.next_pause += 1;
                    if self.next_pause >= self.pauses.len() && stop >= self.total as f32 {
                        // A trailing pause: the text is already fully shown.
                        self.pause_left = 0.0;
                    }
                }
            } else {
                self.shown += self.chars_per_second * remaining;
                remaining = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveals_at_configured_speed() {
        let mut t = Typewriter::new(10, vec![], 10.0);
        assert_eq!(t.revealed(), 0);
        t.update(0.35);
        assert_eq!(t.revealed(), 3);
        assert!(!t.is_complete());
        t.update(10.0);
        assert!(t.is_complete());
        assert_eq!(t.revealed(), 10);
    }

    #[test]
    fn skip_to_end_completes() {
        let mut t = Typewriter::new(
            5,
            vec![Pause {
                grapheme: 2,
                seconds: 9.0,
            }],
            1.0,
        );
        t.skip_to_end();
        assert!(t.is_complete());
        assert_eq!(t.revealed(), 5);
    }

    #[test]
    fn pause_blocks_progress() {
        let mut t = Typewriter::new(
            10,
            vec![Pause {
                grapheme: 4,
                seconds: 1.0,
            }],
            10.0,
        );
        t.update(0.4);
        assert_eq!(t.revealed(), 4);
        t.update(0.9); // still in the pause
        assert_eq!(t.revealed(), 4);
        t.update(0.2); // 0.1 s left of pause, then 0.1 s of typing
        assert_eq!(t.revealed(), 5);
    }

    #[test]
    fn large_dt_crosses_pauses() {
        let mut t = Typewriter::new(
            10,
            vec![Pause {
                grapheme: 5,
                seconds: 1.0,
            }],
            10.0,
        );
        t.update(1.6); // 0.5 type + 1.0 pause + 0.1 type
        assert_eq!(t.revealed(), 6);
    }

    #[test]
    fn pause_at_start_and_trailing_pause() {
        let mut t = Typewriter::new(
            2,
            vec![
                Pause {
                    grapheme: 0,
                    seconds: 0.5,
                },
                Pause {
                    grapheme: 2,
                    seconds: 5.0,
                },
            ],
            10.0,
        );
        t.update(0.4);
        assert_eq!(t.revealed(), 0);
        t.update(0.5); // 0.1 s of pause left, then 2 graphemes in 0.2 s
        assert!(t.is_complete());
        assert_eq!(t.revealed(), 2);
    }

    #[test]
    fn degenerate_inputs() {
        let mut t = Typewriter::new(0, vec![], 10.0);
        assert!(t.is_complete());
        t.update(1.0);
        let mut t = Typewriter::new(3, vec![], 0.0);
        t.update(100.0);
        assert_eq!(t.revealed(), 0);
        let mut t = Typewriter::new(3, vec![], f32::INFINITY);
        t.update(0.001);
        assert!(t.is_complete());
        let mut t = Typewriter::new(3, vec![], f32::NAN);
        t.update(f32::NAN);
        assert_eq!(t.revealed(), 0);
    }
}
