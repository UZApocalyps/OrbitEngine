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

#[test]
#[ignore]
fn ron_shapes() {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct M {
        image: String,
        vertices: Vec<[f32; 2]>,
    }
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    enum A {
        Mesh(M),
    }
    if let Ok(src) = std::env::var("ORBIT_BENCH_PROJECT") {
        let text = std::fs::read_to_string(Path::new(&src).join("assets/rigs/ada.rig.ron")).unwrap();
        for (i, line) in text.lines().enumerate() {
            if line.len() < 5000 {
                continue;
            }
            let mut frag = line.trim().trim_end_matches(',');
            if let Some((_, rest)) = frag.split_once("\": ") {
                frag = rest;
            }
            let t = Instant::now();
            let r: Result<ron::Value, _> = ron::from_str(frag);
            eprintln!("line {} ({} chars): {:?} ok={}", i + 1, line.len(), t.elapsed(), r.is_ok());
            if i == 15 {
                let d = Instant::now();
                let r: Result<orbit_anim::rig::Attachment, _> = ron::from_str(frag);
                eprintln!("   typed: {:?} ok={}", d.elapsed(), r.is_ok());
                if let Some(at) = frag.find("weights:") {
                    let w = &frag[at + 9..frag.rfind(']').unwrap() + 1];
                    let d = Instant::now();
                    let r: Result<Vec<Vec<(String, f32)>>, _> = ron::from_str(w.trim());
                    eprintln!("   weights only ({}): {:?} ok={}", w.len(), d.elapsed(), r.is_ok());
                }
                for key in ["vertices: ", "uvs: ", "triangles: "] {
                    let s = frag.find(key).unwrap() + key.len();
                    let e = s + frag[s..].find(']').unwrap() + 1;
                    let d = Instant::now();
                    let r: Result<ron::Value, _> = ron::from_str(&frag[s..e]);
                    eprintln!("   {key}({}): {:?} ok={}", e - s, d.elapsed(), r.is_ok());
                }
            }
        }
    }
    for n in [5_000usize, 10_000, 20_000] {
        let w: Vec<&str> = (0..n).map(|_| "[(\"head\", 1.0)]").collect();
        let text = format!("[{}]", w.join(", "));
        let t = Instant::now();
        let _: Vec<Vec<(String, f32)>> = ron::from_str(&text).unwrap();
        eprintln!("{n}: weights {:?}", t.elapsed());
        let tr: Vec<String> = (0..n)
            .map(|i| format!("(t: {i}.0, value: (x: 0.2799, y: -0.3484), ease: Preset(Linear))"))
            .collect();
        let text = format!("BoneTranslate(bone: \"chest\", track: [{}])", tr.join(", "));
        let t = Instant::now();
        let _: orbit_anim::rig::RigTrack = ron::from_str(&text).unwrap();
        eprintln!("{n}: track {:?}", t.elapsed());
        let items: Vec<String> = (0..n).map(|i| format!("({i}.5, -{i}.25)")).collect();
        for (name, sep) in [("one line", ", "), ("lines", ",\n")] {
            let text = format!("[{}]", items.join(sep));
            let t = Instant::now();
            let v: Vec<[f32; 2]> = ron::from_str(&text).unwrap();
            eprintln!("{n} {name}: vec {:?} ({})", t.elapsed(), v.len());
            let text = format!("(image: \"a\", vertices: [{}])", items.join(sep));
            let t = Instant::now();
            let _: M = ron::from_str(&text).unwrap();
            eprintln!("{n} {name}: struct {:?}", t.elapsed());
            let text = format!("Mesh({text})");
            let t = Instant::now();
            let _: A = ron::from_str(&text).unwrap();
            eprintln!("{n} {name}: newtype variant {:?}", t.elapsed());
        }
    }
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
    let ctx = Context::default();
    let t = Instant::now();
    let mut core = EditorCore::new(CoreOptions::with_language(orbit_editor_api::Lang::En), &ctx);
    core.open_project(&p);
    eprintln!("open: {:?}", t.elapsed());
    core.state.requests.push(EditorRequest::ShowPanel("orbit.library.library".into()));
    run(&mut core, &ctx, "first frames", 30, Box::new(|_, _| {}), 0.0);
    run(&mut core, &ctx, "idle", 30, Box::new(|_, _| {}), 1.0);
    let one: Vec<PathBuf> = extra[..20].to_vec();
    run(&mut core, &ctx, "declare 20 one by one", 20,
        Box::new(move |c, i| c.import_dropped(vec![one[i].clone()])), 2.0);
    let rest: Vec<PathBuf> = extra[20..].to_vec();
    run(&mut core, &ctx, "declare 180 at once", 1,
        Box::new(move |c, _| c.import_dropped(rest.clone())), 3.0);
    run(&mut core, &ctx, "after declare", 200, Box::new(|_, _| {}), 4.0);
    let t = Instant::now();
    core.save();
    core.wait_for_background_work();
    eprintln!("save: {:?}", t.elapsed());
    eprintln!("assets: {}", core.story().unwrap().assets.assets.len());
}
