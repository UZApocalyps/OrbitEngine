//! The state shared between the Scene View panel (a plugin: egui, no GPU)
//! and the host that owns the real runtime, like [`crate::PreviewState`].
//!
//! The panel asks to see the stage at a node ([`SceneViewState::wanted_node`])
//! and a size; the host simulates the story up to that node, draws the stage
//! with the real renderer into a texture and publishes it together with the
//! placement of every character on it. The panel draws the texture and puts
//! its gizmos on top, from the same numbers.

use orbit_core::{CharacterId, NodeId};

/// A character on the stage shown by the scene view.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneViewActor {
    pub character: CharacterId,
    /// The `Show` node that placed the character (what a click selects).
    pub source: Option<NodeId>,
    /// Normalized position of the bottom-center anchor (origin top-left).
    pub x: f32,
    pub y: f32,
    /// Effective scale `[x, y]`.
    pub scale: [f32; 2],
    pub flip_x: bool,
    pub flip_y: bool,
    /// Size in virtual pixels (1920x1080 canvas) at scale 1.0; `[0, 0]` while
    /// the image is not available.
    pub base_size: [f32; 2],
}

/// Everything about the scene view that is not the runtime itself.
#[derive(Debug, Default)]
pub struct SceneViewState {
    // ---- published by the host
    /// A runtime exists (false without GPU, in headless tests).
    pub available: bool,
    /// The texture the stage is drawn into, registered in egui.
    pub texture: Option<egui::TextureId>,
    pub texture_size: [u32; 2],
    /// The node the picture shows the stage at.
    pub node: Option<NodeId>,
    /// Characters on the stage after `node`, back to front.
    pub actors: Vec<SceneViewActor>,
    /// Why the stage is not exactly what a player sees (no path to the node).
    pub notice: Option<String>,
    /// Frames drawn into the texture (tests, debug).
    pub frames: u64,

    // ---- set by the panel
    /// The panel was drawn this frame (the host clears it before the UI runs).
    pub visible: bool,
    /// The size, in physical pixels, the panel would like to draw at.
    pub wanted_size: [u32; 2],
    /// The node whose stage to show.
    pub wanted_node: Option<NodeId>,
    /// A drag is in progress: the host keeps the path to the node it already
    /// found instead of searching again at every frame.
    pub interacting: bool,
    /// Where the 16:9 picture was drawn (screen points), for tests and tools.
    pub picture_rect: Option<egui::Rect>,
}

impl SceneViewState {
    /// The actor placed by `node`, if any.
    pub fn actor_of(&self, node: NodeId) -> Option<&SceneViewActor> {
        self.actors.iter().find(|a| a.source == Some(node))
    }
}
