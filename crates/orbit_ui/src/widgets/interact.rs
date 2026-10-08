//! Focus, hover, pointer and keyboard handling of a [`Ui`].

use super::Ui;
use super::node::{Anchor, Dir, Kind, ListMode, Node, NodeId, PanelKind, Size, Tag};
use crate::input::{UiInput, UiSound};

/// Tag of the first row of a dropdown popup (row `i` is `POPUP_TAG + i`).
/// Screens must not use tags this high.
pub(crate) const POPUP_TAG: Tag = 0x4000_0000;
/// Rows a dropdown popup shows before it scrolls.
const POPUP_ROWS: usize = 6;
const POPUP_PAD: f32 = 6.0;

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
    /// A sound file named by the style of a widget (play it directly).
    SoundFile(String),
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
        // (A generated list only when it has rows out of view.)
        if let Kind::ScrollList { shown, total, mode, .. } = n.kind
            && out.len() == start
            && n.visible
            && (mode == ListMode::Legacy || shown < total)
        {
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
        if let Some(consumed) = self.popup_input(input) {
            return consumed;
        }
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
        self.finish_popup(&mut out);
        self.ensure_layout();
        out
    }

    fn pointer_move(&mut self, p: [f32; 2], out: &mut Vec<WidgetEvent>) {
        self.pointer = p;
        if let Some(id) = self.drag {
            self.set_slider_from_x(id, p[0], out);
            return;
        }
        if self.drag_list(p) {
            return;
        }
        self.focus_visible = false;
        let hit = self.hit_test(p);
        if hit != self.hover {
            self.hover = hit;
            if let Some(id) = hit {
                out.push(self.sound_event(id, UiSound::Hover));
            }
        }
        if hit.is_some() {
            self.focus = hit;
        }
    }

    fn pointer_down(&mut self, p: [f32; 2], out: &mut Vec<WidgetEvent>) {
        self.pointer = p;
        self.focus_visible = false;
        self.begin_list_drag(p);
        let Some(id) = self.hit_test(p) else { return };
        self.focus = Some(id);
        self.hover = Some(id);
        match self.nodes[id.0].kind {
            Kind::Button(_) | Kind::Toggle { .. } | Kind::Image { clickable: true, .. } => {
                self.pressed = Some(id)
            }
            Kind::Slider { .. } => {
                self.drag = Some(id);
                self.set_slider_from_x(id, p[0], out);
            }
            Kind::Selector { popup: true, .. } => {
                let confirm = self.sound_event(id, UiSound::Confirm);
                out.push(confirm);
                self.open_popup(id);
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
        let dragged = matches!(self.list_drag.take(), Some((_, _, _, true)));
        if let Some(id) = self.pressed.take()
            && !dragged
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
        if let Kind::Selector { options, index, .. } = &mut self.nodes[id.0].kind
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
        let confirm = self.sound_event(id, UiSound::Confirm);
        match &mut self.nodes[id.0].kind {
            Kind::Button(_) | Kind::Image { clickable: true, .. } => {
                out.push(confirm);
                out.push(WidgetEvent::Activated(tag));
            }
            Kind::Toggle { on, .. } => {
                *on = !*on;
                let new = *on;
                out.push(confirm);
                out.push(WidgetEvent::Toggled(tag, new));
            }
            Kind::Selector { popup: true, .. } => {
                out.push(confirm);
                self.open_popup(id);
            }
            Kind::Selector { .. } => self.step_selector(id, 1, out),
            _ => {}
        }
    }

    /// The event for a UI sound cue of widget `id`: the file of its style's
    /// `sounds` (hover / confirm) when it has one, else the generic cue the
    /// host looks up in the theme.
    pub(crate) fn sound_event(&self, id: NodeId, sound: UiSound) -> WidgetEvent {
        let file = self.nodes[id.0]
            .style
            .as_deref()
            .and_then(|name| crate::theme::resolve_style(&self.theme.styles, name))
            .and_then(|style| style.sounds)
            .and_then(|s| match sound {
                UiSound::Hover => s.hover,
                UiSound::Confirm => s.confirm,
                _ => None,
            });
        match file {
            Some(path) if !path.trim().is_empty() => WidgetEvent::SoundFile(path),
            _ => WidgetEvent::Sound(sound),
        }
    }

    // ---------------------------------------------------------- dropdown

    /// Opens the popup list of the dropdown `id` (a modal of buttons in a
    /// scroll list, clipped to the panel) below it, or above when there is
    /// no room.
    fn open_popup(&mut self, id: NodeId) {
        let Kind::Selector { options, index, .. } = self.nodes[id.0].kind.clone() else {
            return;
        };
        if options.is_empty() {
            return;
        }
        let rect = self.nodes[id.0].rect;
        let row_h = self.theme.widgets.button.small_height;
        let gap = 4.0;
        let rows = options.len().min(POPUP_ROWS);
        let height = rows as f32 * row_h + (rows - 1) as f32 * gap + 2.0 * POPUP_PAD;
        let mut y = rect[1] + rect[3] + 4.0;
        if y + height > super::CANVAS[1] {
            y = (rect[1] - 4.0 - height).max(0.0);
        }
        let style = self.nodes[id.0].style.clone();
        let visible = self.focus_visible;
        self.open_modal(|ui| {
            let mut panel = Node::panel(PanelKind::Normal, Dir::Vertical)
                .pad(POPUP_PAD, POPUP_PAD)
                .fixed(rect[2], height)
                .anchor(Anchor::top_left(rect[0], y));
            panel.style = style;
            let root = ui.add_root(panel);
            let list = ui.add(
                root,
                Node::scroll_list(POPUP_TAG - 1)
                    .size(Size::Fill(1.0), Size::Fill(1.0))
                    .gap(gap),
            );
            for (i, option) in options.iter().enumerate() {
                ui.add(
                    list,
                    Node::button(POPUP_TAG + i as Tag, option.clone())
                        .small()
                        .selected(i == index),
                );
            }
            if let Kind::ScrollList { first, .. } = &mut ui.nodes[list.0].kind {
                *first = (index + 1).saturating_sub(POPUP_ROWS);
            }
            root
        });
        self.popup = Some((id, options.len()));
        self.focus_visible = visible;
        self.ensure_layout();
        self.set_focus_tag(POPUP_TAG + index as Tag);
    }

    pub(crate) fn close_popup(&mut self) {
        if self.popup.take().is_some() {
            self.close_modal();
        }
    }

    /// Whether a dropdown popup is open.
    pub fn popup_open(&self) -> bool {
        self.popup.is_some()
    }

    /// Input while a popup is open: Cancel and a click outside close it
    /// (and are not passed on). Returns the events when the input was
    /// consumed.
    fn popup_input(&mut self, input: UiInput) -> Option<Vec<WidgetEvent>> {
        let (_, _) = self.popup?;
        match input {
            UiInput::Cancel => {
                self.close_popup();
                Some(vec![WidgetEvent::Sound(UiSound::Cancel)])
            }
            UiInput::PointerDown(p) => {
                let inside = self
                    .modal
                    .is_some_and(|(root, _, _)| contains(self.nodes[root.0].rect, p));
                if inside {
                    None
                } else {
                    self.close_popup();
                    Some(Vec::new())
                }
            }
            _ => None,
        }
    }

    /// Turns the activation of a popup row into the selection of its
    /// dropdown and closes the popup.
    fn finish_popup(&mut self, out: &mut Vec<WidgetEvent>) {
        let Some((owner, count)) = self.popup else {
            return;
        };
        let picked = out.iter().position(|e| {
            matches!(e, WidgetEvent::Activated(t) if *t >= POPUP_TAG && ((*t - POPUP_TAG) as usize) < count)
        });
        let Some(at) = picked else { return };
        let WidgetEvent::Activated(tag) = out.remove(at) else {
            return;
        };
        let chosen = (tag - POPUP_TAG) as usize;
        let owner_tag = self.nodes[owner.0].tag;
        self.close_popup();
        if let Kind::Selector { index, .. } = &mut self.nodes[owner.0].kind
            && *index != chosen
        {
            *index = chosen;
            out.push(WidgetEvent::Selected(owner_tag, chosen));
        }
        self.focus = Some(owner);
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
            // Not a guard: `scroll_by` needs `&mut self` while the scrutinee
            // borrows `self.nodes`.
            #[allow(clippy::collapsible_match)]
            Kind::ScrollList { .. } if !horizontal => {
                if self.scroll_by(id, sign) {
                    return;
                }
            }
            _ => {}
        }
        // Rows of a scroll list: walk its interactive rows (hidden ones
        // included) so the list scrolls along with the focus.
        if !horizontal
            && let Some((list, idx)) = self.list_ancestor(id)
            && !matches!(self.nodes[list.0].kind, Kind::ScrollList { mode: ListMode::Legacy, .. })
        {
            // A generated list (rows, grid rows, pages): cross the edge of
            // what is in view onto the next / previous row.
            if self.list_step(list, idx, sign) {
                return;
            }
        } else if !horizontal && let Some((list, _)) = self.list_ancestor(id) {
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
        // A candidate "in line" shares a row (horizontal moves) or a column
        // (vertical moves) with the current widget: those win over nearer
        // widgets that merely lie in the right direction (a tab above a grid).
        let from_rect = self.nodes[from.0].rect;
        let in_line = |r: [f32; 4]| -> bool {
            if horizontal {
                r[1] < from_rect[1] + from_rect[3] && from_rect[1] < r[1] + r[3]
            } else {
                r[0] < from_rect[0] + from_rect[2] && from_rect[0] < r[0] + r[2]
            }
        };
        let mut best: Option<(f32, NodeId)> = None;
        let mut best_in_line: Option<(f32, NodeId)> = None;
        let mut any_in_line = false;
        for &id in &list {
            if id == from {
                continue;
            }
            let rect = self.nodes[id.0].rect;
            let line = in_line(rect);
            any_in_line |= line;
            let (along, across) = axes(center(rect));
            if along > 1.0 {
                let score = along + 2.0 * across;
                if best.is_none_or(|(s, _)| score < s) {
                    best = Some((score, id));
                }
                if line && best_in_line.is_none_or(|(s, _)| score < s) {
                    best_in_line = Some((score, id));
                }
            }
        }
        // Horizontal moves stay in their row, wrapping inside it.
        best = best_in_line.or(if horizontal && any_in_line { None } else { best });
        let list: Vec<NodeId> = if best.is_none() && horizontal && any_in_line {
            list.into_iter()
                .filter(|&id| in_line(self.nodes[id.0].rect))
                .collect()
        } else {
            list
        };
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
        let (first, shown, mode) = match self.nodes[list.0].kind {
            Kind::ScrollList { first, shown, mode, .. } => (first, shown, mode),
            _ => return,
        };
        // The rows (units) the child covers; the pager footer covers none.
        let (row, rows) = if mode == ListMode::Legacy {
            (idx, 1)
        } else {
            match self.list_units(list).cells.get(idx) {
                Some(c) => (c.row, c.rows),
                None => return,
            }
        };
        let new_first = if mode == ListMode::Pages {
            let page = self.page_size(list);
            if row >= first && row < first + shown {
                return;
            }
            row / page * page
        } else if row < first {
            row
        } else if row + rows > first + shown {
            self.first_for_tail(list, row + rows - 1)
        } else {
            return;
        };
        if let Kind::ScrollList { first, .. } = &mut self.nodes[list.0].kind {
            *first = new_first;
        }
        self.layout();
    }

    /// Rows a page of the paged list holds (at least 1).
    fn page_size(&self, list: NodeId) -> usize {
        let gap = self.nodes[list.0].gap;
        self.list_units(list)
            .fit(0, self.list_avail(list), gap)
            .max(1)
    }

    /// Scrolls a list by `delta` rows, or a paged list by `delta.signum()`
    /// pages. `false` when it cannot move.
    pub(crate) fn scroll_by(&mut self, list: NodeId, delta: i32) -> bool {
        let Kind::ScrollList { first, total, mode, .. } = self.nodes[list.0].kind else {
            return false;
        };
        if total == 0 {
            return false;
        }
        let new = if mode == ListMode::Pages {
            let page = self.page_size(list);
            let last_page = ((total - 1) / page) as i32;
            ((first / page) as i32 + delta.signum()).clamp(0, last_page) as usize * page
        } else {
            let max_first = self.first_for_tail(list, total - 1);
            (first as i32 + delta).clamp(0, max_first as i32) as usize
        };
        if new == first {
            return false;
        }
        if let Kind::ScrollList { first, .. } = &mut self.nodes[list.0].kind {
            *first = new;
        }
        self.layout();
        true
    }

    /// Moves the focus from the item `idx` of a generated list onto the
    /// next (`sign > 0`) or previous row when the current one is at the edge
    /// of what is in view and more rows lie beyond (scrolling or turning the
    /// page). `false` when the focus should move spatially instead.
    fn list_step(&mut self, list: NodeId, idx: usize, sign: i32) -> bool {
        let Kind::ScrollList { first, shown, total, .. } = self.nodes[list.0].kind else {
            return false;
        };
        let units = self.list_units(list);
        let Some(&cell) = units.cells.get(idx) else {
            return false;
        };
        let down = sign > 0;
        let at_edge = if down {
            cell.row + cell.rows >= first + shown && first + shown < total
        } else {
            cell.row <= first && first > 0
        };
        if !at_edge {
            return false;
        }
        let items: Vec<NodeId> = self.nodes[list.0]
            .children
            .iter()
            .copied()
            .filter(|c| self.nodes[c.0].footer.is_none())
            .collect();
        let mut unit = if down { first + shown } else { first - 1 } as i32;
        while unit >= 0 && (unit as usize) < total {
            let u = unit as usize;
            let mut best: Option<(usize, NodeId)> = None;
            for (i, c) in units.cells.iter().enumerate() {
                let covers = if down { c.row == u } else { c.row + c.rows - 1 == u };
                if !covers {
                    continue;
                }
                let mut found = Vec::new();
                self.collect_focusable(items[i], false, &mut found);
                let Some(&target) = found.first() else {
                    continue;
                };
                let dist = c.col.abs_diff(cell.col);
                if best.is_none_or(|(d, _)| dist < d) {
                    best = Some((dist, target));
                }
            }
            if let Some((_, target)) = best {
                self.focus = Some(target);
                self.reveal(target);
                return true;
            }
            unit += sign;
        }
        false
    }

    /// Remembers the scroll list under a pressed pointer (when it has rows
    /// out of view) so a vertical drag can scroll it.
    fn begin_list_drag(&mut self, p: [f32; 2]) {
        self.list_drag = None;
        let mut found = None;
        for (i, n) in self.nodes.iter().enumerate() {
            if let Kind::ScrollList { first, shown, total, mode } = n.kind
                && n.visible
                && n.enabled
                && contains(n.rect, p)
                && mode != ListMode::Pages
                && shown < total
            {
                found = Some((NodeId(i), p[1], first, false));
            }
        }
        self.list_drag = found;
    }

    /// Scrolls the list being dragged; `true` while a drag is in progress
    /// (the pointer then no longer hovers/presses widgets).
    fn drag_list(&mut self, p: [f32; 2]) -> bool {
        const THRESHOLD: f32 = 8.0;
        let Some((list, y0, first0, active)) = self.list_drag else {
            return false;
        };
        if !active && (p[1] - y0).abs() < THRESHOLD {
            return false;
        }
        if !active {
            // The press turned into a scroll: it must not click a row.
            self.pressed = None;
            self.list_drag = Some((list, y0, first0, true));
        }
        let (rect_h, first, shown) = match self.nodes[list.0].kind {
            Kind::ScrollList { first, shown, .. } => (self.nodes[list.0].rect[3], first, shown),
            _ => return false,
        };
        let pitch = (rect_h / shown.max(1) as f32).max(24.0);
        // Content follows the pointer: dragging down shows earlier rows.
        let target = first0 as i32 - ((p[1] - y0) / pitch).round() as i32;
        let delta = target - first as i32;
        if delta != 0 {
            self.scroll_by(list, delta);
        }
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

    /// Where each generated list (scrolling or paged) is scrolled to, by tag:
    /// kept across a rebuild of the screen.
    pub(crate) fn list_offsets(&self) -> Vec<(Tag, usize)> {
        self.nodes
            .iter()
            .filter_map(|n| match n.kind {
                Kind::ScrollList { first, mode, .. } if mode != ListMode::Legacy && first > 0 => {
                    Some((n.tag, first))
                }
                _ => None,
            })
            .collect()
    }

    /// Scrolls the generated lists back to `offsets` (layout clamps them).
    pub(crate) fn restore_list_offsets(&mut self, offsets: &[(Tag, usize)]) {
        for n in &mut self.nodes {
            if let Kind::ScrollList { first, mode, .. } = &mut n.kind
                && *mode != ListMode::Legacy
                && let Some((_, f)) = offsets.iter().find(|(t, _)| *t == n.tag)
            {
                *first = *f;
            }
        }
        self.laid_out = false;
    }

    /// Turns the paged list `tag` one page forward (`dir > 0`) or back, or
    /// scrolls a scrolling one by `dir` rows. `false` when it cannot move.
    /// A pager button that gets disabled by the move hands its focus on.
    pub fn scroll_page(&mut self, tag: Tag, dir: i32) -> bool {
        self.ensure_layout();
        let Some(list) = self.find(tag) else {
            return false;
        };
        let moved = self.scroll_by(list, dir);
        if let Some(f) = self.focus
            && !self.nodes[f.0].enabled
            && let Some(parent) = self.nodes[f.0].parent
        {
            let next = self.nodes[parent.0]
                .children
                .iter()
                .copied()
                .find(|c| self.nodes[c.0].kind.is_interactive() && self.nodes[c.0].enabled);
            if next.is_some() {
                self.focus = next;
            }
        }
        moved
    }

    /// `(page, pages)` (from 0 and in all) of the list `tag`: the page in
    /// view of a paged list; for a scrolling one the rows scrolled past and
    /// in all.
    pub fn list_position(&mut self, tag: Tag) -> Option<(usize, usize)> {
        self.ensure_layout();
        let id = self.find(tag)?;
        let Kind::ScrollList { first, total, mode, .. } = self.nodes[id.0].kind else {
            return None;
        };
        if mode == ListMode::Pages && total > 0 {
            let page = self.page_size(id);
            Some((first / page, total.div_ceil(page)))
        } else {
            Some((first, total))
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
