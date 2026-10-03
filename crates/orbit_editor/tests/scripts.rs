//! Rhai scripts in the editor (M8-02): the palette draft of a `Script`
//! node, its list summary and kind, the localized problems (validation and
//! syntax errors with line and column), shared modules. GPU-free.

use std::path::PathBuf;

use egui::Context;
use orbit_core::{Expr, NodeKind};
use orbit_editor::builtin::drafts::drafts_for;
use orbit_editor::builtin::{kind_key, kind_label, summary};
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor::diagnostics::{Extras, compute};
use orbit_editor::i18n::translator;
use orbit_editor_api::{Lang, Level};
use orbit_project::Command;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-editor-scripts-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn script_nodes_are_drafted_summarized_and_their_errors_located() {
    let ctx = Context::default();
    let dir = TempDir::new("node");
    let en = translator(Lang::En);
    let fr = translator(Lang::Fr);
    let root = orbit_editor::storage::create_project(&dir.0, "Scripts", &en).unwrap();
    let mut core = EditorCore::new(CoreOptions::default(), &ctx);
    core.open_project(&root);
    let session = core.session.as_mut().unwrap();
    let scene = session.project.story().scenes[0].id;

    let drafts = drafts_for(session.project.story(), &core.registry, &en, "Script", scene, 0)
        .expect("the palette offers a script node");
    // A script and the label of its first branch.
    assert_eq!(drafts.len(), 2);
    let id = drafts[0].node.id;
    assert_eq!(kind_key(&drafts[0].node.kind), "Script");
    for (index, draft) in drafts.into_iter().enumerate() {
        session
            .project
            .execute(Command::InsertNode {
                scene,
                index,
                node: draft.node,
            })
            .unwrap();
    }
    let node = session.project.story().node(id).unwrap();
    assert_eq!(kind_label(&en, &node.kind), "Script");
    assert_eq!(summary(&en, session.project.story(), node), "branch(\"ok\");  ·  1 branch");
    assert_eq!(
        summary(&fr, session.project.story(), node),
        "branch(\"ok\");  ·  1 branche"
    );

    // A syntax error on line 2 of the source.
    let NodeKind::Script {
        branches, on_error, ..
    } = node.kind.clone()
    else {
        panic!("a script");
    };
    let version = session.project.node_version(id);
    session
        .project
        .execute(Command::SetNodeKind {
            id,
            kind: NodeKind::Script {
                source: "let a = 1;\nlet b = ;".to_owned(),
                branches,
                on_error,
            },
            expected_version: version,
        })
        .unwrap();
    let story = session.project.story();
    let script_problems = orbit_project::scripts::check_scripts(story, &Default::default());
    assert!(
        script_problems
            .iter()
            .any(|p| p.is_error() && p.diagnostic.line == 2),
        "{script_problems:?}"
    );
    let extras = Extras {
        script_problems,
        ..Extras::default()
    };
    for texts in [&en, &fr] {
        let diag = compute(texts, story, 1, &extras);
        let found = diag
            .problems
            .iter()
            .find(|p| p.level == Level::Error && p.message.contains("2:"))
            .unwrap_or_else(|| panic!("a located problem: {:?}", diag.problems));
        assert_eq!(
            found.location,
            Some(orbit_editor_api::Location::Node(id)),
            "{}",
            found.message
        );
        assert_eq!(diag.of(id).len(), 1);
    }

    // An empty script is a validation error with a translated message.
    let version = session.project.node_version(id);
    session
        .project
        .execute(Command::SetNodeKind {
            id,
            kind: NodeKind::Script {
                source: String::new(),
                branches: Vec::new(),
                on_error: None,
            },
            expected_version: version,
        })
        .unwrap();
    let story = session.project.story();
    let diag = compute(&fr, story, 2, &Extras::default());
    assert!(
        diag.of(id).iter().any(|(_, m)| m.starts_with("Script :")),
        "{:?}",
        diag.of(id)
    );

    // A Rhai condition is a whole condition, shown as `rhai: ...`.
    let cond = Expr::Rhai("get_var(\"x\") > 1".to_owned());
    assert_eq!(cond.to_string(), "rhai: get_var(\"x\") > 1");
    assert_eq!(Expr::parse(&cond.to_string()).unwrap(), cond);
}

#[test]
fn modules_are_written_listed_and_checked() {
    let dir = TempDir::new("modules");
    let root = &dir.0;
    orbit_project::scripts::write_module(root, "shop", "fn price(x) { x * 2 }\n").unwrap();
    orbit_project::scripts::write_module(root, "broken", "fn (").unwrap();
    assert_eq!(
        orbit_project::scripts::list_modules(root),
        ["broken", "shop"]
    );
    let modules = orbit_project::scripts::read_modules(root);
    let story = orbit_core::Story::new("t");
    let problems = orbit_project::scripts::check_scripts(&story, &modules);
    assert!(
        problems.iter().any(|p| p.is_error()
            && p.site == orbit_project::scripts::ScriptSite::Module("broken".into())),
        "{problems:?}"
    );
    let en = translator(Lang::En);
    let message = orbit_editor::diagnostics::describe_script_problem(&en, &problems[0]);
    assert!(message.starts_with("scripts/broken.rhai:"), "{message}");
    orbit_project::scripts::delete_module(root, "broken").unwrap();
    assert_eq!(orbit_project::scripts::list_modules(root), ["shop"]);
}
