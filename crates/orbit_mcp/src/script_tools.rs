//! MCP tools for scripts (M8-03, ADR 0015): list and read the Rhai sources
//! of the project (script nodes, Rhai conditions, shared modules), edit
//! them, and run the sandbox diagnostics and test runs.
//!
//! Script nodes are story nodes: they are edited through
//! `Command::SetNodeKind` (undoable). Shared modules (`scripts/<name>.rhai`)
//! are project files like rigs and UI screens: written through
//! `orbit_project::scripts::write_module` (atomic, checked name), not
//! covered by undo. Writes are refused, nothing changed, when the source
//! has errors (syntax, unknown module).

use std::collections::BTreeMap;

use orbit_core::{Expr, NodeId, NodeKind};
use orbit_project::Command;
use orbit_project::scripts::{
    ScriptProblem, ScriptSite, check_scripts, delete_module, list_modules, module_path,
    module_users, read_module, read_modules, test_script, write_module,
};
use orbit_script::{Diagnostic, ScriptEngine, Severity};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::convert::{find_label, parse_id};
use crate::error::ToolError;
use crate::tools::{ScriptBranchArg, ToolCx, script_branches};

fn out(value: Json) -> Result<String, ToolError> {
    Ok(serde_json::to_string_pretty(&value).unwrap_or_default())
}

fn io_err(e: impl std::fmt::Display) -> ToolError {
    ToolError::new(crate::ErrorKind::Io, e.to_string())
}

fn modules(cx: &ToolCx<'_>) -> BTreeMap<String, String> {
    read_modules(cx.sandbox.root())
}

fn diag_json(d: &Diagnostic) -> Json {
    json!({
        "severity": match d.severity { Severity::Error => "error", Severity::Warning => "warning" },
        "line": d.line,
        "column": d.column,
        "message": d.message,
    })
}

fn problem_json(p: &ScriptProblem) -> Json {
    let mut j = diag_json(&p.diagnostic);
    j["where"] = json!(p.site.to_string());
    match &p.site {
        ScriptSite::Node(n) => j["node"] = json!(n.to_string()),
        ScriptSite::Condition { node, option } => {
            j["node"] = json!(node.to_string());
            if let Some(i) = option {
                j["option"] = json!(i);
            }
        }
        ScriptSite::Module(m) => j["module"] = json!(m),
    }
    j
}

fn errors_of(diagnostics: &[Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(ToString::to_string)
        .collect()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

/// `list_scripts`: every Rhai source of the project.
pub fn list_scripts(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    let story = cx.project.story();
    let mods = modules(cx);
    let problems = check_scripts(story, &mods);
    let count = |site: &ScriptSite| problems.iter().filter(|p| &p.site == site).count();
    let mut nodes = Vec::new();
    let mut conditions = Vec::new();
    for scene in &story.scenes {
        for node in &scene.nodes {
            match &node.kind {
                NodeKind::Script {
                    source,
                    branches,
                    on_error,
                } => nodes.push(json!({
                    "node": node.id.to_string(),
                    "scene": scene.name,
                    "lines": source.lines().count(),
                    "first_line": source.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim(),
                    "branches": branches.iter().map(|b| json!({"name": b.name, "target": b.target.to_string()})).collect::<Vec<_>>(),
                    "on_error": on_error.map(|n| n.to_string()),
                    "imports": orbit_core::scripting::rhai_imports(source).into_iter().map(|(m, ..)| m).collect::<Vec<_>>(),
                    "problems": count(&ScriptSite::Node(node.id)),
                })),
                NodeKind::If {
                    condition: Expr::Rhai(code),
                    ..
                } => conditions.push(json!({
                    "node": node.id.to_string(), "scene": scene.name, "code": code,
                    "problems": count(&ScriptSite::Condition { node: node.id, option: None }),
                })),
                NodeKind::Choice { options } => {
                    for (i, o) in options.iter().enumerate() {
                        if let Some(Expr::Rhai(code)) = &o.condition {
                            conditions.push(json!({
                                "node": node.id.to_string(), "scene": scene.name, "option": i, "code": code,
                                "problems": count(&ScriptSite::Condition { node: node.id, option: Some(i) }),
                            }));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let module_list: Vec<Json> = mods
        .iter()
        .map(|(name, source)| {
            json!({
                "name": name,
                "path": module_path(name),
                "lines": source.lines().count(),
                "used_by": module_users(story, name).iter().map(ToString::to_string).collect::<Vec<_>>(),
                "problems": count(&ScriptSite::Module(name.clone())),
            })
        })
        .collect();
    out(json!({
        "api_version": orbit_script::API_VERSION,
        "script_nodes": nodes,
        "rhai_conditions": conditions,
        "modules": module_list,
        "problem_count": problems.len(),
    }))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadScriptArgs {
    /// A script node id (its source, branches and error label)...
    node: Option<String>,
    /// ...or a shared module name (`shop` for scripts/shop.rhai).
    module: Option<String>,
}

/// `read_script`: the source of a script node or a module, with its
/// diagnostics.
pub fn read_script(cx: &mut ToolCx<'_>, a: ReadScriptArgs) -> Result<String, ToolError> {
    let mods = modules(cx);
    let engine = ScriptEngine::new(mods.clone());
    match (a.node, a.module) {
        (Some(node), None) => {
            let id: NodeId = parse_id(&node, "node")?;
            let story = cx.project.story();
            let Some(NodeKind::Script {
                source,
                branches,
                on_error,
            }) = story.node(id).map(|n| &n.kind)
            else {
                return Err(ToolError::not_found(format!("node {id} is not a script node")));
            };
            out(json!({
                "node": id.to_string(),
                "source": source,
                "branches": branches.iter().map(|b| json!({"name": b.name, "target": b.target.to_string()})).collect::<Vec<_>>(),
                "on_error": on_error.map(|n| n.to_string()),
                "diagnostics": engine.check(source, false).iter().map(diag_json).collect::<Vec<_>>(),
            }))
        }
        (None, Some(module)) => {
            let source = read_module(cx.sandbox.root(), &module).map_err(ToolError::not_found)?;
            out(json!({
                "module": module,
                "path": module_path(&module),
                "source": source,
                "used_by": module_users(cx.project.story(), &module).iter().map(ToString::to_string).collect::<Vec<_>>(),
                "diagnostics": engine.check_module(&source).iter().map(diag_json).collect::<Vec<_>>(),
            }))
        }
        _ => Err(ToolError::invalid("give exactly one of `node` or `module`")),
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetScriptArgs {
    /// Id of the script node to edit.
    node: String,
    /// New Rhai source (default: unchanged).
    source: Option<String>,
    /// New branches (default: unchanged; `[]` removes them all).
    branches: Option<Vec<ScriptBranchArg>>,
    /// New error label id or name; `""` removes it (default: unchanged).
    on_error: Option<String>,
}

/// `set_script`: edits a script node (undoable). Refused when the new
/// source has errors.
pub fn set_script(cx: &mut ToolCx<'_>, a: SetScriptArgs) -> Result<String, ToolError> {
    let id: NodeId = parse_id(&a.node, "node")?;
    let story = cx.project.story();
    let Some(NodeKind::Script {
        source,
        branches,
        on_error,
    }) = story.node(id).map(|n| n.kind.clone())
    else {
        return Err(ToolError::not_found(format!("node {id} is not a script node")));
    };
    let source = a.source.unwrap_or(source);
    let branches = match &a.branches {
        Some(b) => script_branches(story, b)?,
        None => branches,
    };
    let on_error = match a.on_error.as_deref().map(str::trim) {
        None => on_error,
        Some("") => None,
        Some(l) => Some(find_label(story, l)?),
    };
    let engine = ScriptEngine::new(modules(cx));
    let diagnostics = engine.check(&source, false);
    let errors = errors_of(&diagnostics);
    if !errors.is_empty() {
        return Err(ToolError::refused(format!(
            "the script has errors, nothing changed: {}",
            errors.join("; ")
        )));
    }
    cx.project
        .execute(Command::SetNodeKind {
            id,
            kind: NodeKind::Script {
                source,
                branches,
                on_error,
            },
            expected_version: None,
        })
        .map_err(ToolError::from)?;
    out(json!({
        "node": id.to_string(),
        "diagnostics": diagnostics.iter().map(diag_json).collect::<Vec<_>>(),
    }))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteModuleArgs {
    /// Module name: letters, digits, `_`, `-` (written to scripts/<name>.rhai).
    name: String,
    /// The Rhai source (functions the story's scripts call as `name::f()`).
    source: String,
}

/// `write_script_module`: creates or replaces a shared module. Refused,
/// nothing written, when it has errors.
pub fn write_script_module(cx: &mut ToolCx<'_>, a: WriteModuleArgs) -> Result<String, ToolError> {
    let mut mods = modules(cx);
    mods.insert(a.name.clone(), a.source.clone());
    let engine = ScriptEngine::new(mods);
    let diagnostics = engine.check_module(&a.source);
    let errors = errors_of(&diagnostics);
    if !errors.is_empty() {
        return Err(ToolError::refused(format!(
            "the module has errors, nothing written: {}",
            errors.join("; ")
        )));
    }
    write_module(cx.sandbox.root(), &a.name, &a.source).map_err(|e| {
        if orbit_core::scripting::is_valid_module_name(&a.name) {
            io_err(e)
        } else {
            ToolError::invalid(e)
        }
    })?;
    out(json!({
        "module": a.name,
        "path": module_path(&a.name),
        "diagnostics": diagnostics.iter().map(diag_json).collect::<Vec<_>>(),
    }))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleArg {
    /// Module name (`shop` for scripts/shop.rhai).
    name: String,
}

/// `delete_script_module`: refused while a script node imports it.
pub fn delete_script_module(cx: &mut ToolCx<'_>, a: ModuleArg) -> Result<String, ToolError> {
    if !list_modules(cx.sandbox.root()).contains(&a.name) {
        return Err(ToolError::not_found(format!("no module `{}`", a.name)));
    }
    let users = module_users(cx.project.story(), &a.name);
    if !users.is_empty() {
        return Err(ToolError::refused(format!(
            "module `{}` is imported by {} script node(s): {}",
            a.name,
            users.len(),
            users.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
        )));
    }
    delete_module(cx.sandbox.root(), &a.name).map_err(io_err)?;
    out(json!({"deleted": module_path(&a.name)}))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckArgs {
    /// A source to check instead of the whole project.
    source: Option<String>,
    /// With `source`: check it as a Rhai condition (one expression).
    expression: Option<bool>,
}

/// `check_scripts`: sandbox diagnostics (line and column) of every Rhai
/// source of the project, or of one given source.
pub fn check(cx: &mut ToolCx<'_>, a: CheckArgs) -> Result<String, ToolError> {
    let mods = modules(cx);
    match a.source {
        Some(source) => {
            let engine = ScriptEngine::new(mods);
            let d = engine.check(&source, a.expression.unwrap_or(false));
            out(json!({"diagnostics": d.iter().map(diag_json).collect::<Vec<_>>()}))
        }
        None => {
            let problems = check_scripts(cx.project.story(), &mods);
            out(json!({
                "error_count": problems.iter().filter(|p| p.is_error()).count(),
                "warning_count": problems.iter().filter(|p| !p.is_error()).count(),
                "problems": problems.iter().map(problem_json).collect::<Vec<_>>(),
            }))
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestArgs {
    /// A script node id to run...
    node: Option<String>,
    /// ...or a source to run.
    source: Option<String>,
    /// RNG seed (default 0): same seed, same draws.
    seed: Option<u64>,
}

/// `test_script`: runs a script in the sandbox against the story's initial
/// variables. Nothing is applied: returns the branch it would take (or its
/// error with line and column), the variable changes, its logs and the
/// number of random draws.
pub fn test(cx: &mut ToolCx<'_>, a: TestArgs) -> Result<String, ToolError> {
    let story = cx.project.story();
    let source = match (a.node, a.source) {
        (Some(node), None) => {
            let id: NodeId = parse_id(&node, "node")?;
            match story.node(id).map(|n| &n.kind) {
                Some(NodeKind::Script { source, .. }) => source.clone(),
                _ => return Err(ToolError::not_found(format!("node {id} is not a script node"))),
            }
        }
        (None, Some(source)) => source,
        _ => return Err(ToolError::invalid("give exactly one of `node` or `source`")),
    };
    let run = test_script(story, &modules(cx), &source, a.seed.unwrap_or(0));
    let result = match &run.result {
        Ok(branch) => json!({"ok": true, "branch": branch}),
        Err(e) => json!({"ok": false, "error": e.message, "line": e.line, "column": e.column}),
    };
    out(json!({
        "result": result,
        "changes": run.changes.iter().map(|(n, before, after)| json!({"variable": n, "before": before.to_string(), "after": after.to_string()})).collect::<Vec<_>>(),
        "logs": run.logs,
        "random_draws": run.draws,
    }))
}
