//! End to end with a real library: builds the SDK example
//! (`examples/editor_plugins/hello`, a cdylib) with cargo, probes it in a
//! child process, installs it as a `.orbitplugin` package, loads it into an
//! editor registry and drives it: its window and settings page are drawn with
//! this process's egui context, its menu action edits a project as one undo
//! entry, and its "crash test" action panics inside the library and only
//! disables the plugin.
//!
//! Slow the first time (a separate target folder under this one is built:
//! the outer cargo holds the main one): `cargo test -p orbit_plugin_host
//! --test hello_dll -- --ignored`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use orbit_editor_api::egui;
use orbit_editor_api::native::Fingerprint;
use orbit_editor_api::{EditorContext, EditorRegistry, EditorState, Lang, Translator};
use orbit_plugin_host::{EditorFacts, PluginManager, Prober, ProcessProber};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Builds the example with the toolchain and profile of this test.
fn build_hello() -> PathBuf {
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("hello-build");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(workspace())
        .args(["build", "-p", "orbit_hello_plugin", "--target-dir"])
        .arg(&target)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "building the hello plugin failed");
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let lib = target.join(profile).join(format!(
        "{}orbit_hello_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(lib.is_file(), "{} missing", lib.display());
    lib
}

/// `.orbitplugin` with the manifest, the library and the texts.
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

fn fonts(ctx: &egui::Context) {
    let bytes = std::fs::read(workspace().join("assets/fonts/ui/InterUI-Bold.ttf")).unwrap();
    let mut defs = egui::FontDefinitions::empty();
    defs.font_data.insert("inter".into(), Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        defs.families.insert(family, vec!["inter".into()]);
    }
    defs.families.insert(egui::FontFamily::Name("bold".into()), vec!["inter".into()]);
    ctx.set_fonts(defs);
}

fn say_texts(project: &orbit_project::Project) -> Vec<String> {
    let story = project.story();
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| match &n.kind {
            orbit_core::NodeKind::Say { text_id, .. } => story.strings.entries.get(text_id).cloned(),
            _ => None,
        })
        .collect()
}

#[test]
#[ignore = "builds a cdylib (slow the first time); run with --ignored"]
fn the_hello_plugin_loads_draws_edits_and_survives_a_panic() {
    let lib = build_hello();
    let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join("hello-run");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();

    // 1. Out-of-process probe: the descriptor and an identical fingerprint.
    let prober = ProcessProber {
        program: PathBuf::from(env!("CARGO_BIN_EXE_orbit-plugin-probe")),
        args: Vec::new(),
        timeout: Duration::from_secs(60),
    };
    let info = prober.probe(&lib).expect("probe");
    assert_eq!(info.id, "acme.hello");
    assert_eq!(info.name, "Hello");
    let mismatches = info.fingerprint.mismatches(&Fingerprint::current());
    assert!(mismatches.is_empty(), "fingerprint differs: {mismatches:#?}");

    // 2. Install the package, load it like the editor does at start-up.
    let mut manager = PluginManager::new(work.join("plugins"), EditorFacts::current());
    let installed = manager
        .install(&package(&lib, &work), &prober, &|_| false, &|_| false)
        .expect("install");
    assert_eq!(installed.id, "acme.hello");
    let mut registry = EditorRegistry::new();
    let mut report = manager.load_enabled(&prober, &mut registry, Lang::En, Some(&work));
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    assert_eq!(report.loaded.len(), 1);
    let guard = report.loaded[0].guard.clone();
    assert!(registry.find_action("acme.hello.add_note").is_some());
    assert!(registry.find_action("acme.hello.open").is_some());
    assert_eq!(registry.windows().len(), 1);
    assert!(registry.settings_for("acme.hello").is_some());
    assert!(work.join("plugins/acme.hello/data").is_dir());

    let translator = Translator::new(Lang::Fr);
    for (lang, table) in registry.text_tables() {
        translator.add_table(*lang, table.clone());
    }
    assert_eq!(translator.get("hello.add_note"), "Ajouter une note", "package locales loaded");

    // 3. The menu action edits the project as one undo entry.
    let mut project = orbit_project::Project::new(orbit_core::Story::new("Test"));
    orbit_project::ops::add_scene(&mut project, "Main").unwrap();
    let mut state = EditorState { i18n: translator, ..EditorState::default() };
    {
        let mut ctx = EditorContext::new(Some(&mut project), &mut state, &registry);
        let add = registry.find_action("acme.hello.add_note").unwrap();
        assert!((add.def.enabled)(&ctx));
        (add.def.run)(&mut ctx);
        (add.def.run)(&mut ctx);
    }
    assert_eq!(
        say_texts(&project),
        ["Note du plugin Hello #1", "Note du plugin Hello #2"]
    );
    assert_eq!(project.undo_label(), Some("Hello : ajouter une note"));
    project.undo().unwrap();
    assert_eq!(say_texts(&project), ["Note du plugin Hello #1"]);

    // 4. Its window and settings page draw with our egui context.
    let egui_ctx = egui::Context::default();
    fonts(&egui_ctx);
    {
        let mut ctx = EditorContext::new(Some(&mut project), &mut state, &registry);
        (registry.find_action("acme.hello.open").unwrap().def.run)(&mut ctx);
        for i in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                time: Some(f64::from(i) * 0.1),
                ..egui::RawInput::default()
            };
            let output = egui_ctx.run_ui(input, |ui| {
                let c = ui.ctx().clone();
                for w in registry.windows() {
                    (w.def.ui)(&c, &mut ctx);
                }
                (registry.settings_for("acme.hello").unwrap().ui)(ui, &mut ctx);
            });
            output.drop_without_applying_deltas();
        }
    }
    assert!(!guard.is_failed(), "{:?}", guard.failure());

    // 5. A panic inside the library disables the plugin, not the process.
    {
        let mut ctx = EditorContext::new(Some(&mut project), &mut state, &registry);
        let crash = registry.find_action("acme.hello.crash_test").unwrap();
        (crash.def.run)(&mut ctx);
        assert!(guard.is_failed());
        assert_eq!(guard.failure().as_deref(), Some("hello: crash test"));
        // Disabled from now on: no action runs, the window is not drawn.
        let add = registry.find_action("acme.hello.add_note").unwrap();
        assert!(!(add.def.enabled)(&ctx));
        (add.def.run)(&mut ctx);
    }
    assert_eq!(say_texts(&project).len(), 1);

    for mut p in report.loaded.drain(..) {
        p.plugin.on_unload();
        std::mem::forget(p.plugin);
    }
}
