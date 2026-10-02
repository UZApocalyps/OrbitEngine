//! JSON Schema of the story IR, generated from the Rust types of `orbit_core`
//! (feature `schema`): the same source the editor's auto-generated inspector
//! reads, so the IR, the editor and the MCP tools cannot drift apart.

use orbit_core::Story;
use serde_json::Value as Json;

/// Schema of a whole [`Story`] (draft 2020-12, with `$defs`).
pub fn ir_schema() -> Json {
    let schema = schemars::schema_for!(Story);
    serde_json::to_value(&schema).unwrap_or(Json::Null)
}

/// Names of the `NodeKind` variants found in an IR schema produced by
/// [`ir_schema`]. Used by the coherence tests.
pub fn node_kind_variants(schema: &Json) -> Vec<String> {
    let Some(variants) = schema
        .pointer("/$defs/NodeKind/oneOf")
        .and_then(Json::as_array)
    else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for variant in variants {
        // Struct variants: {"properties": {"Say": {...}}, "required": ["Say"]}.
        if let Some(props) = variant.get("properties").and_then(Json::as_object) {
            names.extend(props.keys().cloned());
        }
        // Unit variants: {"const": "Return"} or {"enum": ["Return", "End"]}.
        if let Some(name) = variant.get("const").and_then(Json::as_str) {
            names.push(name.to_owned());
        }
        if let Some(list) = variant.get("enum").and_then(Json::as_array) {
            names.extend(list.iter().filter_map(Json::as_str).map(str::to_owned));
        }
    }
    names.sort();
    names
}
