//! Focus, hover, pointer and keyboard handling of a [`Ui`].

use super::Ui;
use super::node::{Kind, NodeId, Tag};
use crate::input::{UiInput, UiSound};

/// What a widget reported to the screen that owns it.
#[derive(Clone, Debug, PartialEq)]
pub enum WidgetEvent {
    /// A button was pressed (click or Confirm).
    Activated(Tag),
    SliderChanged(Tag, f32),
    Toggled(Tag, bool),
    /// A selector now shows option `index`.
    Selected(Tag, usize),
    /// Cancel input (Escape / right click) reached the UI.
    Cancelled,
    /// A UI sound cue to forward to the host.
    Sound(UiSound),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir4 {
    Up,
    Down,
    Left,
    Right,
}

fn contains(rect: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= rect[0] && p[0] < rect[0] + rect[2] && p[1] >= rect[1] && p[1] < rect[1] + rect[3]
}

fn center(rect: [f32; 4]) -> [f32; 2] {
    [rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5]
}

impl Ui {
    // ------------------------------------------------------------ queries

    fn active_roots(&self) -> Vec<NodeId> {
        match self.modal {
            Some((root, _, _)) => vec![root],
            None => self.roots.clone(),
        }
    }

    fn collect_focusable(&self, id: NodeId, only_visible: bool, out: &mut Vec<NodeId>) {
        let n = &self.nodes[id.0];
        if only_visible && !n.visible {
            return;
        }
        let start = out.len();
        if n.kind.is_interactive() && n.enabled {
            out.push(id);
        }
        for &c in &n.children {
            self.collect_focusable(c, only_visible, out);
        }
        // A list without interactive rows is focusable itself (keyboard
        // scrolling).
        if matches!(n.kind, Kind::ScrollList { .. }) && out.len() == start && n.visible {
            out.push(id);
        }
    }

    /// Focusable widgets in reading (tree) order, visible ones only.
    pub fn focusables(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        for root in self.active_roots() {
            self.collect_focusable(root, true, &mut out);
        }
        out
    }

    /// Tags of the focusable widgets in focus order.
    pub fn focus_order(&mut self) -> Vec<Tag> {
        self.ensure_layout();
        self.focusables()
            .into_iter()
            .map(|id| self.nodes[id.0].tag)
            .collect()
    }

    fn hit_collect(&self, id: NodeId, p: [f32; 2], best: &mut Option<NodeId>) {
        let n = &self.nodes[id.0];
        if !n.visible {
            return;
        }
        if n.kind.is_interactive() && n.enabled && contains(n.rect, p) {
            *best = Some(id);
        }
        for &c in &n.children {
            self.hit_collect(c, p, best);
        }
    }

    /// The topmost enabled interactive widget under `p`.
    pub fn hit_test(&mut self, p: [f32; 2]) -> Option<NodeId> {
        self.ensure_layout();
        let mut best = None;
        for root in self.active_roots() {
            self.hit_collect(root, p, &mut best);
        }
        best
    }

    /// `true` when `p` is over a widget this UI would react to (a pointer
    /// click there is consumed).
    pub fn wants_pointer(&mut self, p: [f32; 2]) -> bool {
        if self.modal.is_some() {
            return true;
        }
        self.hit_test(p).is_some()
    }

    fn list_ancestor(&self, mut id: NodeId) -> Option<(NodeId, usize)> {
        // Returns the nearest ScrollList ancestor and the index of the child
        // of that list containing `id`.
        let mut child = id;
        while let Some(parent) = self.nodes[id.0].parent {
            if matches!(self.nodes[parent.0].kind, Kind::ScrollList { .. })
                && let Some(idx) = self.nodes[parent.0]
                    .children
                    .iter()
                    .position(|&c| c == child)
            {
                return Some((parent, idx));
            }
            child = parent;
            id = parent;
        }
        None
    }

    // ------------------------------------------------------------- input

    /// Applies one input and returns what the widgets reported.
    pub fn handle(&mut self, input: UiInput) -> Vec<WidgetEvent> {
        self.ensure_layout();
        let mut out = Vec::new();
        match input {
            UiInput::PointerMove(p) => self.pointer_move(p, &mut out),
            UiInput::PointerDown(p) => self.pointer_down(p, &mut out),
            UiInput::PointerUp(p) => self.pointer_up(p, &mut out),
            UiInput::Scroll(dy) => self.wheel(dy),
            UiInput::Up => self.direction(Dir4::Up, &mut out),
            UiInput::Down => self.direction(Dir4::Down, &mut out),
            UiInput::Left => self.direction(Dir4::Left, &mut out),
            UiInput::Right => self.direction(Dir4::Right, &mut out),
            UiInput::Confirm => self.confirm(&mut out),
            UiInput::Cancel => {
                out.push(WidgetEvent::Sound(UiSound::Cancel));
                out.push(WidgetEvent::Cancelled);
            }
            UiInput::Next => self.cycle(1),
            UiInput::Prev => self.cycle(-1),
            UiInput::PageUp => self.page(-1),
            UiInput::PageDown => self.page(1),
        }
        self.ensure_layout();
        out
    }

    fn pointer_move(&mut self, p: [f32; 2], out: &mut Vec<WidgetEvent>) {
        self.pointer = p;
        if let Some(id) = self.drag {
            self.set_slider_from_x(id, p[0], out);
            return;
        }
        self.focus_visible = false;
        let hit = self.hit_test(p);
        if hit != self.hover {
            self.hover = hit;
            if hit.is_some() {
                out.push(WidgetEvent::Sound(UiSound::Hover));
            }
        }
        if hit.is_some() {
            self.focus = hit;
        }
    }

    fn pointer_down(&mut self, p: [f32; 2], out: &mut Vec<WidgetEvent>) {
        self.pointer = p;
        self.focus_visible = false;
        let Some(id) = self.hit_test(p) else { return };
        self.focus = Some(id);
        self.hover = Some(id);
        match self.nodes[id.0].kind {
            Kind::Button(_) | Kind::Toggle { .. } => self.pressed = Some(id),
            Kind::Slider { .. } => {
                self.drag = Some(id);
                self.set_slider_from_x(id, p[0], out);
            }
            Kind::Selector { .. } => {
                let r = self.nodes[id.0].rect;
                let delta = if p[0] < r[0] + r[2] / 3.0 { -1 } else { 1 };
                self.step_selector(id, delta, out);
            }
            _ => {}
        }
    }

    fn pointer_up(&mut self, p: [f32; 2], out: &mut Vec<WidgetEvent>) {
        self.pointer = p;
        self.drag = None;
        if let Some(id) = self.pressed.take()
            && self.hit_test(p) == Some(id)
        {
            self.activate(id, out);
        }
    }

    fn set_slider_from_x(&mut self, id: NodeId, x: f32, out: &mut Vec<WidgetEvent>) {
        let knob = self.theme.widgets.controls.knob_size[0];
        let r = self.nodes[id.0].rect;
        let usable = (r[2] - knob).max(1.0);
        let v = ((x - (r[0] + knob * 0.5)) / usable).clamp(0.0, 1.0);
        self.apply_slider(id, v, out);
    }

    fn apply_slider(&mut self, id: NodeId, new: f32, out: &mut Vec<WidgetEvent>) {
        let tag = self.nodes[id.0].tag;
        if let Kind::Slider { value, .. } = &mut self.nodes[id.0].kind
            && (*value - new).abs() > f32::EPSILON
        {
            *value = new;
            out.push(WidgetEvent::SliderChanged(tag, new));
        }
    }

    fn step_selector(&mut self, id: NodeId, delta: i32, out: &mut Vec<WidgetEvent>) {
        let tag = self.nodes[id.0].tag;
        if let Kind::Selector { options, index } = &mut self.nodes[id.0].kind
            && options.len() > 1
        {
            let n = options.len() as i32;
            *index = (*index as i32 + delta).rem_euclid(n) as usize;
            let new = *index;
            out.push(WidgetEvent::Sound(UiSound::Slide));
            out.push(WidgetEvent::Selected(tag, new));
        }
    }

    fn activate(&mut self, id: NodeId, out: &mut Vec<WidgetEvent>) {
        let tag = self.nodes[id.0].tag;
        match &mut self.nodes[id.0].kind {
            Kind::Button(_) => {
                out.push(WidgetEvent::Sound(UiSound::Confirm));
                out.push(WidgetEvent::Activated(tag));
            }
            Kind::Toggle { on, .. } => {
                *on = !*on;
                let new = *on;
                out.push(WidgetEvent::Sound(UiSound::Confirm));
                out.push(WidgetEvent::Toggled(tag, new));
            }
            Kind::Selector { .. } => self.step_selector(id, 1, out),
            _ => {}
        }
    }

    fn confirm(&mut self, out: &mut Vec<WidgetEvent>) {
        self.focus_visible = true;
        match self.focus {
            Some(id) => self.activate(id, out),
            None => self.focus_first(),
        }
    }

    fn direction(&mut self, dir: Dir4, out: &mut Vec<WidgetEvent>) {
        self.focus_visible = true;
        let Some(id) = self.focus else {
            self.focus_first();
            return;
        };
        let step = self.theme.widgets.controls.slider_step;
        let horizontal = matches!(dir, Dir4::Left | Dir4::Right);
        let sign = if matches!(dir, Dir4::Left | Dir4::Up) {
            -1
        } else {
            1
        };
        match self.nodes[id.0].kind {
            Kind::Slider { value, step: own } if horizontal => {
                let s = own.unwrap_or(step);
                self.apply_slider(id, (value + sign as f32 * s).clamp(0.0, 1.0), out);
                out.push(WidgetEvent::Sound(UiSound::Slide));
                return;
            }
            Kind::Selector { .. } if horizontal => {
                self.step_selector(id, sign, out);
                return;
            }
            Kind::ScrollList { .. } if !horizontal => {
                if self.scroll_by(id, sign) {
                    return;
                }
            }
            _ => {}
        }
        // Rows of a scroll list: walk its interactive rows (hidden ones
        // included) so the list scrolls along with the focus.
        if !horizontal && let Some((list, _)) = self.list_ancestor(id) {
            let mut rows = Vec::new();
            for &c in &self.nodes[list.0].children.clone() {
                self.collect_focusable(c, false, &mut rows);
            }
            if let Some(pos) = rows.iter().position(|&r| r == id) {
                let next = pos as i32 + sign;
                if next >= 0 && (next as usize) < rows.len() {
                    self.focus = Some(rows[next as usize]);
                    self.reveal(rows[next as usize]);
                    return;
                }
            }
        }
        self.move_focus(id, dir);
    }

    /// Spatial navigation: the nearest focusable widget in `dir`, wrapping
    /// to the opposite side when there is none.
    fn move_focus(&mut self, from: NodeId, dir: Dir4) {
        let list = self.focusables();
        let cur = center(self.nodes[from.0].rect);
        let horizontal = matches!(dir, Dir4::Left | Dir4::Right);
        // (distance along the axis toward `dir`, distance across)
        let axes = |c: [f32; 2]| -> (f32, f32) {
            match dir {
                Dir4::Down => (c[1] - cur[1], (c[0] - cur[0]).abs()),
                Dir4::Up => (cur[1] - c[1], (c[0] - cur[0]).abs()),
                Dir4::Right => (c[0] - cur[0], (c[1] - cur[1]).abs()),
                Dir4::Left => (cur[0] - c[0], (c[1] - cur[1]).abs()),
            }
        };
        let mut best: Option<(f32, NodeId)> = None;
        for &id in &list {
            if id == from {
                continue;
            }
            let (along, across) = axes(center(self.nodes[id.0].rect));
            if along > 1.0 {
                let score = along + 2.0 * across;
                if best.is_none_or(|(s, _)| score < s) {
                    best = Some((score, id));
                }
            }
        }
        if best.is_none() {
            // Wrap: the candidate farthest "behind" us, nearest across.
            let extreme = list
                .iter()
                .map(|&id| {
                    let c = center(self.nodes[id.0].rect);
                    match dir {
                        Dir4::Down => c[1],
                        Dir4::Up => -c[1],
                        Dir4::Right => c[0],
                        Dir4::Left => -c[0],
                    }
                })
                .fold(f32::INFINITY, f32::min);
            for &id in &list {
                if id == from {
                    continue;
                }
                let c = center(self.nodes[id.0].rect);
                let (_, across) = axes(c);
                let pos = match dir {
                    Dir4::Down => c[1],
                    Dir4::Up => -c[1],
                    Dir4::Right => c[0],
                    Dir4::Left => -c[0],
                };
                let score = (pos - extreme) + 2.0 * across;
                if best.is_none_or(|(s, _)| score < s) {
                    best = Some((score, id));
                }
            }
        }
        let _ = horizontal;
        if let Some((_, id)) = best {
            self.focus = Some(id);
            self.reveal(id);
        }
    }

    fn cycle(&mut self, delta: i32) {
        self.focus_visible = true;
        let list = self.focusables();
        if list.is_empty() {
            return;
        }
        let next = match self.focus.and_then(|f| list.iter().position(|&i| i == f)) {
            Some(pos) => (pos as i32 + delta).rem_euclid(list.len() as i32) as usize,
            None if delta > 0 => 0,
            None => list.len() - 1,
        };
        self.focus = Some(list[next]);
        self.reveal(list[next]);
    }

    // ------------------------------------------------------------ scrolling

    /// Scrolls so the list child containing `id` is visible.
    fn reveal(&mut self, id: NodeId) {
        let Some((list, idx)) = self.list_ancestor(id) else {
            return;
        };
        let (first, shown) = match self.nodes[list.0].kind {
            Kind::ScrollList { first, shown } => (first, shown),
            _ => return,
        };
        let new_first = if idx < first {
            idx
        } else if idx >= first + shown {
            self.first_for_tail(list, idx)
        } else {
            return;
        };
        if let Kind::ScrollList { first, .. } = &mut self.nodes[list.0].kind {
            *first = new_first;
        }
        self.layout();
    }

    /// Scrolls a list by `delta` items. `false` when it cannot move.
    pub(crate) fn scroll_by(&mut self, list: NodeId, delta: i32) -> bool {
        let count = self.nodes[list.0].children.len();
        if count == 0 {
            return false;
        }
        let max_first = self.first_for_tail(list, count - 1);
        let Kind::ScrollList { first, .. } = &mut self.nodes[list.0].kind else {
            return false;
        };
        let new = (*first as i32 + delta).clamp(0, max_first as i32) as usize;
        if new == *first {
            return false;
        }
        *first = new;
        self.layout();
        true
    }

    fn wheel(&mut self, dy: f32) {
        if dy == 0.0 {
            return;
        }
        // The list under the pointer, else the focused row's list, else the
        // first list.
        let mut target = None;
        for (i, n) in self.nodes.iter().enumerate() {
            if matches!(n.kind, Kind::ScrollList { .. })
                && n.visible
                && contains(n.rect, self.pointer)
            {
                target = Some(NodeId(i));
            }
        }
        if target.is_none() {
            target = self.focus.and_then(|f| {
                if matches!(self.nodes[f.0].kind, Kind::ScrollList { .. }) {
                    Some(f)
                } else {
                    self.list_ancestor(f).map(|(l, _)| l)
                }
            });
        }
        if target.is_none() {
            target = self
                .nodes
                .iter()
                .position(|n| matches!(n.kind, Kind::ScrollList { .. }) && n.visible)
                .map(NodeId);
        }
        if let Some(list) = target {
            let notch = self.theme.widgets.list.wheel_items.max(1) as i32;
            self.scroll_by(list, if dy > 0.0 { -notch } else { notch });
        }
    }

    fn page(&mut self, dir: i32) {
        let target = self
            .nodes
            .iter()
            .position(|n| matches!(n.kind, Kind::ScrollList { .. }) && n.visible);
        if let Some(i) = target {
            let shown = match self.nodes[i].kind {
                Kind::ScrollList { shown, .. } => shown.max(1) as i32,
                _ => 1,
            };
            self.scroll_by(NodeId(i), dir * shown);
        }
    }

    /// Scrolls the list `tag` so its last item is visible.
    pub fn scroll_to_end(&mut self, tag: Tag) {
        self.ensure_layout();
        if let Some(id) = self.find(tag) {
            let count = self.nodes[id.0].children.len();
            if count > 0 {
                let first = self.first_for_tail(id, count - 1);
                if let Kind::ScrollList { first: f, .. } = &mut self.nodes[id.0].kind {
                    *f = first;
                }
                self.layout();
            }
        }
    }
}
