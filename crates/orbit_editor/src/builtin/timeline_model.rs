//! Pure model of the timeline editor (M5A-05): time <-> x mapping, snapping,
//! and the edits of the keys of a timeline's tracks. No egui, no project: the
//! panel feeds these functions and commits the result with one
//! `SetNodeKind` command, so every edit is undoable and a drag (recomputed
//! each frame from the tracks as they were at the start of the drag) is a
//! single undo step.

use std::collections::BTreeSet;

use orbit_anim::{Curve, Key, Step, Track, TrackData, Value};
use orbit_core::StoryTrack;

/// Smallest distance between two keys, in seconds (keys closer than this
/// are the same key).
pub const MIN_GAP: f32 = 0.001;
/// Distance (points) under which a dragged key sticks to a snap target.
pub const SNAP_PX: f32 = 6.0;
/// Zoom limits, in points per second.
pub const PPS_MIN: f32 = 8.0;
pub const PPS_MAX: f32 = 4000.0;

/// A key of a track: `(track index, key index)`.
pub type KeyRef = (usize, usize);

/// The visible part of the time axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeView {
    /// Time at the left edge of the ruler, seconds.
    pub left: f32,
    /// Points per second.
    pub pps: f32,
}

impl Default for TimeView {
    fn default() -> Self {
        Self { left: -0.1, pps: 160.0 }
    }
}

impl TimeView {
    /// Offset (points) from the left edge of the ruler of time `t`.
    pub fn x_of(&self, t: f32) -> f32 {
        (t - self.left) * self.pps
    }

    /// Time at offset `x` (points) from the left edge of the ruler.
    pub fn time_at(&self, x: f32) -> f32 {
        self.left + x / self.pps
    }

    /// Zooms by `factor` keeping the time under `anchor_x` fixed.
    pub fn zoom(&mut self, factor: f32, anchor_x: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let anchor_t = self.time_at(anchor_x);
        self.pps = (self.pps * factor).clamp(PPS_MIN, PPS_MAX);
        self.left = anchor_t - anchor_x / self.pps;
    }

    /// Scrolls by `dx` points (positive: later in time).
    pub fn scroll(&mut self, dx: f32) {
        self.left += dx / self.pps;
    }

    /// Fits `[0, duration]` into `width` points with a small margin.
    pub fn fit(duration: f32, width: f32) -> Self {
        let duration = duration.max(0.1);
        let pps = ((width - 40.0) / duration).clamp(PPS_MIN, PPS_MAX);
        Self { left: -20.0 / pps, pps }
    }
}

/// Rounds a time to the millisecond (no float noise in the saved file) and
/// keeps it non-negative.
pub fn round_time(t: f32) -> f32 {
    if !t.is_finite() {
        return 0.0;
    }
    ((t * 1000.0).round() / 1000.0).max(0.0)
}

/// The grid step (seconds) that keeps grid lines at least `min_px` apart: the
/// first of 1-2-5 times a power of ten that is wide enough.
pub fn grid_step(pps: f32, min_px: f32) -> f32 {
    let want = min_px / pps.max(f32::EPSILON);
    for exp in -3..=6 {
        for m in [1.0_f64, 2.0, 5.0] {
            // f64 so that 5 * 10^-1 is exactly 0.5 once back in f32.
            let s = (m * 10_f64.powi(exp)) as f32;
            if s >= want {
                return s;
            }
        }
    }
    1.0e6
}

/// Snaps `t` to the grid `step` (if any) and to `targets` (the playhead, the
/// other keys, the end of the timeline) when within `threshold` seconds.
/// Targets win over the grid.
pub fn snap_time(t: f32, step: Option<f32>, targets: &[f32], threshold: f32) -> f32 {
    let mut best: Option<(f32, f32)> = None;
    for &g in targets {
        let d = (g - t).abs();
        if d <= threshold && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, g));
        }
    }
    if let Some((_, g)) = best {
        return round_time(g);
    }
    match step {
        Some(s) if s > 0.0 => round_time((t / s).round() * s),
        _ => round_time(t),
    }
}

// ------------------------------------------------------------ typed tracks

macro_rules! each_track {
    ($data:expr, $t:ident => $body:expr) => {
        match $data {
            TrackData::Scalar($t) => $body,
            TrackData::Vec2($t) => $body,
            TrackData::Color($t) => $body,
            TrackData::Angle($t) => $body,
            TrackData::Name($t) => $body,
        }
    };
}

/// Times of the keys of a track.
pub fn key_times(data: &TrackData) -> Vec<f32> {
    each_track!(data, t => t.keys.iter().map(|k| k.t).collect())
}

pub fn key_count(data: &TrackData) -> usize {
    each_track!(data, t => t.keys.len())
}

/// Easing of the segment starting at key `i`.
pub fn key_ease(data: &TrackData, i: usize) -> Option<Curve> {
    each_track!(data, t => t.keys.get(i).map(|k| k.ease.clone()))
}

pub fn set_key_ease(data: &mut TrackData, i: usize, ease: Curve) {
    each_track!(data, t => if let Some(k) = t.keys.get_mut(i) { k.ease = ease })
}

pub fn key_value(data: &TrackData, i: usize) -> Option<Value> {
    Some(match data {
        TrackData::Scalar(t) => Value::Scalar(t.keys.get(i)?.value),
        TrackData::Vec2(t) => Value::Vec2(t.keys.get(i)?.value),
        TrackData::Color(t) => Value::Color(t.keys.get(i)?.value),
        TrackData::Angle(t) => Value::Angle(t.keys.get(i)?.value),
        TrackData::Name(t) => Value::Name(t.keys.get(i)?.value.0.clone()),
    })
}

/// Sets the value of key `i`; a value of the wrong type is ignored.
pub fn set_key_value(data: &mut TrackData, i: usize, value: Value) -> bool {
    match (data, value) {
        (TrackData::Scalar(t), Value::Scalar(v)) => set(t, i, v),
        (TrackData::Vec2(t), Value::Vec2(v)) => set(t, i, v),
        (TrackData::Color(t), Value::Color(v)) => set(t, i, v),
        (TrackData::Angle(t), Value::Angle(v)) => set(t, i, v),
        (TrackData::Name(t), Value::Name(v)) => set(t, i, Step(v)),
        _ => false,
    }
}

fn set<T>(t: &mut Track<T>, i: usize, v: T) -> bool {
    match t.keys.get_mut(i) {
        Some(k) => {
            k.value = v;
            true
        }
        None => false,
    }
}

/// The value of the track at `t` (named curves resolved through `resolve`).
pub fn sample<'a>(
    data: &TrackData,
    t: f32,
    resolve: impl Fn(&str) -> Option<&'a Curve>,
) -> Option<Value> {
    data.sample(t, resolve)
}

/// Index of the key at time `t` (within [`MIN_GAP`]).
pub fn key_at(data: &TrackData, t: f32) -> Option<usize> {
    key_times(data).iter().position(|&k| (k - t).abs() < MIN_GAP)
}

/// Puts a key with `value` at `t`: replaces the key already there (keeping
/// its easing) or inserts a new one. Returns the key's index, or `None`
/// when `value` does not fit the track.
pub fn put_key(data: &mut TrackData, t: f32, value: Value, ease: Option<Curve>) -> Option<usize> {
    let t = round_time(t);
    match (data, value) {
        (TrackData::Scalar(tr), Value::Scalar(v)) => Some(put_in_track(tr, t, v, ease)),
        (TrackData::Vec2(tr), Value::Vec2(v)) => Some(put_in_track(tr, t, v, ease)),
        (TrackData::Color(tr), Value::Color(v)) => Some(put_in_track(tr, t, v, ease)),
        (TrackData::Angle(tr), Value::Angle(v)) => Some(put_in_track(tr, t, v, ease)),
        (TrackData::Name(tr), Value::Name(v)) => Some(put_in_track(tr, t, Step(v), ease)),
        _ => None,
    }
}

pub fn put_in_track<T>(tr: &mut Track<T>, t: f32, v: T, ease: Option<Curve>) -> usize {
    if let Some(i) = tr.keys.iter().position(|k| (k.t - t).abs() < MIN_GAP) {
        tr.keys[i].value = v;
        if let Some(e) = ease {
            tr.keys[i].ease = e;
        }
        return i;
    }
    let i = tr.keys.partition_point(|k| k.t < t);
    tr.keys.insert(i, Key::with_ease(t, v, ease.unwrap_or_default()));
    i
}

/// Adds a key at `t` holding the value the track has there (so the curve is
/// unchanged). Returns its index; an existing key at `t` is just reported.
pub fn add_key_at<'a>(
    data: &mut TrackData,
    t: f32,
    resolve: impl Fn(&str) -> Option<&'a Curve>,
) -> Option<usize> {
    let t = round_time(t);
    if let Some(i) = key_at(data, t) {
        return Some(i);
    }
    let value = sample(data, t, resolve)?;
    put_key(data, t, value, None)
}

/// Removes key `i` of a track.
pub fn remove_key(data: &mut TrackData, i: usize) {
    each_track!(data, t => if i < t.keys.len() { t.keys.remove(i); })
}

/// Deletes the keys in `sel`. Returns how many were removed.
pub fn delete_keys(tracks: &mut [StoryTrack], sel: &BTreeSet<KeyRef>) -> usize {
    let mut n = 0;
    // Highest index first so the remaining indices stay valid.
    for &(ti, ki) in sel.iter().rev() {
        if let Some(tr) = tracks.get_mut(ti)
            && ki < key_count(&tr.data)
        {
            remove_key(&mut tr.data, ki);
            n += 1;
        }
    }
    n
}

/// Moves the keys in `sel` by `dt` seconds (clamped so none goes before 0).
/// A selected key landing on an unselected one replaces it. Returns the new
/// selection (indices change when keys pass each other).
pub fn move_keys(tracks: &mut [StoryTrack], sel: &BTreeSet<KeyRef>, dt: f32) -> BTreeSet<KeyRef> {
    let min_t = sel
        .iter()
        .filter_map(|&(ti, ki)| key_times(&tracks.get(ti)?.data).get(ki).copied())
        .fold(f32::INFINITY, f32::min);
    let dt = if min_t.is_finite() { dt.max(-min_t) } else { 0.0 };
    let mut out = BTreeSet::new();
    for (ti, tr) in tracks.iter_mut().enumerate() {
        let picked: BTreeSet<usize> = sel
            .iter()
            .filter(|&&(t, _)| t == ti)
            .map(|&(_, k)| k)
            .collect();
        if picked.is_empty() {
            continue;
        }
        for k in each_track!(&mut tr.data, t => move_in_track(t, &picked, dt)) {
            out.insert((ti, k));
        }
    }
    out
}

pub fn move_in_track<T>(tr: &mut Track<T>, picked: &BTreeSet<usize>, dt: f32) -> Vec<usize> {
    let mut keys: Vec<(bool, Key<T>)> = tr
        .keys
        .drain(..)
        .enumerate()
        .map(|(i, mut k)| {
            let sel = picked.contains(&i);
            if sel {
                k.t = round_time(k.t + dt);
            }
            (sel, k)
        })
        .collect();
    let moved: Vec<f32> = keys.iter().filter(|(s, _)| *s).map(|(_, k)| k.t).collect();
    keys.retain(|(s, k)| *s || !moved.iter().any(|t| (t - k.t).abs() < MIN_GAP));
    keys.sort_by(|a, b| a.1.t.total_cmp(&b.1.t));
    let mut sel_out = Vec::new();
    for (i, (s, _)) in keys.iter().enumerate() {
        if *s {
            sel_out.push(i);
        }
    }
    tr.keys = keys.into_iter().map(|(_, k)| k).collect();
    sel_out
}

// ------------------------------------------------------------ clipboard

/// A copied key: the track it came from, its time relative to the earliest
/// copied key, its value and its easing.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipKey {
    pub track: usize,
    pub dt: f32,
    pub value: Value,
    pub ease: Curve,
}

pub fn copy_keys(tracks: &[StoryTrack], sel: &BTreeSet<KeyRef>) -> Vec<ClipKey> {
    let mut out: Vec<ClipKey> = Vec::new();
    for &(ti, ki) in sel {
        let Some(tr) = tracks.get(ti) else { continue };
        let (Some(v), Some(e)) = (key_value(&tr.data, ki), key_ease(&tr.data, ki)) else {
            continue;
        };
        let t = key_times(&tr.data)[ki];
        out.push(ClipKey { track: ti, dt: t, value: v, ease: e });
    }
    let base = out.iter().map(|k| k.dt).fold(f32::INFINITY, f32::min);
    if base.is_finite() {
        for k in &mut out {
            k.dt = round_time(k.dt - base);
        }
    }
    out
}

/// Pastes `clip` with its earliest key at `at`. Keys go to their original
/// track, or to `fallback_track` when the clipboard holds one track (a copy
/// from track 2 pasted with track 0 selected lands on track 0). A key whose
/// value does not fit the destination is skipped. Returns the new selection.
pub fn paste_keys(
    tracks: &mut [StoryTrack],
    clip: &[ClipKey],
    at: f32,
    fallback_track: Option<usize>,
) -> BTreeSet<KeyRef> {
    let single = clip.iter().all(|k| k.track == clip.first().map_or(0, |f| f.track));
    let mut placed: Vec<(usize, f32)> = Vec::new();
    for k in clip {
        let ti = match (single, fallback_track) {
            (true, Some(f)) => f,
            _ => k.track,
        };
        let Some(tr) = tracks.get_mut(ti) else { continue };
        let t = round_time(at + k.dt);
        if put_key(&mut tr.data, t, k.value.clone(), Some(k.ease.clone())).is_some() {
            placed.push((ti, t));
        }
    }
    placed
        .into_iter()
        .filter_map(|(ti, t)| key_at(&tracks[ti].data, t).map(|ki| (ti, ki)))
        .collect()
}

// ------------------------------------------------------------ duration

/// Time of the last key of any track (0 without keys).
pub fn last_key_time(tracks: &[StoryTrack]) -> f32 {
    tracks.iter().map(|t| t.data.end_time()).fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_anim::{Angle, Ease};
    use orbit_core::{AnimProp, AnimTarget};

    fn scalar(keys: &[(f32, f32)]) -> StoryTrack {
        StoryTrack {
            target: AnimTarget::Background,
            prop: AnimProp::Opacity,
            data: TrackData::Scalar(Track::new(
                keys.iter().map(|&(t, v)| Key::new(t, v)).collect(),
            )),
        }
    }

    fn times(t: &StoryTrack) -> Vec<f32> {
        key_times(&t.data)
    }

    fn sel(items: &[KeyRef]) -> BTreeSet<KeyRef> {
        items.iter().copied().collect()
    }

    #[test]
    fn time_and_x_are_inverse() {
        let v = TimeView { left: 0.5, pps: 200.0 };
        assert_eq!(v.x_of(0.5), 0.0);
        assert_eq!(v.x_of(1.5), 200.0);
        assert!((v.time_at(v.x_of(2.345)) - 2.345).abs() < 1e-5);
    }

    #[test]
    fn zoom_keeps_the_anchor_and_clamps() {
        let mut v = TimeView { left: 0.0, pps: 100.0 };
        let before = v.time_at(250.0);
        v.zoom(2.0, 250.0);
        assert_eq!(v.pps, 200.0);
        assert!((v.time_at(250.0) - before).abs() < 1e-4);
        v.zoom(1.0e9, 0.0);
        assert_eq!(v.pps, PPS_MAX);
        v.zoom(0.0, 0.0);
        v.zoom(f32::NAN, 0.0);
        assert_eq!(v.pps, PPS_MAX);
        let before_scroll = v.left;
        v.scroll(PPS_MAX);
        assert!((v.left - before_scroll - 1.0).abs() < 1e-3);
    }

    #[test]
    fn the_grid_widens_when_zoomed_out() {
        assert!((grid_step(1000.0, 50.0) - 0.05).abs() < 1e-6);
        assert_eq!(grid_step(100.0, 50.0), 0.5);
        assert_eq!(grid_step(10.0, 50.0), 5.0);
        assert!(grid_step(0.0, 50.0).is_finite());
    }

    #[test]
    fn snapping_prefers_targets_then_the_grid() {
        assert_eq!(snap_time(0.52, Some(0.1), &[], 0.02), 0.5);
        assert_eq!(snap_time(0.52, Some(0.1), &[0.53], 0.02), 0.53);
        assert_eq!(snap_time(0.52, Some(0.1), &[0.9], 0.02), 0.5);
        assert_eq!(snap_time(0.5234, None, &[], 0.02), 0.523);
        assert_eq!(snap_time(-1.0, Some(0.1), &[], 0.0), 0.0);
        assert_eq!(snap_time(f32::NAN, None, &[], 0.0), 0.0);
    }

    #[test]
    fn adding_a_key_keeps_the_curve() {
        let mut tr = scalar(&[(0.0, 0.0), (1.0, 1.0)]);
        let i = add_key_at(&mut tr.data, 0.5, |_| None).unwrap();
        assert_eq!(i, 1);
        assert_eq!(times(&tr), vec![0.0, 0.5, 1.0]);
        assert_eq!(key_value(&tr.data, 1), Some(Value::Scalar(0.5)));
        // A key already there is reused, not duplicated.
        assert_eq!(add_key_at(&mut tr.data, 0.5004, |_| None), Some(1));
        assert_eq!(key_count(&tr.data), 3);
        // Past the end the last value is held.
        let j = add_key_at(&mut tr.data, 2.0, |_| None).unwrap();
        assert_eq!(key_value(&tr.data, j), Some(Value::Scalar(1.0)));
    }

    #[test]
    fn put_key_checks_the_value_type() {
        let mut tr = scalar(&[(0.0, 0.0)]);
        assert_eq!(put_key(&mut tr.data, 1.0, Value::Angle(Angle(5.0)), None), None);
        assert_eq!(key_count(&tr.data), 1);
        let i = put_key(&mut tr.data, 1.0, Value::Scalar(3.0), Some(Curve::Preset(Ease::InQuad)));
        assert_eq!(i, Some(1));
        assert_eq!(key_ease(&tr.data, 1), Some(Curve::Preset(Ease::InQuad)));
        // Replacing keeps the easing when none is given.
        put_key(&mut tr.data, 1.0, Value::Scalar(4.0), None);
        assert_eq!(key_ease(&tr.data, 1), Some(Curve::Preset(Ease::InQuad)));
        assert_eq!(key_value(&tr.data, 1), Some(Value::Scalar(4.0)));
    }

    #[test]
    fn moving_keys_clamps_at_zero_and_replaces_the_key_it_lands_on() {
        let mut tracks = vec![scalar(&[(0.5, 1.0), (1.0, 2.0), (2.0, 3.0)])];
        let out = move_keys(&mut tracks, &sel(&[(0, 0), (0, 1)]), -9.0);
        assert_eq!(times(&tracks[0]), vec![0.0, 0.5, 2.0]);
        assert_eq!(out, sel(&[(0, 0), (0, 1)]));

        let mut tracks = vec![scalar(&[(0.0, 1.0), (1.0, 2.0)])];
        let out = move_keys(&mut tracks, &sel(&[(0, 0)]), 1.0);
        assert_eq!(times(&tracks[0]), vec![1.0]);
        assert_eq!(key_value(&tracks[0].data, 0), Some(Value::Scalar(1.0)));
        assert_eq!(out, sel(&[(0, 0)]));
    }

    #[test]
    fn a_moved_key_passing_another_gets_a_new_index() {
        let mut tracks = vec![scalar(&[(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)])];
        let out = move_keys(&mut tracks, &sel(&[(0, 0)]), 1.5);
        assert_eq!(times(&tracks[0]), vec![1.0, 1.5, 2.0]);
        assert_eq!(out, sel(&[(0, 1)]));
        assert!(move_keys(&mut tracks, &sel(&[]), 1.0).is_empty());
        assert!(move_keys(&mut tracks, &sel(&[(9, 9)]), 1.0).is_empty());
    }

    #[test]
    fn a_drag_recomputed_from_its_origin_does_not_drift() {
        let origin = vec![scalar(&[(0.0, 1.0), (1.0, 2.0)])];
        for dt in [0.4, 1.0, 0.2, -3.0, 0.3] {
            let mut t = origin.clone();
            move_keys(&mut t, &sel(&[(0, 0)]), dt);
        }
        let mut t = origin.clone();
        move_keys(&mut t, &sel(&[(0, 0)]), 0.3);
        assert_eq!(times(&t[0]), vec![0.3, 1.0]);
        assert_eq!(times(&origin[0]), vec![0.0, 1.0]);
    }

    #[test]
    fn deleting_keys_uses_stable_indices() {
        let mut tracks = vec![scalar(&[(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)]), scalar(&[(0.0, 5.0)])];
        let n = delete_keys(&mut tracks, &sel(&[(0, 0), (0, 2), (1, 0), (7, 0)]));
        assert_eq!(n, 3);
        assert_eq!(times(&tracks[0]), vec![1.0]);
        assert_eq!(key_count(&tracks[1].data), 0);
    }

    #[test]
    fn copy_paste_keeps_spacing_values_and_easing() {
        let mut tracks = vec![scalar(&[(1.0, 1.0), (1.5, 2.0), (3.0, 3.0)]), scalar(&[(0.0, 9.0)])];
        set_key_ease(&mut tracks[0].data, 1, Curve::Preset(Ease::OutQuad));
        let clip = copy_keys(&tracks, &sel(&[(0, 0), (0, 1)]));
        assert_eq!(clip.iter().map(|k| k.dt).collect::<Vec<_>>(), vec![0.0, 0.5]);
        let out = paste_keys(&mut tracks, &clip, 4.0, None);
        assert_eq!(times(&tracks[0]), vec![1.0, 1.5, 3.0, 4.0, 4.5]);
        assert_eq!(out, sel(&[(0, 3), (0, 4)]));
        assert_eq!(key_ease(&tracks[0].data, 4), Some(Curve::Preset(Ease::OutQuad)));
        // A single-track clip follows the selected track.
        let out = paste_keys(&mut tracks, &clip, 2.0, Some(1));
        assert_eq!(times(&tracks[1]), vec![0.0, 2.0, 2.5]);
        assert_eq!(out, sel(&[(1, 1), (1, 2)]));
    }

    #[test]
    fn pasting_a_wrong_type_or_missing_track_is_ignored() {
        let mut tracks = vec![scalar(&[(0.0, 0.0)])];
        let clip = vec![
            ClipKey { track: 0, dt: 0.0, value: Value::Angle(Angle(1.0)), ease: Curve::default() },
            ClipKey { track: 5, dt: 0.0, value: Value::Scalar(1.0), ease: Curve::default() },
        ];
        assert!(paste_keys(&mut tracks, &clip, 1.0, None).is_empty());
        assert_eq!(key_count(&tracks[0].data), 1);
    }

    #[test]
    fn set_key_value_rejects_a_foreign_type() {
        let mut tr = scalar(&[(0.0, 0.0)]);
        assert!(!set_key_value(&mut tr.data, 0, Value::Angle(Angle(1.0))));
        assert!(set_key_value(&mut tr.data, 0, Value::Scalar(2.0)));
        assert!(!set_key_value(&mut tr.data, 4, Value::Scalar(2.0)));
        assert_eq!(key_value(&tr.data, 0), Some(Value::Scalar(2.0)));
        assert_eq!(last_key_time(&[scalar(&[(0.0, 0.0), (2.5, 1.0)]), scalar(&[(1.0, 1.0)])]), 2.5);
    }
}
