//! Point & click navigation (imagemap): clickable zones drawn over the
//! background, presented by a [`NodeKind::Hotspots`](crate::NodeKind) node.
//!
//! For the VM a hotspot map is a choice: every hotspot is an option (its
//! `text_id` is the tooltip, `target` a `Label`), so waiting, saves,
//! rollback, skip and replay reuse the choice machinery unchanged. Only the
//! host draws it differently: zones over the picture instead of a menu.
//!
//! Coordinates are normalized to the virtual screen (`0..1` on both axes,
//! origin top-left), so a map survives a resolution change. Hit testing is
//! pure geometry, cheap enough to run every frame on every hotspot.

use serde::{Deserialize, Serialize};

use crate::defs::Color;
use crate::expr::Expr;
use crate::ids::{AssetId, NodeId, SceneId, StringId};
use crate::notify::Notify;
use crate::varops::{OpKind, Operand};

/// Maximum vertices of a polygon hotspot.
pub const MAX_POLYGON_POINTS: usize = 64;

/// The data of a `Hotspots` node.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HotspotMap {
    /// Background shown when the node runs (as a `Background` node without
    /// transition). `None` keeps the current background.
    #[serde(default)]
    pub background: Option<AssetId>,
    /// The zones, drawn back to front (the last one wins a click where
    /// zones overlap).
    #[serde(default)]
    pub hotspots: Vec<Hotspot>,
    /// Look shared by every hotspot (each can override it).
    #[serde(default)]
    pub style: HotspotStyle,
}

/// One clickable zone.
///
/// `Serialize` is written by hand: `actions` is left out of RON files when
/// empty and of the binary layout of game packs older than version 9 (see
/// [`with_legacy_hotspot_layout`]).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Hotspot {
    /// Name for the author (editor list, DSL); not shown to the player.
    #[serde(default)]
    pub name: String,
    /// Tooltip shown on hover / focus (localizable; may be an empty string).
    pub text_id: StringId,
    pub shape: HotspotShape,
    /// The `Label` the story jumps to when the zone is activated.
    pub target: NodeId,
    /// The zone exists only when this evaluates to `true`.
    #[serde(default)]
    pub condition: Option<Expr>,
    /// When this evaluates to `false` the zone is shown dimmed and cannot be
    /// activated.
    #[serde(default)]
    pub enabled: Option<Expr>,
    /// Overrides the map style.
    #[serde(default)]
    pub style: Option<HotspotStyle>,
    /// Image (same frame as the screen) drawn clipped to the zone on hover,
    /// instead of the procedural glow (Ren'Py imagemap `hover`).
    #[serde(default)]
    pub hover_image: Option<AssetId>,
    /// Sound played when the zone gets the hover / focus.
    #[serde(default)]
    pub hover_sound: Option<AssetId>,
    /// Zone actions, run in order when the zone is activated. Empty: the
    /// story jumps to `target` (the behavior of every map written before
    /// actions existed). The first flow action (jump, scene, call, return,
    /// stay) decides where the story goes and ends the list; without one
    /// the story jumps to `target` after the other actions.
    #[serde(default, deserialize_with = "actions_field")]
    pub actions: Vec<HotspotAction>,
}

/// Something a zone does when it is activated (see [`Hotspot::actions`]).
/// New variants are appended at the end (stable indices).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum HotspotAction {
    /// A no-code variable operation (`=`, `+=`, `-=`, toggle, ...).
    SetVar { var: String, op: OpKind, value: Operand },
    /// Jump to a `Label` (flow: ends the list).
    Jump { label: NodeId },
    /// Jump to another scene, at its start or at one of its labels (flow).
    JumpScene {
        scene: SceneId,
        #[serde(default)]
        label: Option<NodeId>,
    },
    /// Call the routine at a `Label`; its `Return` comes back to this map,
    /// shown again (flow).
    Call { label: NodeId },
    /// Return from the innermost `Call` (flow).
    Return,
    /// Play a sound effect.
    PlaySound { asset: AssetId },
    /// Show an in-game notification (toast).
    Notify(Notify),
    /// Show the same map again after the actions (pick up an item, ...).
    Stay,
}

impl HotspotAction {
    /// The action decides where the story goes (and ends the list).
    pub fn is_flow(&self) -> bool {
        matches!(
            self,
            Self::Jump { .. } | Self::JumpScene { .. } | Self::Call { .. } | Self::Return | Self::Stay
        )
    }

    /// The labels the action refers to.
    pub fn labels(&self) -> Vec<NodeId> {
        match self {
            Self::Jump { label } | Self::Call { label } => vec![*label],
            Self::JumpScene { label, .. } => label.iter().copied().collect(),
            _ => Vec::new(),
        }
    }

    /// The assets the action refers to.
    pub fn assets(&self) -> Vec<AssetId> {
        match self {
            Self::PlaySound { asset } => vec![*asset],
            Self::Notify(n) => n.assets(),
            _ => Vec::new(),
        }
    }
}

impl Hotspot {
    /// A zone jumping to `target`, with the map look and no action.
    pub fn new(text_id: StringId, shape: HotspotShape, target: NodeId) -> Self {
        Self {
            name: String::new(),
            text_id,
            shape,
            target,
            condition: None,
            enabled: None,
            style: None,
            hover_image: None,
            hover_sound: None,
            actions: Vec::new(),
        }
    }

    /// The flow action that decides where the zone leads, if any: the first
    /// one of the list (the ones after it never run).
    pub fn flow(&self) -> Option<&HotspotAction> {
        self.actions.iter().find(|a| a.is_flow())
    }

    /// The actions that actually run: up to and including the first flow
    /// action.
    pub fn effective_actions(&self) -> &[HotspotAction] {
        match self.actions.iter().position(HotspotAction::is_flow) {
            Some(p) => &self.actions[..=p],
            None => &self.actions,
        }
    }

    /// Every label the zone may lead to: `target` (unless a flow action
    /// replaces it) and the labels of its actions.
    pub fn targets(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        if self.flow().is_none() {
            out.push(self.target);
        }
        for a in self.effective_actions() {
            out.extend(a.labels());
        }
        out
    }

    /// Uses what game pack version 9 added (actions, a mask shape).
    pub fn uses_v9(&self) -> bool {
        !self.actions.is_empty() || matches!(self.shape, HotspotShape::Mask { .. })
    }
}

thread_local! {
    static LEGACY_HOTSPOT_LAYOUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` (a binary encode or decode of a game pack of version 8 or
/// older) with the layout of those packs: no `Hotspot::actions`. Postcard is
/// not self-describing, so the reader has to be told. Frozen, never remove
/// (backward compatibility).
pub fn with_legacy_hotspot_layout<R>(f: impl FnOnce() -> R) -> R {
    let before = LEGACY_HOTSPOT_LAYOUT.with(|c| c.replace(true));
    let out = f();
    LEGACY_HOTSPOT_LAYOUT.with(|c| c.set(before));
    out
}

fn is_legacy_hotspot_layout() -> bool {
    LEGACY_HOTSPOT_LAYOUT.with(std::cell::Cell::get) || crate::is_legacy_v15_layout()
}

fn actions_field<'de, D>(deserializer: D) -> Result<Vec<HotspotAction>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    if !deserializer.is_human_readable() && is_legacy_hotspot_layout() {
        return Ok(Vec::new());
    }
    Vec::deserialize(deserializer)
}

impl Serialize for Hotspot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let write_actions = if serializer.is_human_readable() {
            !self.actions.is_empty()
        } else {
            !is_legacy_hotspot_layout()
        };
        let mut st = serializer.serialize_struct("Hotspot", 9 + usize::from(write_actions))?;
        st.serialize_field("name", &self.name)?;
        st.serialize_field("text_id", &self.text_id)?;
        st.serialize_field("shape", &self.shape)?;
        st.serialize_field("target", &self.target)?;
        st.serialize_field("condition", &self.condition)?;
        st.serialize_field("enabled", &self.enabled)?;
        st.serialize_field("style", &self.style)?;
        st.serialize_field("hover_image", &self.hover_image)?;
        st.serialize_field("hover_sound", &self.hover_sound)?;
        if write_actions {
            st.serialize_field("actions", &self.actions)?;
        } else {
            st.skip_field("actions")?;
        }
        st.end()
    }
}

/// A downsampled alpha bitmap of a picture: the hit test of a
/// [`HotspotShape::Mask`] zone (built once when the picture loads; a few
/// kilobytes). Row-major, top-left origin.
#[derive(Clone, Debug, PartialEq)]
pub struct AlphaMask {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
}

/// Longest side of an [`AlphaMask`] built from a picture.
pub const MASK_MAX_SIDE: u32 = 256;

impl AlphaMask {
    /// Builds the mask of a non-premultiplied RGBA8 picture, shrunk so its
    /// longest side is at most `max_side` cells; a cell keeps the highest
    /// alpha of the pixels it covers (thin parts stay clickable).
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8], max_side: u32) -> Self {
        if width == 0 || height == 0 || rgba.len() < (width as usize * height as usize * 4) {
            return Self { width: 0, height: 0, alpha: Vec::new() };
        }
        let scale = (width.max(height) as f32 / max_side.max(1) as f32).max(1.0);
        let mw = ((width as f32 / scale).ceil() as u32).max(1);
        let mh = ((height as f32 / scale).ceil() as u32).max(1);
        let mut alpha = vec![0u8; (mw * mh) as usize];
        for y in 0..height {
            let my = ((y as f32 / scale) as u32).min(mh - 1);
            let row = y as usize * width as usize * 4;
            for x in 0..width {
                let a = rgba[row + x as usize * 4 + 3];
                if a == 0 {
                    continue;
                }
                let mx = ((x as f32 / scale) as u32).min(mw - 1);
                let cell = &mut alpha[(my * mw + mx) as usize];
                *cell = (*cell).max(a);
            }
        }
        Self { width: mw, height: mh, alpha }
    }

    /// Alpha at the normalized point `(u, v)` of the picture (`0` outside).
    pub fn sample(&self, u: f32, v: f32) -> u8 {
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) || self.alpha.is_empty() {
            return 0;
        }
        let x = ((u * self.width as f32) as u32).min(self.width - 1);
        let y = ((v * self.height as f32) as u32).min(self.height - 1);
        self.alpha[(y * self.width + x) as usize]
    }

    fn opaque(&self, x: i64, y: i64, threshold: u8) -> bool {
        x >= 0
            && y >= 0
            && (x as u32) < self.width
            && (y as u32) < self.height
            && self.alpha[(y as u32 * self.width + x as u32) as usize] >= threshold
    }

    /// Centers (normalized to the picture) of the opaque cells touching a
    /// transparent one: the edge the glow follows, drawn as small dots. At
    /// most `limit` points (evenly thinned; `0`: no limit).
    pub fn edge_points(&self, threshold: f32, limit: usize) -> Vec<(f32, f32)> {
        let t = threshold_byte(threshold);
        let mut out = Vec::new();
        for y in 0..i64::from(self.height) {
            for x in 0..i64::from(self.width) {
                if !self.opaque(x, y, t) {
                    continue;
                }
                let edge = !self.opaque(x - 1, y, t)
                    || !self.opaque(x + 1, y, t)
                    || !self.opaque(x, y - 1, t)
                    || !self.opaque(x, y + 1, t);
                if edge {
                    out.push((
                        (x as f32 + 0.5) / self.width as f32,
                        (y as f32 + 0.5) / self.height as f32,
                    ));
                }
            }
        }
        if limit > 0 && out.len() > limit {
            let step = out.len() as f32 / limit as f32;
            out = (0..limit).map(|i| out[(i as f32 * step) as usize]).collect();
        }
        out
    }
}

/// A `0..1` threshold as an alpha byte (at least 1: a fully transparent
/// pixel never counts).
pub fn threshold_byte(threshold: f32) -> u8 {
    ((threshold.clamp(0.0, 1.0) * 255.0).round() as u8).max(1)
}

/// The geometry of a hotspot, in normalized screen coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum HotspotShape {
    Rect { x: f32, y: f32, w: f32, h: f32 },
    Ellipse { cx: f32, cy: f32, rx: f32, ry: f32 },
    Polygon { points: Vec<(f32, f32)> },
    /// The opaque pixels of a picture placed over the rectangle `x, y, w, h`
    /// (`0, 0, 1, 1`: placed like the background). Only pixels whose alpha
    /// reaches `threshold` (0..1) react; `draw` also draws the picture
    /// (an object overlaid on the background, a Ren'Py `imagebutton`).
    /// Appended last: older encodings keep their variant indices.
    Mask {
        image: AssetId,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default = "default_threshold")]
        threshold: f32,
        #[serde(default)]
        draw: bool,
    },
}

fn default_threshold() -> f32 {
    DEFAULT_MASK_THRESHOLD
}

/// The usual alpha threshold of a mask shape.
pub const DEFAULT_MASK_THRESHOLD: f32 = 0.5;

/// How hotspots look. Colors are RGB; alphas go from 0 to 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HotspotStyle {
    #[serde(default = "HotspotStyle::default_color")]
    pub color: Color,
    /// Opacity of the fill tint on hover.
    #[serde(default = "HotspotStyle::default_fill")]
    pub fill_alpha: f32,
    /// Width of the glowing outline, in virtual pixels.
    #[serde(default = "HotspotStyle::default_glow")]
    pub glow_width: f32,
    /// Opacity of the outline on hover (0..1).
    #[serde(default = "HotspotStyle::default_intensity")]
    pub intensity: f32,
    /// Pulses per second of the outline (0: steady).
    #[serde(default = "HotspotStyle::default_pulse")]
    pub pulse_speed: f32,
    /// Opacity of the faint shimmering edge drawn when idle (0: invisible
    /// until hovered).
    #[serde(default = "HotspotStyle::default_idle")]
    pub idle_hint: f32,
    /// The mouse cursor becomes a hand over the zone.
    #[serde(default = "HotspotStyle::default_true")]
    pub hand_cursor: bool,
}

impl HotspotStyle {
    fn default_color() -> Color {
        Color { r: 255, g: 230, b: 150 }
    }
    fn default_fill() -> f32 {
        0.18
    }
    fn default_glow() -> f32 {
        4.0
    }
    fn default_intensity() -> f32 {
        0.9
    }
    fn default_pulse() -> f32 {
        1.2
    }
    fn default_idle() -> f32 {
        0.12
    }
    fn default_true() -> bool {
        true
    }
}

impl Default for HotspotStyle {
    fn default() -> Self {
        Self {
            color: Self::default_color(),
            fill_alpha: Self::default_fill(),
            glow_width: Self::default_glow(),
            intensity: Self::default_intensity(),
            pulse_speed: Self::default_pulse(),
            idle_hint: Self::default_idle(),
            hand_cursor: true,
        }
    }
}

impl HotspotShape {
    /// Is the normalized point `(x, y)` inside the shape?
    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            HotspotShape::Rect { x: rx, y: ry, w, h } => {
                let (x0, x1) = ordered(*rx, rx + w);
                let (y0, y1) = ordered(*ry, ry + h);
                x >= x0 && x <= x1 && y >= y0 && y <= y1
            }
            HotspotShape::Ellipse { cx, cy, rx, ry } => {
                if *rx <= 0.0 || *ry <= 0.0 {
                    return false;
                }
                let dx = (x - cx) / rx;
                let dy = (y - cy) / ry;
                dx * dx + dy * dy <= 1.0
            }
            HotspotShape::Polygon { points } => point_in_polygon(points, x, y),
            // Without the picture's pixels: its rectangle (see
            // `contains_masked`).
            HotspotShape::Mask { .. } => {
                let (x0, y0, x1, y1) = self.bounds();
                x >= x0 && x <= x1 && y >= y0 && y <= y1
            }
        }
    }

    /// [`contains`](Self::contains), testing the opaque pixels of a mask
    /// shape when its alpha bitmap is known (`None`: its rectangle).
    pub fn contains_masked(&self, x: f32, y: f32, mask: Option<&AlphaMask>) -> bool {
        match (self, mask) {
            (HotspotShape::Mask { x: rx, y: ry, w, h, threshold, .. }, Some(m)) => {
                if *w == 0.0 || *h == 0.0 {
                    return false;
                }
                m.sample((x - rx) / w, (y - ry) / h) >= threshold_byte(*threshold)
            }
            _ => self.contains(x, y),
        }
    }

    /// The picture of a mask shape.
    pub fn mask_image(&self) -> Option<AssetId> {
        match self {
            HotspotShape::Mask { image, .. } => Some(*image),
            _ => None,
        }
    }

    /// Bounding box `(min_x, min_y, max_x, max_y)`.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        match self {
            HotspotShape::Rect { x, y, w, h } => {
                let (x0, x1) = ordered(*x, x + w);
                let (y0, y1) = ordered(*y, y + h);
                (x0, y0, x1, y1)
            }
            HotspotShape::Ellipse { cx, cy, rx, ry } => {
                (cx - rx.abs(), cy - ry.abs(), cx + rx.abs(), cy + ry.abs())
            }
            HotspotShape::Polygon { points } => points.iter().fold(
                (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
                |(a, b, c, d), &(x, y)| (a.min(x), b.min(y), c.max(x), d.max(y)),
            ),
            HotspotShape::Mask { x, y, w, h, .. } => {
                let (x0, x1) = ordered(*x, x + w);
                let (y0, y1) = ordered(*y, y + h);
                (x0, y0, x1, y1)
            }
        }
    }

    /// Center of the bounding box (keyboard focus order, tooltip anchor).
    pub fn center(&self) -> (f32, f32) {
        let (a, b, c, d) = self.bounds();
        ((a + c) * 0.5, (b + d) * 0.5)
    }

    /// The outline as a closed polyline (`segments` points for an ellipse),
    /// for drawing the glow.
    pub fn outline(&self, segments: usize) -> Vec<(f32, f32)> {
        match self {
            HotspotShape::Rect { .. } | HotspotShape::Mask { .. } => {
                let (x0, y0, x1, y1) = self.bounds();
                vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            }
            HotspotShape::Ellipse { cx, cy, rx, ry } => {
                let n = segments.max(8);
                (0..n)
                    .map(|i| {
                        let a = i as f32 / n as f32 * std::f32::consts::TAU;
                        (cx + rx * a.cos(), cy + ry * a.sin())
                    })
                    .collect()
            }
            HotspotShape::Polygon { points } => points.clone(),
        }
    }

    /// Moves the shape by `(dx, dy)`.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            HotspotShape::Rect { x, y, .. } | HotspotShape::Mask { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            HotspotShape::Ellipse { cx, cy, .. } => {
                *cx += dx;
                *cy += dy;
            }
            HotspotShape::Polygon { points } => {
                for p in points {
                    p.0 += dx;
                    p.1 += dy;
                }
            }
        }
    }

    /// The shape is usable: finite numbers, a non-empty area, a polygon of
    /// 3 to [`MAX_POLYGON_POINTS`] vertices.
    pub fn is_valid(&self) -> bool {
        match self {
            HotspotShape::Rect { x, y, w, h } => {
                [x, y, w, h].iter().all(|v| v.is_finite()) && *w != 0.0 && *h != 0.0
            }
            HotspotShape::Ellipse { cx, cy, rx, ry } => {
                [cx, cy, rx, ry].iter().all(|v| v.is_finite()) && *rx > 0.0 && *ry > 0.0
            }
            HotspotShape::Polygon { points } => {
                (3..=MAX_POLYGON_POINTS).contains(&points.len())
                    && points.iter().all(|(x, y)| x.is_finite() && y.is_finite())
            }
            HotspotShape::Mask { x, y, w, h, threshold, .. } => {
                [x, y, w, h, threshold].iter().all(|v| v.is_finite())
                    && *w != 0.0
                    && *h != 0.0
                    && (0.0..=1.0).contains(threshold)
            }
        }
    }
}

fn ordered(a: f32, b: f32) -> (f32, f32) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Even-odd ray casting.
fn point_in_polygon(points: &[(f32, f32)], x: f32, y: f32) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let (xi, yi) = points[i];
        let (xj, yj) = points[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl HotspotMap {
    /// Every asset the map uses: its background, hover images and sounds.
    pub fn assets(&self) -> Vec<AssetId> {
        self.background
            .into_iter()
            .chain(self.hotspots.iter().flat_map(|h| {
                h.hover_image
                    .into_iter()
                    .chain(h.hover_sound)
                    .chain(h.shape.mask_image())
                    .chain(h.actions.iter().flat_map(HotspotAction::assets))
            }))
            .collect()
    }

    /// Uses what game pack version 9 added (zone actions, mask shapes).
    pub fn uses_v9(&self) -> bool {
        self.hotspots.iter().any(Hotspot::uses_v9)
    }

    /// [`hit`](Self::hit) testing the opaque pixels of mask shapes:
    /// `mask(i)` is the alpha bitmap of hotspot `i` when known.
    pub fn hit_masked<'a>(
        &self,
        candidates: &[usize],
        x: f32,
        y: f32,
        mask: impl Fn(usize) -> Option<&'a AlphaMask>,
    ) -> Option<usize> {
        candidates.iter().rev().copied().find(|&i| {
            self.hotspots
                .get(i)
                .is_some_and(|h| h.shape.contains_masked(x, y, mask(i)))
        })
    }

    /// The topmost hotspot among `candidates` (indices into `hotspots`)
    /// containing the point: the last one in drawing order.
    pub fn hit(&self, candidates: &[usize], x: f32, y: f32) -> Option<usize> {
        candidates
            .iter()
            .rev()
            .copied()
            .find(|&i| self.hotspots.get(i).is_some_and(|h| h.shape.contains(x, y)))
    }

    /// The effective style of hotspot `i`.
    pub fn style_of(&self, i: usize) -> &HotspotStyle {
        self.hotspots
            .get(i)
            .and_then(|h| h.style.as_ref())
            .unwrap_or(&self.style)
    }

    /// `candidates` in keyboard / gamepad focus order: reading order (top to
    /// bottom by rows, then left to right) of their centers.
    pub fn focus_order(&self, candidates: &[usize]) -> Vec<usize> {
        let mut order: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|&i| i < self.hotspots.len())
            .collect();
        order.sort_by(|&a, &b| {
            let (ax, ay) = self.hotspots[a].shape.center();
            let (bx, by) = self.hotspots[b].shape.center();
            // Same row when the centers are within 5% of the height.
            let row = |y: f32| (y / 0.05).floor() as i32;
            row(ay)
                .cmp(&row(by))
                .then(ax.total_cmp(&bx))
                .then(a.cmp(&b))
        });
        order
    }

    /// The hotspot focused after `current` (`forward`) or before it in
    /// [`focus_order`](Self::focus_order), wrapping around. With no current
    /// focus: the first (or last) one.
    pub fn next_focus(&self, candidates: &[usize], current: Option<usize>, forward: bool) -> Option<usize> {
        let order = self.focus_order(candidates);
        if order.is_empty() {
            return None;
        }
        let pos = current.and_then(|c| order.iter().position(|&i| i == c));
        let n = order.len();
        let next = match (pos, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(p), true) => (p + 1) % n,
            (Some(p), false) => (p + n - 1) % n,
        };
        Some(order[next])
    }

    /// The nearest hotspot from `current` in a direction (arrow keys /
    /// D-pad): `(dx, dy)` is the unit direction. Falls back to
    /// [`next_focus`](Self::next_focus) when nothing lies that way.
    pub fn directional_focus(
        &self,
        candidates: &[usize],
        current: Option<usize>,
        dx: f32,
        dy: f32,
    ) -> Option<usize> {
        let Some(cur) = current.filter(|c| candidates.contains(c)) else {
            return self.next_focus(candidates, None, dx + dy >= 0.0);
        };
        let (cx, cy) = self.hotspots[cur].shape.center();
        candidates
            .iter()
            .copied()
            .filter(|&i| i != cur && i < self.hotspots.len())
            .filter_map(|i| {
                let (x, y) = self.hotspots[i].shape.center();
                let (vx, vy) = (x - cx, y - cy);
                let along = vx * dx + vy * dy;
                if along <= 1e-4 {
                    return None;
                }
                let across = (vx * dy - vy * dx).abs();
                Some((along + 2.0 * across, i))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
            .or_else(|| self.next_focus(candidates, Some(cur), dx + dy >= 0.0))
    }
}

/// Triangles (indices into `points`) filling a simple polygon, by ear
/// clipping (concave shapes work; O(n²), fine for a few dozen vertices).
/// A degenerate or self-intersecting outline gives what it can.
pub fn triangulate(points: &[(f32, f32)]) -> Vec<u32> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let area: f32 = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    let ccw = area > 0.0;
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity((n - 2) * 3);
    let cross = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (ia, ib, ic) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (points[ia], points[ib], points[ic]);
            let turn = cross(a, b, c);
            if (turn > 0.0) != ccw || turn == 0.0 {
                continue; // reflex or flat corner
            }
            let inside = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = points[j];
                let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
                if ccw { d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0 } else { d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0 }
            });
            if inside {
                continue;
            }
            out.extend([ia as u32, ib as u32, ic as u32]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            // Self-intersecting: fan the rest.
            break;
        }
    }
    for k in 1..idx.len().saturating_sub(1) {
        out.extend([idx[0] as u32, idx[k] as u32, idx[k + 1] as u32]);
    }
    out
}

/// Pulse factor in `0..1` of an outline at `time` seconds.
pub fn pulse(style: &HotspotStyle, time: f64) -> f32 {
    if style.pulse_speed <= 0.0 {
        return 1.0;
    }
    let phase = (time * style.pulse_speed as f64 * std::f64::consts::TAU).sin() as f32;
    0.7 + 0.3 * phase
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(shape: HotspotShape) -> Hotspot {
        Hotspot::new(StringId::new(), shape, NodeId::new())
    }

    fn checker() -> AlphaMask {
        // 8x4 picture: the left half opaque, the right half transparent,
        // except an opaque pixel at (6, 1).
        let mut rgba = vec![0u8; 8 * 4 * 4];
        for y in 0..4 {
            for x in 0..8 {
                let a = if x < 4 || (x == 6 && y == 1) { 255 } else { 0 };
                rgba[(y * 8 + x) * 4 + 3] = a;
            }
        }
        AlphaMask::from_rgba(8, 4, &rgba, 256)
    }

    #[test]
    fn mask_hit_test_follows_opaque_pixels_and_scaling() {
        let m = checker();
        assert_eq!((m.width, m.height), (8, 4));
        // Placed over the right half of the screen, the full height.
        let shape = HotspotShape::Mask {
            image: AssetId::new(),
            x: 0.5,
            y: 0.0,
            w: 0.5,
            h: 1.0,
            threshold: 0.5,
            draw: false,
        };
        assert!(shape.contains_masked(0.6, 0.5, Some(&m)), "opaque pixel");
        assert!(!shape.contains_masked(0.9, 0.9, Some(&m)), "transparent pixel");
        // Pixel (6, 1): u = 6.5/8, v = 1.5/4.
        assert!(shape.contains_masked(0.5 + 0.5 * 6.5 / 8.0, 1.5 / 4.0, Some(&m)));
        assert!(!shape.contains_masked(0.2, 0.5, Some(&m)), "outside the rectangle");
        // Without the bitmap (not loaded yet): the rectangle.
        assert!(shape.contains_masked(0.9, 0.9, None));
        assert!(shape.is_valid());
        // Downsampling keeps thin opaque parts (max of the cell).
        let small = {
            let mut rgba = vec![0u8; 8 * 4 * 4];
            for y in 0..4 {
                for x in 0..8 {
                    rgba[(y * 8 + x) * 4 + 3] = if x < 4 || (x == 6 && y == 1) { 255 } else { 0 };
                }
            }
            AlphaMask::from_rgba(8, 4, &rgba, 4)
        };
        assert_eq!((small.width, small.height), (4, 2));
        assert_eq!(small.sample(6.5 / 8.0, 1.5 / 4.0), 255);
        assert_eq!(small.sample(7.5 / 8.0, 3.5 / 4.0), 0);
        // A threshold above the pixel's alpha refuses it.
        let mut half = checker();
        half.alpha.iter_mut().for_each(|a| *a /= 2);
        assert!(!shape.contains_masked(0.6, 0.5, Some(&half)));
        // The glow edge: opaque cells next to a transparent one.
        let edge = m.edge_points(0.5, 0);
        assert!(edge.contains(&(3.5 / 8.0, 1.5 / 4.0)));
        assert!(!edge.contains(&(1.5 / 8.0, 1.5 / 4.0)), "inner cell");
        assert!(m.edge_points(0.5, 3).len() == 3);
    }

    #[test]
    fn hotspot_actions_flow_and_targets() {
        let back = NodeId::new();
        let mut h = spot(HotspotShape::Rect { x: 0.0, y: 0.0, w: 0.1, h: 0.1 });
        assert_eq!(h.targets(), vec![h.target]);
        assert!(!h.uses_v9());
        h.actions = vec![
            HotspotAction::SetVar { var: "key".into(), op: OpKind::Set, value: Operand::Lit(crate::Value::Bool(true)) },
            HotspotAction::Jump { label: back },
            HotspotAction::Stay,
        ];
        assert!(h.uses_v9());
        assert_eq!(h.effective_actions().len(), 2);
        assert_eq!(h.targets(), vec![back], "the flow action replaces the target");
        // RON: actions written only when present; older files load.
        let text = ron::to_string(&h).unwrap();
        assert!(text.contains("actions"));
        let back_h: Hotspot = ron::from_str(&text).unwrap();
        assert_eq!(back_h, h);
        let plain = spot(HotspotShape::Rect { x: 0.0, y: 0.0, w: 0.1, h: 0.1 });
        assert!(!ron::to_string(&plain).unwrap().contains("actions"));
        // Binary: the legacy layout reads and writes no actions.
        let bytes = postcard::to_stdvec(&plain).unwrap();
        let legacy = with_legacy_hotspot_layout(|| postcard::to_stdvec(&plain).unwrap());
        assert_eq!(bytes.len(), legacy.len() + 1, "one byte: the empty list");
        let old: Hotspot = with_legacy_hotspot_layout(|| postcard::from_bytes(&legacy).unwrap());
        assert_eq!(old, plain);
        let new: Hotspot = postcard::from_bytes(&postcard::to_stdvec(&h).unwrap()).unwrap();
        assert_eq!(new, h);
    }

    #[test]
    fn rect_hit_including_negative_size() {
        let r = HotspotShape::Rect { x: 0.1, y: 0.1, w: 0.2, h: 0.2 };
        assert!(r.contains(0.2, 0.2));
        assert!(r.contains(0.1, 0.3));
        assert!(!r.contains(0.31, 0.2));
        let flipped = HotspotShape::Rect { x: 0.3, y: 0.3, w: -0.2, h: -0.2 };
        assert!(flipped.contains(0.2, 0.2));
    }

    #[test]
    fn ellipse_hit() {
        let e = HotspotShape::Ellipse { cx: 0.5, cy: 0.5, rx: 0.2, ry: 0.1 };
        assert!(e.contains(0.5, 0.5));
        assert!(e.contains(0.69, 0.5));
        assert!(!e.contains(0.5, 0.61));
        assert!(!e.contains(0.65, 0.58)); // inside the box, outside the ellipse
        let flat = HotspotShape::Ellipse { cx: 0.5, cy: 0.5, rx: 0.0, ry: 0.1 };
        assert!(!flat.contains(0.5, 0.5));
    }

    #[test]
    fn polygon_hit_concave() {
        // An "L": the notch at the top right is outside.
        let l = HotspotShape::Polygon {
            points: vec![(0.0, 0.0), (0.2, 0.0), (0.2, 0.3), (0.5, 0.3), (0.5, 0.5), (0.0, 0.5)],
        };
        assert!(l.contains(0.1, 0.1));
        assert!(l.contains(0.4, 0.4));
        assert!(!l.contains(0.4, 0.1));
        assert!(!l.contains(0.6, 0.4));
        let degenerate = HotspotShape::Polygon { points: vec![(0.0, 0.0), (1.0, 1.0)] };
        assert!(!degenerate.contains(0.5, 0.5));
        assert!(!degenerate.is_valid());
    }

    #[test]
    fn topmost_visible_wins() {
        let map = HotspotMap {
            hotspots: vec![
                spot(HotspotShape::Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }),
                spot(HotspotShape::Rect { x: 0.4, y: 0.4, w: 0.2, h: 0.2 }),
            ],
            ..Default::default()
        };
        assert_eq!(map.hit(&[0, 1], 0.5, 0.5), Some(1));
        // Hidden by a condition: not a candidate, the zone under it gets it.
        assert_eq!(map.hit(&[0], 0.5, 0.5), Some(0));
        assert_eq!(map.hit(&[1], 0.1, 0.1), None);
    }

    #[test]
    fn focus_cycles_in_reading_order() {
        let at = |x: f32, y: f32| spot(HotspotShape::Ellipse { cx: x, cy: y, rx: 0.05, ry: 0.05 });
        let map = HotspotMap {
            hotspots: vec![at(0.8, 0.8), at(0.2, 0.2), at(0.7, 0.21), at(0.1, 0.8)],
            ..Default::default()
        };
        let all = [0, 1, 2, 3];
        assert_eq!(map.focus_order(&all), vec![1, 2, 3, 0]);
        assert_eq!(map.next_focus(&all, None, true), Some(1));
        assert_eq!(map.next_focus(&all, Some(0), true), Some(1));
        assert_eq!(map.next_focus(&all, Some(1), false), Some(0));
        assert_eq!(map.next_focus(&[], None, true), None);
        // Right of 1 is 2; down from 1 is 3.
        assert_eq!(map.directional_focus(&all, Some(1), 1.0, 0.0), Some(2));
        assert_eq!(map.directional_focus(&all, Some(1), 0.0, 1.0), Some(3));
        // Filtered candidates are skipped.
        assert_eq!(map.next_focus(&[1, 3], Some(1), true), Some(3));
    }

    #[test]
    fn triangulation_covers_a_concave_polygon() {
        let l = [(0.0, 0.0), (0.2, 0.0), (0.2, 0.3), (0.5, 0.3), (0.5, 0.5), (0.0, 0.5)];
        let tris = triangulate(&l);
        assert_eq!(tris.len(), (l.len() - 2) * 3);
        let area = |t: &[u32]| {
            let (a, b, c) = (l[t[0] as usize], l[t[1] as usize], l[t[2] as usize]);
            ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).abs() * 0.5
        };
        let total: f32 = tris.chunks(3).map(area).sum();
        assert!((total - (0.2 * 0.3 + 0.5 * 0.2)).abs() < 1e-5, "{total}");
        // Clockwise works too.
        let mut cw = l.to_vec();
        cw.reverse();
        assert_eq!(triangulate(&cw).len(), 12);
        assert!(triangulate(&l[..2]).is_empty());
    }

    #[test]
    fn ron_round_trip_and_defaults() {
        let mut map = HotspotMap::default();
        map.hotspots.push(spot(HotspotShape::Polygon {
            points: vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)],
        }));
        let text = ron::to_string(&map).unwrap();
        let back: HotspotMap = ron::from_str(&text).unwrap();
        assert_eq!(back, map);
        let style: HotspotStyle = ron::from_str("(glow_width: 9.0)").unwrap();
        assert_eq!(style.glow_width, 9.0);
        assert_eq!(style.fill_alpha, HotspotStyle::default().fill_alpha);
    }
}
