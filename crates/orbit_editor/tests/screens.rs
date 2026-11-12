//! Story-side UI screen nodes (`OpenScreen` / `CloseScreen`, ADR 0016):
//! palette drafts, editing through the command API with undo, localized
//! diagnostics.

use std::path::PathBuf;

use egui::Context;
use orbit_core::{NodeKind, Operand, ScreenParam, Value};
use orbit_editor::builtin::drafts::drafts_for;
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor::i18n::translator;
use orbit_editor_api::Lang;
use orbit_project::Command;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-editor-screens-{tag}-{}-{:?}",
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
fn open_screen_node_is_created_edited_with_params_and_undone() {
    let ctx = Context::default();
    let dir = TempDir::new("open");
    let texts = translator(Lang::En);
    let root = orbit_editor::storage::create_project(&dir.0, "Screens", &texts).unwrap();
    let mut core = EditorCore::new(CoreOptions::default(), &ctx);
    core.open_project(&root);
    let session = core.session.as_mut().unwrap();
    let scene = session.project.story().scenes[0].id;
    let nodes = session.project.story().scenes[0].nodes.len();

    // Both nodes give drafts (they are in the palette).
    let draft = |key: &str| {
        drafts_for(session.project.story(), &core.registry, &texts, key, scene, 0)
            .unwrap()
            .remove(0)
    };
    let open = draft("OpenScreen");
    let close = draft("CloseScreen");
    assert!(matches!(close.node.kind, NodeKind::CloseScreen { .. }));
    let id = open.node.id;
    session
        .project
        .execute(Command::InsertNode {
            scene,
            index: 0,
            node: open.node,
        })
        .unwrap();

    // An empty screen id is a localized problem on that node.
    let report = orbit_core::validate(session.project.story());
    let error = report
        .errors
        .iter()
        .find(|e| matches!(e, orbit_core::ValidationError::InvalidScreenNode { .. }))
        .expect("an empty screen id is invalid");
    let (at, text) =
        orbit_editor::diagnostics::describe_error(&texts, session.project.story(), error);
    assert_eq!(at, Some(id));
    assert!(text.starts_with("Screen node:"), "{text}");
    let fr = translator(Lang::Fr);
    let (_, text_fr) =
        orbit_editor::diagnostics::describe_error(&fr, session.project.story(), error);
    assert!(text_fr.starts_with("Nœud d'écran"), "{text_fr}");

    // Edit it through the command the inspector uses.
    let version = session.project.node_version(id);
    session
        .project
        .execute(Command::SetNodeKind {
            id,
            kind: NodeKind::OpenScreen {
                screen: "inventory".into(),
                wait: true,
                result: None,
                params: vec![ScreenParam {
                    name: "title".into(),
                    value: Operand::Lit(Value::Str("Bag".into())),
                }],
            },
            expected_version: version,
        })
        .unwrap();
    let node = session.project.story().node(id).unwrap();
    assert_eq!(
        orbit_editor::builtin::summary(&texts, session.project.story(), node),
        "inventory (1)"
    );
    assert!(
        orbit_core::validate(session.project.story())
            .errors
            .iter()
            .all(|e| !matches!(e, orbit_core::ValidationError::InvalidScreenNode { .. }))
    );

    session.project.undo().unwrap();
    assert!(matches!(
        &session.project.story().node(id).unwrap().kind,
        NodeKind::OpenScreen { screen, params, .. } if screen.is_empty() && params.is_empty()
    ));
    session.project.undo().unwrap();
    assert_eq!(session.project.story().scenes[0].nodes.len(), nodes);
}
