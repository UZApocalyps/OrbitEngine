//! What the VM reports to its host.

use orbit_core::{AssetId, CharacterId, NodeId, Position, StringId, Transition};
use serde::{Deserialize, Serialize};

use crate::error::VmError;

/// Outcome of [`crate::Vm::step`].
#[derive(Clone, Debug, PartialEq)]
pub enum StepResult {
    /// A line of dialogue to display; the host waits for the player's input
    /// (this is the "WaitInput" state), then calls `step()` again.
    Say {
        node: NodeId,
        /// `None` is the narrator.
        speaker: Option<CharacterId>,
        /// Text key, resolved with `Vm::text`.
        string_id: StringId,
        /// The line was already shown in a previous playthrough / before a
        /// rollback: candidates for skipping.
        already_read: bool,
    },
    /// Options to present (only the visible ones). The host must call
    /// `Vm::choose(i)` with an index into `options`; calling `step()` again
    /// before that returns the same result.
    Choice {
        node: NodeId,
        options: Vec<ChoiceView>,
    },
    /// Wait this many seconds, then call `step()` again.
    WaitTime(f32),
    /// A transition of this duration was started (see the matching
    /// presentation event); call `step()` again when it has finished.
    WaitTransition { duration: f32 },
    /// A blocking `Effect` / `Timeline` was started: wait `duration` seconds
    /// of scene clock (`Vm::advance_clock`), then call `step()` again. A
    /// skip calls `Vm::complete_anim(node)` first.
    WaitAnimation { node: NodeId, duration: f32 },
    /// An `OpenScreen { wait: true }` node opened the screen `screen` (it is
    /// in `Vm::open_screens`): show it and call `Vm::close_screen` when the
    /// player closes it. Calling `step()` before that returns this again.
    Screen { node: NodeId, screen: String },
    /// The story is over. Further `step()` calls return `End` again.
    End,
    /// Fatal error. The VM stays in this state until `restore()`.
    Error(VmError),
}

impl StepResult {
    /// A line, a choice or a screen the story waits for: the results that
    /// are rollback checkpoints.
    pub fn is_interaction(&self) -> bool {
        matches!(
            self,
            StepResult::Say { .. } | StepResult::Choice { .. } | StepResult::Screen { .. }
        )
    }
}

/// A visible choice option.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceView {
    pub string_id: StringId,
}

/// A non-blocking command for the presentation layer (renderer, audio),
/// emitted in execution order and retrieved with `Vm::take_events`. The
/// resulting state is also applied to the declarative `SceneState`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PresentationEvent {
    Background {
        asset: AssetId,
        transition: Option<Transition>,
    },
    Show {
        character: CharacterId,
        expression: String,
        /// Image of that expression, resolved for the host's convenience.
        image: AssetId,
        position: Position,
        /// Normalized stage coordinates of `position` (origin top-left,
        /// sprite anchored by its bottom-center).
        x: f32,
        y: f32,
        /// Effective sprite scale `[x, y]` (the `Show` override or the
        /// character's default scale; 1.0 = native size).
        scale: [f32; 2],
        flip_x: bool,
        flip_y: bool,
        transition: Option<Transition>,
    },
    Hide {
        character: CharacterId,
        transition: Option<Transition>,
    },
    PlayMusic {
        asset: AssetId,
        fade_in: f32,
        looping: bool,
    },
    StopMusic {
        fade_out: f32,
    },
    PlaySound {
        asset: AssetId,
    },
    /// A plugin-defined node was reached (`NodeKind::Custom`). The VM does
    /// not interpret it; a runtime plugin may.
    Custom {
        plugin_id: String,
        kind: String,
        data: orbit_core::CustomData,
    },
    /// The 3D scene on screen is now `scene`, seen from `camera` (`None`:
    /// the first camera). Emitted by `ShowScene3d` and `Camera3d`. It takes
    /// over the background layer (the 2D background is kept underneath).
    Scene3d {
        scene: AssetId,
        camera: Option<String>,
        transition: Option<Transition>,
    },
    /// The 3D scene is removed: the 2D background shows again.
    HideScene3d {
        transition: Option<Transition>,
    },
    /// The background is now a stack of parallax layers (back to front,
    /// M5A-07); like `Background`, it clears the 3D layer. Appended last.
    BackgroundLayers {
        layers: Vec<orbit_core::BgLayer>,
        transition: Option<Transition>,
    },
}
