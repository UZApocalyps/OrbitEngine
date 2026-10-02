//! The graph widget behind a trait.
//!
//! The panel (`mod.rs`) knows the story graph and the editing rules; a
//! [`GraphView`] only knows how to draw blocks and wires, move them, and
//! report gestures as [`GraphEvent`]s. The shipped implementation is built
//! on `egui-snarl` ([`super::snarl_view::SnarlView`]); a hand-made widget
//! (painter-based pan / zoom / wires) can replace it without touching the
//! panel.

use egui::{Color32, Pos2, Rect, Ui};
use orbit_project::graph::{BlockId, SlotKind, SlotRef};

/// One output pin of a block.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewSlot {
    pub slot: SlotRef,
    pub label: String,
    pub color: Color32,
    /// Where it leads; `None` for a slot that leads nowhere.
    pub target: Option<BlockId>,
    /// The target is not a label (or is gone).
    pub broken: bool,
    /// Drawn dashed / dimmed: the fall-through and the empty `else`.
    pub soft: bool,
    /// The option only exists under a condition.
    pub conditional: bool,
}

/// How a block is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewNode {
    pub id: BlockId,
    pub title: String,
    /// Scene the block belongs to, for a block that does not head a scene.
    pub subtitle: String,
    pub icon: &'static str,
    pub accent: Color32,
    /// Summary lines: counts, characters.
    pub meta: Vec<String>,
    pub first_line: Option<String>,
    pub slots: Vec<ViewSlot>,
    /// Worst validation finding on the block and its text.
    pub problem: Option<(orbit_editor_api::Level, String)>,
    /// The node of the block the first finding is about (a click on the
    /// badge reveals it).
    pub problem_node: Option<orbit_core::NodeId>,
    /// No path from the start of the story reaches it.
    pub orphan: bool,
    /// Nodes after the terminator, which never run.
    pub dead_nodes: usize,
    pub selected: bool,
    /// Matches the search box (highlighted).
    pub highlighted: bool,
    /// Estimated height, for fitting and the mini-map.
    pub height: f32,
}

/// Everything the view needs to show the graph.
#[derive(Clone, Debug, Default)]
pub struct ViewContent {
    pub nodes: Vec<(ViewNode, (f32, f32))>,
    /// Identity of the structure (story revision, diagnostics...): while it
    /// is unchanged the view keeps what it has and only refreshes flags.
    pub signature: u64,
}

/// A gesture made in the view; the panel turns it into an edit (or a
/// selection) through `orbit_project`.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphEvent {
    /// The user selected these blocks (clicks, rectangle).
    Select(Vec<BlockId>),
    /// Double click: open in the scene editor.
    Open(BlockId),
    PlayFrom(BlockId),
    /// Show a node (the one a validation finding is about).
    Reveal(orbit_core::NodeId),
    /// A block is being dragged to this position.
    Moved(BlockId, (f32, f32)),
    /// The pointer was released after moving blocks.
    MoveEnded,
    /// A link was dragged from an output pin onto a block.
    Connect(SlotRef, BlockId),
    /// A link was dropped on empty space: create a scene there and link it.
    ConnectNew(SlotRef, (f32, f32)),
    Disconnect(SlotRef),
    NewScene((f32, f32)),
    NewLabel((f32, f32)),
    AddChoice(BlockId),
    AddCondition(BlockId),
    AddEnd(BlockId),
    RemoveEnd(BlockId),
    AddOption(SlotRef),
    Rename(BlockId, String),
    SetOptionText(SlotRef, String),
    /// Asks to delete a block (the panel confirms when something is lost).
    Delete(BlockId),
    /// The user confirmed a deletion.
    DeleteConfirmed(BlockId),
}

/// What the panel can ask of the view besides drawing it.
pub trait GraphView {
    /// Replaces the shown graph (called when the story, the diagnostics, the
    /// selection or the search changed). Positions come with the nodes.
    fn set_content(&mut self, content: ViewContent);

    /// Draws the view in `ui` and returns the gestures of this frame.
    fn show(&mut self, ui: &mut Ui) -> Vec<GraphEvent>;

    /// Frames every block.
    fn fit_all(&mut self);

    /// Scrolls so that this point (graph units) is at the center, zooming in
    /// to at least `min_zoom` when given.
    fn focus(&mut self, point: Pos2, min_zoom: Option<f32>);

    /// Moves the view by `pan` screen pixels and scales it by `zoom` around
    /// the center (debug motion for measurements).
    fn nudge(&mut self, pan: egui::Vec2, zoom: f32);

    /// The part of the graph (graph units) that is on screen, as of the last
    /// frame (mini-map).
    fn visible_rect(&self) -> Option<Rect>;
}

/// Pin / wire color of a slot, by what it does.
pub fn slot_color(kind: SlotKind) -> Color32 {
    match kind {
        SlotKind::Next => Color32::from_rgb(140, 148, 166),
        SlotKind::Jump => Color32::from_rgb(98, 190, 168),
        SlotKind::JumpScene => Color32::from_rgb(110, 160, 240),
        SlotKind::Call => Color32::from_rgb(176, 124, 236),
        SlotKind::Option(_) => Color32::from_rgb(236, 168, 70),
        SlotKind::Then => Color32::from_rgb(86, 200, 130),
        SlotKind::Else => Color32::from_rgb(232, 110, 120),
    }
}
