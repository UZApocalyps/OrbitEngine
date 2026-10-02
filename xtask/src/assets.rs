//! `cargo xtask gen-assets`: procedurally generates the sample assets.
//!
//! * `assets/logo.png` (M0)
//! * `assets/demo/*.png`: backgrounds, character sprites, dissolve mask
//! * `assets/demo/music_theme.wav` and `sfx_door.wav`: synthesized audio
//!   (no pure-Rust permissive Vorbis *encoder* exists, so WAV is used)
//!
//! Everything is deterministic: running the command twice gives identical
//! files.

use image::{Rgba, RgbaImage};

use crate::{Result, workspace_root};

// ---------------------------------------------------------------- logo (M0)

const WIDTH: u32 = 384;
const HEIGHT: u32 = 256; // 3:2, so aspect-ratio preservation is visible
const TILE: u32 = 32;
const RADIUS: f32 = 28.0;

/// Pixel of the logo: a colored checkerboard with a diagonal gradient, a thin
/// border and rounded, transparent corners.
fn pixel(x: u32, y: u32) -> Rgba<u8> {
    // Rounded corners (alpha mask).
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
    let (w, h) = (WIDTH as f32, HEIGHT as f32);
    let dx = (RADIUS - fx).max(fx - (w - RADIUS)).max(0.0);
    let dy = (RADIUS - fy).max(fy - (h - RADIUS)).max(0.0);
    let dist = (dx * dx + dy * dy).sqrt();
    let alpha = (RADIUS + 0.5 - dist).clamp(0.0, 1.0);

    let t = (x + y) as f32 / (WIDTH + HEIGHT) as f32;
    let dark = (x / TILE + y / TILE).is_multiple_of(2);
    let base: [f32; 3] = if dark {
        [40.0, 60.0, 120.0]
    } else {
        [235.0, 240.0, 250.0]
    };
    let accent = [250.0, 140.0, 60.0];
    let mix = if dark { t * 0.6 } else { t * 0.35 };
    let mut rgb = [0u8; 3];
    for i in 0..3 {
        rgb[i] = (base[i] * (1.0 - mix) + accent[i] * mix).round() as u8;
    }
    // Border.
    let edge = x.min(y).min(WIDTH - 1 - x).min(HEIGHT - 1 - y);
    if edge < 4 {
        rgb = [250, 140, 60];
    }
    Rgba([rgb[0], rgb[1], rgb[2], (alpha * 255.0).round() as u8])
}

// ------------------------------------------------------------ drawing kit

type Color = [f32; 4];

fn rgb(r: u8, g: u8, b: u8) -> Color {
    [r as f32, g as f32, b as f32, 255.0]
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
        lerp(a[3], b[3], t),
    ]
}

/// A software canvas: shapes are described by signed-distance functions
/// (negative inside) and composited with 1px antialiasing.
struct Canvas {
    img: RgbaImage,
}

impl Canvas {
    fn new(w: u32, h: u32, fill: [u8; 4]) -> Self {
        Self {
            img: RgbaImage::from_pixel(w, h, Rgba(fill)),
        }
    }

    fn blend(&mut self, x: u32, y: u32, color: Color, coverage: f32) {
        let a = (color[3] / 255.0) * coverage.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let dst = self.img.get_pixel_mut(x, y);
        let da = dst[3] as f32 / 255.0;
        let out_a = a + da * (1.0 - a);
        for i in 0..3 {
            let c = (color[i] * a + dst[i] as f32 * da * (1.0 - a)) / out_a.max(1e-6);
            dst[i] = c.round().clamp(0.0, 255.0) as u8;
        }
        dst[3] = (out_a * 255.0).round() as u8;
    }

    /// Paints `color` where `sdf < 0` inside the pixel box `[x0,x1) x [y0,y1)`.
    fn paint(&mut self, bbox: [f32; 4], color: Color, sdf: impl Fn(f32, f32) -> f32) {
        let (w, h) = (self.img.width() as f32, self.img.height() as f32);
        let x0 = (bbox[0] - 1.0).floor().clamp(0.0, w) as u32;
        let y0 = (bbox[1] - 1.0).floor().clamp(0.0, h) as u32;
        let x1 = (bbox[2] + 1.0).ceil().clamp(0.0, w) as u32;
        let y1 = (bbox[3] + 1.0).ceil().clamp(0.0, h) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                let d = sdf(x as f32 + 0.5, y as f32 + 0.5);
                let coverage = 0.5 - d;
                if coverage > 0.0 {
                    self.blend(x, y, color, coverage);
                }
            }
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        self.paint([cx - r, cy - r, cx + r, cy + r], color, |x, y| {
            ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r
        });
    }

    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, color: Color) {
        // Approximate distance: scale space so the ellipse becomes a circle.
        let k = rx.min(ry);
        self.paint([cx - rx, cy - ry, cx + rx, cy + ry], color, |x, y| {
            let (nx, ny) = ((x - cx) / rx, (y - cy) / ry);
            ((nx * nx + ny * ny).sqrt() - 1.0) * k
        });
    }

    fn rrect(&mut self, cx: f32, cy: f32, hw: f32, hh: f32, r: f32, color: Color) {
        self.paint([cx - hw, cy - hh, cx + hw, cy + hh], color, |x, y| {
            let (qx, qy) = ((x - cx).abs() - hw + r, (y - cy).abs() - hh + r);
            (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
        });
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Color) {
        self.rrect(
            (x0 + x1) * 0.5,
            (y0 + y1) * 0.5,
            (x1 - x0) * 0.5,
            (y1 - y0) * 0.5,
            0.0,
            color,
        );
    }

    /// Thick line segment with round caps.
    fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, color: Color) {
        let r = width * 0.5;
        self.paint(
            [
                a.0.min(b.0) - r,
                a.1.min(b.1) - r,
                a.0.max(b.0) + r,
                a.1.max(b.1) + r,
            ],
            color,
            |x, y| {
                let (pax, pay, bax, bay) = (x - a.0, y - a.1, b.0 - a.0, b.1 - a.1);
                let h =
                    ((pax * bax + pay * bay) / (bax * bax + bay * bay).max(1e-6)).clamp(0.0, 1.0);
                ((pax - bax * h).powi(2) + (pay - bay * h).powi(2)).sqrt() - r
            },
        );
    }

    /// Arc of a circle from angle `a0` to `a1` (radians, 0 = +x, y down).
    #[allow(clippy::too_many_arguments)]
    fn arc(&mut self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, width: f32, color: Color) {
        let steps = 24;
        let mut prev = (cx + r * a0.cos(), cy + r * a0.sin());
        for i in 1..=steps {
            let a = lerp(a0, a1, i as f32 / steps as f32);
            let p = (cx + r * a.cos(), cy + r * a.sin());
            self.line(prev, p, width, color);
            prev = p;
        }
    }

    /// Filled convex polygon (counter-clockwise on screen: y down means
    /// clockwise in math terms; any consistent winding works since both are
    /// tried) with optional rounding of the outline.
    fn polygon(&mut self, pts: &[(f32, f32)], color: Color) {
        let n = pts.len();
        let area: f32 = (0..n)
            .map(|i| pts[i].0 * pts[(i + 1) % n].1 - pts[(i + 1) % n].0 * pts[i].1)
            .sum();
        let sign = if area > 0.0 { 1.0 } else { -1.0 };
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
        self.paint([x0, y0, x1, y1], color, |x, y| {
            let mut d = f32::MIN;
            for i in 0..n {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                let (ex, ey) = (b.0 - a.0, b.1 - a.1);
                let len = (ex * ex + ey * ey).sqrt().max(1e-6);
                // Outward normal distance.
                let dist = sign * ((x - a.0) * ey - (y - a.1) * ex) / len;
                d = d.max(dist);
            }
            d
        });
    }

    fn vertical_gradient(&mut self, y0: u32, y1: u32, top: Color, bottom: Color) {
        let w = self.img.width();
        for y in y0..y1.min(self.img.height()) {
            let t = (y - y0) as f32 / (y1 - y0).max(1) as f32;
            let c = lerp_color(top, bottom, t);
            for x in 0..w {
                self.blend(x, y, c, 1.0);
            }
        }
    }

    fn save(self, name: &str, dir: &std::path::Path) -> Result {
        let path = dir.join(name);
        self.img.save(&path)?;
        println!("wrote {}", path.display());
        Ok(())
    }
}

// ------------------------------------------------------------ backgrounds

fn bg_room() -> Canvas {
    let mut c = Canvas::new(1920, 1080, [0, 0, 0, 255]);
    // Wall, floor.
    c.vertical_gradient(0, 760, rgb(236, 214, 178), rgb(205, 170, 130));
    c.vertical_gradient(760, 1080, rgb(139, 90, 58), rgb(92, 56, 36));
    // Wainscot line and baseboard.
    c.rect(0.0, 740.0, 1920.0, 760.0, rgb(120, 78, 50));
    // Floor boards.
    for i in 0..14 {
        let x = i as f32 * 150.0;
        c.line(
            (960.0 + (x - 960.0) * 0.55, 760.0),
            (x - 120.0, 1080.0),
            3.0,
            [60.0, 36.0, 22.0, 120.0],
        );
    }
    // Window with sky, frame and cross bars.
    c.rrect(430.0, 330.0, 190.0, 210.0, 8.0, rgb(98, 62, 40));
    c.vertical_gradient_rect(
        250.0,
        140.0,
        610.0,
        520.0,
        rgb(150, 205, 245),
        rgb(225, 240, 252),
    );
    c.circle(520.0, 230.0, 46.0, rgb(255, 245, 190));
    c.rect(425.0, 140.0, 435.0, 520.0, rgb(98, 62, 40));
    c.rect(250.0, 325.0, 610.0, 335.0, rgb(98, 62, 40));
    // Bookshelf.
    c.rect(1350.0, 160.0, 1790.0, 740.0, rgb(112, 70, 44));
    for row in 0..4 {
        let y = 200.0 + row as f32 * 140.0;
        c.rect(1370.0, y + 100.0, 1770.0, y + 114.0, rgb(80, 48, 30));
        for k in 0..9 {
            let x = 1385.0 + k as f32 * 42.0;
            let h = 70.0 + ((k * 37 + row * 53) % 28) as f32;
            let hue = (k * 5 + row * 3) % 6;
            let col = [
                rgb(180, 60, 60),
                rgb(60, 110, 170),
                rgb(80, 150, 90),
                rgb(210, 170, 60),
                rgb(130, 80, 160),
                rgb(60, 60, 70),
            ][hue as usize];
            c.rect(x, y + 100.0 - h, x + 32.0, y + 100.0, col);
        }
    }
    // Rug.
    c.ellipse(960.0, 930.0, 520.0, 90.0, rgb(150, 50, 60));
    c.ellipse(960.0, 930.0, 440.0, 68.0, rgb(200, 90, 80));
    c.ellipse(960.0, 930.0, 330.0, 44.0, rgb(150, 50, 60));
    c
}

fn bg_street() -> Canvas {
    let mut c = Canvas::new(1920, 1080, [0, 0, 0, 255]);
    // Dusk sky.
    c.vertical_gradient(0, 700, rgb(40, 36, 92), rgb(246, 150, 110));
    c.circle(1500.0, 560.0, 90.0, rgb(255, 214, 140));
    // Distant skyline.
    for i in 0..16 {
        let x = i as f32 * 130.0;
        let h = 160.0 + ((i * 97) % 5) as f32 * 50.0;
        c.rect(x, 720.0 - h, x + 124.0, 720.0, rgb(58, 50, 96));
    }
    // Near buildings with lit windows.
    let shades = [rgb(34, 30, 56), rgb(44, 36, 62), rgb(28, 26, 48)];
    for i in 0..5 {
        let x = i as f32 * 420.0 - 40.0;
        let h = 380.0 + ((i * 61) % 4) as f32 * 60.0;
        c.rect(x, 760.0 - h, x + 380.0, 760.0, shades[i % 3]);
        for wy in 0..(h as i32 / 80) {
            for wx in 0..4 {
                if (wx + wy * 3 + i as i32) % 3 != 0 {
                    let (px, py) = (
                        x + 40.0 + wx as f32 * 85.0,
                        760.0 - h + 30.0 + wy as f32 * 80.0,
                    );
                    c.rect(px, py, px + 44.0, py + 52.0, rgb(255, 214, 120));
                }
            }
        }
    }
    // Sidewalk and road.
    c.rect(0.0, 760.0, 1920.0, 830.0, rgb(120, 116, 128));
    c.vertical_gradient(830, 1080, rgb(52, 52, 62), rgb(30, 30, 38));
    for i in 0..8 {
        let x = i as f32 * 260.0 + 40.0;
        c.rect(x, 940.0, x + 140.0, 956.0, rgb(230, 220, 160));
    }
    // Street lamps.
    for i in 0..3 {
        let x = 300.0 + i as f32 * 650.0;
        c.rect(x - 8.0, 520.0, x + 8.0, 780.0, rgb(24, 24, 30));
        c.circle(x, 510.0, 34.0, rgb(255, 232, 160));
        c.circle(x, 510.0, 64.0, [255.0, 232.0, 160.0, 50.0]);
    }
    c
}

impl Canvas {
    fn vertical_gradient_rect(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        top: Color,
        bottom: Color,
    ) {
        for y in y0 as u32..y1 as u32 {
            let t = (y as f32 - y0) / (y1 - y0).max(1.0);
            let col = lerp_color(top, bottom, t);
            for x in x0 as u32..x1 as u32 {
                self.blend(x, y, col, 1.0);
            }
        }
    }
}

// ------------------------------------------------------------- characters

#[derive(Clone, Copy, PartialEq)]
enum Expression {
    Neutral,
    Happy,
    Angry,
}

struct Look {
    skin: Color,
    hair: Color,
    outfit: Color,
    outfit_dark: Color,
    long_hair: bool,
}

const OUTLINE: Color = [28.0, 22.0, 36.0, 255.0];

fn character(look: &Look, expression: Expression) -> Canvas {
    let mut c = Canvas::new(600, 900, [0, 0, 0, 0]);
    let cx = 300.0;
    let skin = if expression == Expression::Angry {
        lerp_color(look.skin, rgb(235, 110, 100), 0.35)
    } else {
        look.skin
    };

    // Back hair (long hair falls behind the shoulders).
    if look.long_hair {
        c.ellipse(cx, 330.0, 150.0, 210.0, OUTLINE);
        c.ellipse(cx, 330.0, 144.0, 204.0, look.hair);
    }
    // Torso: trapezoid + outline.
    let torso = [
        (cx - 110.0, 380.0),
        (cx + 110.0, 380.0),
        (cx + 270.0, 900.0),
        (cx - 270.0, 900.0),
    ];
    let outline_torso = [
        (cx - 116.0, 374.0),
        (cx + 116.0, 374.0),
        (cx + 278.0, 900.0),
        (cx - 278.0, 900.0),
    ];
    c.polygon(&outline_torso, OUTLINE);
    c.polygon(&torso, look.outfit);
    // Collar / neckline accent.
    c.polygon(
        &[(cx - 60.0, 380.0), (cx + 60.0, 380.0), (cx, 470.0)],
        look.outfit_dark,
    );
    // Arms.
    for side in [-1.0f32, 1.0] {
        c.line(
            (cx + side * 120.0, 410.0),
            (cx + side * 225.0, 700.0),
            74.0,
            OUTLINE,
        );
        c.line(
            (cx + side * 120.0, 410.0),
            (cx + side * 225.0, 700.0),
            64.0,
            look.outfit,
        );
        c.circle(cx + side * 228.0, 722.0, 36.0, OUTLINE);
        c.circle(cx + side * 228.0, 722.0, 30.0, skin);
    }
    // Neck and head.
    if !look.long_hair {
        c.circle(cx, 225.0, 124.0, OUTLINE);
        c.circle(cx, 225.0, 119.0, look.hair);
    }
    c.rect(cx - 36.0, 330.0, cx + 36.0, 400.0, skin);
    c.circle(cx, 235.0, 118.0, OUTLINE);
    c.circle(cx, 235.0, 112.0, skin);
    // Front hair.
    if look.long_hair {
        c.arc(
            cx,
            220.0,
            112.0,
            std::f32::consts::PI * 1.02,
            std::f32::consts::PI * 1.98,
            34.0,
            look.hair,
        );
        c.ellipse(cx - 70.0, 180.0, 62.0, 40.0, look.hair);
        c.ellipse(cx + 40.0, 170.0, 80.0, 44.0, look.hair);
    } else {
        c.ellipse(cx, 168.0, 112.0, 52.0, OUTLINE);
        c.ellipse(cx, 168.0, 106.0, 46.0, look.hair);
        c.ellipse(cx - 40.0, 205.0, 40.0, 26.0, look.hair);
    }
    // Face.
    let ink = rgb(40, 30, 46);
    let (ex, ey) = (52.0, 240.0);
    match expression {
        Expression::Neutral => {
            for s in [-1.0f32, 1.0] {
                c.ellipse(cx + s * ex, ey, 15.0, 20.0, rgb(255, 255, 255));
                c.ellipse(cx + s * ex, ey + 2.0, 9.0, 14.0, ink);
            }
            c.line((cx - 22.0, 308.0), (cx + 22.0, 308.0), 6.0, ink);
        }
        Expression::Happy => {
            for s in [-1.0f32, 1.0] {
                // Closed, smiling eyes (upward arcs).
                c.arc(
                    cx + s * ex,
                    ey + 8.0,
                    20.0,
                    std::f32::consts::PI * 1.1,
                    std::f32::consts::PI * 1.9,
                    7.0,
                    ink,
                );
                c.circle(cx + s * 78.0, 280.0, 20.0, [255.0, 130.0, 150.0, 120.0]);
            }
            c.arc(
                cx,
                288.0,
                34.0,
                std::f32::consts::PI * 0.12,
                std::f32::consts::PI * 0.88,
                7.0,
                ink,
            );
        }
        Expression::Angry => {
            for s in [-1.0f32, 1.0] {
                c.ellipse(cx + s * ex, ey + 6.0, 14.0, 14.0, rgb(255, 255, 255));
                c.ellipse(cx + s * ex, ey + 8.0, 9.0, 10.0, ink);
                // Brows slanted down toward the nose.
                c.line(
                    (cx + s * (ex + 34.0), ey - 34.0),
                    (cx + s * (ex - 22.0), ey - 14.0),
                    9.0,
                    ink,
                );
            }
            c.arc(
                cx,
                330.0,
                30.0,
                std::f32::consts::PI * 1.15,
                std::f32::consts::PI * 1.85,
                7.0,
                ink,
            );
            // Anger vein.
            c.line(
                (cx + 70.0, 150.0),
                (cx + 96.0, 170.0),
                6.0,
                rgb(200, 30, 40),
            );
            c.line(
                (cx + 96.0, 150.0),
                (cx + 70.0, 170.0),
                6.0,
                rgb(200, 30, 40),
            );
        }
    }
    c
}

fn alice() -> Look {
    Look {
        skin: rgb(250, 222, 198),
        hair: rgb(232, 120, 160),
        outfit: rgb(120, 90, 190),
        outfit_dark: rgb(250, 240, 250),
        long_hair: true,
    }
}

fn bob() -> Look {
    Look {
        skin: rgb(226, 182, 150),
        hair: rgb(52, 40, 34),
        outfit: rgb(48, 96, 160),
        outfit_dark: rgb(230, 230, 236),
        long_hair: false,
    }
}

// ------------------------------------------------------------------- mask

/// Grayscale dissolve mask: a left-to-right wipe broken up by diagonal bands.
fn mask_wipe() -> RgbaImage {
    let (w, h) = (512u32, 288u32);
    RgbaImage::from_fn(w, h, |x, y| {
        let (u, v) = (x as f32 / (w - 1) as f32, y as f32 / (h - 1) as f32);
        let band = 0.5 + 0.5 * (u * 18.0 + v * 7.0).sin();
        let value = (0.82 * u + 0.18 * band).clamp(0.0, 1.0);
        let g = (value * 255.0).round() as u8;
        Rgba([g, g, g, 255])
    })
}

// ------------------------------------------------------------------ audio

const SAMPLE_RATE: u32 = 22_050;

fn write_wav(path: &std::path::Path, samples: &[f32]) -> Result {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        writer.write_sample((s.clamp(-1.0, 1.0) * 32767.0).round() as i16)?;
    }
    writer.finalize()?;
    println!("wrote {} ({} samples)", path.display(), samples.len());
    Ok(())
}

fn midi_hz(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// 8 s loop at 120 bpm: Am - F - C - G, a soft pad plus a plucked arpeggio.
/// Every note decays to silence before its slot ends so the loop is seamless.
fn music_theme() -> Vec<f32> {
    let secs = 8.0;
    let n = (SAMPLE_RATE as f32 * secs) as usize;
    let mut out = vec![0.0f32; n];
    let beat = 0.5f32; // seconds
    let bar = beat * 4.0;
    // (root note, [chord tones]) per bar.
    let chords: [(f32, [f32; 3]); 4] = [
        (45.0, [57.0, 60.0, 64.0]), // A minor
        (41.0, [57.0, 60.0, 65.0]), // F major
        (48.0, [55.0, 60.0, 64.0]), // C major
        (43.0, [55.0, 59.0, 62.0]), // G major
    ];
    for (i, sample) in out.iter_mut().enumerate() {
        let t = i as f32 / SAMPLE_RATE as f32;
        let bar_idx = ((t / bar) as usize).min(3);
        let tb = t - bar_idx as f32 * bar; // time inside the bar
        let (root, tones) = chords[bar_idx];

        // Pad: bass + chord, with a slow attack/release inside the bar.
        let env = (tb / 0.25).min(1.0) * ((bar - tb) / 0.25).clamp(0.0, 1.0);
        let mut pad = 0.30 * (std::f32::consts::TAU * midi_hz(root) * t).sin();
        for tone in tones {
            let f = midi_hz(tone - 12.0);
            pad += 0.07 * (std::f32::consts::TAU * f * t).sin()
                + 0.03 * (std::f32::consts::TAU * f * 2.0 * t).sin();
        }
        *sample += pad * env * 0.55;

        // Arpeggio: eighth notes over the chord, upper octave.
        let eighth = beat * 0.5;
        let step = (tb / eighth) as usize;
        let ts = tb - step as f32 * eighth;
        let pattern = [0usize, 1, 2, 1, 0, 1, 2, 1];
        let note = tones[pattern[step % 8] % 3] + 12.0;
        let f = midi_hz(note);
        let decay = (-ts * 9.0).exp();
        let fade_in = (ts / 0.004).min(1.0);
        let pluck = (std::f32::consts::TAU * f * ts).sin()
            + 0.4 * (std::f32::consts::TAU * f * 2.0 * ts).sin() * (-ts * 14.0).exp();
        *sample += 0.20 * pluck * decay * fade_in;
    }
    // Fade the very first/last milliseconds so the loop point is click-free.
    let ramp = (SAMPLE_RATE as f32 * 0.01) as usize;
    for k in 0..ramp {
        let g = k as f32 / ramp as f32;
        out[k] *= g;
        let last = n - 1 - k;
        out[last] *= g;
    }
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    for v in &mut out {
        *v = *v / peak * 0.7;
    }
    out
}

/// Short door knock/slam: a low thump plus lowpassed noise, decaying fast.
fn sfx_door() -> Vec<f32> {
    let n = (SAMPLE_RATE as f32 * 0.45) as usize;
    let mut seed: u32 = 0x1234_5678;
    let mut lp = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
        lp += 0.12 * (noise - lp);
        let thump = (std::f32::consts::TAU * (95.0 - 40.0 * t) * t).sin() * (-t * 14.0).exp();
        let slap = lp * (-t * 22.0).exp() * 2.2;
        let attack = (t / 0.002).min(1.0);
        out.push((0.8 * thump + 0.6 * slap) * attack);
    }
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    for v in &mut out {
        *v = *v / peak * 0.8;
    }
    out
}

// ------------------------------------------------------------------- main

pub fn generate() -> Result {
    let assets = workspace_root().join("assets");
    std::fs::create_dir_all(&assets)?;

    let logo = RgbaImage::from_fn(WIDTH, HEIGHT, pixel);
    let path = assets.join("logo.png");
    logo.save(&path)?;
    println!("wrote {}", path.display());

    let demo = assets.join("demo");
    std::fs::create_dir_all(&demo)?;
    bg_room().save("bg_room.png", &demo)?;
    bg_street().save("bg_street.png", &demo)?;
    character(&alice(), Expression::Neutral).save("alice_neutral.png", &demo)?;
    character(&alice(), Expression::Happy).save("alice_happy.png", &demo)?;
    character(&bob(), Expression::Neutral).save("bob_neutral.png", &demo)?;
    character(&bob(), Expression::Angry).save("bob_angry.png", &demo)?;
    let mask = mask_wipe();
    let path = demo.join("mask_wipe.png");
    mask.save(&path)?;
    println!("wrote {}", path.display());
    write_wav(&demo.join("music_theme.wav"), &music_theme())?;
    write_wav(&demo.join("sfx_door.wav"), &sfx_door())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_loop_is_click_free_and_normalized() {
        let m = music_theme();
        assert_eq!(m.len(), (SAMPLE_RATE * 8) as usize);
        assert!(m[0].abs() < 1e-3 && m[m.len() - 1].abs() < 1e-3);
        let peak = m.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!((0.5..=0.75).contains(&peak), "peak {peak}");
    }

    #[test]
    fn characters_have_transparent_background_and_distinct_expressions() {
        let a = character(&alice(), Expression::Neutral);
        let b = character(&alice(), Expression::Happy);
        assert_eq!(a.img.get_pixel(2, 2)[3], 0);
        assert_ne!(a.img.as_raw(), b.img.as_raw());
        assert!(a.img.pixels().any(|p| p[3] == 255));
    }

    #[test]
    fn door_sfx_is_short_and_audible() {
        let s = sfx_door();
        assert!(s.len() < SAMPLE_RATE as usize);
        assert!(s.iter().any(|v| v.abs() > 0.3));
    }

    #[test]
    fn mask_is_monotone_on_average() {
        let m = mask_wipe();
        assert!(m.get_pixel(0, 100)[0] < 60 && m.get_pixel(511, 100)[0] > 190);
    }
}
