//! MCP tools for hotspot maps (point & click navigation): clickable zones
//! drawn over the background, each jumping to a label.
//!
//! Coordinates are fractions of the screen (0 to 1, origin top-left), so a
//! zone does not depend on the resolution. Every change is an undoable
//! project command.

use orbit_core::{
    AssetKind, Hotspot, HotspotMap, HotspotShape, HotspotStyle, Node, NodeId, NodeKind, StringId,
};
use orbit_project::Command;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::convert::{find_asset, find_label, find_scene, insertion_index, parse_color, parse_id};
use crate::error::ToolError;
use crate::tools::ToolCx;

/// The geometry of a zone, in screen fractions.
#[derive(Deserialize, JsonSchema, Clone)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ShapeArg {
    /// `[x, y, width, height]` of the top-left corner and the size.
    Rect([f32; 4]),
    /// `[center_x, center_y, radius_x, radius_y]`.
    Ellipse([f32; 4]),
    /// The vertices `[[x, y], ...]` (3 to 64), in order around the zone.
    Polygon(Vec<[f32; 2]>),
}

impl ShapeArg {
    fn shape(&self) -> Result<HotspotShape, ToolError> {
        let shape = match self {
            ShapeArg::Rect([x, y, w, h]) => HotspotShape::Rect { x: *x, y: *y, w: *w, h: *h },
            ShapeArg::Ellipse([cx, cy, rx, ry]) => {
                HotspotShape::Ellipse { cx: *cx, cy: *cy, rx: *rx, ry: *ry }
            }
            ShapeArg::Polygon(points) => HotspotShape::Polygon {
                points: points.iter().map(|p| (p[0], p[1])).collect(),
            },
        };
        if shape.is_valid() {
            Ok(shape)
        } else {
            Err(ToolError::invalid(
                "invalid shape: a non-empty rect / ellipse, or a polygon of 3 to 64 points",
            ))
        }
    }
}

/// How zones look; omitted fields keep their current (or default) value.
#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct StyleArg {
    /// Highlight color, `#rrggbb` or a name.
    color: Option<String>,
    /// Opacity of the fill tint on hover, 0 to 1 (default 0.18).
    fill: Option<f32>,
    /// Width of the glowing outline in pixels of a 1920x1080 screen (default 4).
    glow: Option<f32>,
    /// Opacity of the outline on hover, 0 to 1 (default 0.9).
    intensity: Option<f32>,
    /// Pulses per second of the outline, 0 for a steady glow (default 1.2).
    pulse: Option<f32>,
    /// Opacity of the faint shimmering edge shown when idle, 0 to hide the zones until hovered (default 0.12).
    idle: Option<f32>,
    /// The mouse becomes a hand over the zone (default true).
    hand_cursor: Option<bool>,
}

impl StyleArg {
    fn apply(&self, mut style: HotspotStyle) -> Result<HotspotStyle, ToolError> {
        if let Some(c) = &self.color {
            style.color = parse_color(c)?;
        }
        let unit = |v: f32, what: &str| {
            if (0.0..=1.0).contains(&v) {
                Ok(v)
            } else {
                Err(ToolError::invalid(format!("{what} must be between 0 and 1")))
            }
        };
        if let Some(v) = self.fill {
            style.fill_alpha = unit(v, "fill")?;
        }
        if let Some(v) = self.intensity {
            style.intensity = unit(v, "intensity")?;
        }
        if let Some(v) = self.idle {
            style.idle_hint = unit(v, "idle")?;
        }
        if let Some(v) = self.glow {
            if !(0.0..=64.0).contains(&v) {
                return Err(ToolError::invalid("glow must be between 0 and 64"));
            }
            style.glow_width = v;
        }
        if let Some(v) = self.pulse {
            if !(0.0..=10.0).contains(&v) {
                return Err(ToolError::invalid("pulse must be between 0 and 10"));
            }
            style.pulse_speed = v;
        }
        if let Some(v) = self.hand_cursor {
            style.hand_cursor = v;
        }
        Ok(style)
    }
}

/// A zone. For an update, omitted fields keep their value.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZoneArg {
    /// Name for the author (not shown to the player).
    name: Option<String>,
    /// Tooltip shown when the zone is hovered or focused ("" for none).
    tooltip: Option<String>,
    /// The geometry (required for a new zone).
    shape: Option<ShapeArg>,
    /// Label the zone jumps to: a label id or a unique label name (required for a new zone). To leave the scene, point at a label followed by a jump_scene.
    target: Option<String>,
    /// The zone exists only when this expression is true, e.g. `has_key`. "" removes it.
    condition: Option<String>,
    /// The zone is shown dimmed and cannot be clicked while this is false. "" removes it.
    enabled: Option<String>,
    /// Image (full-screen frame) shown inside the zone on hover instead of the glow. "" removes it.
    hover_image: Option<String>,
    /// Sound played when the zone gets the hover / focus. "" removes it.
    hover_sound: Option<String>,
    /// A style of its own (over the map's style).
    style: Option<StyleArg>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddHotspotsArgs {
    /// Scene id or name.
    scene: String,
    /// Background image shown with the map (default: keep the current background).
    background: Option<String>,
    /// The zones (at least one).
    zones: Vec<ZoneArg>,
    /// Look of every zone.
    style: Option<StyleArg>,
    /// Insert after this node id (`start` inserts at the beginning). Default: at the end of the scene.
    after: Option<String>,
    /// Alternative to `after`: explicit index in the scene.
    index: Option<usize>,
}

/// A zone to change.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZoneUpdateArg {
    /// Index of the zone (0-based, see read_scene).
    index: usize,
    #[serde(flatten)]
    zone: ZoneArg,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditHotspotsArgs {
    /// The hotspots node id.
    node: String,
    /// Background image of the map; "" keeps the current background of the story.
    background: Option<String>,
    /// Changes to the map's style.
    style: Option<StyleArg>,
    /// Zones to change (applied before removals).
    update: Option<Vec<ZoneUpdateArg>>,
    /// Indices of zones to delete.
    remove: Option<Vec<usize>>,
    /// Zones to append.
    add: Option<Vec<ZoneArg>>,
    /// Version from read_scene, to avoid overwriting a concurrent change.
    expected_version: Option<u64>,
}

fn opt_expr(text: &str) -> Result<Option<orbit_core::Expr>, ToolError> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    orbit_core::Expr::parse(t)
        .map(Some)
        .map_err(|e| ToolError::invalid(format!("expression `{t}`: {e}")))
}

/// Applies `arg` to `zone`; a new tooltip is queued in `strings`.
fn apply_zone(
    story: &orbit_core::Story,
    zone: &mut Hotspot,
    arg: &ZoneArg,
    strings: &mut Vec<(StringId, String)>,
) -> Result<(), ToolError> {
    if let Some(n) = &arg.name {
        zone.name = n.trim().to_owned();
    }
    if let Some(t) = &arg.tooltip {
        strings.push((zone.text_id, t.trim().to_owned()));
    }
    if let Some(s) = &arg.shape {
        zone.shape = s.shape()?;
    }
    if let Some(t) = &arg.target {
        zone.target = find_label(story, t.trim())?;
    }
    if let Some(c) = &arg.condition {
        zone.condition = opt_expr(c)?;
    }
    if let Some(c) = &arg.enabled {
        zone.enabled = opt_expr(c)?;
    }
    let asset = |key: &str, kind| -> Result<Option<orbit_core::AssetId>, ToolError> {
        if key.trim().is_empty() {
            Ok(None)
        } else {
            find_asset(story, key.trim(), Some(kind)).map(Some)
        }
    };
    if let Some(a) = &arg.hover_image {
        zone.hover_image = asset(a, AssetKind::Image)?;
    }
    if let Some(a) = &arg.hover_sound {
        zone.hover_sound = asset(a, AssetKind::Sound)?;
    }
    if let Some(s) = &arg.style {
        let base = zone.style.clone().unwrap_or_default();
        zone.style = Some(s.apply(base)?);
    }
    Ok(())
}

fn new_zone(
    story: &orbit_core::Story,
    arg: &ZoneArg,
    strings: &mut Vec<(StringId, String)>,
) -> Result<Hotspot, ToolError> {
    let (Some(shape), Some(target)) = (&arg.shape, &arg.target) else {
        return Err(ToolError::invalid("a new zone needs a `shape` and a `target`"));
    };
    let mut zone = Hotspot {
        name: String::new(),
        text_id: StringId::new(),
        shape: shape.shape()?,
        target: find_label(story, target.trim())?,
        condition: None,
        enabled: None,
        style: None,
        hover_image: None,
        hover_sound: None,
    };
    if arg.tooltip.is_none() {
        strings.push((zone.text_id, String::new()));
    }
    apply_zone(story, &mut zone, arg, strings)?;
    Ok(zone)
}

fn summary(cx: &ToolCx<'_>, id: NodeId) -> Result<String, ToolError> {
    let story = cx.project.story();
    let node = story
        .node(id)
        .ok_or_else(|| ToolError::not_found(format!("node {id}")))?;
    let value = json!({
        "id": id.to_string(),
        "version": cx.project.node_version(id),
        "summary": crate::views::describe_kind(story, &node.kind),
    });
    Ok(serde_json::to_string_pretty(&value).unwrap_or_default())
}

fn exec(cx: &mut ToolCx<'_>, command: Command) -> Result<(), ToolError> {
    cx.project.execute(command).map_err(Into::into)
}

/// `add_hotspots`.
pub fn add_hotspots(cx: &mut ToolCx<'_>, a: AddHotspotsArgs) -> Result<String, ToolError> {
    if a.zones.is_empty() {
        return Err(ToolError::invalid("a hotspot map needs at least one zone"));
    }
    let story = cx.project.story();
    let scene = find_scene(story, &a.scene)?;
    let scene_id = scene.id;
    let nodes: Vec<&Node> = scene.nodes.iter().collect();
    let index = insertion_index(&nodes, a.after.as_deref(), a.index)?;
    let mut strings = Vec::new();
    let mut map = HotspotMap {
        background: match a.background.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(key) => Some(find_asset(story, key, Some(AssetKind::Image))?),
        },
        ..Default::default()
    };
    if let Some(s) = &a.style {
        map.style = s.apply(map.style)?;
    }
    for zone in &a.zones {
        map.hotspots.push(new_zone(story, zone, &mut strings)?);
    }
    let node = Node { id: NodeId::new(), kind: NodeKind::Hotspots(map) };
    let id = node.id;
    for (text_id, text) in strings {
        exec(cx, Command::SetString { id: text_id, text: Some(text) })?;
    }
    exec(cx, Command::InsertNode { scene: scene_id, index, node })?;
    summary(cx, id)
}

/// `edit_hotspots`.
pub fn edit_hotspots(cx: &mut ToolCx<'_>, a: EditHotspotsArgs) -> Result<String, ToolError> {
    let id: NodeId = parse_id(&a.node, "node")?;
    let story = cx.project.story();
    let Some(NodeKind::Hotspots(map)) = story.node(id).map(|n| &n.kind) else {
        return Err(ToolError::invalid(format!("node {id} is not a hotspot map")));
    };
    let mut map = map.clone();
    let mut strings = Vec::new();
    let mut released = Vec::new();
    if let Some(bg) = &a.background {
        map.background = match bg.trim() {
            "" => None,
            key => Some(find_asset(story, key, Some(AssetKind::Image))?),
        };
    }
    if let Some(s) = &a.style {
        map.style = s.apply(map.style.clone())?;
    }
    for u in a.update.iter().flatten() {
        let count = map.hotspots.len();
        let zone = map
            .hotspots
            .get_mut(u.index)
            .ok_or_else(|| ToolError::invalid(format!("no zone {} (the map has {count})", u.index)))?;
        apply_zone(story, zone, &u.zone, &mut strings)?;
    }
    let mut remove = a.remove.clone().unwrap_or_default();
    remove.sort_unstable();
    remove.dedup();
    for &i in remove.iter().rev() {
        if i >= map.hotspots.len() {
            return Err(ToolError::invalid(format!("no zone {i} to remove")));
        }
        released.push(map.hotspots.remove(i).text_id);
    }
    for zone in a.add.iter().flatten() {
        map.hotspots.push(new_zone(story, zone, &mut strings)?);
    }
    if map.hotspots.is_empty() {
        return Err(ToolError::invalid(
            "a hotspot map needs at least one zone (delete the node instead)",
        ));
    }
    for (text_id, text) in strings {
        exec(cx, Command::SetString { id: text_id, text: Some(text) })?;
    }
    exec(
        cx,
        Command::SetNodeKind {
            id,
            kind: NodeKind::Hotspots(map),
            expected_version: a.expected_version,
        },
    )?;
    for text_id in released {
        exec(cx, Command::SetString { id: text_id, text: None })?;
    }
    summary(cx, id)
}
