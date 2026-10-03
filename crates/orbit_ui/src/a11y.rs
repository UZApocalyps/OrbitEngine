//! Screen reader model (ADR 0017, M7-09).
//!
//! The player describes what is on screen as a small, platform-free
//! [`A11yTree`]: the widgets of the visible screen (role, localized name,
//! state), the choice menu and a **live region** carrying the current
//! dialogue line ("speaker: text"), which screen readers announce when it
//! changes. The tree is built from the data-driven widgets without any OS
//! access, so it is unit tested; [`A11yTree::to_accesskit`] turns it into an
//! AccessKit update for the desktop adapter (`accesskit_winit`), and the web
//! host mirrors the same update in a hidden ARIA DOM.
//!
//! Node ids are stable as long as the screen does not change: they encode
//! the screen slot and the widget index (see [`widget_id`]), so the host can
//! route an action request (click, increment...) back to the widget.

pub use accesskit;

use accesskit::{Action, Live, Node, NodeId, Rect, Role, Toggled, Tree, TreeId, TreeUpdate};

/// Id of the root (window) node.
pub const ROOT_ID: u64 = 1;
/// Id of the live region announcing dialogue lines.
pub const LINE_ID: u64 = 2;
/// Id of the choice menu (a list of buttons).
pub const CHOICES_ID: u64 = 3;
/// Id of the live region announcing toasts ("Saved"...).
pub const TOAST_ID: u64 = 4;
/// Id of the "click to start" button shown before the game starts.
pub const START_ID: u64 = 5;
/// First id of the choice buttons (`CHOICE_BASE + index`).
pub const CHOICE_BASE: u64 = 0x100;
/// Ids below this are fixed nodes; widget ids start here.
const SLOT_SHIFT: u32 = 24;

/// Id of widget `index` (its arena index in its [`Ui`](crate::Ui)) of the
/// screen in `slot` (`slot >= 1`).
pub fn widget_id(slot: u32, index: usize) -> u64 {
    (u64::from(slot) << SLOT_SHIFT) | (index as u64 + 1)
}

/// Id of the group node holding the widgets of the screen in `slot`.
pub fn screen_id(slot: u32) -> u64 {
    u64::from(slot) << SLOT_SHIFT
}

/// What an id designates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum A11yTarget {
    /// A fixed node (root, live regions, choice list).
    Fixed(u64),
    /// A choice button.
    Choice(usize),
    /// A screen group.
    Screen(u32),
    /// A widget: screen slot and arena index.
    Widget(u32, usize),
}

/// Decodes an id built by [`widget_id`], [`screen_id`] or the constants.
pub fn target_of(id: u64) -> A11yTarget {
    let slot = (id >> SLOT_SHIFT) as u32;
    let low = id & ((1 << SLOT_SHIFT) - 1);
    match (slot, low) {
        (0, low) if low >= CHOICE_BASE => A11yTarget::Choice((low - CHOICE_BASE) as usize),
        (0, low) => A11yTarget::Fixed(low),
        (slot, 0) => A11yTarget::Screen(slot),
        (slot, low) => A11yTarget::Widget(slot, (low - 1) as usize),
    }
}

/// Accessible role (a subset of AccessKit's, enough for the game UI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum A11yRole {
    Window,
    Group,
    /// A modal dialog (confirmation...).
    Dialog,
    Label,
    Button,
    CheckBox,
    Slider,
    /// A selector (left / right cycles the options).
    ComboBox,
    List,
    Image,
    /// A polite live region: its text is announced when it changes.
    Live,
}

/// One accessible node.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct A11yNode {
    pub id: u64,
    pub role: Option<A11yRole>,
    /// Localized name (label text, button caption...), markup removed.
    pub name: String,
    /// Current value as text (selector option, slider readout).
    pub value: Option<String>,
    /// Slider position in `0..=1`.
    pub numeric: Option<f32>,
    pub toggled: Option<bool>,
    pub selected: bool,
    pub disabled: bool,
    /// `[x, y, w, h]` on the 1920x1080 canvas.
    pub bounds: [f32; 4],
    pub children: Vec<u64>,
}

impl A11yNode {
    pub fn new(id: u64, role: A11yRole, name: impl Into<String>) -> Self {
        Self {
            id,
            role: Some(role),
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn role(&self) -> A11yRole {
        self.role.unwrap_or(A11yRole::Group)
    }
}

/// What the screen reader sees. `nodes[0]` is the root.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct A11yTree {
    pub nodes: Vec<A11yNode>,
    /// The focused node (the root when nothing is).
    pub focus: u64,
}

impl A11yTree {
    /// A tree holding only the window node.
    pub fn root(name: impl Into<String>) -> Self {
        Self {
            nodes: vec![A11yNode {
                bounds: [0.0, 0.0, crate::CANVAS[0], crate::CANVAS[1]],
                ..A11yNode::new(ROOT_ID, A11yRole::Window, name)
            }],
            focus: ROOT_ID,
        }
    }

    pub fn node(&self, id: u64) -> Option<&A11yNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    fn node_mut(&mut self, id: u64) -> Option<&mut A11yNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Adds `node` as the last child of `parent`.
    pub fn push(&mut self, parent: u64, node: A11yNode) {
        let id = node.id;
        self.nodes.push(node);
        if let Some(p) = self.node_mut(parent) {
            p.children.push(id);
        }
    }

    /// Adds the nodes `ui` produced (see [`crate::Ui::access_nodes`]) under a
    /// group of the screen in `slot`, named `name`. Returns the focused
    /// widget id, if any.
    pub fn push_ui(&mut self, parent: u64, slot: u32, name: &str, ui: &mut crate::Ui) -> Option<u64> {
        let mut nodes = Vec::new();
        let (top, focus, modal) = ui.access_nodes(slot, &mut nodes);
        let role = if modal { A11yRole::Dialog } else { A11yRole::Group };
        let mut group = A11yNode::new(screen_id(slot), role, name);
        group.bounds = [0.0, 0.0, crate::CANVAS[0], crate::CANVAS[1]];
        group.children = top;
        self.push(parent, group);
        self.nodes.extend(nodes);
        focus
    }

    /// Texts of the live regions (announcements), in tree order.
    pub fn announcements(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|n| n.role() == A11yRole::Live && !n.name.is_empty())
            .map(|n| n.name.as_str())
            .collect()
    }

    /// The AccessKit update describing the whole tree. `to_window` maps a
    /// canvas rectangle `[x, y, w, h]` to window pixels `[x0, y0, x1, y1]`.
    pub fn to_accesskit(&self, to_window: impl Fn([f32; 4]) -> [f64; 4]) -> TreeUpdate {
        let nodes = self
            .nodes
            .iter()
            .map(|n| (NodeId(n.id), ak_node(n, &to_window)))
            .collect();
        let mut tree = Tree::new(NodeId(ROOT_ID));
        tree.toolkit_name = Some("OrbitEngine".to_owned());
        TreeUpdate {
            nodes,
            tree: Some(tree),
            tree_id: TreeId::ROOT,
            focus: NodeId(self.focus),
        }
    }
}

fn ak_node(n: &A11yNode, to_window: &impl Fn([f32; 4]) -> [f64; 4]) -> Node {
    let role = match n.role() {
        A11yRole::Window => Role::Window,
        A11yRole::Group => Role::Group,
        A11yRole::Dialog => Role::Dialog,
        A11yRole::Label | A11yRole::Live => Role::Label,
        A11yRole::Button => Role::Button,
        A11yRole::CheckBox => Role::CheckBox,
        A11yRole::Slider => Role::Slider,
        A11yRole::ComboBox => Role::ComboBox,
        A11yRole::List => Role::List,
        A11yRole::Image => Role::Image,
    };
    let mut node = Node::new(role);
    if !n.name.is_empty() {
        node.set_label(n.name.clone());
    }
    if let Some(value) = &n.value {
        node.set_value(value.clone());
    }
    if n.role() == A11yRole::Live {
        node.set_live(Live::Polite);
    }
    if n.role() == A11yRole::Dialog {
        node.set_modal();
    }
    if let Some(v) = n.numeric {
        node.set_numeric_value(f64::from(v) * 100.0);
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(100.0);
    }
    if let Some(on) = n.toggled {
        node.set_toggled(if on { Toggled::True } else { Toggled::False });
    }
    if n.selected {
        node.set_selected(true);
    }
    if n.disabled {
        node.set_disabled();
    }
    let [x0, y0, x1, y1] = to_window(n.bounds);
    node.set_bounds(Rect { x0, y0, x1, y1 });
    if !n.children.is_empty() {
        node.set_children(n.children.iter().map(|&c| NodeId(c)).collect::<Vec<_>>());
    }
    let interactive = matches!(
        n.role(),
        A11yRole::Button | A11yRole::CheckBox | A11yRole::Slider | A11yRole::ComboBox
    );
    if interactive && !n.disabled {
        node.add_action(Action::Focus);
        node.add_action(Action::Click);
        if matches!(n.role(), A11yRole::Slider | A11yRole::ComboBox) {
            node.add_action(Action::Increment);
            node.add_action(Action::Decrement);
        }
    }
    node
}

/// What the screen reader asked of a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum A11yAction {
    Click,
    Focus,
    Increment,
    Decrement,
}

impl A11yAction {
    /// The supported AccessKit actions (`None` for the others).
    pub fn from_accesskit(action: Action) -> Option<Self> {
        match action {
            Action::Click => Some(Self::Click),
            Action::Focus => Some(Self::Focus),
            Action::Increment => Some(Self::Increment),
            Action::Decrement => Some(Self::Decrement),
            _ => None,
        }
    }
}

/// The text a dialogue line is announced with: `"Speaker: text"` (markup
/// removed), or the text alone for narration.
pub fn line_text(speaker: Option<&str>, markup: &str) -> String {
    let text = plain(markup);
    match speaker.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => format!("{name}: {text}"),
        None => text,
    }
}

/// `markup` without its `orbit_text` tags, whitespace collapsed.
pub fn plain(markup: &str) -> String {
    let text = orbit_text::parse_markup(markup).plain();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Turns successive lines into live region texts. A live region is only
/// announced when its text changes, so a line identical to the previous one
/// (a repeated "..." for instance) gets an invisible variation to be heard
/// again.
#[derive(Clone, Debug, Default)]
pub struct Announcer {
    key: Option<u64>,
    text: String,
    variant: bool,
}

impl Announcer {
    /// The live region text for the line identified by `key` (a sequence
    /// number that changes with every new line).
    pub fn text(&mut self, key: u64, text: &str) -> String {
        if self.key != Some(key) {
            if self.key.is_some() && self.text == text {
                self.variant = !self.variant;
            } else {
                self.variant = false;
            }
            self.key = Some(key);
            text.clone_into(&mut self.text);
        }
        if self.variant {
            // A no-break space: not spoken, but a different string.
            format!("{}\u{a0}", self.text)
        } else {
            self.text.clone()
        }
    }

    /// No line is shown (menus, choices): the next line is new even if its
    /// text equals the last one.
    pub fn clear(&mut self) {
        self.key = None;
        self.text.clear();
        self.variant = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Align, Node as W, PanelKind, Dir, Size, Theme, Ui};

    #[test]
    fn ids_round_trip() {
        assert_eq!(target_of(ROOT_ID), A11yTarget::Fixed(ROOT_ID));
        assert_eq!(target_of(CHOICE_BASE + 3), A11yTarget::Choice(3));
        assert_eq!(target_of(screen_id(5)), A11yTarget::Screen(5));
        assert_eq!(target_of(widget_id(5, 0)), A11yTarget::Widget(5, 0));
        assert_eq!(target_of(widget_id(2, 41)), A11yTarget::Widget(2, 41));
    }

    #[test]
    fn lines_are_spoken_with_the_speaker_and_without_markup() {
        assert_eq!(line_text(Some("Aiko"), "{b}Hello{/b} there"), "Aiko: Hello there");
        assert_eq!(line_text(None, "It was  raining.\nAgain."), "It was raining. Again.");
        assert_eq!(line_text(Some("  "), "Hi"), "Hi");
    }

    #[test]
    fn a_repeated_line_is_announced_again() {
        let mut a = Announcer::default();
        let first = a.text(1, "...");
        assert_eq!(first, "...");
        // Same line polled again: unchanged (no new announcement).
        assert_eq!(a.text(1, "..."), first);
        // A new line with the same text: a different string.
        let second = a.text(2, "...");
        assert_ne!(second, first);
        assert_eq!(second.trim_end_matches('\u{a0}'), "...");
        // A different text resets the variation.
        assert_eq!(a.text(3, "Hello"), "Hello");
        a.clear();
        assert_eq!(a.text(4, "Hello"), "Hello");
    }

    fn sample_ui() -> Ui {
        let mut ui = Ui::new(Theme::default());
        let root = ui.add_root(W::stack().fill().align(Align::Center, Align::Center));
        ui.add(root, W::panel(PanelKind::Dim, Dir::Stack).fill());
        let panel = ui.add(root, W::vbox().w(Size::Fixed(900.0)));
        ui.add(panel, W::label("{b}Settings{/b}").markup());
        let row = ui.add(panel, W::hbox());
        ui.add(row, W::label("Music"));
        ui.add(row, W::slider(1, 0.5));
        ui.add(row, W::label("50%"));
        let row = ui.add(panel, W::hbox());
        ui.add(row, W::label("Fullscreen"));
        ui.add(row, W::toggle(2, "", true));
        ui.add(panel, W::selector(3, vec!["English".into(), "Français".into()], 1));
        ui.add(panel, W::button(4, "Back"));
        ui.add(panel, W::button(5, "Disabled").enabled(false));
        ui.layout();
        ui.focus_first();
        ui
    }

    #[test]
    fn widgets_become_accessible_nodes() {
        let mut ui = sample_ui();
        let mut tree = A11yTree::root("Game");
        let focus = tree.push_ui(ROOT_ID, 1, "Preferences", &mut ui);
        let find = |name: &str| tree.nodes.iter().find(|n| n.name == name).cloned();
        let find_role = |name: &str, role: A11yRole| {
            tree.nodes.iter().find(|n| n.name == name && n.role() == role).cloned()
        };

        let group = tree.node(screen_id(1)).unwrap();
        assert_eq!(group.role(), A11yRole::Group);
        assert_eq!(group.name, "Preferences");
        // Containers and panels are flattened away.
        assert!(tree.nodes.iter().all(|n| n.role() != A11yRole::Group || n.id == screen_id(1)));

        assert_eq!(find("Settings").unwrap().role(), A11yRole::Label);
        let slider = find_role("Music", A11yRole::Slider).unwrap();
        assert_eq!(slider.numeric, Some(0.5));
        assert_eq!(slider.value.as_deref(), Some("50%"));
        let toggle = find_role("Fullscreen", A11yRole::CheckBox).unwrap();
        assert_eq!(toggle.toggled, Some(true));
        let selector = tree.nodes.iter().find(|n| n.role() == A11yRole::ComboBox).unwrap();
        assert_eq!(selector.value.as_deref(), Some("Français"));
        assert!(!find("Back").unwrap().disabled);
        assert!(find("Disabled").unwrap().disabled);

        // The focus is the first focusable widget (the slider).
        assert_eq!(focus, Some(slider.id));
        // Every child id exists.
        for n in &tree.nodes {
            for c in &n.children {
                assert!(tree.node(*c).is_some(), "dangling child {c}");
            }
        }
    }

    #[test]
    fn a_modal_hides_the_screen_below() {
        let mut ui = sample_ui();
        ui.open_modal(|ui| {
            let root = ui.add_root(W::vbox());
            ui.add(root, W::label("Overwrite?"));
            ui.add(root, W::button(9001, "Yes"));
            root
        });
        let mut tree = A11yTree::root("Game");
        tree.push_ui(ROOT_ID, 1, "Save", &mut ui);
        assert_eq!(tree.node(screen_id(1)).unwrap().role(), A11yRole::Dialog);
        assert!(tree.nodes.iter().any(|n| n.name == "Overwrite?"));
        assert!(!tree.nodes.iter().any(|n| n.name == "Back"));
    }

    #[test]
    fn card_buttons_are_named_by_their_labels() {
        let mut ui = Ui::new(Theme::default());
        let root = ui.add_root(W::vbox());
        let card = ui.add(root, W::button(1, ""));
        ui.add(card, W::label("Slot 3"));
        ui.add(card, W::label("Chapter 2"));
        ui.layout();
        let mut tree = A11yTree::root("Game");
        tree.push_ui(ROOT_ID, 2, "", &mut ui);
        let button = tree.nodes.iter().find(|n| n.role() == A11yRole::Button).unwrap();
        assert_eq!(button.name, "Slot 3, Chapter 2");
        assert!(button.children.is_empty());
    }

    #[test]
    fn accesskit_update_is_complete() {
        let mut ui = sample_ui();
        let mut tree = A11yTree::root("Game");
        let focus = tree.push_ui(ROOT_ID, 1, "Preferences", &mut ui);
        tree.push(ROOT_ID, A11yNode::new(LINE_ID, A11yRole::Live, "Aiko: Hi"));
        tree.focus = focus.unwrap();
        let update = tree.to_accesskit(|[x, y, w, h]| {
            [f64::from(x), f64::from(y), f64::from(x + w), f64::from(y + h)]
        });
        assert_eq!(update.nodes.len(), tree.nodes.len());
        assert_eq!(update.tree.as_ref().unwrap().root, NodeId(ROOT_ID));
        assert_eq!(update.focus, NodeId(tree.focus));
        let live = &update.nodes.iter().find(|(id, _)| *id == NodeId(LINE_ID)).unwrap().1;
        assert_eq!(live.live(), Some(Live::Polite));
        assert_eq!(live.label(), Some("Aiko: Hi"));
        let slider = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Slider)
            .unwrap();
        assert_eq!(slider.1.numeric_value(), Some(50.0));
        assert!(slider.1.supports_action(Action::Increment));
        assert_eq!(tree.announcements(), vec!["Aiko: Hi"]);
    }
}
