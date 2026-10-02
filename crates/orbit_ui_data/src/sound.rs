//! UI sound cues.

/// A UI sound cue. The host looks the file up in
/// [`crate::UiSounds`] (`theme.sounds`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiSound {
    Hover,
    Confirm,
    Cancel,
    Slide,
}
