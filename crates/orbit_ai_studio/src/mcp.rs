//! MCP tools of the studio (plugin tools, keyed `orbit.ai_studio.*`). An
//! MCP client is itself an LLM: it gets the DSL guide and the bible schema,
//! and checks its bible and scene scripts with the same validators as the
//! wizard, then applies them with the built-in `apply_dsl` / character /
//! asset tools.

use orbit_plugin_api::{McpToolDef, Plugin, PluginManifest, ProjectRegistry};
use serde_json::{Value as Json, json};

use crate::bible::{self, BIBLE_EXAMPLE, EXPRESSIONS};
use crate::script::{self, DSL_EXAMPLE, DSL_GUIDE};

pub const PLUGIN_ID: &str = "orbit.ai_studio";

/// Registers the studio's MCP tools.
pub struct AiStudioTools;

impl Plugin for AiStudioTools {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(PLUGIN_ID, "AI Studio", "0.1.0")
    }

    fn register(&self, registry: &mut ProjectRegistry) {
        for tool in tools() {
            if let Err(e) = registry.register_mcp_tool(tool) {
                eprintln!("ai_studio: {e}");
            }
        }
    }
}

fn args(text: &str) -> Result<Json, String> {
    serde_json::from_str(text).map_err(|e| format!("invalid arguments: {e}"))
}

fn bible_arg(a: &Json) -> Result<bible::Bible, String> {
    let value = a.get("bible").ok_or("missing `bible`")?;
    let text = match value {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    };
    bible::parse_bible(&text).map_err(|e| format!("bible is invalid:\n- {}", e.join("\n- ")))
}

pub fn guide_text() -> String {
    format!(
        "{DSL_GUIDE}\n\nExample scene:\n{DSL_EXAMPLE}\n\nAllowed sprite expressions: {}\nAsset naming: backgrounds `{}`, sprites `{}`.\n\nBible JSON schema, by example:\n{BIBLE_EXAMPLE}",
        EXPRESSIONS.join(", "),
        script::background_path("<location_id>"),
        script::sprite_path("<character_id>", "<expression>"),
    )
}

pub fn tools() -> Vec<McpToolDef> {
    vec![
        McpToolDef {
            name: "dsl_guide".to_owned(),
            description: "AI studio: the compact OrbitEngine script grammar, an example scene, the story bible JSON schema and the asset naming the studio uses. Read it before writing scenes.".to_owned(),
            input_schema: r#"{"type":"object","properties":{}}"#.to_owned(),
            destructive: false,
            call: Box::new(|_, _| Ok(guide_text())),
        },
        McpToolDef {
            name: "check_bible".to_owned(),
            description: "AI studio: validates a story bible (JSON object or string). Returns `ok` or the list of problems.".to_owned(),
            input_schema: r#"{"type":"object","properties":{"bible":{"description":"The bible (object or JSON text)."}},"required":["bible"]}"#.to_owned(),
            destructive: false,
            call: Box::new(|_, text| {
                let a = args(text)?;
                bible_arg(&a).map(|b| {
                    format!("ok: {} characters, {} locations, {} scenes", b.characters.len(), b.locations.len(), b.scenes.len())
                })
            }),
        },
        McpToolDef {
            name: "check_scene".to_owned(),
            description: "AI studio: parses and validates one scene script against a bible (as the wizard does). Returns `ok`, the expressions used, or the errors with line numbers.".to_owned(),
            input_schema: r#"{"type":"object","properties":{"bible":{"description":"The bible (object or JSON text)."},"scene":{"type":"string","description":"Scene id of the bible."},"text":{"type":"string","description":"The scene script."}},"required":["bible","scene","text"]}"#.to_owned(),
            destructive: false,
            call: Box::new(|_, text| {
                let a = args(text)?;
                let b = bible_arg(&a)?;
                let scene = a["scene"].as_str().ok_or("missing `scene`")?;
                if b.scene(scene).is_none() {
                    return Err(format!("the bible has no scene `{scene}`"));
                }
                let src = script::normalize_scene(a["text"].as_str().ok_or("missing `text`")?, scene);
                let base = script::scratch_story(&b);
                match script::check_scene(&base, &b, scene, &src) {
                    Ok(_) => {
                        let used = script::used_expressions(&base, &b, std::iter::once((scene, src.as_str())));
                        Ok(json!({"ok": true, "expressions": used}).to_string())
                    }
                    Err(errors) => Err(format!("invalid script:\n- {}", errors.join("\n- "))),
                }
            }),
        },
    ]
}
