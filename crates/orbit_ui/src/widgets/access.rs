//! The widget tree seen by a screen reader (see [`crate::a11y`]).

use super::{Kind, NodeId, Ui};
use crate::a11y::{A11yNode, A11yRole, plain, widget_id};

impl Ui {
    /// Appends the accessible nodes of the visible widgets to `out` (ids from
    /// [`widget_id`]`(slot, index)`). Containers and panels are flattened,
    /// decorative images are left out, a button's inner labels become its
    /// name, and an unnamed control takes the name of the label just before
    /// it in its row (and the value of the label just after a slider). While
    /// a modal is open only the modal is exposed.
    ///
    /// Returns the top-level ids, the focused widget id and whether a modal
    /// is open.
    pub fn access_nodes(&mut self, slot: u32, out: &mut Vec<A11yNode>) -> (Vec<u64>, Option<u64>, bool) {
        self.ensure_layout();
        let roots = match self.modal {
            Some((root, _, _)) => vec![root],
            None => self.roots.clone(),
        };
        let mut top = Vec::new();
        for root in roots {
            self.access_walk(root, slot, &mut top, out);
        }
        let focus = self
            .focus
            .map(|f| widget_id(slot, f.0))
            .filter(|id| out.iter().any(|n| n.id == *id));
        (top, focus, self.modal.is_some())
    }

    /// Focuses widget `index` (an arena index, as in the accessible ids) when
    /// it is focusable. Shows the focus ring.
    pub fn focus_node(&mut self, index: usize) -> bool {
        self.ensure_layout();
        let id = NodeId(index);
        if index < self.nodes.len() && self.focusables().contains(&id) {
            self.focus = Some(id);
            self.focus_visible = true;
            true
        } else {
            false
        }
    }

    fn label_text(&self, id: NodeId) -> Option<String> {
        match &self.nodes[id.0].kind {
            Kind::Label(l) => Some(if l.markup { plain(&l.text) } else { l.text.trim().to_owned() })
                .filter(|t| !t.is_empty()),
            _ => None,
        }
    }

    /// Texts of the labels under `id`, in order.
    fn inner_texts(&self, id: NodeId, out: &mut Vec<String>) {
        for &c in &self.nodes[id.0].children {
            if !self.nodes[c.0].visible {
                continue;
            }
            if let Some(t) = self.label_text(c) {
                out.push(t);
            }
            self.inner_texts(c, out);
        }
    }

    /// The label right before / after `id` among its siblings.
    fn sibling_label(&self, id: NodeId, before: bool) -> Option<String> {
        let parent = self.nodes[id.0].parent?;
        let siblings = &self.nodes[parent.0].children;
        let at = siblings.iter().position(|&c| c == id)?;
        let other = if before {
            *siblings.get(at.checked_sub(1)?)?
        } else {
            *siblings.get(at + 1)?
        };
        self.label_text(other)
    }

    fn access_walk(&self, id: NodeId, slot: u32, parent: &mut Vec<u64>, out: &mut Vec<A11yNode>) {
        let n = &self.nodes[id.0];
        if !n.visible {
            return;
        }
        let node = |role: A11yRole, name: String| {
            let mut a = A11yNode::new(widget_id(slot, id.0), role, name);
            a.bounds = n.rect;
            a.disabled = !n.enabled;
            a
        };
        let named = |own: &str| {
            let own = plain(own);
            if own.is_empty() {
                self.sibling_label(id, true).unwrap_or_default()
            } else {
                own
            }
        };
        let made = match &n.kind {
            Kind::Container | Kind::Panel(_) => {
                for &c in &n.children {
                    self.access_walk(c, slot, parent, out);
                }
                return;
            }
            Kind::Label(_) => match self.label_text(id) {
                Some(text) => node(A11yRole::Label, text),
                None => return,
            },
            Kind::Image { clickable: false, .. } => return,
            Kind::Image { .. } => node(A11yRole::Button, named("")),
            Kind::Button(b) => {
                let mut name = plain(&b.label);
                if name.is_empty() {
                    let mut texts = Vec::new();
                    self.inner_texts(id, &mut texts);
                    name = texts.join(", ");
                }
                let mut a = node(A11yRole::Button, name);
                a.selected = b.selected;
                a
            }
            Kind::Toggle { on, label } => {
                let mut a = node(A11yRole::CheckBox, named(label));
                a.toggled = Some(*on);
                a
            }
            Kind::Slider { value, .. } => {
                let mut a = node(A11yRole::Slider, named(""));
                a.numeric = Some(*value);
                a.value = Some(
                    self.sibling_label(id, false)
                        .unwrap_or_else(|| format!("{:.0}%", value * 100.0)),
                );
                a
            }
            Kind::Selector { options, index, .. } => {
                let mut a = node(A11yRole::ComboBox, named(""));
                a.value = options.get(*index).map(|o| plain(o));
                a
            }
            Kind::ScrollList { .. } => {
                let mut a = node(A11yRole::List, String::new());
                let mut children = Vec::new();
                for &c in &n.children {
                    self.access_walk(c, slot, &mut children, out);
                }
                a.children = children;
                a
            }
        };
        parent.push(made.id);
        out.push(made);
    }
}
