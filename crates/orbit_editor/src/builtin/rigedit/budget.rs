//! Frame-time budgets of the rig editor (headless, no GPU): opening a rig
//! whose picture is 2048 px and drawing every rig panel for the first time
//! must not freeze the UI. Run with `--nocapture` to see the frame times.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use egui::{Pos2, Rect, Ui, vec2};
use orbit_editor_api::{EditorContext, EditorRegistry, EditorState};

use super::State;
use crate::rig_host::{RigLink, SharedLink};

/// Longest acceptable frame: under the hitch threshold (100 ms) in release,
/// with room for unoptimized code in debug.
pub(crate) fn budget() -> Duration {
    if cfg!(debug_assertions) { Duration::from_millis(150) } else { Duration::from_millis(50) }
}

type Panel = fn(&mut Ui, &mut EditorContext<'_>, &Rc<RefCell<State>>);

struct Harness {
    ctx: egui::Context,
    registry: EditorRegistry,
    state: EditorState,
    st: Rc<RefCell<State>>,
    time: f64,
}

impl Harness {
    fn new(link: SharedLink) -> Self {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        Self {
            ctx,
            registry: EditorRegistry::new(),
            state: EditorState::default(),
            st: Rc::new(RefCell::new(State::new(link))),
            time: 0.0,
        }
    }

    /// One frame of `panel`, timed.
    fn frame(&mut self, panel: Panel) -> Duration {
        self.time += 0.016;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(self.time),
            ..egui::RawInput::default()
        };
        let (registry, state, st) = (&self.registry, &mut self.state, &self.st);
        let t = Instant::now();
        self.ctx
            .run_ui(input, |ui| {
                let mut ctx = EditorContext::new(None, state, registry);
                panel(ui, &mut ctx, st);
            })
            .drop_without_applying_deltas();
        t.elapsed()
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// A 2048 x 2048 picture with an opaque disc (so that the rig gets a mesh)
/// and a rig made from it, saved next to it.
fn big_rig(dir: &std::path::Path) -> std::path::PathBuf {
    let n = 2048u32;
    let mut rgba = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - 1024.0, y as f32 - 1024.0);
            if dx * dx + dy * dy < 900.0 * 900.0 {
                let i = ((y * n + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[(x % 251) as u8, (y % 241) as u8, 120, 255]);
            }
        }
    }
    super::pixeltools::write_png(&dir.join("body.png"), n, n, &rgba).unwrap();
    let pic = orbit_project::rigmesh::workflow::Picture { path: "body.png".into(), width: n, height: n, rgba };
    let (mut doc, ..) = super::workflow::rig_from_picture(&pic).unwrap();
    let path = dir.join("hero.rig.ron");
    doc.path = Some(path.clone());
    doc.save().unwrap();
    path
}

#[test]
fn opening_a_rig_with_a_2048_px_picture_stays_within_the_frame_budget() {
    let dir = std::env::temp_dir().join(format!("orbit_rig_budget_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = big_rig(&dir);

    let link: SharedLink = Rc::new(RefCell::new(RigLink::default()));
    let mut h = Harness::new(link.clone());
    // Warm egui (fonts) on an empty editor: not what is measured.
    h.frame(super::panels::inspector);

    let open = {
        let t = Instant::now();
        let (registry, state) = (&h.registry, &mut h.state);
        let mut ctx = EditorContext::new(None, state, registry);
        h.st.borrow_mut().open_path(&path, &mut ctx);
        t.elapsed()
    };
    {
        let mut st = h.st.borrow_mut();
        st.selected_slot = st.doc.as_ref().unwrap().rig().slots.first().map(|s| s.id.clone());
    }
    let panels: [(&str, Panel); 5] = [
        ("viewport", super::viewport_panel),
        ("hierarchy", super::panels::hierarchy),
        ("inspector", super::panels::inspector),
        ("clips", super::clippanel::clips_panel),
        ("timeline", super::clippanel::timeline_panel),
    ];
    let mut report = vec![format!("open (read + parse + prepare): {:.1} ms", ms(open))];
    let mut worst = open;
    for (name, panel) in panels {
        let first = h.frame(panel);
        let second = h.frame(panel);
        report.push(format!("{name}: first frame {:.1} ms, next {:.1} ms", ms(first), ms(second)));
        worst = worst.max(first).max(second);
    }
    // What the host is asked to draw names the 2048 px picture.
    assert!(link.borrow().request.is_some(), "the viewport asks the host for a picture");

    // The rig host must not decode the picture on the UI thread: the old
    // path (read + decode), for comparison.
    let t = Instant::now();
    let bytes = std::fs::read(dir.join("body.png")).unwrap();
    let _ = orbit_assets::decode_image(&bytes).unwrap();
    report.push(format!("(for reference) one 2048 px PNG read + decode: {:.1} ms", ms(t.elapsed())));
    let img = orbit_assets::decode_image(&bytes).unwrap();
    let t = Instant::now();
    super::pixeltools::write_png(&dir.join("again.png"), img.width, img.height, &img.rgba).unwrap();
    report.push(format!("(for reference) one 2048 px PNG encode + write: {:.1} ms", ms(t.elapsed())));
    let t = Instant::now();
    h.st.borrow_mut().doc.as_mut().unwrap().save().unwrap();
    report.push(format!("(for reference) rig save (RON + write): {:.1} ms", ms(t.elapsed())));
    eprintln!("rig editor frames ({}):\n  {}", if cfg!(debug_assertions) { "debug" } else { "release" }, report.join("\n  "));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(worst <= budget(), "a rig editor frame took {:.1} ms (budget {:?}):\n{}", ms(worst), budget(), report.join("\n"));
}
