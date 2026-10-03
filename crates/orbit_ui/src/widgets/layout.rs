//! Measure / arrange pass. Pure arithmetic, no GPU.

use super::node::{Align, Dir, Kind, Node, NodeId, Size, estimate_text_width};
use super::{CANVAS, Ui};

/// Line height multiplier of `orbit_text` styles.
pub(crate) const LINE_HEIGHT: f32 = 1.35;

impl Ui {
    pub(crate) fn ensure_layout(&mut self) {
        if !self.laid_out {
            self.layout();
        }
    }

    /// Computes every node rectangle (virtual pixels).
    pub fn layout(&mut self) {
        let roots = self.roots.clone();
        for root in roots {
            let pref = self.measure(root);
            let node = &self.nodes[root.0];
            let w = match node.width {
                Size::Fill(_) => CANVAS[0],
                _ => pref[0],
            };
            let h = match node.height {
                Size::Fill(_) => CANVAS[1],
                _ => pref[1],
            };
            let a = node.anchor;
            let x = a.canvas[0] * CANVAS[0] - a.pivot[0] * w + a.offset[0];
            let y = a.canvas[1] * CANVAS[1] - a.pivot[1] * h + a.offset[1];
            self.arrange(root, [x, y, w, h], true);
        }
        self.laid_out = true;
    }

    fn label_size(&self, node: &Node) -> f32 {
        match &node.kind {
            Kind::Label(l) => l
                .size
                .or_else(|| {
                    node.style
                        .as_deref()
                        .and_then(|n| crate::theme::resolve_style(&self.theme.styles, n))
                        .and_then(|s| s.text_size)
                })
                .unwrap_or(self.theme.widgets.text_size),
            _ => self.theme.widgets.text_size,
        }
    }

    /// Preferred size of a subtree (children first).
    fn measure(&mut self, id: NodeId) -> [f32; 2] {
        let children = self.nodes[id.0].children.clone();
        let prefs: Vec<[f32; 2]> = children.iter().map(|&c| self.measure(c)).collect();
        let spans: Vec<[usize; 2]> = children
            .iter()
            .map(|c| [self.nodes[c.0].col_span, self.nodes[c.0].row_span])
            .collect();
        let node = &self.nodes[id.0];
        let w = &self.theme.widgets;
        let flow = |dir: Dir, gap: f32| -> [f32; 2] {
            if prefs.is_empty() {
                return [0.0, 0.0];
            }
            let gaps = gap * (prefs.len() - 1) as f32;
            let max = |i: usize| prefs.iter().map(|p| p[i]).fold(0.0, f32::max);
            let sum = |i: usize| prefs.iter().map(|p| p[i]).sum::<f32>();
            match dir {
                Dir::Vertical => [max(0), sum(1) + gaps],
                Dir::Horizontal => [sum(0) + gaps, max(1)],
                Dir::Stack => [max(0), max(1)],
                Dir::Grid(cols) => {
                    let cells = grid_place(&spans, cols);
                    let (cw, rh) = grid_tracks(&prefs, &cells, gap);
                    [
                        cw.iter().sum::<f32>() + gap * cw.len().saturating_sub(1) as f32,
                        rh.iter().sum::<f32>() + gap * rh.len().saturating_sub(1) as f32,
                    ]
                }
            }
        };
        let content = match &node.kind {
            Kind::Label(l) => {
                let size = self.label_size(node);
                [
                    estimate_text_width(&l.text, size),
                    size * LINE_HEIGHT * l.lines as f32,
                ]
            }
            Kind::Button(b) => {
                let h = match b.kind {
                    super::ButtonKind::Normal => w.button.height,
                    super::ButtonKind::Small => w.button.small_height,
                };
                let size = match b.kind {
                    super::ButtonKind::Normal => w.text_size,
                    super::ButtonKind::Small => w.small_text_size,
                };
                let inner = flow(node.dir, node.gap);
                [
                    (estimate_text_width(&b.label, size) + 2.0 * w.button.padding_x).max(inner[0]),
                    h.max(inner[1]),
                ]
            }
            Kind::Slider { .. } => [360.0, w.controls.height],
            Kind::Toggle { label, .. } => [
                w.controls.toggle_size + 20.0 + estimate_text_width(label, w.text_size),
                w.controls.height,
            ],
            Kind::Selector { options, .. } => [
                options
                    .iter()
                    .map(|o| estimate_text_width(o, w.text_size))
                    .fold(0.0, f32::max)
                    + 2.0 * w.button.padding_x
                    + 2.0 * 40.0,
                w.button.height,
            ],
            Kind::Image { .. } => [0.0, 0.0],
            Kind::ScrollList { .. } => {
                let [cw, ch] = flow(Dir::Vertical, node.gap);
                [cw + w.list.bar_width, ch]
            }
            Kind::Container | Kind::Panel(_) => flow(node.dir, node.gap),
        };
        let has_children = !prefs.is_empty();
        let padded = |c: f32, p: f32| {
            if has_children || matches!(node.kind, Kind::Container | Kind::Panel(_)) {
                c + 2.0 * p
            } else {
                c
            }
        };
        let m = node.margin;
        let pref = [
            match node.width {
                Size::Fixed(v) => v,
                _ => padded(content[0], node.pad[0]),
            } + m[0] + m[2],
            match node.height {
                Size::Fixed(v) => v,
                _ => padded(content[1], node.pad[1]),
            } + m[1] + m[3],
        ];
        self.nodes[id.0].pref = pref;
        pref
    }

    /// Places a subtree in `rect`.
    fn arrange(&mut self, id: NodeId, rect: [f32; 4], visible: bool) {
        let m = self.nodes[id.0].margin;
        let rect = [
            rect[0] + m[0],
            rect[1] + m[1],
            (rect[2] - m[0] - m[2]).max(0.0),
            (rect[3] - m[1] - m[3]).max(0.0),
        ];
        let (children, pad, gap, dir, main, cross, is_list) = {
            let n = &mut self.nodes[id.0];
            n.rect = rect;
            n.visible = visible;
            (
                n.children.clone(),
                n.pad,
                n.gap,
                n.dir,
                n.main,
                n.cross,
                matches!(n.kind, Kind::ScrollList { .. }),
            )
        };
        if children.is_empty() {
            return;
        }
        let inner = [
            rect[0] + pad[0],
            rect[1] + pad[1],
            (rect[2] - 2.0 * pad[0]).max(0.0),
            (rect[3] - 2.0 * pad[1]).max(0.0),
        ];
        if is_list {
            self.arrange_list(id, &children, inner, gap, visible);
            return;
        }
        if let Dir::Grid(cols) = dir {
            self.arrange_grid(&children, inner, gap, cols.max(1), main, cross, visible);
            return;
        }
        if dir == Dir::Stack {
            for &c in &children {
                let r = self.place_in(c, inner, main, cross);
                self.arrange(c, r, visible);
            }
            return;
        }
        // Axis-generic flow: m = main axis index, c = cross axis index.
        let (m, c) = if dir == Dir::Horizontal {
            (0, 1)
        } else {
            (1, 0)
        };
        let fill_weight = |n: &Node| match if m == 0 { n.width } else { n.height } {
            Size::Fill(w) => w.max(0.0),
            _ => 0.0,
        };
        let weights: Vec<f32> = children
            .iter()
            .map(|&ch| fill_weight(&self.nodes[ch.0]))
            .collect();
        let total_weight: f32 = weights.iter().sum();
        let fixed: f32 = children
            .iter()
            .zip(&weights)
            .filter(|(_, w)| **w == 0.0)
            .map(|(&ch, _)| self.nodes[ch.0].pref[m])
            .sum();
        let gaps = gap * (children.len() - 1) as f32;
        let free = (inner[2 + m] - fixed - gaps).max(0.0);
        let mut pos = inner[m]
            + if total_weight > 0.0 {
                0.0
            } else {
                match main {
                    Align::Center => free * 0.5,
                    Align::End => free,
                    _ => 0.0,
                }
            };
        for (&ch, &weight) in children.iter().zip(&weights) {
            let n = &self.nodes[ch.0];
            let main_size = if weight > 0.0 {
                free * weight / total_weight
            } else {
                n.pref[m]
            };
            let cross_decl = if c == 0 { n.width } else { n.height };
            let cross_size = match cross_decl {
                Size::Fixed(_) => n.pref[c],
                Size::Fill(_) => inner[2 + c],
                Size::Fit if cross == Align::Stretch => inner[2 + c],
                Size::Fit => n.pref[c],
            };
            let cross_off = match cross {
                Align::Center => (inner[2 + c] - cross_size) * 0.5,
                Align::End => inner[2 + c] - cross_size,
                _ => 0.0,
            };
            let mut r = [0.0; 4];
            r[m] = pos;
            r[2 + m] = main_size;
            r[c] = inner[c] + cross_off;
            r[2 + c] = cross_size;
            pos += main_size + gap;
            self.arrange(ch, r, visible);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn arrange_grid(
        &mut self,
        children: &[NodeId],
        inner: [f32; 4],
        gap: f32,
        cols: usize,
        main: Align,
        cross: Align,
        visible: bool,
    ) {
        let prefs: Vec<[f32; 2]> = children.iter().map(|c| self.nodes[c.0].pref).collect();
        let spans: Vec<[usize; 2]> = children
            .iter()
            .map(|c| [self.nodes[c.0].col_span, self.nodes[c.0].row_span])
            .collect();
        let cells = grid_place(&spans, cols);
        let (mut cw, mut rh) = grid_tracks(&prefs, &cells, gap);
        let mut fill_c = vec![false; cw.len()];
        let mut fill_r = vec![false; rh.len()];
        for (cell, &ch) in cells.iter().zip(children) {
            let n = &self.nodes[ch.0];
            if matches!(n.width, Size::Fill(_)) {
                fill_c[cell.col..cell.col + cell.cols].fill(true);
            }
            if matches!(n.height, Size::Fill(_)) {
                fill_r[cell.row..cell.row + cell.rows].fill(true);
            }
        }
        spread(&mut cw, &fill_c, inner[2], gap);
        spread(&mut rh, &fill_r, inner[3], gap);
        let mut ys = Vec::with_capacity(rh.len());
        let mut y = inner[1];
        for h in &rh {
            ys.push(y);
            y += h + gap;
        }
        let mut xs = Vec::with_capacity(cw.len());
        let mut x = inner[0];
        for w in &cw {
            xs.push(x);
            x += w + gap;
        }
        for (cell, &ch) in cells.iter().zip(children) {
            let width = cw[cell.col..cell.col + cell.cols].iter().sum::<f32>()
                + gap * (cell.cols - 1) as f32;
            let height = rh[cell.row..cell.row + cell.rows].iter().sum::<f32>()
                + gap * (cell.rows - 1) as f32;
            let area = [xs[cell.col], ys[cell.row], width, height];
            let rect = self.place_in(ch, area, main, cross);
            self.arrange(ch, rect, visible);
        }
    }

    /// A child of a stack: sized by its own declaration, aligned with the
    /// container's `main` (vertical) and `cross` (horizontal) alignment.
    fn place_in(
        &self,
        child: NodeId,
        inner: [f32; 4],
        vertical: Align,
        horizontal: Align,
    ) -> [f32; 4] {
        let n = &self.nodes[child.0];
        let size = |decl: Size, pref: f32, avail: f32, align: Align| match decl {
            Size::Fill(_) => avail,
            Size::Fit if align == Align::Stretch => avail,
            _ => pref,
        };
        let w = size(n.width, n.pref[0], inner[2], horizontal);
        let h = size(n.height, n.pref[1], inner[3], vertical);
        let off = |avail: f32, own: f32, align: Align| match align {
            Align::Center => (avail - own) * 0.5,
            Align::End => avail - own,
            _ => 0.0,
        };
        [
            inner[0] + off(inner[2], w, horizontal),
            inner[1] + off(inner[3], h, vertical),
            w,
            h,
        ]
    }

    fn arrange_list(
        &mut self,
        id: NodeId,
        children: &[NodeId],
        inner: [f32; 4],
        gap: f32,
        visible: bool,
    ) {
        let bar = self.theme.widgets.list.bar_width;
        let width = (inner[2] - bar - 8.0).max(0.0);
        let first = match &mut self.nodes[id.0].kind {
            Kind::ScrollList { first, .. } => {
                *first = (*first).min(children.len() - 1);
                *first
            }
            _ => 0,
        };
        let bottom = inner[1] + inner[3];
        let mut y = inner[1];
        let mut shown = 0;
        let mut full = false;
        for (i, &ch) in children.iter().enumerate() {
            if i < first || full {
                self.arrange(ch, [0.0; 4], false);
                continue;
            }
            let h = self.nodes[ch.0].pref[1];
            if shown > 0 && y + h > bottom + 0.5 {
                full = true;
                self.arrange(ch, [0.0; 4], false);
                continue;
            }
            self.arrange(ch, [inner[0], y, width, h], visible);
            y += h + gap;
            shown += 1;
        }
        if let Kind::ScrollList { shown: s, .. } = &mut self.nodes[id.0].kind {
            *s = shown;
        }
    }

    /// Smallest `first` index such that items `first..=last` fit in the
    /// list viewport (used to show the tail of a list).
    pub(crate) fn first_for_tail(&self, list: NodeId, last: usize) -> usize {
        let n = &self.nodes[list.0];
        let avail = (n.rect[3] - 2.0 * n.pad[1]).max(0.0);
        let mut used = 0.0;
        let mut first = last;
        loop {
            let h = self.nodes[n.children[first].0].pref[1];
            let need = used + h + if used > 0.0 { n.gap } else { 0.0 };
            if need > avail + 0.5 && first < last {
                return first + 1;
            }
            used = need;
            if first == 0 {
                return 0;
            }
            first -= 1;
        }
    }
}

/// Where a grid child sits: first column / row and how many it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GridCell {
    pub col: usize,
    pub row: usize,
    pub cols: usize,
    pub rows: usize,
}

/// Auto-places children (`[col_span, row_span]` each, 0 counts as 1) in a
/// grid `cols` wide: row by row, each child in the first free spot where
/// it fits (a span wider than the grid is cut to the grid).
pub(crate) fn grid_place(spans: &[[usize; 2]], cols: usize) -> Vec<GridCell> {
    let cols = cols.max(1);
    let mut taken: Vec<Vec<bool>> = Vec::new();
    let mut out = Vec::with_capacity(spans.len());
    for &[cs, rs] in spans {
        let (cs, rs) = (cs.clamp(1, cols), rs.max(1));
        let mut at = 0;
        let (col, row) = loop {
            let (r, c) = (at / cols, at % cols);
            at += 1;
            if c + cs > cols {
                continue;
            }
            let free = (r..r + rs).all(|y| {
                taken
                    .get(y)
                    .is_none_or(|line| line[c..c + cs].iter().all(|t| !t))
            });
            if free {
                break (c, r);
            }
        };
        if taken.len() < row + rs {
            taken.resize(row + rs, vec![false; cols]);
        }
        for line in &mut taken[row..row + rs] {
            line[col..col + cs].fill(true);
        }
        out.push(GridCell {
            col,
            row,
            cols: cs,
            rows: rs,
        });
    }
    out
}

/// Natural column widths and row heights of the placed children. A child
/// spanning several tracks first lets the single-track ones size them, then
/// the missing room is shared evenly among the tracks it covers.
fn grid_tracks(prefs: &[[f32; 2]], cells: &[GridCell], gap: f32) -> (Vec<f32>, Vec<f32>) {
    let cols = cells.iter().map(|c| c.col + c.cols).max().unwrap_or(0);
    let rows = cells.iter().map(|c| c.row + c.rows).max().unwrap_or(0);
    let mut cw = vec![0.0f32; cols];
    let mut rh = vec![0.0f32; rows];
    for (p, c) in prefs.iter().zip(cells) {
        if c.cols == 1 {
            cw[c.col] = cw[c.col].max(p[0]);
        }
        if c.rows == 1 {
            rh[c.row] = rh[c.row].max(p[1]);
        }
    }
    for (p, c) in prefs.iter().zip(cells) {
        if c.cols > 1 {
            grow(&mut cw[c.col..c.col + c.cols], p[0], gap);
        }
        if c.rows > 1 {
            grow(&mut rh[c.row..c.row + c.rows], p[1], gap);
        }
    }
    (cw, rh)
}

/// Widens `tracks` evenly until they (and the gaps between) hold `need`.
fn grow(tracks: &mut [f32], need: f32, gap: f32) {
    let have = tracks.iter().sum::<f32>() + gap * (tracks.len() - 1) as f32;
    if need > have {
        let extra = (need - have) / tracks.len() as f32;
        for t in tracks {
            *t += extra;
        }
    }
}

/// Shares the room left in `avail` among the tracks flagged in `fill`.
fn spread(tracks: &mut [f32], fill: &[bool], avail: f32, gap: f32) {
    let used: f32 = tracks.iter().sum::<f32>() + gap * tracks.len().saturating_sub(1) as f32;
    let count = fill.iter().filter(|f| **f).count();
    let free = avail - used;
    if count > 0 && free > 0.0 {
        for (t, f) in tracks.iter_mut().zip(fill) {
            if *f {
                *t += free / count as f32;
            }
        }
    }
}
