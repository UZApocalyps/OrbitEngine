//! Screen reader plumbing (ADR 0017, M7-09).
//!
//! The app describes its UI as AccessKit [`TreeUpdate`]s
//! ([`crate::App::accessibility_tree`]). On desktop they go to the
//! `accesskit_winit` adapter (UI Automation, AT-SPI, macOS); on the web,
//! where AccessKit has no stable adapter, they are mirrored in a hidden ARIA
//! DOM next to the canvas: `aria-live="polite"` regions for announcements and
//! real buttons for the controls, whose clicks come back as action requests.
//!
//! The web mirror is computed by [`WebView::from_update`], which is
//! platform-free and unit tested; only applying it touches the DOM.

use std::collections::HashMap;

use accesskit::{Live, Node, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate};

/// The tree sent before the app has one: the window alone, named `title`.
pub fn placeholder_tree(title: &str) -> TreeUpdate {
    let root = NodeId(1);
    let mut node = Node::new(Role::Window);
    node.set_label(title.to_owned());
    TreeUpdate {
        nodes: vec![(root, node)],
        tree: Some(Tree::new(root)),
        tree_id: TreeId::ROOT,
        focus: root,
    }
}

/// Kind of a mirrored element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebKind {
    /// A paragraph of text.
    Text,
    Button,
    CheckBox,
    Slider,
    ComboBox,
}

/// One mirrored element.
#[derive(Clone, Debug, PartialEq)]
pub struct WebItem {
    pub id: u64,
    pub kind: WebKind,
    pub label: String,
    pub value: Option<String>,
    pub checked: Option<bool>,
    pub disabled: bool,
}

/// What the hidden DOM shows for one tree.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct WebView {
    /// Texts of the live regions, in tree order.
    pub live: Vec<String>,
    /// Controls and texts, in tree (reading) order.
    pub items: Vec<WebItem>,
    /// Spoken description of the focused control (announced when it
    /// changes, as the canvas keeps the keyboard focus).
    pub focus: Option<String>,
}

impl WebView {
    /// Flattens a full tree update (as the player sends) in reading order.
    pub fn from_update(update: &TreeUpdate) -> Self {
        let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
        let mut view = Self::default();
        let root = update.tree.as_ref().map_or(NodeId(1), |t| t.root);
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = nodes.get(&id) else { continue };
            stack.extend(node.children().iter().rev().copied());
            let label = node.label().unwrap_or_default().to_owned();
            if node.live().is_some_and(|l| l != Live::Off) {
                if !label.is_empty() {
                    view.live.push(label);
                }
                continue;
            }
            let kind = match node.role() {
                Role::Label => WebKind::Text,
                Role::Button => WebKind::Button,
                Role::CheckBox => WebKind::CheckBox,
                Role::Slider => WebKind::Slider,
                Role::ComboBox => WebKind::ComboBox,
                _ => continue,
            };
            if kind == WebKind::Text && label.is_empty() {
                continue;
            }
            let value = node.value().map(str::to_owned);
            let checked = node.toggled().map(|t| t == Toggled::True);
            if id == update.focus && kind != WebKind::Text {
                let mut spoken = label.clone();
                if let Some(v) = &value {
                    spoken = format!("{spoken}: {v}");
                }
                view.focus = Some(spoken);
            }
            view.items.push(WebItem {
                id: id.0,
                kind,
                label,
                value,
                checked,
                disabled: node.is_disabled(),
            });
        }
        view
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod web {
    //! The hidden ARIA DOM.

    use accesskit::{Action, ActionRequest, NodeId, TreeId};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{Document, HtmlElement};
    use winit::event_loop::EventLoopProxy;

    use super::{WebKind, WebView};
    use crate::UserEvent;

    /// Visually hidden but read by screen readers.
    const HIDDEN: &str = "position:absolute;left:-10000px;top:auto;width:1px;height:1px;overflow:hidden;";

    pub(crate) struct WebAccess {
        document: Document,
        live: HtmlElement,
        focus: HtmlElement,
        items: HtmlElement,
        shown: WebView,
        clicks: Vec<Closure<dyn FnMut()>>,
        proxy: EventLoopProxy<UserEvent>,
    }

    fn element(document: &Document, tag: &str) -> Option<HtmlElement> {
        document.create_element(tag).ok()?.dyn_into::<HtmlElement>().ok()
    }

    impl WebAccess {
        pub(crate) fn install(proxy: EventLoopProxy<UserEvent>) -> Option<Self> {
            let document = web_sys::window()?.document()?;
            let root = element(&document, "div")?;
            root.set_id("orbit-a11y");
            root.set_attribute("style", HIDDEN).ok()?;
            let live = element(&document, "div")?;
            live.set_attribute("aria-live", "polite").ok()?;
            live.set_attribute("aria-atomic", "true").ok()?;
            let focus = element(&document, "div")?;
            focus.set_attribute("aria-live", "polite").ok()?;
            focus.set_attribute("aria-atomic", "true").ok()?;
            let items = element(&document, "div")?;
            for child in [&live, &focus, &items] {
                root.append_child(child).ok()?;
            }
            document.body()?.append_child(&root).ok()?;
            Some(Self {
                document,
                live,
                focus,
                items,
                shown: WebView::default(),
                clicks: Vec::new(),
                proxy,
            })
        }

        pub(crate) fn apply(&mut self, view: WebView) {
            if view.live != self.shown.live {
                self.live.set_text_content(Some(&view.live.join("\n")));
            }
            if view.focus != self.shown.focus {
                self.focus.set_text_content(view.focus.as_deref());
            }
            if view.items != self.shown.items {
                self.rebuild(&view);
            }
            self.shown = view;
        }

        fn rebuild(&mut self, view: &WebView) {
            self.items.set_inner_html("");
            self.clicks.clear();
            for item in &view.items {
                let tag = if item.kind == WebKind::Text { "p" } else { "button" };
                let Some(el) = element(&self.document, tag) else { continue };
                let mut text = item.label.clone();
                let _ = match item.kind {
                    WebKind::Text | WebKind::Button => Ok(()),
                    WebKind::CheckBox => el.set_attribute("role", "checkbox"),
                    WebKind::Slider => el.set_attribute("role", "slider"),
                    WebKind::ComboBox => el.set_attribute("role", "combobox"),
                };
                if let Some(on) = item.checked {
                    let _ = el.set_attribute("aria-checked", if on { "true" } else { "false" });
                }
                if let Some(value) = &item.value {
                    if item.kind == WebKind::Slider {
                        let _ = el.set_attribute("aria-valuetext", value);
                    } else {
                        text = format!("{text}: {value}");
                    }
                }
                if item.disabled {
                    let _ = el.set_attribute("aria-disabled", "true");
                }
                el.set_text_content(Some(&text));
                if item.kind != WebKind::Text && !item.disabled {
                    let proxy = self.proxy.clone();
                    let id = item.id;
                    let click = Closure::<dyn FnMut()>::new(move || {
                        let _ = proxy.send_event(UserEvent::AccessAction(ActionRequest {
                            action: Action::Click,
                            target_tree: TreeId::ROOT,
                            target_node: NodeId(id),
                            data: None,
                        }));
                    });
                    let _ = el.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
                    self.clicks.push(click);
                }
                let _ = self.items.append_child(&el);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(role: Role, label: &str) -> Node {
        let mut n = Node::new(role);
        n.set_label(label.to_owned());
        n
    }

    #[test]
    fn the_web_mirror_follows_reading_order() {
        let mut root = node(Role::Window, "Game");
        root.set_children(vec![NodeId(2), NodeId(3), NodeId(10)]);
        let mut line = node(Role::Label, "Aiko: Hello");
        line.set_live(Live::Polite);
        let mut group = node(Role::Group, "Menu");
        group.set_children(vec![NodeId(4), NodeId(5), NodeId(6)]);
        let title = node(Role::Label, "Settings");
        let mut toggle = node(Role::CheckBox, "Fullscreen");
        toggle.set_toggled(Toggled::False);
        let mut combo = node(Role::ComboBox, "Language");
        combo.set_value("English".to_owned());
        let mut back = node(Role::Button, "Back");
        back.set_disabled();
        let update = TreeUpdate {
            nodes: vec![
                (NodeId(1), root),
                (NodeId(2), line),
                (NodeId(3), group),
                (NodeId(4), title),
                (NodeId(5), toggle),
                (NodeId(6), combo),
                (NodeId(10), back),
            ],
            tree: Some(Tree::new(NodeId(1))),
            tree_id: TreeId::ROOT,
            focus: NodeId(6),
        };
        let view = WebView::from_update(&update);
        assert_eq!(view.live, vec!["Aiko: Hello"]);
        let labels: Vec<_> = view.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Settings", "Fullscreen", "Language", "Back"]);
        assert_eq!(view.items[1].checked, Some(false));
        assert_eq!(view.items[2].kind, WebKind::ComboBox);
        assert!(view.items[3].disabled);
        assert_eq!(view.focus.as_deref(), Some("Language: English"));
    }

    #[test]
    fn the_placeholder_is_a_lone_window() {
        let t = placeholder_tree("My game");
        assert_eq!(t.nodes.len(), 1);
        assert_eq!(t.nodes[0].1.label(), Some("My game"));
        assert_eq!(t.focus, NodeId(1));
        assert!(WebView::from_update(&t).items.is_empty());
    }
}
