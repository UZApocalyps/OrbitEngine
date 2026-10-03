//! End to end: exports the demo for every target and checks what comes out.
//!
//! * `every_target_produces_the_expected_files` uses stand-in templates, so
//!   it runs everywhere: it checks the structure of the zip, the tar.gz and
//!   the web bundle, and that the pak inside is the game.
//! * `the_exported_native_player_runs_on_its_own` needs the real player
//!   (`cargo xtask templates`) and a GPU; it is ignored unless asked for:
//!   `cargo test -p orbit_export --test e2e -- --ignored`. It copies the
//!   exported player to a folder outside the repository and runs it for a few
//!   seconds: no access to the sources, nothing but the exported files.

use std::io::Read;
use std::path::{Path, PathBuf};

use orbit_export::testing::{TempDir, demo_project, repo_root};
use orbit_export::{
    ExportOptions, ExportProject, ExportTarget, Templates, build, export, export_many,
};
use orbit_pak::{GamePack, trailer};

fn fake_templates(dir: &Path) -> Templates {
    for (sub, file, bytes) in [
        (
            "windows",
            "orbit-player.exe",
            &b"MZ fake windows player"[..],
        ),
        ("linux", "orbit-player", &b"\x7fELF fake linux player"[..]),
        (
            "web",
            "orbit_player.js",
            &b"export default function init() {}"[..],
        ),
        ("web", "orbit_player_bg.wasm", &b"\0asm\x01\0\0\0 fake"[..]),
    ] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
        std::fs::write(dir.join(sub).join(file), bytes).unwrap();
    }
    Templates::new(vec![dir.to_owned()])
}

/// The pak glued to the end of `exe`, found through its trailer.
fn pak_of(exe: &[u8]) -> &[u8] {
    let tail = &exe[exe.len() - trailer::TRAILER_LEN..];
    let len = trailer::decode(tail).expect("the executable ends with a trailer") as usize;
    &exe[exe.len() - trailer::TRAILER_LEN - len..exe.len() - trailer::TRAILER_LEN]
}

fn check_pak(pak: &[u8], version: &str) {
    let (_, index) = orbit_pak::container::parse_prefix(pak).expect("the pak parses");
    assert_eq!(index.meta("name"), Some("Demo"));
    assert_eq!(index.meta("version"), Some(version));
    let pack = GamePack::from_bytes(&index.read_from(pak, "game.pack").unwrap()).unwrap();
    // Property assertions only: the demo is free to change (no exact counts or titles).
    assert!(!pack.story.title.is_empty());
    assert!(!pack.translations.is_empty());
    assert!(!pack.story.assets.assets.is_empty());
    for asset in &pack.story.assets.assets {
        // Every asset of the story is in the pak, intact (checksum checked).
        index.read_from(pak, &asset.path).expect(&asset.path);
    }
    assert!(index.contains("engine/assets/fonts/Inter-Regular.ttf"));
    // Unreferenced files of the demo folder are not shipped.
    let referenced: Vec<&str> = pack
        .story
        .assets
        .assets
        .iter()
        .map(|a| a.path.as_str())
        .collect();
    for entry in index.entries() {
        if entry.path.starts_with("assets/") {
            assert!(
                referenced.contains(&entry.path.as_str()),
                "{} is not referenced",
                entry.path
            );
        }
    }
}
#[test]
fn every_target_produces_the_expected_files() {
    let project_dir = TempDir::new("e2e_project");
    let templates_dir = TempDir::new("e2e_templates");
    let out = TempDir::new("e2e_out");
    let project = demo_project(project_dir.path());
    let profile = project.profile(None).unwrap();
    let options = ExportOptions {
        templates: fake_templates(templates_dir.path()),
        cancel: None,
        android: Default::default(),
    };

    let mut events = Vec::new();
    let reports = export_many(
        &project,
        &profile,
        &[ExportTarget::Windows, ExportTarget::Linux, ExportTarget::Web],
        out.path(),
        &options,
        &mut |e| events.push(e.clone()),
    )
    .expect("export of every target");
    assert_eq!(reports.len(), 3);
    assert!(!events.is_empty());
    // Progress only goes forward and ends at 100 %.
    let fractions: Vec<f32> = events
        .iter()
        .filter_map(|e| match e {
            orbit_export::Event::Step { fraction, .. } => Some(*fraction),
            _ => None,
        })
        .collect();
    assert!(
        fractions.windows(2).all(|w| w[1] >= w[0] - 1e-6),
        "{fractions:?}"
    );
    assert!((fractions.last().unwrap() - 1.0).abs() < 1e-6);

    // ---- Windows: zip
    let zip_path = out.path().join("demo-1.0.0-windows.zip");
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
    let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
    assert!(names.contains(&"Demo/Demo.exe".to_owned()), "{names:?}");
    assert!(
        names.contains(&"Demo/licenses/Inter-OFL.txt".to_owned()),
        "{names:?}"
    );
    let mut exe = Vec::new();
    zip.by_name("Demo/Demo.exe")
        .unwrap()
        .read_to_end(&mut exe)
        .unwrap();
    assert!(exe.starts_with(b"MZ fake windows player"));
    let windows_pak = pak_of(&exe).to_vec();
    check_pak(&windows_pak, "1.0.0");

    // ---- Linux: tar.gz, executable
    let tgz = std::fs::File::open(out.path().join("demo-1.0.0-linux.tar.gz")).unwrap();
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tgz));
    let mut seen = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        if path == "Demo/demo" {
            assert_eq!(
                entry.header().mode().unwrap() & 0o111,
                0o111,
                "not executable"
            );
            let mut exe = Vec::new();
            entry.read_to_end(&mut exe).unwrap();
            assert!(exe.starts_with(b"\x7fELF fake linux player"));
            assert_eq!(pak_of(&exe), windows_pak, "the same game in every target");
        }
        seen.push(path);
    }
    assert!(seen.contains(&"Demo/demo".to_owned()), "{seen:?}");
    assert!(
        seen.contains(&"Demo/licenses/Inter-OFL.txt".to_owned()),
        "{seen:?}"
    );

    // ---- Web: a folder
    let web = out.path().join("demo-1.0.0-web");
    for file in [
        "index.html",
        "orbit_player.js",
        "orbit_player_bg.wasm",
        "game.pak",
        "icon.png",
        "favicon.png",
        "orbit-logo.png",
        "HOSTING.txt",
        "orbit-bundle.json",
        "index.html.br",
        "index.html.gz",
        "orbit_player_bg.wasm.br",
        "orbit_player_bg.wasm.gz",
        "orbit_player.js.br",
        "orbit_player.js.gz",
    ] {
        assert!(
            web.join(file).is_file(),
            "{file} is missing from the web bundle"
        );
    }
    assert_eq!(std::fs::read(web.join("game.pak")).unwrap(), windows_pak);
    let html = std::fs::read_to_string(web.join("index.html")).unwrap();
    assert!(!html.contains("@@"), "a placeholder is left in the page");
    assert!(html.contains("<title>Demo</title>") && html.contains("game.pak?v="));
    // The precompressed siblings decompress to the original.
    let wasm = std::fs::read(web.join("orbit_player_bg.wasm")).unwrap();
    let mut back = Vec::new();
    brotli::BrotliDecompress(
        &mut &std::fs::read(web.join("orbit_player_bg.wasm.br")).unwrap()[..],
        &mut back,
    )
    .unwrap();
    assert_eq!(back, wasm);
    let mut back = Vec::new();
    flate2::read::GzDecoder::new(&std::fs::read(web.join("index.html.gz")).unwrap()[..])
        .read_to_end(&mut back)
        .unwrap();
    assert_eq!(back, html.as_bytes());
    for report in &reports {
        assert!(report.bytes > 0 && report.output.exists());
        assert!(
            report.warnings.iter().any(|w| w.contains("mask_wipe.png")),
            "{:?}",
            report.warnings
        );
    }

    // ---- A second export replaces the first one: no stale files.
    let mut renamed = profile.clone();
    renamed.web_precompress = false;
    export(
        &project,
        &renamed,
        ExportTarget::Web,
        out.path(),
        &options,
        &mut |_| {},
    )
    .unwrap();
    assert!(!web.join("orbit_player_bg.wasm.br").exists());
    assert!(web.join("index.html").is_file());

    // ---- `orbit build`: the pak alone.
    let built = build(
        &project,
        &profile,
        &out.path().join("pak"),
        &options,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(std::fs::read(&built.pak_path).unwrap(), windows_pak);
}

#[test]
fn a_broken_project_fails_before_writing_anything() {
    let project_dir = TempDir::new("e2e_broken");
    let templates_dir = TempDir::new("e2e_broken_templates");
    let out = TempDir::new("e2e_broken_out");
    demo_project(project_dir.path());
    let story = std::fs::read_to_string(project_dir.path().join("story.orbit")).unwrap();
    std::fs::write(
        project_dir.path().join("story.orbit"),
        story.replace(
            "\"01JBX0T0000000000000000004\": \"Neo-Kyoto, 2087. Rain hammers the megatower blocks while neon bleeds across every wet window.\",",
            "",
        ),
    )
    .unwrap();
    let project = ExportProject::open(project_dir.path()).unwrap();
    let profile = project.profile(None).unwrap();
    let options = ExportOptions {
        templates: fake_templates(templates_dir.path()),
        cancel: None,
        android: Default::default(),
    };
    let err = export(
        &project,
        &profile,
        ExportTarget::Linux,
        &out.path().join("o"),
        &options,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(err.to_string().contains("unknown string"), "{err}");
    assert!(
        !out.path().join("o").exists(),
        "nothing is written for an invalid story"
    );
}

/// Where `cargo xtask templates` puts the real players.
fn real_templates() -> Templates {
    Templates::new(vec![repo_root().join("target/templates")])
}

#[test]
#[ignore = "needs `cargo xtask templates`, a GPU and a display"]
fn the_exported_native_player_runs_on_its_own() {
    let host = ExportTarget::host().expect("a Windows or Linux machine");
    let templates = real_templates();
    if !templates.has(host) {
        panic!(
            "no {} template in target/templates: run `cargo xtask templates` first",
            host.id()
        );
    }
    let project_dir = TempDir::new("e2e_run_project");
    let project = demo_project(project_dir.path());
    let profile = project.profile(None).unwrap();
    // Everything of the run happens in a folder outside the repository.
    let clean = TempDir::new("e2e_run_clean");
    export(
        &project,
        &profile,
        host,
        clean.path(),
        &ExportOptions {
            templates,
            cancel: None,
            android: Default::default(),
        },
        &mut |_| {},
    )
    .unwrap();

    let game_dir = clean.path().join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    let exe: PathBuf = match host {
        ExportTarget::Windows => {
            let mut zip = zip::ZipArchive::new(
                std::fs::File::open(clean.path().join("demo-1.0.0-windows.zip")).unwrap(),
            )
            .unwrap();
            let path = game_dir.join("Demo.exe");
            let mut file = std::fs::File::create(&path).unwrap();
            std::io::copy(&mut zip.by_name("Demo/Demo.exe").unwrap(), &mut file).unwrap();
            path
        }
        _ => {
            let tgz = std::fs::File::open(clean.path().join("demo-1.0.0-linux.tar.gz")).unwrap();
            let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tgz));
            archive.unpack(&game_dir).unwrap();
            game_dir.join("Demo/demo")
        }
    };
    let data = clean.path().join("data");
    let mut child = std::process::Command::new(&exe)
        .args(["--new-game", "--data-dir"])
        .arg(&data)
        .current_dir(&game_dir)
        .env("RUST_LOG", "info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the player starts");
    let mut stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    std::thread::sleep(std::time::Duration::from_secs(8));
    let still_running = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    let log = reader.join().unwrap();
    assert!(still_running, "the player exited early:\n{log}");
    assert!(!log.contains("panicked"), "{log}");
    assert!(!log.contains("player failed"), "{log}");
    assert!(log.contains("game pack opened"), "{log}");
    assert!(log.contains("story `Demo` ready"), "{log}");
    assert!(
        !log.contains("asset problem"),
        "an asset of the pak could not be loaded:\n{log}"
    );
    // The game ran from the pak and kept its progress under its own id.
    assert!(
        data.join("saves").is_dir(),
        "no save was written under {}",
        data.display()
    );
}
