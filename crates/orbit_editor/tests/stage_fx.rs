//! Parallax layers and particle nodes in the editor (M5A-07 / M5A-08):
//! palette drafts, editing through commands with undo, localized
//! diagnostics, list summaries.

use std::path::PathBuf;

use egui::Context;
use orbit_core::{Emitter, NodeKind, ParticlePreset};
use orbit_editor::builtin::drafts::drafts_for;
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor::i18n::translator;
use orbit_editor_api::Lang;
use orbit_project::Command;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-editor-stagefx-{tag}-{}-{:?}",
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
fn particle_nodes_are_drafted_edited_checked_and_undone() {
    let ctx = Context::default();
    let dir = TempDir::new("particles");
    let texts = translator(Lang::En);
    let root = orbit_editor::storage::create_project(&dir.0, "Stage", &texts).unwrap();
    let mut core = EditorCore::new(CoreOptions::default(), &ctx);
    core.open_project(&root);
    let session = core.session.as_mut().unwrap();
    let scene = session.project.story().scenes[0].id;
    let nodes = session.project.story().scenes[0].nodes.len();

    let draft = |key: &str| {
        drafts_for(session.project.story(), &core.registry, &texts, key, scene, 0)
            .map(|mut d| d.remove(0))
    };
    let particles = draft("Particles").expect("the palette offers particles");
    let stop = draft("StopParticles").expect("and a way to stop them");
    assert!(matches!(&particles.node.kind, NodeKind::Particles(e) if e.preset == ParticlePreset::Rain));
    assert!(matches!(&stop.node.kind, NodeKind::StopParticles { name } if name.is_empty()));
    // Parallax needs a picture in the project: asked for, not invented.
    if session.project.story().assets.assets.iter().all(|a| a.kind != orbit_core::AssetKind::Image) {
        assert!(draft("BackgroundLayers").is_err());
    }

    let id = particles.node.id;
    session
        .project
        .execute(Command::InsertNode { scene, index: 0, node: particles.node })
        .unwrap();
    // A broken emitter is a localized problem on its node.
    let mut broken = Emitter::preset(ParticlePreset::Snow);
    broken.rate = 1.0e6;
    let version = session.project.node_version(id);
    session
        .project
        .execute(Command::SetNodeKind { id, kind: NodeKind::Particles(broken), expected_version: version })
        .unwrap();
    let report = orbit_core::validate(session.project.story());
    let error = report
        .errors
        .iter()
        .find(|e| matches!(e, orbit_core::ValidationError::InvalidStageNode { .. }))
        .expect("too many particles is an error");
    let (at, text) = orbit_editor::diagnostics::describe_error(&texts, session.project.story(), error);
    assert_eq!(at, Some(id));
    assert!(text.starts_with("Background layers or particles:"), "{text}");
    let fr = translator(Lang::Fr);
    let (_, text_fr) = orbit_editor::diagnostics::describe_error(&fr, session.project.story(), error);
    assert!(text_fr.starts_with("Fond en parallaxe ou particules"), "{text_fr}");

    // Fixed again, the summary names the emitter and its preset.
    let version = session.project.node_version(id);
    session
        .project
        .execute(Command::SetNodeKind {
            id,
            kind: NodeKind::Particles(Emitter::preset(ParticlePreset::Petals)),
            expected_version: version,
        })
        .unwrap();
    assert!(
        orbit_core::validate(session.project.story())
            .errors
            .iter()
            .all(|e| !matches!(e, orbit_core::ValidationError::InvalidStageNode { .. }))
    );
    let node = session.project.story().node(id).unwrap();
    assert_eq!(
        orbit_editor::builtin::summary(&texts, session.project.story(), node),
        "petals · petals"
    );

    session.project.undo().unwrap();
    session.project.undo().unwrap();
    session.project.undo().unwrap();
    assert_eq!(session.project.story().scenes[0].nodes.len(), nodes);
}
