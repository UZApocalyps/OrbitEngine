//! Asset library: declaring many undeclared files keeps the UI responsive.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor_api::EditorRequest;
use orbit_project::MANIFEST_FILE;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("orbit-lib-{tag}-{}", std::process::id()));
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

fn project(root: &Path, n: usize, side: u32) -> (PathBuf, Vec<PathBuf>) {
    let project = root.join("p");
    std::fs::create_dir_all(project.join("assets/images")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stories/demo.ron"),
        project.join("story.ron"),
    )
    .unwrap();
    std::fs::write(
        project.join(MANIFEST_FILE),
        "(format: 1, name: \"Demo\", story: \"story.ron\", plugins: [])",
    )
    .unwrap();
    let mut files = Vec::new();
    for i in 0..n {
        let img = image::RgbaImage::from_fn(side, side, |x, y| {
            image::Rgba([(x + i as u32) as u8, y as u8, (i * 7) as u8, 255])
        });
        let path = project.join(format!("assets/images/img_{i:03}.png"));
        img.save(&path).unwrap();
        files.push(path);
    }
    (project, files)
}

fn frame(core: &mut EditorCore, ctx: &Context, time: f64) -> Duration {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1360.0, 820.0))),
        time: Some(time),
        ..RawInput::default()
    };
    let t = Instant::now();
    let output = ctx.run_ui(input, |ui| core.ui(ui));
    output.drop_without_applying_deltas();
    t.elapsed()
}

fn open(dir: &TempDir, n: usize, side: u32) -> (EditorCore, Context, Vec<PathBuf>) {
    let ctx = Context::default();
    let (p, files) = project(&dir.0, n, side);
    let mut core = EditorCore::new(CoreOptions::with_language(orbit_editor_api::Lang::En), &ctx);
    core.open_project(&p);
    core.state.requests.push(EditorRequest::ShowPanel("orbit.library.library".into()));
    for i in 0..30 {
        frame(&mut core, &ctx, f64::from(i) * 0.016);
    }
    (core, ctx, files)
}

/// Declaring 200 undeclared PNGs: the decoding runs on the task pool, no
/// frame stalls, the whole declaration is one undo step.
#[test]
fn declaring_200_files_is_one_step_and_keeps_frames_short() {
    let dir = TempDir::new("declare200");
    let (mut core, ctx, _files) = open(&dir, 200, 48);
    let before = core.story().unwrap().assets.assets.len();
    let start = Instant::now();
    core.state
        .requests
        .push(EditorRequest::RunAction("orbit.library.declare_all".into()));
    let mut worst = Duration::ZERO;
    let mut frames = 0;
    while core.story().unwrap().assets.assets.len() == before {
        worst = worst.max(frame(&mut core, &ctx, 3.0 + f64::from(frames) * 0.016));
        frames += 1;
        assert!(start.elapsed() < Duration::from_secs(60), "the declaration never landed");
        std::thread::sleep(Duration::from_millis(2));
    }
    let total = start.elapsed();
    for i in 0..5 {
        worst = worst.max(frame(&mut core, &ctx, 10.0 + f64::from(i) * 0.016));
    }
    let after = core.story().unwrap().assets.assets.len();
    eprintln!("declare 200: total {total:?} in {frames} frames, worst frame {worst:?}");
    assert_eq!(after, before + 200);
    // Frames stay short: 16 ms in release, a looser bound for debug builds.
    let budget = if cfg!(debug_assertions) { 250 } else { 16 };
    assert!(worst < Duration::from_millis(budget), "a frame took {worst:?}");
    // One undo removes the whole declaration.
    core.run_action_now("orbit.core.undo", &ctx);
    assert_eq!(core.story().unwrap().assets.assets.len(), before);
}

/// The one-pass usage index (bulk removal) agrees with `asset_users`.
#[test]
fn the_asset_users_index_matches_asset_users() {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stories/demo.ron"),
    )
    .unwrap();
    let story = orbit_core::Story::from_ron(&text).unwrap();
    let index = orbit_project::asset_users_index(&story);
    assert!(!index.is_empty());
    for a in &story.assets.assets {
        let one = orbit_project::asset_users(&story, a.id);
        assert_eq!(index.get(&a.id).cloned().unwrap_or_default(), one, "{}", a.path);
    }
}

#[test]
#[ignore]
fn baseline_declare_one_by_one() {
    for side in [32u32, 512] {
        let dir = TempDir::new(&format!("base{side}"));
        let (mut core, ctx, files) = open(&dir, 200, side);
        let start = Instant::now();
        let mut worst = Duration::ZERO;
        orbit_profile::set_enabled(true);
        orbit_profile::clear();
        for (i, f) in files.iter().enumerate() {
            orbit_profile::begin_frame();
            core.import_dropped(vec![f.clone()]);
            worst = worst.max(frame(&mut core, &ctx, 1.0 + i as f64 * 0.016));
            orbit_profile::finish_frame();
        }
        eprintln!("side {side}: one by one: total {:?}, worst frame {:?}", start.elapsed(), worst);
        let mut stats = orbit_profile::aggregate(&orbit_profile::frames(200));
        stats.sort_by(|a, b| b.mean_ms.total_cmp(&a.mean_ms));
        for s in stats.iter().take(14) {
            eprintln!("   {:<40} mean {:7.3} ms  max {:7.3} ms  calls {:.1}", s.name, s.mean_ms, s.max_ms, s.calls);
        }
        drop(core);
        let dir = TempDir::new(&format!("batch{side}"));
        let (mut core, ctx, files) = open(&dir, 200, side);
        let start = Instant::now();
        core.import_dropped(files.clone());
        let mut worst = frame(&mut core, &ctx, 2.0);
        let first = start.elapsed();
        for i in 0..5 {
            worst = worst.max(frame(&mut core, &ctx, 2.1 + f64::from(i) * 0.016));
        }
        eprintln!("side {side}: batch: first {first:?}, worst frame {worst:?}");
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

fn profile_top(label: &str, n: usize) {
    let mut stats = orbit_profile::aggregate(&orbit_profile::frames(n));
    stats.sort_by(|a, b| b.max_ms.total_cmp(&a.max_ms));
    eprintln!("  [{label}] top spans:");
    for s in stats.iter().take(12) {
        eprintln!(
            "   {:<40} mean {:8.3} ms  max {:8.3} ms  calls {:.1}",
            s.name, s.mean_ms, s.max_ms, s.calls
        );
    }
}

#[test]
#[ignore]
fn real_project_lint_parts() {
    let Ok(src) = std::env::var("ORBIT_BENCH_PROJECT") else {
        return;
    };
    let root = Path::new(&src);
    let t = Instant::now();
    let ok = orbit_ui::UiBundle::load_dir(&root.join("ui")).is_ok();
    eprintln!("ui bundle: {:?} ok={ok}", t.elapsed());
    for e in std::fs::read_dir(root.join("assets/rigs")).unwrap().flatten() {
        let rel = format!("assets/rigs/{}", e.file_name().to_string_lossy());
        let t = Instant::now();
        let text = std::fs::read_to_string(e.path()).unwrap();
        let read = t.elapsed();
        let t = Instant::now();
        let _ = orbit_anim::rig::rig_text_version(&text);
        let probe = t.elapsed();
        let t = Instant::now();
        let _: ron::Value = ron::from_str(&text).unwrap();
        let value = t.elapsed();
        eprintln!("{rel}: probe {probe:?} value {value:?}");
        let t = Instant::now();
        let rig = orbit_anim::rig::load_rig(&text).unwrap().rig;
        let parse = t.elapsed();
        let t = Instant::now();
        let _ = rig.validate();
        let validate = t.elapsed();
        let t = Instant::now();
        let _ = rig.rest_bounds(None, false);
        let bounds = t.elapsed();
        let t = Instant::now();
        let _ = orbit_anim::rig::PreparedRig::new(rig.clone());
        let prepared = t.elapsed();
        eprintln!("{rel}: read {read:?} parse {parse:?} validate {validate:?} bounds {bounds:?} prepared {prepared:?}");
    }
}

/// Working set and peak working set of this process (Windows; empty
/// elsewhere). Read through PowerShell: the workspace forbids `unsafe`.
fn memory() -> String {
    let pid = std::process::id();
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("$p = Get-Process -Id {pid}; '{{0:N0}} MB (peak {{1:N0}} MB)' -f ($p.WorkingSet64/1MB), ($p.PeakWorkingSet64/1MB)"),
        ])
        .output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// The editor preview on a headless GPU: time to the first picture after
/// Play, the slowest host update, and the typing frames while it plays.
fn preview_start(core: &mut EditorCore, ctx: &Context) {
    use orbit_editor::preview_host::PreviewHost;
    let Ok(mut gfx) = pollster::block_on(orbit_gfx::Gfx::headless((64, 64))) else {
        eprintln!("preview: no GPU adapter, skipped");
        return;
    };
    gfx.enable_gamma_view();
    let mut preview = PreviewHost::new(gfx).unwrap();
    let mut er = egui_wgpu::Renderer::new(
        preview.gfx().device(),
        egui_wgpu::wgpu::TextureFormat::Rgba8Unorm,
        egui_wgpu::RendererOptions::default(),
    );
    core.state.requests.push(EditorRequest::ShowPanel("orbit.preview.preview".into()));
    let t = Instant::now();
    let mut worst = Duration::ZERO;
    let mut first_update = None;
    let mut n = 0u32;
    let mut played = false;
    let start_frames = preview.frames;
    while t.elapsed() < Duration::from_secs(60) {
        frame(core, ctx, 30.0 + f64::from(n) * 0.016);
        let u = Instant::now();
        preview.update(core, &mut er, Instant::now());
        let took = u.elapsed();
        first_update.get_or_insert(took);
        worst = worst.max(took);
        n += 1;
        if !played {
            core.state.preview.commands.push(orbit_editor_api::PreviewCommand::Play);
            played = true;
        } else if preview.runtime().status() == orbit_runtime::PlayStatus::Playing
            && preview.frames > start_frames + 2
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    eprintln!(
        "preview start: playing after {:?} ({n} frames), first host update {first_update:?}, worst host update {worst:?}",
        t.elapsed()
    );
    let mut worst = Duration::ZERO;
    let t = Instant::now();
    for i in 0..60 {
        let f = Instant::now();
        drop(core.edit_nth_say(1, &"hot ".repeat(i + 1)));
        frame(core, ctx, 40.0 + i as f64 * 0.016);
        preview.update(core, &mut er, Instant::now());
        worst = worst.max(f.elapsed());
        std::thread::sleep(Duration::from_millis(4));
    }
    // Past the debounce: the hot reload frame.
    std::thread::sleep(Duration::from_millis(400));
    let f = Instant::now();
    frame(core, ctx, 42.0);
    preview.update(core, &mut er, Instant::now());
    eprintln!(
        "typing while playing: worst frame {worst:?} (total {:?}), hot reload frame {:?}",
        t.elapsed(),
        f.elapsed()
    );
}

type Before = Box<dyn FnMut(&mut EditorCore, usize)>;

fn run(core: &mut EditorCore, ctx: &Context, label: &str, frames: usize, mut before: Before, t0: f64) {
    orbit_profile::clear();
    let t = Instant::now();
    let mut worst = Duration::ZERO;
    for i in 0..frames {
        orbit_profile::begin_frame();
        let f = Instant::now();
        before(core, i);
        frame(core, ctx, t0 + i as f64 * 0.016);
        worst = worst.max(f.elapsed());
        orbit_profile::finish_frame();
    }
    eprintln!("{label}: total {:?}, worst frame {worst:?}", t.elapsed());
    profile_top(label, frames.min(200));
}

/// `ORBIT_BENCH_PROJECT=<folder> cargo test --test library_perf real -- --ignored --nocapture`:
/// copies the project, adds 200 undeclared copies of its images, and times
/// open, idle frames, declaring one by one (20), declaring the rest at once, save.
#[test]
#[ignore]
fn real_project_bench() {
    let Ok(src) = std::env::var("ORBIT_BENCH_PROJECT") else {
        return;
    };
    let dir = TempDir::new("real");
    let p = dir.0.join("p");
    copy_dir(Path::new(&src), &p);
    let images: Vec<PathBuf> = std::fs::read_dir(p.join("assets/images"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|f| f.extension().is_some_and(|e| e == "png"))
        .collect();
    std::fs::create_dir_all(p.join("assets/images/bench")).unwrap();
    let mut extra = Vec::new();
    for i in 0..200 {
        let to = p.join(format!("assets/images/bench/copy_{i:03}.png"));
        std::fs::copy(&images[i % images.len()], &to).unwrap();
        extra.push(to);
    }
    orbit_profile::set_enabled(true);
    eprintln!("memory before open: {}", memory());
    let ctx = Context::default();
    let mut core = EditorCore::new(CoreOptions::with_language(orbit_editor_api::Lang::En), &ctx);
    // Open as the interface does (background job), drawing frames meanwhile:
    // "interactive" is the first frame under 16 ms once the project is in.
    let t = Instant::now();
    core.open_project_in_background(&p);
    let (mut installed, mut interactive) = (None, None);
    let mut worst = Duration::ZERO;
    let mut slow = 0;
    let mut n = 0u32;
    while interactive.is_none() && t.elapsed() < Duration::from_secs(120) {
        let f = frame(&mut core, &ctx, f64::from(n) * 0.016);
        n += 1;
        worst = worst.max(f);
        slow += usize::from(f > Duration::from_millis(16));
        if installed.is_none() && core.has_project() {
            installed = Some(t.elapsed());
        } else if installed.is_some() && f <= Duration::from_millis(16) {
            interactive = Some(t.elapsed());
        }
        if core.dialog.is_some() {
            core.dialog = None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    eprintln!(
        "open (background): installed {installed:?}, interactive {interactive:?}, {n} frames, {slow} over 16 ms, worst {worst:?}"
    );
    eprintln!("memory after open: {}", memory());
    run(&mut core, &ctx, "first frames", 30, Box::new(|_, _| {}), 0.0);
    run(&mut core, &ctx, "idle", 30, Box::new(|_, _| {}), 1.0);
    for panel in ["orbit.graph.graph", "orbit.translations.translations", "orbit.search.search", "orbit.problems.problems"] {
        core.state.requests.push(EditorRequest::ShowPanel(panel.into()));
        run(&mut core, &ctx, &format!("show {panel}"), 30, Box::new(|_, _| {}), 1.5);
    }
    // Typing in a line: one edit per frame.
    run(&mut core, &ctx, "typing (60 edits)", 60,
        Box::new(|c, i| drop(c.edit_nth_say(0, &"typed ".repeat(i + 1)))), 1.8);
    preview_start(&mut core, &ctx);
    eprintln!("memory after preview: {}", memory());
    core.state.requests.push(EditorRequest::ShowPanel("orbit.library.library".into()));
    run(&mut core, &ctx, "show library", 30, Box::new(|_, _| {}), 1.9);
    let one: Vec<PathBuf> = extra[..20].to_vec();
    run(&mut core, &ctx, "declare 20 one by one", 20,
        Box::new(move |c, i| c.import_dropped(vec![one[i].clone()])), 2.0);
    let rest: Vec<PathBuf> = extra[20..].to_vec();
    run(&mut core, &ctx, "declare 180 at once", 1,
        Box::new(move |c, _| c.import_dropped(rest.clone())), 3.0);
    run(&mut core, &ctx, "after declare", 200, Box::new(|_, _| {}), 4.0);
    // The library's "Declare all" (task pool) on 200 more copies.
    for i in 0..200 {
        let to = p.join(format!("assets/images/bench/more_{i:03}.png"));
        std::fs::copy(&images[i % images.len()], &to).unwrap();
    }
    run(&mut core, &ctx, "rescan", 5, Box::new(|c, _| c.state.asset_epoch += 1), 8.0);
    let before = core.story().unwrap().assets.assets.len();
    orbit_profile::clear();
    let t = Instant::now();
    core.state
        .requests
        .push(EditorRequest::RunAction("orbit.library.declare_all".into()));
    let mut worst = Duration::ZERO;
    let mut n = 0;
    while core.story().unwrap().assets.assets.len() == before && t.elapsed() < Duration::from_secs(120) {
        orbit_profile::begin_frame();
        worst = worst.max(frame(&mut core, &ctx, 9.0 + f64::from(n) * 0.016));
        orbit_profile::finish_frame();
        n += 1;
        std::thread::sleep(Duration::from_millis(5));
    }
    eprintln!(
        "declare all (200 large PNGs, task pool): total {:?}, {n} frames, worst frame {worst:?}, declared {}",
        t.elapsed(),
        core.story().unwrap().assets.assets.len() - before
    );
    profile_top("declare all", n.min(200) as usize);
    run(&mut core, &ctx, "after declare all", 60, Box::new(|_, _| {}), 20.0);
    let t = Instant::now();
    core.save();
    core.wait_for_background_work();
    eprintln!("save: {:?}", t.elapsed());
    eprintln!("assets: {}", core.story().unwrap().assets.assets.len());
}
