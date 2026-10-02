//! Editor state of the graph panel that the host persists.

use orbit_project::graph_layout::GraphLayout;

/// Positions of the graph blocks and what the host must do with them.
///
/// The host loads the `.layout` sidecar when a project opens and writes it
/// back once a drag has ended (`dirty`); the graph panel only reads and
/// edits `layout`.
#[derive(Debug, Default)]
pub struct GraphState {
    pub layout: GraphLayout,
    /// The layout changed since it was written.
    pub dirty: bool,
    /// Positions are persisted: a sidecar already exists, or the user moved
    /// something. Until then, merely looking at the graph writes no file.
    pub persist: bool,
    /// Bumped by the host each time a project is opened or closed, so the
    /// panel starts from a fitted view again.
    pub generation: u64,
    /// A block whose deletion waits for the user's confirmation.
    pub pending_delete: Option<orbit_project::graph::BlockId>,
}

impl GraphState {
    /// Records a new position chosen by the user (drag, gesture).
    pub fn set_position(&mut self, id: orbit_project::graph::BlockId, pos: (f32, f32)) {
        self.layout.set(id, pos);
        self.persist = true;
        self.dirty = true;
    }
}
