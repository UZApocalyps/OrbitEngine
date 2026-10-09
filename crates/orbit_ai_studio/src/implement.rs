//! Step e: puts the bible, the scripts and the pictures into the project,
//! through the **same tools as the MCP server** (`import_asset`,
//! `create_character`, `declare_variable`, `apply_dsl`), as one undo entry.

use std::path::Path;

use orbit_mcp::ToolRunner;
use orbit_mcp::sandbox::Sandbox;
use orbit_plugin_api::ProjectRegistry;
use orbit_project::{Project, ProjectManifest};
use serde_json::{Value as Json, json};

use crate::assets::AssetKindAi;
use crate::pipeline::PipelineState;

/// Undo label of an implementation.
pub const UNDO_LABEL: &str = "AI Studio: implement";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImplementOptions {
    /// Replace every scene of the story by the generated ones (the first
    /// generated scene then starts the game). Otherwise they are appended.
    pub replace_scenes: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImplementReport {
    pub assets: usize,
    pub characters: usize,
    pub variables: usize,
    pub scenes: usize,
    /// Validation errors of the resulting story (empty when it plays).
    pub errors: Vec<String>,
    pub warnings: usize,
}

/// Whether a story is still the empty starter (nothing worth keeping).
pub fn is_blank(project: &Project) -> bool {
    let story = project.story();
    story.characters.is_empty() && story.scenes.iter().map(|s| s.nodes.len()).sum::<usize>() <= 2
}

pub fn implement(
    project: &mut Project,
    root: &Path,
    manifest: &ProjectManifest,
    registry: &ProjectRegistry,
    state: &PipelineState,
    options: ImplementOptions,
) -> Result<ImplementReport, String> {
    let bible = state.bible.as_ref().ok_or("write the bible first")?;
    if !state.script_complete() {
        return Err(format!(
            "some scenes have no valid script yet: {}",
            state.scenes_to_write().join(", ")
        ));
    }
    let sandbox = Sandbox::new(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let runner = ToolRunner::new();
    let call = |p: &mut Project, name: &str, args: Json| -> Result<String, String> {
        runner
            .dispatch(p, &sandbox, manifest, registry, name, args)
            .0
            .map_err(|e| format!("{name}: {e}"))
    };
    if !options.replace_scenes {
        let story = project.story();
        if let Some(s) = bible.scenes.iter().find(|s| story.scenes.iter().any(|x| x.name == s.id)) {
            return Err(format!(
                "the project already has a scene named `{}`: rename it or replace the scenes",
                s.id
            ));
        }
    }

    let mut report = ImplementReport::default();
    project.group(UNDO_LABEL, |p| -> Result<(), String> {
        // 1. Pictures (already written under assets/ai/: registered in place).
        for job in &state.assets {
            if root.join(&job.path).is_file() {
                call(p, "import_asset", json!({"source": job.path}))?;
                report.assets += 1;
            }
        }
        // 2. Characters with their expressions.
        for c in &bible.characters {
            let expressions: Vec<Json> = state
                .assets
                .iter()
                .filter(|j| j.kind == AssetKindAi::Sprite && j.owner == c.id && root.join(&j.path).is_file())
                .filter_map(|j| j.expression.as_ref().map(|e| json!({"name": e, "asset": j.path})))
                .collect();
            let name = c.name.trim();
            let exists = {
                let story = p.story();
                story
                    .characters
                    .iter()
                    .any(|x| story.text(x.name_id).is_some_and(|n| n.eq_ignore_ascii_case(name)))
            };
            if exists {
                call(p, "edit_character", json!({"character": name, "set_expressions": expressions}))?;
            } else {
                let mut args = json!({"name": name, "expressions": expressions});
                if crate::bible::parse_hex_color(&c.color).is_some() {
                    args["color"] = json!(c.color.trim());
                }
                call(p, "create_character", args)?;
            }
            report.characters += 1;
        }
        // 3. Variables.
        for v in &bible.variables {
            if p.story().variables.iter().any(|x| x.name == v.name) {
                continue;
            }
            call(p, "declare_variable", json!({"name": v.name, "initial": v.initial}))?;
            report.variables += 1;
        }
        // 4. Scenes, as one DSL text.
        let script = state.full_script();
        let text = if options.replace_scenes {
            script
        } else {
            format!("{}\n\n{script}\n", orbit_dsl::print_story(p.story()))
        };
        call(p, "apply_dsl", json!({"text": text}))?;
        report.scenes = bible.scenes.len();
        Ok(())
    })?;

    let validation = orbit_core::validate::validate(project.story());
    report.errors = validation.errors.iter().map(ToString::to_string).collect();
    report.warnings = validation.warnings.len();
    Ok(report)
}
