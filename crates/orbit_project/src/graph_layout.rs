//! Positions of the graph blocks: the `.layout` sidecar and the automatic
//! layout.
//!
//! Positions are editor state, not story: they live next to the story file
//! (`story.ron` -> `story.layout`), never in its canonical RON, so moving a
//! node around does not make a diff of the story.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::graph::{Block, BlockId, GraphModel, SlotTarget};
use crate::storage::{ProjectFileError, ProjectStorage};

/// Width of a block, in graph units.
pub const BLOCK_WIDTH: f32 = 236.0;
/// Space kept between two layers (columns) and two blocks of a layer.
pub const H_GAP: f32 = 120.0;
pub const V_GAP: f32 = 34.0;
/// Space between two rows of layers when a long graph is wrapped.
const ROW_GAP: f32 = 90.0;
/// Width / height the automatic layout aims for.
const TARGET_ASPECT: f32 = 1.7;
/// Format of the sidecar.
pub const LAYOUT_FORMAT: u32 = 1;

/// Height of a block as drawn by the graph panel (estimate, with a margin so
/// that the automatic layout never overlaps blocks).
pub fn block_height(block: &Block) -> f32 {
    const HEADER: f32 = 34.0;
    const META: f32 = 74.0;
    const SLOT: f32 = 24.0;
    const FOOTER: f32 = 16.0;
    let warn = if block.dead_nodes > 0 { 18.0 } else { 0.0 };
    (HEADER + META + SLOT * block.slots.len() as f32 + FOOTER + warn) * 1.08
}

/// Positions of the blocks (top-left corner, graph units).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphLayout {
    #[serde(default)]
    pub format: u32,
    /// `BlockId::key()` -> `(x, y)`.
    #[serde(default)]
    pub positions: BTreeMap<String, (f32, f32)>,
}

impl GraphLayout {
    pub fn get(&self, id: BlockId) -> Option<(f32, f32)> {
        self.positions.get(&id.key()).copied()
    }

    pub fn set(&mut self, id: BlockId, pos: (f32, f32)) {
        self.positions.insert(id.key(), pos);
    }

    pub fn remove(&mut self, id: BlockId) {
        self.positions.remove(&id.key());
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    pub fn to_ron(&self) -> Result<String, ProjectFileError> {
        let cfg = ron::ser::PrettyConfig::new().indentor("    ".to_owned());
        let mut out = self.clone();
        out.format = LAYOUT_FORMAT;
        ron::ser::to_string_pretty(&out, cfg).map_err(|e| ProjectFileError::Manifest(e.to_string()))
    }

    pub fn from_ron(source: &str) -> Result<Self, ProjectFileError> {
        let layout: Self =
            ron::from_str(source).map_err(|e| ProjectFileError::Manifest(e.to_string()))?;
        if layout.format > LAYOUT_FORMAT {
            return Err(ProjectFileError::Manifest(format!(
                "layout format {} is newer than supported ({LAYOUT_FORMAT})",
                layout.format
            )));
        }
        Ok(layout)
    }

    /// Forgets the blocks that no longer exist.
    pub fn prune(&mut self, model: &GraphModel) {
        let alive: HashSet<String> = model.blocks.iter().map(|b| b.id.key()).collect();
        self.positions.retain(|k, _| alive.contains(k));
    }

    /// Gives a position to every block of `model` that has none: right of a
    /// positioned predecessor when it has one, else below the rest; then
    /// pushes it down until it overlaps nothing. With an empty layout this is
    /// [`auto_layout`]. Returns whether anything was placed.
    pub fn place_missing(&mut self, model: &GraphModel) -> bool {
        let missing: Vec<&Block> = model
            .blocks
            .iter()
            .filter(|b| self.get(b.id).is_none())
            .collect();
        if missing.is_empty() {
            return false;
        }
        if self.is_empty() || missing.len() == model.blocks.len() {
            for (id, pos) in auto_layout(model) {
                self.set(id, pos);
            }
            return true;
        }
        let edges = model.edges();
        let mut taken: Vec<(f32, f32, f32, f32)> = model
            .blocks
            .iter()
            .filter_map(|b| {
                self.get(b.id)
                    .map(|(x, y)| (x, y, BLOCK_WIDTH, block_height(b)))
            })
            .collect();
        for block in missing {
            let height = block_height(block);
            let anchor = edges
                .iter()
                .filter(|e| e.to == block.id)
                .find_map(|e| self.get(e.from.block).map(|p| (p, true)))
                .or_else(|| {
                    edges
                        .iter()
                        .filter(|e| e.from.block == block.id)
                        .find_map(|e| self.get(e.to).map(|p| (p, false)))
                });
            let (mut x, mut y) = match anchor {
                Some(((px, py), true)) => (px + BLOCK_WIDTH + H_GAP, py),
                Some(((px, py), false)) => (px - BLOCK_WIDTH - H_GAP, py),
                None => {
                    let min_x = taken.iter().map(|r| r.0).fold(f32::INFINITY, f32::min);
                    let max_y = taken
                        .iter()
                        .map(|r| r.1 + r.3)
                        .fold(f32::NEG_INFINITY, f32::max);
                    (min_x, max_y + V_GAP * 2.0)
                }
            };
            // Slide down until free.
            while let Some(hit) = taken.iter().find(|r| {
                x < r.0 + r.2 + 8.0
                    && r.0 < x + BLOCK_WIDTH + 8.0
                    && y < r.1 + r.3 + 8.0
                    && r.1 < y + height + 8.0
            }) {
                y = hit.1 + hit.3 + V_GAP;
            }
            taken.push((x, y, BLOCK_WIDTH, height));
            x = x.round();
            y = y.round();
            self.set(block.id, (x, y));
        }
        true
    }
}

/// Layered layout (longest path from the start, back edges ignored, then a
/// few barycenter sweeps to cut crossings): columns left to right, no two
/// blocks overlap. Deterministic.
pub fn auto_layout(model: &GraphModel) -> HashMap<BlockId, (f32, f32)> {
    let n = model.blocks.len();
    let pos: HashMap<BlockId, usize> = model
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, block) in model.blocks.iter().enumerate() {
        for slot in &block.slots {
            if let SlotTarget::Block(to) = slot.target
                && let Some(&j) = pos.get(&to)
                && j != i
                && !succ[i].contains(&j)
            {
                succ[i].push(j);
            }
        }
    }

    // DFS from the entry: the edges that close a cycle are ignored for
    // layering. Blocks the entry does not reach are laid out apart, below.
    let mut state = vec![0u8; n]; // 0 new, 1 on the stack, 2 done
    let mut forward: Vec<Vec<usize>> = vec![Vec::new(); n];
    for root in 0..n.min(1) {
        if state[root] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        state[root] = 1;
        while let Some(top) = stack.last_mut() {
            let v = top.0;
            if top.1 < succ[v].len() {
                let w = succ[v][top.1];
                top.1 += 1;
                match state[w] {
                    0 => {
                        forward[v].push(w);
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    2 => forward[v].push(w),
                    _ => {} // back edge
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    // Longest path layering over the DAG of forward edges.
    let mut indegree = vec![0usize; n];
    for targets in &forward {
        for &w in targets {
            indegree[w] += 1;
        }
    }
    let mut layer = vec![0usize; n];
    let mut ready: Vec<usize> = (0..n)
        .filter(|&i| state[i] != 0 && indegree[i] == 0)
        .collect();
    while let Some(v) = ready.pop() {
        for &w in &forward[v] {
            layer[w] = layer[w].max(layer[v] + 1);
            indegree[w] -= 1;
            if indegree[w] == 0 {
                ready.push(w);
            }
        }
    }

    let layers = (0..n)
        .filter(|&i| state[i] != 0)
        .map(|i| layer[i])
        .max()
        .map_or(0, |m| m + 1);
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); layers];
    let mut orphans: Vec<usize> = Vec::new();
    for i in 0..n {
        if state[i] == 0 {
            orphans.push(i);
        } else {
            columns[layer[i]].push(i);
        }
    }
    // Barycenter ordering: sweep right then left a few times.
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (v, targets) in forward.iter().enumerate() {
        for &w in targets {
            preds[w].push(v);
        }
    }
    let mut rank = vec![0f32; n];
    let refresh = |columns: &[Vec<usize>], rank: &mut [f32]| {
        for col in columns {
            for (r, &v) in col.iter().enumerate() {
                rank[v] = r as f32;
            }
        }
    };
    refresh(&columns, &mut rank);
    for sweep in 0..4 {
        let down = sweep % 2 == 0;
        let order: Vec<usize> = if down {
            (1..layers).collect()
        } else {
            (0..layers.saturating_sub(1)).rev().collect()
        };
        for c in order {
            let key = |v: usize| -> f32 {
                let neighbors = if down { &preds[v] } else { &forward[v] };
                if neighbors.is_empty() {
                    rank[v]
                } else {
                    neighbors.iter().map(|&u| rank[u]).sum::<f32>() / neighbors.len() as f32
                }
            };
            let mut keyed: Vec<(f32, usize)> = columns[c].iter().map(|&v| (key(v), v)).collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            columns[c] = keyed.into_iter().map(|(_, v)| v).collect();
            refresh(&columns, &mut rank);
        }
    }

    // Coordinates. Columns are stacked and centered in their row; a long
    // story (many layers) is wrapped into several rows so that the whole
    // graph stays close to a screen's aspect ratio.
    let heights: Vec<f32> = columns
        .iter()
        .map(|col| {
            col.iter()
                .map(|&v| block_height(&model.blocks[v]))
                .sum::<f32>()
                + V_GAP * col.len().saturating_sub(1) as f32
        })
        .collect();
    let per_row = layers_per_row(&heights);
    let cell = BLOCK_WIDTH + H_GAP;
    let mut out = HashMap::with_capacity(n);
    let mut row_top = 0.0f32;
    for (row, chunk) in columns.chunks(per_row).enumerate() {
        let row_heights = &heights[row * per_row..row * per_row + chunk.len()];
        let tallest = row_heights.iter().copied().fold(0.0, f32::max);
        for (i, col) in chunk.iter().enumerate() {
            let x = (i as f32 * cell).round();
            let mut y = (row_top + (tallest - row_heights[i]) / 2.0).round();
            for &v in col {
                out.insert(model.blocks[v].id, (x, y));
                y = (y + block_height(&model.blocks[v]) + V_GAP).round();
            }
        }
        row_top += tallest + ROW_GAP;
    }
    // Orphans: a grid under the story, as wide as its widest row.
    let grid_columns = per_row.max(3);
    let mut y = row_top + ROW_GAP / 2.0;
    for row in orphans.chunks(grid_columns) {
        let tallest = row
            .iter()
            .map(|&v| block_height(&model.blocks[v]))
            .fold(0.0, f32::max);
        for (i, &v) in row.iter().enumerate() {
            out.insert(model.blocks[v].id, ((i as f32 * cell).round(), y.round()));
        }
        y += tallest + V_GAP;
    }
    out
}

/// How many layers go in a row: the count whose wrapped layout is closest to
/// [`TARGET_ASPECT`] (a short story stays on one row only when it is wide
/// enough to be read at once).
fn layers_per_row(heights: &[f32]) -> usize {
    let layers = heights.len();
    if layers <= 3 {
        return layers.max(1);
    }
    let cell = BLOCK_WIDTH + H_GAP;
    let mut best = (f32::INFINITY, layers);
    for per_row in 1..=layers {
        let width = per_row as f32 * cell - H_GAP;
        let height: f32 = heights
            .chunks(per_row)
            .map(|c| c.iter().copied().fold(0.0, f32::max))
            .sum::<f32>()
            + ROW_GAP * (layers.div_ceil(per_row) - 1) as f32;
        let aspect = width / height.max(1.0);
        let score = (aspect / TARGET_ASPECT).ln().abs();
        if score < best.0 {
            best = (score, per_row);
        }
    }
    best.1
}

/// File of the sidecar for a story file (`story.ron` -> `story.layout`).
pub fn layout_file(story_file: &str) -> String {
    match story_file.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() && !stem.ends_with('/') => format!("{stem}.layout"),
        _ => format!("{story_file}.layout"),
    }
}

/// Reads the sidecar. A missing file is an empty layout; an unreadable one
/// is too, with the reason returned so that the host can log it.
pub fn load_layout(
    storage: &dyn ProjectStorage,
    story_file: &str,
) -> (GraphLayout, Option<String>) {
    let file = layout_file(story_file);
    if !storage.exists(&file) {
        return (GraphLayout::default(), None);
    }
    match storage
        .read_to_string(&file)
        .map_err(|e| e.to_string())
        .and_then(|text| GraphLayout::from_ron(&text).map_err(|e| e.to_string()))
    {
        Ok(layout) => (layout, None),
        Err(e) => (GraphLayout::default(), Some(format!("{file} ignoré : {e}"))),
    }
}

/// Writes the sidecar atomically; an empty layout removes the file.
pub fn save_layout(
    storage: &dyn ProjectStorage,
    story_file: &str,
    layout: &GraphLayout,
) -> Result<(), String> {
    let file = layout_file(story_file);
    if layout.is_empty() {
        return storage.remove(&file).map_err(|e| e.to_string());
    }
    let text = layout.to_ron().map_err(|e| e.to_string())?;
    storage
        .write_atomic(&file, &text)
        .map_err(|e| format!("{file} : {e}"))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io;

    use orbit_core::{ChoiceOption, Node, NodeId, NodeKind, Scene, SceneId, Story};

    use super::*;

    fn rect(model: &GraphModel, id: BlockId, p: (f32, f32)) -> (f32, f32, f32, f32) {
        (
            p.0,
            p.1,
            BLOCK_WIDTH,
            block_height(model.block(id).unwrap()),
        )
    }

    fn overlap(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
        a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
    }

    /// A binary tree of choices over `n` scenes, with back edges.
    fn big_story(n: usize) -> Story {
        let mut story = Story::new("big");
        let labels: Vec<NodeId> = (0..n).map(|_| NodeId::new()).collect();
        for i in 0..n {
            let options: Vec<ChoiceOption> = [2 * i + 1, 2 * i + 2, (i + n / 2) % n]
                .into_iter()
                .filter(|&t| t < n && t != i)
                .map(|t| ChoiceOption {
                    text_id: story.strings.add(format!("vers {t}")),
                    target: labels[t],
                    condition: None,
                })
                .collect();
            let mut nodes = vec![Node {
                id: labels[i],
                kind: NodeKind::Label {
                    name: format!("n{i}"),
                },
            }];
            nodes.push(Node {
                id: NodeId::new(),
                kind: if options.is_empty() {
                    NodeKind::End
                } else {
                    NodeKind::Choice { options }
                },
            });
            story.scenes.push(Scene {
                id: SceneId::new(),
                name: format!("scène {i}"),
                nodes,
            });
        }
        story
    }

    #[test]
    fn auto_layout_never_overlaps_and_puts_the_start_first() {
        for n in [1, 2, 7, 40, 200] {
            let model = GraphModel::build(&big_story(n));
            let layout = auto_layout(&model);
            assert_eq!(layout.len(), n);
            let rects: Vec<_> = model
                .blocks
                .iter()
                .map(|b| rect(&model, b.id, layout[&b.id]))
                .collect();
            for i in 0..rects.len() {
                for j in i + 1..rects.len() {
                    assert!(!overlap(rects[i], rects[j]), "{i} overlaps {j} (n = {n})");
                }
            }
            let entry = layout[&model.entry().unwrap()];
            assert_eq!(entry.0, 0.0, "the start is in the first column");
            if n >= 40 {
                // Long stories wrap into rows instead of one endless line.
                let right = rects.iter().map(|r| r.0 + r.2).fold(f32::MIN, f32::max);
                let bottom = rects.iter().map(|r| r.1 + r.3).fold(f32::MIN, f32::max);
                let top = rects.iter().map(|r| r.1).fold(f32::MAX, f32::min);
                let aspect = right / (bottom - top);
                assert!(
                    (0.8..3.5).contains(&aspect),
                    "aspect {aspect} for {n} blocks"
                );
            }
        }
        assert!(auto_layout(&GraphModel::default()).is_empty());
    }

    #[test]
    fn auto_layout_is_deterministic_and_flows_left_to_right() {
        let model = GraphModel::build(&big_story(15));
        let a = auto_layout(&model);
        assert_eq!(a, auto_layout(&model));
        // Block 1 and 2 are the first two options of block 0.
        assert!(a[&model.blocks[1].id].0 > a[&model.blocks[0].id].0);
        assert!(a[&model.blocks[2].id].0 > a[&model.blocks[0].id].0);
    }

    #[test]
    fn missing_blocks_are_placed_without_moving_the_others() {
        let mut story = big_story(6);
        let model = GraphModel::build(&story);
        let mut layout = GraphLayout::default();
        assert!(layout.place_missing(&model));
        let before = layout.clone();
        assert!(!layout.place_missing(&model));

        // A new scene linked from block 0 appears next to it.
        let label = NodeId::new();
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "neuve".into(),
            nodes: vec![
                Node {
                    id: label,
                    kind: NodeKind::Label {
                        name: "neuve".into(),
                    },
                },
                Node {
                    id: NodeId::new(),
                    kind: NodeKind::End,
                },
            ],
        });
        let model = GraphModel::build(&story);
        assert!(layout.place_missing(&model));
        for b in model.blocks.iter().take(6) {
            assert_eq!(layout.get(b.id), before.get(b.id), "existing blocks stay");
        }
        let new_id = model.blocks.last().unwrap().id;
        let rects: Vec<_> = model
            .blocks
            .iter()
            .map(|b| rect(&model, b.id, layout.get(b.id).unwrap()))
            .collect();
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                assert!(!overlap(rects[i], rects[j]));
            }
        }
        assert!(layout.get(new_id).is_some());

        // Dead blocks are forgotten by `prune`.
        story.scenes.pop();
        layout.prune(&GraphModel::build(&story));
        assert_eq!(layout.positions.len(), 6);
    }

    #[derive(Default)]
    struct Memory(RefCell<HashMap<String, String>>);

    impl ProjectStorage for Memory {
        fn read_to_string(&self, path: &str) -> io::Result<String> {
            self.0
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing"))
        }
        fn write_atomic(&self, path: &str, contents: &str) -> io::Result<()> {
            self.0.borrow_mut().insert(path.into(), contents.into());
            Ok(())
        }
        fn exists(&self, path: &str) -> bool {
            self.0.borrow().contains_key(path)
        }
        fn remove(&self, path: &str) -> io::Result<()> {
            self.0.borrow_mut().remove(path);
            Ok(())
        }
    }

    #[test]
    fn layout_round_trips_through_its_sidecar() {
        let storage = Memory::default();
        let model = GraphModel::build(&big_story(5));
        let mut layout = GraphLayout::default();
        layout.place_missing(&model);
        layout.set(model.blocks[2].id, (-12.5, 300.0));
        save_layout(&storage, "story.ron", &layout).unwrap();
        assert!(storage.exists("story.layout"));
        let (back, warning) = load_layout(&storage, "story.ron");
        assert_eq!(warning, None);
        assert_eq!(back.positions, layout.positions);
        // An empty layout removes the file.
        save_layout(&storage, "story.ron", &GraphLayout::default()).unwrap();
        assert!(!storage.exists("story.layout"));
    }

    #[test]
    fn broken_or_newer_layouts_are_ignored_with_a_reason() {
        let storage = Memory::default();
        storage.write_atomic("story.layout", "((((").unwrap();
        let (layout, warning) = load_layout(&storage, "story.ron");
        assert!(layout.is_empty() && warning.is_some());
        storage
            .write_atomic("story.layout", "(format: 99, positions: {})")
            .unwrap();
        assert!(load_layout(&storage, "story.ron").1.is_some());
        assert_eq!(load_layout(&Memory::default(), "story.ron").1, None);
    }

    #[test]
    fn sidecar_name_follows_the_story_file() {
        assert_eq!(layout_file("story.ron"), "story.layout");
        assert_eq!(layout_file("stories/demo.ron"), "stories/demo.layout");
        assert_eq!(layout_file("noext"), "noext.layout");
    }
}
