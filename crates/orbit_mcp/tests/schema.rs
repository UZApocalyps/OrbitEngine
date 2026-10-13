//! The MCP tool schemas and the story IR must not drift apart (ROADMAP M3f).
//!
//! The IR schema is generated from the Rust types of `orbit_core` (feature
//! `schema`), the tool schemas from the argument types of `orbit_mcp`. These
//! tests fail when someone adds or changes an IR node kind without updating
//! the MCP tools.

use std::collections::BTreeSet;

use orbit_mcp::schema::{ir_schema, node_kind_variants};
use orbit_mcp::tools::{NODE_KIND_TOOLS, builtin_tools};
use serde_json::Value;

fn snake_to_pascal(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn add_command_variants() -> Vec<(String, BTreeSet<String>)> {
    let tools = builtin_tools();
    let tool = tools.iter().find(|t| t.name == "add_command").unwrap();
    let schema = Value::Object(tool.input_schema.clone());
    let one_of = schema
        .pointer("/properties/command/oneOf")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("add_command command schema has no oneOf: {schema}"));
    one_of
        .iter()
        .map(|variant| {
            let props = variant["properties"].as_object().unwrap();
            let ty = props["type"]["const"]
                .as_str()
                .unwrap_or_else(|| panic!("no type const in {variant}"))
                .to_owned();
            let fields = props.keys().filter(|k| *k != "type").cloned().collect();
            (ty, fields)
        })
        .collect()
}

fn ir_variant_fields(schema: &Value, variant: &str) -> BTreeSet<String> {
    let one_of = schema
        .pointer("/$defs/NodeKind/oneOf")
        .unwrap()
        .as_array()
        .unwrap();
    one_of
        .iter()
        .find_map(|v| v.pointer(&format!("/properties/{variant}/properties")))
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn the_ir_schema_describes_the_story() {
    let schema = ir_schema();
    assert!(schema.pointer("/$defs/NodeKind").is_some(), "{schema}");
    assert!(schema.pointer("/$defs/Story").is_some() || schema.get("properties").is_some());
    for def in [
        "Scene",
        "Node",
        "Character",
        "Expr",
        "Value",
        "CustomData",
        "Position",
    ] {
        assert!(
            schema.pointer(&format!("/$defs/{def}")).is_some(),
            "{def} missing"
        );
    }
    // Ids are strings in the schema, like in the RON.
    assert_eq!(
        schema.pointer("/$defs/NodeId/type").and_then(Value::as_str),
        Some("string")
    );
}

#[test]
fn every_ir_node_kind_has_an_mcp_tool_and_the_reverse() {
    let schema = ir_schema();
    let ir: BTreeSet<String> = node_kind_variants(&schema).into_iter().collect();
    let covered: BTreeSet<String> = NODE_KIND_TOOLS
        .iter()
        .map(|(v, _)| (*v).to_owned())
        .collect();
    assert_eq!(
        ir, covered,
        "NodeKind variants of the IR and NODE_KIND_TOOLS differ: add (or remove) an MCP tool"
    );

    // add_command's `type` values are exactly the commands listed in the table.
    let spec = add_command_variants();
    let from_spec: BTreeSet<String> = spec.iter().map(|(t, _)| snake_to_pascal(t)).collect();
    let expected: BTreeSet<String> = NODE_KIND_TOOLS
        .iter()
        .filter(|(_, tool)| tool.starts_with("add_command"))
        .map(|(v, _)| (*v).to_owned())
        .collect();
    assert_eq!(from_spec, expected);
}

#[test]
fn command_fields_match_the_ir_fields() {
    let schema = ir_schema();
    for (ty, fields) in add_command_variants() {
        let variant = snake_to_pascal(&ty);
        if variant == "Effect" {
            // `NodeKind::Effect(Effect)` wraps an enum: the command is one
            // flat form of its variants, plus the `effect` selector and the
            // `bezier` alternative to `ease`.
            let union: BTreeSet<String> = schema
                .pointer("/$defs/Effect/oneOf")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .flat_map(|v| {
                    v.pointer("/properties")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flat_map(|o| o.values())
                        .flat_map(|inner| {
                            inner
                                .pointer("/properties")
                                .and_then(Value::as_object)
                                .into_iter()
                                .flat_map(|p| p.keys().cloned())
                        })
                })
                .collect();
            let mut expected = union;
            expected.extend(["effect", "bezier"].map(String::from));
            assert_eq!(fields, expected, "`add_command type=effect` vs IR Effect");
            continue;
        }
        if variant == "Particles" {
            // `NodeKind::Particles(Emitter)` wraps a struct: the command is
            // the emitter's fields flat.
            let emitter: BTreeSet<String> = schema
                .pointer("/$defs/Emitter/properties")
                .and_then(Value::as_object)
                .unwrap()
                .keys()
                .cloned()
                .collect();
            assert_eq!(fields, emitter, "`add_command type=particles` vs IR Emitter");
            continue;
        }
        if variant == "Notify" {
            // `NodeKind::Notify(Notify)` wraps a struct; the command takes
            // the literal `text` where the IR holds a `text_id`.
            let mut notify: BTreeSet<String> = schema
                .pointer("/$defs/Notify/properties")
                .and_then(Value::as_object)
                .unwrap()
                .keys()
                .cloned()
                .collect();
            notify.remove("text_id");
            notify.insert("text".to_owned());
            assert_eq!(fields, notify, "`add_command type=notify` vs IR Notify");
            continue;
        }
        let ir_fields = ir_variant_fields(&schema, &variant);
        // `repeat` is the pass count of the IR's `looping: Loop(n)`.
        let mut fields = fields;
        if variant == "Timeline" {
            fields.remove("repeat");
        }
        assert_eq!(
            fields, ir_fields,
            "`add_command type={ty}` takes different fields than IR NodeKind::{variant}"
        );
    }
}

#[test]
fn dialogue_and_choice_tools_cover_the_ir_fields() {
    let schema = ir_schema();
    let tools = builtin_tools();
    let props = |name: &str| -> BTreeSet<String> {
        tools.iter().find(|t| t.name == name).unwrap().input_schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    };
    // Say { speaker, text_id, voice }: speaker, the text itself (the string table entry) and the voice.
    let say = ir_variant_fields(&schema, "Say");
    assert_eq!(say, ["speaker", "text_id", "voice"].map(String::from).into());
    let dialogue = props("add_dialogue");
    assert!(dialogue.contains("speaker") && dialogue.contains("text") && dialogue.contains("voice"));
    // Choice { options: [{ text_id, target, condition }] }.
    let option = schema
        .pointer("/$defs/ChoiceOption/properties")
        .unwrap()
        .as_object()
        .unwrap();
    let option_fields: BTreeSet<String> = option.keys().cloned().collect();
    assert_eq!(
        option_fields,
        ["condition", "target", "text_id"].map(String::from).into()
    );
    let choice_tool = tools.iter().find(|t| t.name == "add_choice").unwrap();
    let option_schema = &choice_tool.input_schema["properties"]["options"]["items"]["properties"];
    let tool_option: BTreeSet<String> =
        option_schema.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        tool_option,
        ["condition", "target", "text"].map(String::from).into()
    );
}

#[test]
fn every_tool_schema_is_a_closed_object() {
    for tool in builtin_tools() {
        let schema = Value::Object(tool.input_schema.clone());
        assert_eq!(schema["type"], "object", "{}", tool.name);
        assert_eq!(
            schema["additionalProperties"],
            Value::Bool(false),
            "{} must reject unknown fields (typos)",
            tool.name
        );
        // Required fields must be properties.
        if let Some(required) = schema["required"].as_array() {
            for r in required {
                assert!(
                    schema["properties"].get(r.as_str().unwrap()).is_some(),
                    "{}",
                    tool.name
                );
            }
        }
    }
}
