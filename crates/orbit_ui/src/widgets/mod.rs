//! A small retained widget toolkit: a [`Ui`] owns an arena of [`Node`]s
//! (panels, labels, buttons, sliders, toggles, selectors, scroll lists,
//! images), lays them out on the virtual canvas, keeps focus / hover /
//! pressed state, turns [`UiInput`](crate::UiInput)s into typed
//! [`WidgetEvent`]s and draws itself into a `Scene`.
//!
//! Layout, state and hit testing never touch the GPU or any font, so they
//! are unit tested directly; only [`Ui::draw`] needs the renderer.

mod access;
mod draw;
mod interact;
mod layout;
mod node;

pub use draw::DrawCtx;
pub use interact::WidgetEvent;
pub use node::{
    Align, Anchor, ButtonData, ButtonKind, Dir, Kind, LabelData, NO_TAG, Node, NodeId, PanelKind,
    Size, Tag, ellipsize, escape_markup, estimate_lines, estimate_text_width,
};

use orbit_render2d::Layer;

use crate::theme::Theme;

/// Size of the virtual canvas widgets are laid out in.
pub const CANVAS: [f32; 2] = [1920.0, 1080.0];

/// The widget tree of one screen plus its interaction state.
pub struct Ui {
    pub(crate) theme: Theme,
    pub(crate) nodes: Vec<Node>,
    pub(crate) roots: Vec<NodeId>,
    /// Modal root, the arena length before it was added, and the focus to
    /// restore when it closes.
    pub(crate) modal: Option<(NodeId, usize, Option<NodeId>)>,
    pub(crate) focus: Option<NodeId>,
    pub(crate) hover: Option<NodeId>,
    pub(crate) pressed: Option<NodeId>,
    pub(crate) drag: Option<NodeId>,
    /// Touch/mouse drag of a scroll list: list, pointer Y and `first` at
    /// press time, and whether the drag passed the threshold.
    pub(crate) list_drag: Option<(NodeId, f32, usize, bool)>,
    /// `true` after keyboard input: the focus ring is drawn.
    pub(crate) focus_visible: bool,
    pub(crate) pointer: [f32; 2],
    pub(crate) laid_out: bool,
    layer: Layer,
    alpha: f32,
    pub(crate) time: f32,
}

impl Ui {
    /// An empty UI drawn on the overlay layer.
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            nodes: Vec::new(),
            roots: Vec::new(),
            modal: None,
            focus: None,
            hover: None,
            pressed: None,
            drag: None,
            list_drag: None,
            focus_visible: false,
            pointer: [-1.0, -1.0],
            laid_out: false,
            layer: Layer::OVERLAY,
            alpha: 1.0,
            time: 0.0,
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Replaces the theme (layout is recomputed on next use).
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.laid_out = false;
    }

    /// Layer the widgets are drawn on (default `Layer::OVERLAY`; a modal is
    /// drawn one layer above).
    pub fn set_layer(&mut self, layer: Layer) {
        self.layer = layer;
    }

    pub fn layer(&self) -> Layer {
        self.layer
    }

    /// Global opacity multiplier (fades).
    pub fn set_alpha(&mut self, alpha: f32) {
        self.alpha = alpha.clamp(0.0, 1.0);
    }

    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// Adds a root node (positioned on the canvas by its anchor).
    pub fn add_root(&mut self, node: Node) -> NodeId {
        let id = self.push(node, None);
        self.roots.push(id);
        self.laid_out = false;
        id
    }

    /// Adds `node` as the last child of `parent`.
    pub fn add(&mut self, parent: NodeId, node: Node) -> NodeId {
        let id = self.push(node, Some(parent));
        self.nodes[parent.0].children.push(id);
        self.laid_out = false;
        id
    }

    fn push(&mut self, mut node: Node, parent: Option<NodeId>) -> NodeId {
        node.parent = parent;
        self.nodes.push(node);
        NodeId(self.nodes.len() - 1)
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    /// Mutable access; call [`Ui::layout`] afterwards if sizes changed (it
    /// is also done lazily by `handle`/`draw`).
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        self.laid_out = false;
        &mut self.nodes[id.0]
    }

    /// First node carrying `tag`.
    pub fn find(&self, tag: Tag) -> Option<NodeId> {
        if tag == NO_TAG {
            return None;
        }
        self.nodes.iter().position(|n| n.tag == tag).map(NodeId)
    }

    /// Computed rectangle of the node carrying `tag`.
    pub fn rect_of(&mut self, tag: Tag) -> Option<[f32; 4]> {
        self.ensure_layout();
        self.find(tag).map(|id| self.nodes[id.0].rect)
    }

    /// Computed rectangle of a node.
    pub fn node_rect(&mut self, id: NodeId) -> [f32; 4] {
        self.ensure_layout();
        self.nodes[id.0].rect
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focus
    }

    /// Tag of the focused widget.
    pub fn focused_tag(&self) -> Option<Tag> {
        self.focus.map(|id| self.nodes[id.0].tag)
    }

    /// Focuses the widget carrying `tag` (no-op when absent or not
    /// focusable).
    pub fn set_focus_tag(&mut self, tag: Tag) -> bool {
        self.ensure_layout();
        match self.find(tag) {
            Some(id) if self.focusables().contains(&id) => {
                self.focus = Some(id);
                true
            }
            _ => false,
        }
    }

    /// Focuses the first focusable widget if nothing is focused.
    pub fn focus_first(&mut self) {
        self.ensure_layout();
        if self.focus.is_none() {
            self.focus = self.focusables().first().copied();
        }
    }

    /// Whether a modal dialog is open.
    pub fn has_modal(&self) -> bool {
        self.modal.is_some()
    }

    /// Advances animations (focus blink etc.).
    pub fn update(&mut self, dt: f32) {
        self.time += dt;
    }

    // ---- state readers / writers addressed by tag ----

    pub fn slider_value(&self, tag: Tag) -> Option<f32> {
        match self.find(tag).map(|id| &self.nodes[id.0].kind) {
            Some(Kind::Slider { value, .. }) => Some(*value),
            _ => None,
        }
    }

    pub fn set_slider(&mut self, tag: Tag, new: f32) {
        if let Some(id) = self.find(tag)
            && let Kind::Slider { value, .. } = &mut self.nodes[id.0].kind
        {
            *value = new.clamp(0.0, 1.0);
        }
    }

    pub fn toggle_value(&self, tag: Tag) -> Option<bool> {
        match self.find(tag).map(|id| &self.nodes[id.0].kind) {
            Some(Kind::Toggle { on, .. }) => Some(*on),
            _ => None,
        }
    }

    pub fn selector_index(&self, tag: Tag) -> Option<usize> {
        match self.find(tag).map(|id| &self.nodes[id.0].kind) {
            Some(Kind::Selector { index, .. }) => Some(*index),
            _ => None,
        }
    }

    /// Replaces the text of a label (or the caption of a button).
    pub fn set_text(&mut self, tag: Tag, text: impl Into<String>) {
        if let Some(id) = self.find(tag) {
            match &mut self.nodes[id.0].kind {
                Kind::Label(l) => l.text = text.into(),
                Kind::Button(b) => b.label = text.into(),
                _ => {}
            }
        }
    }

    pub fn set_enabled(&mut self, tag: Tag, enabled: bool) {
        if let Some(id) = self.find(tag) {
            self.nodes[id.0].enabled = enabled;
            self.laid_out = false;
        }
    }

    /// First visible item index of the scroll list `tag`.
    pub fn scroll_first(&mut self, tag: Tag) -> Option<usize> {
        self.ensure_layout();
        match self.find(tag).map(|id| &self.nodes[id.0].kind) {
            Some(Kind::ScrollList { first, .. }) => Some(*first),
            _ => None,
        }
    }

    // ---- modal ----

    /// Starts a modal: `build` adds the modal root (and its children) to
    /// this UI. While it is open only its widgets are focusable or
    /// clickable, and focus moves to its first widget.
    pub fn open_modal(&mut self, build: impl FnOnce(&mut Ui) -> NodeId) {
        if self.modal.is_some() {
            self.close_modal();
        }
        let start = self.nodes.len();
        let previous = self.focus;
        let root = build(self);
        if !self.roots.contains(&root) {
            self.roots.push(root);
        }
        self.modal = Some((root, start, previous));
        self.focus = None;
        self.hover = None;
        self.pressed = None;
        self.drag = None;
        self.list_drag = None;
        self.laid_out = false;
        self.focus_first();
    }

    /// Closes the modal and forgets its nodes. Focus returns to the widget
    /// that had it before (by tag), if any.
    pub fn close_modal(&mut self) {
        if let Some((root, start, previous)) = self.modal.take() {
            self.roots.retain(|r| *r != root && r.0 < start);
            self.nodes.truncate(start);
            self.focus = previous;
            self.hover = None;
            self.pressed = None;
            self.drag = None;
            self.list_drag = None;
            self.laid_out = false;
            self.focus_first();
        }
    }
}

#[cfg(test)]
mod tests;
