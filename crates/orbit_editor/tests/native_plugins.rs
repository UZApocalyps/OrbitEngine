//! Native plugins in the editor, end to end, with a real library: the SDK
//! example `examples/editor_plugins/hello` (a cdylib built against
//! `orbit_editor_sdk`).
//!
//! The library is probed by the real editor binary (`orbit-editor
//! --probe-plugin`), installed as a `.orbitplugin` package, loaded by
//! `EditorCore` at start-up, and driven through the editor: its window is
//! drawn by the host each frame, its menu action and an MCP tool call edit
//! the project as undo entries, and its "crash test" action panics inside the
//! library: the plugin is disabled with a notice, the editor goes on.
//!
//! The library must be built first (the outer cargo holds the target folder):
//!
//! ```text
//! cargo build -p orbit_hello_plugin
//! cargo test -p orbit_editor --test native_plugins -- --ignored
//! ```
//!
//! (`ORBIT_HELLO_PLUGIN=<path>` points to another build.)

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_core::NodeKind;
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor_api::native::Fingerprint;
use orbit_editor_api::{EditorRequest, Lang, Level};
use orbit_plugin_host::store::PluginStore;
use orbit_plugin_host::{EditorFacts, PluginManager, Problem, Prober, ProcessProber};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh folder under the target directory (never the system temp).
fn temp(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("native-plugins").join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hello_library() -> PathBuf {
    if let Some(path) = std::env::var_os("ORBIT_HELLO_PLUGIN") {
        return path.into();
    }
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let lib = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .unwrap()
        .join(profile)
        .join(format!(
            "{}orbit_hello_plugin{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ));
    assert!(
        lib.is_file(),
        "{} is missing: run `cargo build -p orbit_hello_plugin` first",
        lib.display()
    );
    lib
}

/// `.orbitplugin` with the example's manifest, the library and its texts.
fn package(lib: &Path, dir: &Path) -> PathBuf {
    let example = workspace().join("examples/editor_plugins/hello");
    let lib_name = lib.file_name().unwrap().to_string_lossy().into_owned();
    let path = dir.join("hello.orbitplugin");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    let manifest = std::fs::read_to_string(example.join("manifest.ron"))
        .unwrap()
        .replace("orbit_hello_plugin.dll", &lib_name);
    zip.start_file("manifest.ron", options).unwrap();
    zip.write_all(manifest.as_bytes()).unwrap();
    zip.start_file(lib_name.as_str(), options).unwrap();
    zip.write_all(&std::fs::read(lib).unwrap()).unwrap();
    for lang in ["en", "fr"] {
        zip.start_file(format!("locales/{lang}/hello.ron"), options).unwrap();
        zip.write_all(&std::fs::read(example.join(format!("locales/{lang}/hello.ron"))).unwrap())
            .unwrap();
    }
    zip.finish().unwrap();
    path
}

fn frame(core: &mut EditorCore, ctx: &Context, time: f64) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
        time: Some(time),
        ..RawInput::default()
    };
    let output = ctx.run_ui(input, |ui| core.ui(ui));
    output.drop_without_applying_deltas();
}

fn narration(core: &EditorCore) -> Vec<String> {
    let story = core.story().unwrap();
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| match &n.kind {
            NodeKind::Say { speaker: None, text_id, .. } => story.strings.entries.get(text_id).cloned(),
            _ => None,
        })
        .filter(|t| t.contains("Hello"))
        .collect()
}

fn editor_prober() -> ProcessProber {
    ProcessProber {
        program: PathBuf::from(env!("CARGO_BIN_EXE_orbit-editor")),
        args: vec!["--probe-plugin".into()],
        timeout: Duration::from_secs(60),
    }
}

#[test]
#[ignore = "needs `cargo build -p orbit_hello_plugin` first; run with --ignored"]
fn the_hello_plugin_runs_in_the_editor_and_a_crash_only_disables_it() {
    let lib = hello_library();
    let work = temp("hello");
    let config = work.join("config");

    // 1. The editor binary probes the library: an identical fingerprint.
    let info = editor_prober().probe(&lib).expect("probe");
    assert_eq!((info.id.as_str(), info.name.as_str()), ("acme.hello", "Hello"));
    let mismatches = info.fingerprint.mismatches(&Fingerprint::current());
    assert!(
        mismatches.is_empty(),
        "rebuild the plugin (cargo build -p orbit_hello_plugin): {mismatches:#?}"
    );

    // 2. Installed from a package (the probe is cached for this build).
    let mut manager = PluginManager::for_config_dir(&config, EditorFacts::current());
    let done = manager
        .install(&package(&lib, &work), &editor_prober(), &|_| false, &|_| false)
        .expect("install");
    assert_eq!(done.id, "acme.hello");

    // 3. The editor loads it at start-up.
    let ctx = Context::default();
    let options = CoreOptions {
        config_dir: Some(config.clone()),
        language: Some(Lang::Fr),
        native_plugins: true,
        ..CoreOptions::default()
    };
    let mut core = EditorCore::new(options, &ctx);
    {
        let plugins = core.plugins();
        let plugins = plugins.borrow();
        assert!(plugins.problems.is_empty(), "{:?}", plugins.problems);
        assert!(plugins.is_loaded("acme.hello"));
    }
    assert!(core.registry.find_action("acme.hello.add_note").is_some());
    assert!(
        core.registry.find_action("acme.hello.open").is_some_and(|a| a.def.menu.as_ref().is_some_and(|m| m.menu == "tools"))
    );
    assert_eq!(core.state.t("hello.add_note"), "Ajouter une note", "package texts merged");

    let texts = orbit_editor::i18n::translator(Lang::Fr);
    let root = orbit_editor::storage::create_project(&work, "Notes", &texts).unwrap();
    core.open_project(&root);

    // 4. Its menu action edits the project: one undo entry.
    core.run_action_now("acme.hello.add_note", &ctx);
    assert_eq!(narration(&core), ["Note du plugin Hello #1"]);
    assert_eq!(
        core.session.as_ref().unwrap().project.undo_label(),
        Some("Hello : ajouter une note")
    );

    // 5. A plugin asks for an MCP tool: run between frames, one undo entry.
    let scene = core.story().unwrap().scenes[0].name.clone();
    core.state.requests.push(EditorRequest::RunTool {
        ticket: 7,
        name: "add_dialogue".into(),
        args: format!("{{\"scene\": {scene:?}, \"text\": \"Hello through MCP\"}}"),
    });
    core.pump(&ctx);
    let result = core.state.tool_results.remove(&7).expect("tool result");
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(narration(&core), ["Note du plugin Hello #1", "Hello through MCP"]);

    // 6. Its window (drawn by the host each frame) and the plugin manager.
    core.run_action_now("acme.hello.open", &ctx);
    core.state
        .requests
        .push(EditorRequest::ShowPanel("orbit.plugins.manager".into()));
    for i in 0..4 {
        frame(&mut core, &ctx, f64::from(i) * 0.1);
    }
    assert!(core.plugins().borrow().failure("acme.hello").is_none());

    // 7. A panic inside the library: the plugin is disabled, with a notice.
    core.run_action_now("acme.hello.crash_test", &ctx);
    frame(&mut core, &ctx, 1.0);
    assert!(matches!(
        core.plugins().borrow().problems.get("acme.hello"),
        Some(Problem::Panicked(m)) if m == "hello: crash test"
    ));
    assert!(core
        .state
        .toasts
        .iter()
        .any(|t| t.level == Level::Error && t.message.contains("acme.hello")));
    let record = &PluginStore::load(&config.join("plugins")).plugins["acme.hello"];
    assert!(!record.enabled, "disabled for the next start");
    assert!(record.error.is_some());
    // Its actions do nothing any more; the editor keeps working.
    core.run_action_now("acme.hello.add_note", &ctx);
    assert_eq!(narration(&core).len(), 2);
    for i in 0..2 {
        frame(&mut core, &ctx, 2.0 + f64::from(i));
    }
    core.shutdown();

    // 8. The next start does not load it.
    let options = CoreOptions {
        config_dir: Some(config.clone()),
        native_plugins: true,
        ..CoreOptions::default()
    };
    let core = EditorCore::new(options, &ctx);
    assert!(!core.plugins().borrow().is_loaded("acme.hello"));
    assert!(core.registry.find_action("acme.hello.add_note").is_none());
}

#[test]
fn a_plugin_that_is_not_a_library_is_refused_and_the_editor_starts() {
    let work = temp("bogus");
    let config = work.join("config");
    let plugins = config.join("plugins");
    let dir = plugins.join("acme.bogus");
    std::fs::create_dir_all(&dir).unwrap();
    let lib = format!("acme_bogus.{}", orbit_plugin_host::binary::library_extension());
    std::fs::write(dir.join(&lib), b"not a library").unwrap();
    std::fs::write(
        dir.join("manifest.ron"),
        format!("(id: \"acme.bogus\", name: \"Bogus\", version: \"1.0.0\", library: \"{lib}\")"),
    )
    .unwrap();
    std::fs::write(plugins.join("plugins.ron"), "(plugins: {\"acme.bogus\": (enabled: true)})").unwrap();

    let ctx = Context::default();
    let options = CoreOptions {
        config_dir: Some(config),
        native_plugins: true,
        ..CoreOptions::default()
    };
    let core = EditorCore::new(options, &ctx);
    let plugins = core.plugins();
    let plugins = plugins.borrow();
    assert!(!plugins.is_loaded("acme.bogus"));
    assert!(matches!(plugins.problems.get("acme.bogus"), Some(Problem::Binary(_))), "{:?}", plugins.problems);
}
