//! Profiler overlay of the player (TL-04): F3 shows the frame times (CPU,
//! GPU when available) and the most expensive spans. Development builds
//! only: when `orbit_profile` is compiled out (release players, the web),
//! [`ENABLED`] is `false` and the key does nothing.

use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId};
use orbit_text::{TextEngine, TextRenderer};
use orbit_ui::Banner;

/// Whether the overlay exists in this build.
pub const ENABLED: bool = orbit_profile::COMPILED;

/// Frames summarized.
const WINDOW: usize = 120;
/// Seconds between two refreshes of the text (laying it out every frame
/// would show up in the numbers).
const REFRESH: f32 = 0.25;
const LEFT: f32 = 24.0;
const TOP: f32 = 24.0;
const WIDTH: f32 = 760.0;
const FONT_SIZE: f32 = 22.0;

#[derive(Default)]
pub struct ProfilerOverlay {
    banner: Option<Banner>,
    since: f32,
}

/// The overlay text from the profiler history.
pub fn summary_text(frames: &[orbit_profile::FrameRecord]) -> String {
    // The first frame of the window may predate a pause: skipped.
    let frames = frames.get(1..).unwrap_or_default();
    if frames.is_empty() {
        return "Profiler: waiting for frames (F3 hides)".to_owned();
    }
    let n = frames.len() as f64;
    let cpu = frames.iter().map(|f| f.cpu_ms).sum::<f64>() / n;
    let worst = frames.iter().map(|f| f.cpu_ms).fold(0.0, f64::max);
    let mut text = format!("CPU {cpu:.2} ms (worst {worst:.2})");
    let gpu: Vec<f64> = frames.iter().filter_map(|f| f.gpu_ms()).collect();
    if gpu.is_empty() {
        text.push_str("   GPU n/a");
    } else {
        let mean = gpu.iter().sum::<f64>() / gpu.len() as f64;
        text.push_str(&format!("   GPU {mean:.2} ms"));
    }
    for s in orbit_profile::aggregate(frames).iter().take(6) {
        text.push_str(&format!(
            "\n{:<22} {:>7.3} ms  x{:.0}",
            s.name, s.mean_ms, s.calls
        ));
    }
    for (name, ms) in orbit_profile::aggregate_gpu(frames) {
        text.push_str(&format!("\nGPU {name:<18} {ms:>7.3} ms"));
    }
    text
}

impl ProfilerOverlay {
    pub fn update(&mut self, dt: f32) {
        self.since += dt;
        if self.banner.is_none() || self.since >= REFRESH {
            self.since = 0.0;
            let text = summary_text(&orbit_profile::frames(WINDOW));
            self.banner = Some(Banner::left(
                &text,
                FONT_SIZE,
                [235, 240, 245, 255],
                LEFT + 12.0,
                WIDTH - 24.0,
            ));
        }
    }

    /// A scene with the panel, drawn over everything else.
    pub fn scene(
        &mut self,
        engine: &mut TextEngine,
        text_renderer: &mut TextRenderer,
        renderer: &Renderer2d,
        white: TextureId,
    ) -> Option<Scene> {
        let banner = self.banner.as_mut()?;
        let mut scene = Scene::new();
        let height = banner.height().unwrap_or(FONT_SIZE * 2.0) + 24.0;
        scene.push(
            Sprite2d::new(white, [LEFT, TOP], [WIDTH, height])
                .anchor([0.0, 0.0])
                .layer(Layer(250))
                .z(-1.0)
                .tint([0.0, 0.0, 0.0, 0.72]),
        );
        let mut text = Scene::new();
        banner.draw(engine, text_renderer, renderer, &mut text, TOP + 12.0, 1.0);
        for s in text.sprites() {
            scene.push(s.clone().layer(Layer(251)));
        }
        Some(scene)
    }
}

#[cfg(test)]
mod tests {
    use orbit_profile::{FrameRecord, ScopeRecord};

    use super::*;

    #[test]
    fn the_summary_lists_frame_times_and_spans() {
        let frame = |cpu: f64, gpu: Vec<(&'static str, f64)>| FrameRecord {
            index: 0,
            cpu_ms: cpu,
            scopes: vec![ScopeRecord {
                name: "runtime draw",
                depth: 0,
                start_ms: 0.0,
                duration_ms: cpu / 2.0,
            }],
            gpu,
        };
        assert!(summary_text(&[]).contains("waiting"));
        let text = summary_text(&[
            frame(100.0, vec![]),
            frame(4.0, vec![("2d scene", 1.0)]),
            frame(6.0, vec![]),
        ]);
        assert!(text.starts_with("CPU 5.00 ms (worst 6.00)"), "{text}");
        assert!(text.contains("GPU 1.00 ms"), "{text}");
        assert!(text.contains("runtime draw"), "{text}");
        assert!(text.contains("GPU 2d scene"), "{text}");
        assert!(!text.contains('{'));
    }
}
