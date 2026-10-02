//! The state shared between the Preview panel (a plugin: egui, no GPU) and
//! the host that owns the real runtime.
//!
//! The panel never touches the runtime: it reads what the host published
//! (status, texture, current node, errors) and posts [`PreviewCommand`]s and
//! [`PreviewInput`]s that the host applies between two frames. That keeps the
//! plugin API free of wgpu and lets the whole panel be tested headlessly.

use orbit_core::NodeId;

/// Where the preview is in its life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewStatus {
    #[default]
    Stopped,
    Playing,
    Paused,
}

/// Time multiplier of the preview (skip is a separate switch).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewSpeed {
    #[default]
    Normal,
    Double,
}

/// What the panel asks of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewCommand {
    /// Starts from the beginning when stopped, resumes when paused.
    Play,
    Pause,
    /// Back to a blank stage.
    Stop,
    /// A new playthrough from the first node.
    Restart,
    /// A new playthrough with the state a player would have at this node.
    PlayFrom(NodeId),
    SetSpeed(PreviewSpeed),
    /// The Skip button.
    SetSkip(bool),
    SetMuted(bool),
}

/// Input for the game, already local to the panel: pointer positions are in
/// virtual pixels of the 1920x1080 canvas (letterbox removed by the panel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreviewInput {
    PointerMoved([f32; 2]),
    PointerLeft,
    Primary {
        pressed: bool,
    },
    /// Right button pressed (the game's "back").
    Secondary,
    /// Wheel, positive = up.
    Scroll(f32),
    Key {
        key: egui::Key,
        pressed: bool,
        shift: bool,
        alt: bool,
    },
    /// The Ctrl key (temporary skip).
    Ctrl(bool),
}

/// Everything about the preview that is not the runtime itself.
#[derive(Debug, Default)]
pub struct PreviewState {
    // ---- published by the host
    /// A runtime exists (false without GPU, in headless tests).
    pub available: bool,
    pub status: PreviewStatus,
    pub speed: PreviewSpeed,
    pub skipping: bool,
    pub muted: bool,
    /// The texture the game is drawn into, registered in egui.
    pub texture: Option<egui::TextureId>,
    pub texture_size: [u32; 2],
    /// The node of the line or choice on screen.
    pub current_node: Option<NodeId>,
    /// What the story is doing, in a few words ("Réplique", "Choix", "Fin").
    pub flow: String,
    /// Why the last edit could not be played (validation errors), if so.
    pub error: Option<String>,

    // ---- set by the panel
    /// The size, in physical pixels, the panel would like to draw at.
    pub wanted_size: [u32; 2],
    /// Where the 16:9 picture was drawn (screen points), for tests and tools.
    pub picture_rect: Option<egui::Rect>,
    /// The panel was drawn this frame (the host clears it before the UI runs).
    pub visible: bool,
    /// The panel has the keyboard focus: its input goes to the game.
    pub focused: bool,
    /// Selection follows the line being played.
    pub follow_playback: bool,
    /// Commands and input for the host.
    pub commands: Vec<PreviewCommand>,
    pub input: Vec<PreviewInput>,
}

impl PreviewState {
    /// Posts a command for the host.
    pub fn command(&mut self, command: PreviewCommand) {
        self.commands.push(command);
    }

    /// Pointer and keyboard input are only posted while the panel has the
    /// focus and a game runs; this is the check the panel uses.
    pub fn accepts_input(&self) -> bool {
        self.focused && self.status == PreviewStatus::Playing
    }
}
