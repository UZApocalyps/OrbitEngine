//! Engine-free helpers about the scripts of a story (ADR 0015): which
//! nodes use Rhai and which shared modules they import. The engine itself
//! lives in `orbit_script`; players, exporters and tools that do not link
//! it still need to know these.

use crate::expr::Expr;
use crate::story::{NodeKind, Story};

/// Folder of the shared modules in a project (and of their pak entries).
pub const SCRIPT_MODULE_DIR: &str = "scripts";
/// Extension of a module file.
pub const SCRIPT_MODULE_EXT: &str = "rhai";

/// Whether `name` can name a module (`scripts/<name>.rhai`): ASCII
/// letters, digits, `_` and `-`, 1 to 64 characters.
pub fn is_valid_module_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Project path of the module `name` (`scripts/<name>.rhai`).
pub fn module_path(name: &str) -> String {
    format!("{SCRIPT_MODULE_DIR}/{name}.{SCRIPT_MODULE_EXT}")
}

/// The module name of a project path (`scripts/shop.rhai` -> `shop`).
pub fn module_name(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(SCRIPT_MODULE_DIR)?.strip_prefix('/')?;
    let name = rest.strip_suffix(SCRIPT_MODULE_EXT)?.strip_suffix('.')?;
    is_valid_module_name(name).then_some(name)
}

/// Whether a story uses scripts (a `Script` node or a Rhai condition): such
/// a game needs a player built with scripting.
pub fn story_uses_scripts(story: &Story) -> bool {
    !script_sources(story).is_empty()
}

/// Every Rhai source of the story, in story order: script nodes and Rhai
/// conditions.
pub fn script_sources(story: &Story) -> Vec<&str> {
    let mut out = Vec::new();
    for node in story.scenes.iter().flat_map(|s| &s.nodes) {
        match &node.kind {
            NodeKind::Script { source, .. } => out.push(source.as_str()),
            NodeKind::If {
                condition: Expr::Rhai(code),
                ..
            } => out.push(code.as_str()),
            NodeKind::Choice { options } => {
                for o in options {
                    if let Some(Expr::Rhai(code)) = &o.condition {
                        out.push(code.as_str());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// `import "name"` statements of a Rhai source: (name, line, column),
/// 1-based. A lexical scan (no engine): imports inside a `//` comment are
/// skipped, those inside a multi-line string or block comment are not.
pub fn rhai_imports(source: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    for (i, line) in source.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let mut offset = 0;
        let mut rest = code;
        while let Some(at) = rest.find("import") {
            let before_ok = rest[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            let after = rest[at + 6..].trim_start();
            if before_ok
                && let Some(lit) = after.strip_prefix('"')
                && let Some(end) = lit.find('"')
            {
                let column = code[..offset + at].chars().count() + 1;
                out.push((lit[..end].to_owned(), i + 1, column));
            }
            offset += at + 6;
            rest = &rest[at + 6..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_are_found_with_their_position() {
        let src = "let a = 1;\n  import \"shop\" as s; import \"inv\" as i;\n// import \"no\" as n;\nreimport(\"x\")";
        assert_eq!(
            rhai_imports(src),
            vec![("shop".to_owned(), 2, 3), ("inv".to_owned(), 2, 23)]
        );
    }

    #[test]
    fn module_paths() {
        assert_eq!(module_path("shop"), "scripts/shop.rhai");
        assert_eq!(module_name("scripts/shop.rhai"), Some("shop"));
        assert_eq!(module_name("scripts/a/b.rhai"), None);
        assert_eq!(module_name("assets/x.rhai"), None);
        assert!(!is_valid_module_name("../x"));
    }
}
