//! Resolving what a client writes (ids, names, JSON) into IR values.

use std::str::FromStr;

use orbit_core::{
    AssetId, AssetKind, CharacterId, Color, CustomData, Node, NodeId, NodeKind, Position, Scene,
    Story, Transition, Value,
};
use serde_json::Value as Json;

use crate::error::ToolError;

/// Parses a ULID-based id, with a clear message.
pub fn parse_id<T: FromStr>(text: &str, what: &str) -> Result<T, ToolError> {
    text.trim().parse::<T>().map_err(|_| {
        ToolError::invalid(format!(
            "`{text}` is not a valid {what} id (26-character ULID)"
        ))
    })
}

fn looks_like_id(text: &str) -> bool {
    text.len() == 26 && text.chars().all(|c| c.is_ascii_alphanumeric())
}

/// A scene by id or (case-insensitive) name.
pub fn find_scene<'a>(story: &'a Story, key: &str) -> Result<&'a Scene, ToolError> {
    if looks_like_id(key)
        && let Ok(id) = key.parse::<orbit_core::SceneId>()
        && let Some(scene) = story.scene(id)
    {
        return Ok(scene);
    }
    let named: Vec<&Scene> = story
        .scenes
        .iter()
        .filter(|s| s.name.eq_ignore_ascii_case(key))
        .collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => Err(ToolError::not_found(format!(
            "no scene with id or name `{key}` (known scenes: {})",
            scene_names(story)
        ))),
        many => Err(ToolError::invalid(format!(
            "scene name `{key}` is ambiguous ({} scenes); use the id",
            many.len()
        ))),
    }
}

fn scene_names(story: &Story) -> String {
    if story.scenes.is_empty() {
        return "none".to_owned();
    }
    story
        .scenes
        .iter()
        .map(|s| format!("`{}`", s.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Display name of a character (first of: source string, id).
pub fn character_name(story: &Story, id: CharacterId) -> String {
    story
        .character(id)
        .and_then(|c| story.text(c.name_id))
        .map_or_else(|| id.to_string(), str::to_owned)
}

/// A character by id or (case-insensitive) display name.
pub fn find_character(story: &Story, key: &str) -> Result<CharacterId, ToolError> {
    if looks_like_id(key)
        && let Ok(id) = key.parse::<CharacterId>()
        && story.character(id).is_some()
    {
        return Ok(id);
    }
    let named: Vec<CharacterId> = story
        .characters
        .iter()
        .filter(|c| {
            story
                .text(c.name_id)
                .is_some_and(|n| n.eq_ignore_ascii_case(key))
        })
        .map(|c| c.id)
        .collect();
    match named.as_slice() {
        [one] => Ok(*one),
        [] => Err(ToolError::not_found(format!(
            "no character with id or name `{key}` (known: {})",
            story
                .characters
                .iter()
                .map(|c| format!("`{}`", character_name(story, c.id)))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        _ => Err(ToolError::invalid(format!(
            "character name `{key}` is ambiguous; use the id"
        ))),
    }
}

/// The speaker of a line: `None` (narrator) for absent / `narrator`.
pub fn find_speaker(story: &Story, key: Option<&str>) -> Result<Option<CharacterId>, ToolError> {
    match key.map(str::trim) {
        None | Some("") => Ok(None),
        Some(k) if k.eq_ignore_ascii_case("narrator") && find_character(story, k).is_err() => {
            Ok(None)
        }
        Some(k) => find_character(story, k).map(Some),
    }
}

/// An asset by id, exact manifest path, or file name / stem.
pub fn find_asset(story: &Story, key: &str, kind: Option<AssetKind>) -> Result<AssetId, ToolError> {
    let matches_kind = |k: AssetKind| kind.is_none_or(|want| want == k);
    if looks_like_id(key)
        && let Ok(id) = key.parse::<AssetId>()
        && let Some(entry) = story.assets.get(id)
    {
        return if matches_kind(entry.kind) {
            Ok(id)
        } else {
            Err(ToolError::invalid(format!(
                "asset `{key}` is a {} but a {} is needed",
                entry.kind,
                kind.map_or_else(String::new, |k| k.to_string())
            )))
        };
    }
    let key_norm = key.replace('\\', "/");
    let candidates: Vec<_> = story
        .assets
        .assets
        .iter()
        .filter(|a| matches_kind(a.kind))
        .filter(|a| {
            let file = a.path.rsplit('/').next().unwrap_or(&a.path);
            let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
            a.path == key_norm || file == key_norm || stem == key_norm
        })
        .collect();
    match candidates.as_slice() {
        [one] => Ok(one.id),
        [] => Err(ToolError::not_found(format!(
            "no {}asset `{key}` in the manifest (use list_assets, or import_asset first)",
            kind.map_or_else(String::new, |k| format!("{k} "))
        ))),
        _ => Err(ToolError::invalid(format!(
            "asset name `{key}` is ambiguous; use the asset id or its full path"
        ))),
    }
}

/// A `Label` node by id or by unique name.
pub fn find_label(story: &Story, key: &str) -> Result<NodeId, ToolError> {
    if looks_like_id(key)
        && let Ok(id) = key.parse::<NodeId>()
        && let Some(node) = story.node(id)
    {
        return if matches!(node.kind, NodeKind::Label { .. }) {
            Ok(id)
        } else {
            Err(ToolError::invalid(format!(
                "node `{key}` is not a label; targets must be label nodes (add_command type=label)"
            )))
        };
    }
    let labels: Vec<NodeId> = story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter(|n| matches!(&n.kind, NodeKind::Label { name } if name == key))
        .map(|n| n.id)
        .collect();
    match labels.as_slice() {
        [one] => Ok(*one),
        [] => Err(ToolError::not_found(format!(
            "no label named `{key}`; create it first with add_command type=label"
        ))),
        many => Err(ToolError::invalid(format!(
            "label name `{key}` is ambiguous ({} labels); use a node id",
            many.len()
        ))),
    }
}

/// Where to insert in `nodes` (the nodes of a scene, possibly minus a node
/// being moved): after a node id, at `"start"`, at an explicit index, or at
/// the end by default.
pub fn insertion_index(
    nodes: &[&Node],
    after: Option<&str>,
    index: Option<usize>,
) -> Result<usize, ToolError> {
    match (after, index) {
        (Some(_), Some(_)) => Err(ToolError::invalid(
            "give either `after` or `index`, not both",
        )),
        (None, None) => Ok(nodes.len()),
        (None, Some(i)) if i <= nodes.len() => Ok(i),
        (None, Some(i)) => Err(ToolError::invalid(format!(
            "index {i} is out of range (the scene has {} nodes here; 0..={})",
            nodes.len(),
            nodes.len()
        ))),
        (Some(a), None) if a.eq_ignore_ascii_case("start") => Ok(0),
        (Some(a), None) if a.eq_ignore_ascii_case("end") => Ok(nodes.len()),
        (Some(a), None) => {
            let id: NodeId = parse_id(a, "node")?;
            nodes
                .iter()
                .position(|n| n.id == id)
                .map(|p| p + 1)
                .ok_or_else(|| {
                    ToolError::not_found(format!(
                        "node {a} is not in this scene (`after` must be a node of the target scene, `start` or `end`)"
                    ))
                })
        }
    }
}

/// `left` / `center` / `right` (any case), or the name of a custom position.
pub fn parse_position(story: &Story, text: &str) -> Result<Position, ToolError> {
    match text.trim().to_ascii_lowercase().as_str() {
        "left" => Ok(Position::Left),
        "center" | "centre" => Ok(Position::Center),
        "right" => Ok(Position::Right),
        _ => {
            if story.positions.iter().any(|p| p.name == text) {
                Ok(Position::Custom(text.to_owned()))
            } else {
                Err(ToolError::not_found(format!(
                    "unknown position `{text}` (use left, center, right{})",
                    story
                        .positions
                        .iter()
                        .map(|p| format!(", {}", p.name))
                        .collect::<String>()
                )))
            }
        }
    }
}

/// A free stage position `(x, y)` in normalized coordinates (origin top-left,
/// the sprite is anchored by its bottom-center): finite and within
/// `FREE_POSITION_RANGE`.
pub fn free_position(x: f32, y: f32) -> Result<Position, ToolError> {
    let range = orbit_core::FREE_POSITION_RANGE;
    if range.contains(&x) && range.contains(&y) {
        Ok(Position::Free { x, y })
    } else {
        Err(ToolError::invalid(format!(
            "free position ({x}, {y}) must be finite and within {}..{} on both axes",
            range.start(),
            range.end()
        )))
    }
}

/// A sprite scale: finite and within `SCALE_MIN..=SCALE_MAX`.
pub fn check_scale(name: &str, scale: f32) -> Result<f32, ToolError> {
    if orbit_core::scale_is_valid(scale) {
        Ok(scale)
    } else {
        Err(ToolError::invalid(format!(
            "`{name}` must be between {} and {} (got {scale})",
            orbit_core::SCALE_MIN,
            orbit_core::SCALE_MAX
        )))
    }
}

/// `#rrggbb`, `#rgb` or a few color names.
pub fn parse_color(text: &str) -> Result<Color, ToolError> {
    let t = text.trim().to_ascii_lowercase();
    match t.as_str() {
        "black" => return Ok(Color::BLACK),
        "white" => return Ok(Color::WHITE),
        "red" => return Ok(Color::rgb(220, 50, 50)),
        "green" => return Ok(Color::rgb(60, 170, 80)),
        "blue" => return Ok(Color::rgb(60, 100, 220)),
        _ => {}
    }
    let hex = t.strip_prefix('#').unwrap_or(&t);
    let expand = |c: char| c.to_digit(16).map(|d| (d * 17) as u8);
    let digits: Vec<char> = hex.chars().collect();
    let bad = || ToolError::invalid(format!("`{text}` is not a color (use #rrggbb or a name)"));
    match digits.len() {
        6 => {
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
            if !hex.is_ascii() {
                return Err(bad());
            }
            Ok(Color::rgb(byte(0)?, byte(2)?, byte(4)?))
        }
        3 => {
            let r = expand(digits[0]).ok_or_else(bad)?;
            let g = expand(digits[1]).ok_or_else(bad)?;
            let b = expand(digits[2]).ok_or_else(bad)?;
            Ok(Color::rgb(r, g, b))
        }
        _ => Err(bad()),
    }
}

/// A transition requested by a client.
pub fn build_transition(
    kind: &str,
    duration: Option<f32>,
    color: Option<&str>,
) -> Result<Transition, ToolError> {
    let duration = duration.unwrap_or(0.5);
    if !duration.is_finite() || duration < 0.0 {
        return Err(ToolError::invalid(format!(
            "transition duration {duration} must be a finite number >= 0"
        )));
    }
    match kind.to_ascii_lowercase().as_str() {
        "fade" => Ok(Transition::fade(
            color.map_or(Ok(Color::BLACK), parse_color)?,
            duration,
        )),
        "dissolve" => Ok(Transition::dissolve(duration)),
        other => Err(ToolError::invalid(format!(
            "unknown transition `{other}` (use fade or dissolve)"
        ))),
    }
}

/// Variable names are identifiers of the expression language.
pub fn check_identifier(name: &str) -> Result<(), ToolError> {
    let mut chars = name.chars();
    let ok = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
        && !matches!(name, "true" | "false");
    if ok {
        Ok(())
    } else {
        Err(ToolError::invalid(format!(
            "`{name}` is not a valid variable name (letters, digits and `_`, not starting with a digit, not true/false)"
        )))
    }
}

/// JSON scalar -> variable value.
pub fn json_to_value(json: &Json) -> Result<Value, ToolError> {
    match json {
        Json::Bool(b) => Ok(Value::Bool(*b)),
        Json::String(s) => Ok(Value::Str(s.clone())),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Int(i))
            } else if let Some(f) = n.as_f64().filter(|f| f.is_finite()) {
                Ok(Value::Float(f))
            } else {
                Err(ToolError::invalid(format!("number {n} is out of range")))
            }
        }
        _ => Err(ToolError::invalid(
            "a variable value must be a boolean, a number or a string",
        )),
    }
}

/// Arbitrary JSON -> plugin node data.
pub fn json_to_custom(json: &Json) -> CustomData {
    match json {
        Json::Null => CustomData::Null,
        Json::Bool(b) => CustomData::Bool(*b),
        Json::Number(n) => n.as_i64().map_or_else(
            || CustomData::Float(n.as_f64().unwrap_or(0.0)),
            CustomData::Int,
        ),
        Json::String(s) => CustomData::Str(s.clone()),
        Json::Array(items) => CustomData::List(items.iter().map(json_to_custom).collect()),
        Json::Object(map) => CustomData::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), json_to_custom(v)))
                .collect(),
        ),
    }
}

/// Plugin node data -> JSON (for reading nodes back).
pub fn custom_to_json(data: &CustomData) -> Json {
    match data {
        CustomData::Null => Json::Null,
        CustomData::Bool(b) => Json::Bool(*b),
        CustomData::Int(i) => Json::from(*i),
        CustomData::Float(f) => serde_json::Number::from_f64(*f).map_or(Json::Null, Json::Number),
        CustomData::Str(s) => Json::String(s.clone()),
        CustomData::List(items) => Json::Array(items.iter().map(custom_to_json).collect()),
        CustomData::Map(entries) => Json::Object(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), custom_to_json(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse() {
        assert_eq!(parse_color("#ff0000").unwrap(), Color::rgb(255, 0, 0));
        assert_eq!(parse_color("#fff").unwrap(), Color::WHITE);
        assert_eq!(parse_color("black").unwrap(), Color::BLACK);
        assert!(parse_color("#12").is_err());
        assert!(parse_color("#gggggg").is_err());
        assert!(parse_color("#éééé").is_err());
    }

    #[test]
    fn identifiers_are_checked() {
        assert!(check_identifier("trust_2").is_ok());
        assert!(check_identifier("2fast").is_err());
        assert!(check_identifier("a b").is_err());
        assert!(check_identifier("true").is_err());
        assert!(check_identifier("").is_err());
    }

    #[test]
    fn custom_data_round_trips_through_json() {
        let json = serde_json::json!({"a": [1, 2.5, "x", null, true], "b": {"c": 3}});
        let data = json_to_custom(&json);
        assert_eq!(custom_to_json(&data), json);
    }

    #[test]
    fn insertion_positions() {
        let nodes: Vec<Node> = (0..3)
            .map(|_| Node {
                id: NodeId::new(),
                kind: NodeKind::End,
            })
            .collect();
        let refs: Vec<&Node> = nodes.iter().collect();
        assert_eq!(insertion_index(&refs, None, None).unwrap(), 3);
        assert_eq!(insertion_index(&refs, Some("start"), None).unwrap(), 0);
        assert_eq!(insertion_index(&refs, None, Some(1)).unwrap(), 1);
        assert!(insertion_index(&refs, None, Some(9)).is_err());
        assert_eq!(
            insertion_index(&refs, Some(&nodes[1].id.to_string()), None).unwrap(),
            2
        );
        assert!(insertion_index(&refs, Some(&NodeId::new().to_string()), None).is_err());
        assert!(insertion_index(&refs, Some("start"), Some(0)).is_err());
    }
}
