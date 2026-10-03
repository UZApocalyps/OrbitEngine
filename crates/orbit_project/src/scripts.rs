//! Scripts of a project (ADR 0015, M8-01 / M8-02 / M8-03): the shared
//! modules (`scripts/<name>.rhai`) and the checks of every Rhai source of
//! the story (script nodes, Rhai conditions, modules) with the sandboxed
//! engine of `orbit_script`.
//!
//! Inline scripts live in the story and are edited like any node
//! (`Command::SetNodeKind`, undoable). Modules are project files, like the
//! rig files: the editor and the MCP server write them through
//! [`write_module`] (atomic write, checked name), never directly.

use std::collections::BTreeMap;
use std::path::Path;

use orbit_core::{Expr, NodeId, NodeKind, Story, Value, VarScope};
use orbit_script::{Diagnostic, ScriptEngine, Severity, TestRun};

pub use orbit_core::scripting::{
    SCRIPT_MODULE_DIR, SCRIPT_MODULE_EXT, is_valid_module_name, module_name, module_path,
    script_sources, story_uses_scripts,
};

/// Where a Rhai source is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptSite {
    /// The source of a `Script` node.
    Node(NodeId),
    /// The Rhai condition of an `If` node (`option: None`) or of a choice
    /// option.
    Condition { node: NodeId, option: Option<usize> },
    /// A shared module (`scripts/<name>.rhai`).
    Module(String),
}

impl ScriptSite {
    /// The node, for a site in the story.
    pub fn node(&self) -> Option<NodeId> {
        match self {
            ScriptSite::Node(n) | ScriptSite::Condition { node: n, .. } => Some(*n),
            ScriptSite::Module(_) => None,
        }
    }
}

impl std::fmt::Display for ScriptSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptSite::Node(n) => write!(f, "script node {n}"),
            ScriptSite::Condition { node, option: None } => write!(f, "condition of node {node}"),
            ScriptSite::Condition {
                node,
                option: Some(i),
            } => write!(f, "condition of option {} of node {node}", i + 1),
            ScriptSite::Module(name) => f.write_str(&module_path(name)),
        }
    }
}

/// A problem in a Rhai source, with its line and column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptProblem {
    pub site: ScriptSite,
    pub diagnostic: Diagnostic,
}

impl ScriptProblem {
    pub fn is_error(&self) -> bool {
        self.diagnostic.severity == Severity::Error
    }
}

impl std::fmt::Display for ScriptProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.diagnostic.line > 0 {
            write!(
                f,
                "{} (line {}, column {}): {}",
                self.site, self.diagnostic.line, self.diagnostic.column, self.diagnostic.message
            )
        } else {
            write!(f, "{}: {}", self.site, self.diagnostic.message)
        }
    }
}

/// Checks every Rhai source of the story and every module: syntax (with
/// the sandbox limits on nesting), unknown modules, unknown variables named
/// literally (`get_var("x")` / `set_var("x", …)` of a variable the story
/// does not declare: a warning, the script would fail at run time).
pub fn check_scripts(story: &Story, modules: &BTreeMap<String, String>) -> Vec<ScriptProblem> {
    let engine = ScriptEngine::new(modules.clone());
    let declared: std::collections::HashSet<&str> =
        story.variables.iter().map(|v| v.name.as_str()).collect();
    let mut out = Vec::new();
    let mut push = |site: ScriptSite, source: &str, expression: bool| {
        for diagnostic in engine.check(source, expression) {
            out.push(ScriptProblem {
                site: site.clone(),
                diagnostic,
            });
        }
        for (name, line, column) in literal_vars(source) {
            if !declared.contains(name.as_str()) {
                out.push(ScriptProblem {
                    site: site.clone(),
                    diagnostic: Diagnostic {
                        severity: Severity::Warning,
                        line,
                        column,
                        message: format!("variable `{name}` is not declared in the project"),
                    },
                });
            }
        }
    };
    for node in story.scenes.iter().flat_map(|s| &s.nodes) {
        match &node.kind {
            NodeKind::Script { source, .. } => push(ScriptSite::Node(node.id), source, false),
            NodeKind::If {
                condition: Expr::Rhai(code),
                ..
            } => push(
                ScriptSite::Condition {
                    node: node.id,
                    option: None,
                },
                code,
                true,
            ),
            NodeKind::Choice { options } => {
                for (i, o) in options.iter().enumerate() {
                    if let Some(Expr::Rhai(code)) = &o.condition {
                        push(
                            ScriptSite::Condition {
                                node: node.id,
                                option: Some(i),
                            },
                            code,
                            true,
                        );
                    }
                }
            }
            _ => {}
        }
    }
    for (name, source) in modules {
        push(ScriptSite::Module(name.clone()), source, false);
    }
    out
}

/// Variable names given literally to the script API, with their position
/// (1-based line and column of the name).
fn literal_vars(source: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    for (i, line) in source.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        for f in ["get_var(", "set_var(", "has_var("] {
            let mut from = 0;
            while let Some(at) = code[from..].find(f) {
                let start = from + at + f.len();
                let rest = &code[start..];
                let trimmed = rest.trim_start();
                if let Some(lit) = trimmed.strip_prefix('"')
                    && let Some(end) = lit.find('"')
                {
                    let skipped = rest.len() - trimmed.len();
                    let column = code[..start + skipped].chars().count() + 1;
                    if f != "has_var(" {
                        out.push((lit[..end].to_owned(), i + 1, column));
                    }
                }
                from = start;
            }
        }
    }
    out
}

/// The initial values of the story's variables (both scopes): what a test
/// run in the sandbox starts from.
pub fn initial_variables(story: &Story) -> BTreeMap<String, Value> {
    story
        .variables
        .iter()
        .filter(|v| matches!(v.scope, VarScope::Game | VarScope::Persistent))
        .map(|v| (v.name.clone(), v.initial.clone()))
        .collect()
}

/// Runs a script in the sandbox against the story's initial variables
/// (editor "Run in sandbox" button, MCP `script_test`): nothing is applied.
pub fn test_script(
    story: &Story,
    modules: &BTreeMap<String, String>,
    source: &str,
    seed: u64,
) -> TestRun {
    ScriptEngine::new(modules.clone()).test_run(source, &initial_variables(story), seed)
}

// ------------------------------------------------------------- files

fn check_name(name: &str) -> Result<(), String> {
    if is_valid_module_name(name) {
        Ok(())
    } else {
        Err(format!(
            "`{name}` is not a valid module name (letters, digits, `_` and `-`, at most 64)"
        ))
    }
}

/// Names of the modules of the project (`scripts/*.rhai`), sorted.
pub fn list_modules(root: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(root.join(SCRIPT_MODULE_DIR)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = dir
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            module_name(&format!("{SCRIPT_MODULE_DIR}/{file}")).map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

/// Every module of the project: name -> source (unreadable files skipped).
pub fn read_modules(root: &Path) -> BTreeMap<String, String> {
    list_modules(root)
        .into_iter()
        .filter_map(|name| read_module(root, &name).ok().map(|s| (name, s)))
        .collect()
}

/// The source of a module.
pub fn read_module(root: &Path, name: &str) -> Result<String, String> {
    check_name(name)?;
    let path = module_path(name);
    std::fs::read_to_string(root.join(&path)).map_err(|e| format!("{path}: {e}"))
}

/// Writes a module (creating `scripts/`), atomically: temporary file then
/// rename.
pub fn write_module(root: &Path, name: &str, source: &str) -> Result<(), String> {
    check_name(name)?;
    let dir = root.join(SCRIPT_MODULE_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{SCRIPT_MODULE_DIR}: {e}"))?;
    let path = module_path(name);
    let tmp = dir.join(format!(".{name}.{SCRIPT_MODULE_EXT}.tmp"));
    std::fs::write(&tmp, source).map_err(|e| format!("{path}: {e}"))?;
    std::fs::rename(&tmp, root.join(&path)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{path}: {e}")
    })
}

/// Deletes a module file (a missing file is not an error).
pub fn delete_module(root: &Path, name: &str) -> Result<(), String> {
    check_name(name)?;
    let path = module_path(name);
    match std::fs::remove_file(root.join(&path)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{path}: {e}")),
    }
}

/// Where in a project the module or node of a site is used: script nodes
/// importing `name`.
pub fn module_users(story: &Story, name: &str) -> Vec<NodeId> {
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter(|n| match &n.kind {
            NodeKind::Script { source, .. } => orbit_core::scripting::rhai_imports(source)
                .iter()
                .any(|(m, ..)| m == name),
            _ => false,
        })
        .map(|n| n.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use orbit_core::{Node, Scene, SceneId, VarDecl};

    use super::*;

    fn story(source: &str) -> (Story, NodeId) {
        let mut story = Story::new("t");
        story.variables.push(VarDecl::game("gold", Value::Int(3)));
        let node = Node {
            id: NodeId::new(),
            kind: NodeKind::Script {
                source: source.into(),
                branches: Vec::new(),
                on_error: None,
            },
        };
        let id = node.id;
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes: vec![node],
        });
        (story, id)
    }

    #[test]
    fn problems_have_a_site_a_line_and_a_column() {
        let (s, id) = story("let a = get_var(\"gold\");\nset_var(\"silver\", 1);\nlet b = ;");
        let problems = check_scripts(&s, &BTreeMap::new());
        let error = problems.iter().find(|p| p.is_error()).expect("syntax error");
        assert_eq!(error.site, ScriptSite::Node(id));
        assert_eq!(error.diagnostic.line, 3);
        let warning = problems.iter().find(|p| !p.is_error()).expect("unknown var");
        assert_eq!((warning.diagnostic.line, warning.diagnostic.column), (2, 9));
        assert!(warning.to_string().contains("silver"));
        // A module with an error is reported under its path.
        let modules = BTreeMap::from([("bad".to_owned(), "fn (".to_owned())]);
        let problems = check_scripts(&story("1").0, &modules);
        assert!(problems.iter().any(|p| p.site == ScriptSite::Module("bad".into())));
    }

    #[test]
    fn modules_are_files_with_checked_names() {
        let dir = std::env::temp_dir().join(format!("orbit_scripts_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(list_modules(&dir).is_empty());
        write_module(&dir, "shop", "fn buy() { 1 }").unwrap();
        assert!(write_module(&dir, "../evil", "x").is_err());
        assert_eq!(list_modules(&dir), vec!["shop".to_owned()]);
        assert_eq!(read_module(&dir, "shop").unwrap(), "fn buy() { 1 }");
        let (s, id) = story("import \"shop\" as shop; shop::buy();");
        assert_eq!(module_users(&s, "shop"), vec![id]);
        assert!(check_scripts(&s, &read_modules(&dir)).is_empty());
        let run = test_script(&s, &read_modules(&dir), "import \"shop\" as shop; set_var(\"gold\", shop::buy());", 0);
        assert_eq!(run.changes, vec![("gold".into(), Value::Int(3), Value::Int(1))]);
        delete_module(&dir, "shop").unwrap();
        delete_module(&dir, "shop").unwrap();
        assert!(list_modules(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
