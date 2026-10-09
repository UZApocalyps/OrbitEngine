//! Asset step: image prompts from the bible, generation with a concurrency
//! limit, placeholders when a picture cannot be made.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::bible::Bible;
use crate::image_gen::{ImageModel, ImageRequest};
use crate::script::{background_path, character_color, sprite_path};
use crate::settings::ImageSettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKindAi {
    Background,
    Sprite,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetStatus {
    #[default]
    Pending,
    Generated,
    /// Generation failed (message): a placeholder was written instead.
    Placeholder(String),
}

/// One picture of the plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetJob {
    pub kind: AssetKindAi,
    /// Location id or character id.
    pub owner: String,
    /// Expression, for a sprite.
    pub expression: Option<String>,
    /// Manifest path (relative to the project).
    pub path: String,
    pub prompt: String,
    pub seed: u64,
    /// The character's own LoRA, if any (global LoRAs come from settings).
    #[serde(default)]
    pub loras: Vec<crate::sd::LoraPick>,
    pub status: AssetStatus,
}

/// Per-character image options, for consistency across expressions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterLook {
    /// A character LoRA (trained on that character).
    pub lora: Option<crate::sd::LoraPick>,
    /// A fixed seed instead of the derived one.
    pub seed: Option<u64>,
}

/// FNV-1a: stable across runs and platforms (unlike `DefaultHasher`).
pub fn stable_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn join_prompt(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| p.trim().trim_end_matches(','))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every picture the story needs: one background per location, one sprite
/// per (character, used expression). A character keeps one seed for all its
/// expressions, so its look stays consistent.
pub fn plan(
    b: &Bible,
    used: &BTreeMap<String, BTreeSet<String>>,
    s: &ImageSettings,
    art_style: &str,
    looks: &BTreeMap<String, CharacterLook>,
) -> Vec<AssetJob> {
    let style = if art_style.trim().is_empty() { s.style_prefix.as_str() } else { art_style };
    let mut jobs = Vec::new();
    for l in &b.locations {
        jobs.push(AssetJob {
            kind: AssetKindAi::Background,
            owner: l.id.clone(),
            expression: None,
            path: background_path(&l.id),
            prompt: join_prompt(&[
                style,
                &l.visual,
                "visual novel background, scenery, wide shot, no people, no characters",
            ]),
            seed: s.seed.wrapping_add(stable_hash(&l.id) % 1_000_000),
            loras: Vec::new(),
            status: AssetStatus::Pending,
        });
    }
    for c in &b.characters {
        let look = looks.get(&c.id).cloned().unwrap_or_default();
        let seed = look.seed.unwrap_or_else(|| s.seed.wrapping_add(stable_hash(&c.id) % 1_000_000));
        let loras: Vec<_> = look.lora.into_iter().filter(|l| !l.name.is_empty()).collect();
        let expressions = used.get(&c.id).cloned().unwrap_or_else(|| BTreeSet::from(["neutral".to_owned()]));
        for e in expressions {
            jobs.push(AssetJob {
                kind: AssetKindAi::Sprite,
                owner: c.id.clone(),
                expression: Some(e.clone()),
                path: sprite_path(&c.id, &e),
                prompt: join_prompt(&[
                    style,
                    "visual novel character sprite, solo, standing, cowboy shot, facing viewer",
                    &c.visual,
                    &format!("{e} expression"),
                    "plain white background, simple background",
                ]),
                seed,
                loras: loras.clone(),
                status: AssetStatus::Pending,
            });
        }
    }
    jobs
}

/// Keeps the status of jobs already done (same path and prompt) when the
/// plan is rebuilt: a resumed step only makes what is missing.
pub fn merge_plan(old: &[AssetJob], mut new: Vec<AssetJob>) -> Vec<AssetJob> {
    for job in &mut new {
        if let Some(o) = old
            .iter()
            .find(|o| o.path == job.path && o.prompt == job.prompt && o.seed == job.seed && o.loras == job.loras)
        {
            job.status = o.status.clone();
        }
    }
    new
}

/// A stand-in picture: a gradient for a background, a colored silhouette on
/// transparency for a sprite.
pub fn placeholder(job: &AssetJob, b: &Bible, s: &ImageSettings) -> RgbaImage {
    let h = stable_hash(&job.owner);
    match job.kind {
        AssetKindAi::Background => {
            let size = s.background_size;
            let (w, hgt) = (size.width.clamp(16, 4096), size.height.clamp(16, 4096));
            let base = [(h & 0xff) as u8 / 2 + 40, ((h >> 8) & 0xff) as u8 / 2 + 40, ((h >> 16) & 0xff) as u8 / 2 + 60];
            RgbaImage::from_fn(w, hgt, |_, y| {
                let t = y as f32 / hgt as f32;
                let f = |c: u8| (f32::from(c) * (0.6 + 0.4 * t)) as u8;
                Rgba([f(base[0]), f(base[1]), f(base[2]), 255])
            })
        }
        AssetKindAi::Sprite => {
            let size = s.sprite_size;
            let (w, hgt) = (size.width.clamp(16, 4096), size.height.clamp(16, 4096));
            let index = b.characters.iter().position(|c| c.id == job.owner).unwrap_or(0);
            let color = b
                .characters
                .get(index)
                .map(|c| character_color(&c.color, index))
                .unwrap_or(orbit_core::Color::rgb(200, 200, 200));
            let fill = Rgba([color.r, color.g, color.b, 230]);
            let (cx, w2) = (w as f32 / 2.0, w as f32);
            let head_r = w2 * 0.16;
            let head_cy = hgt as f32 * 0.18;
            RgbaImage::from_fn(w, hgt, |x, y| {
                let (xf, yf) = (x as f32, y as f32);
                let in_head = (xf - cx).powi(2) + (yf - head_cy).powi(2) <= head_r * head_r;
                let body_top = head_cy + head_r * 1.1;
                let in_body = yf >= body_top && {
                    let half = w2 * (0.18 + 0.14 * ((yf - body_top) / (hgt as f32 - body_top)).min(1.0));
                    (xf - cx).abs() <= half
                };
                if in_head || in_body { fill } else { Rgba([0, 0, 0, 0]) }
            })
        }
    }
}

/// Makes the plain background around a sprite transparent: flood fill from
/// the borders over pixels close to the corner color.
pub fn remove_plain_background(img: &mut RgbaImage, tolerance: u8) {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return;
    }
    let key = *img.get_pixel(0, 0);
    let close = |p: &Rgba<u8>| {
        (0..3).all(|i| p.0[i].abs_diff(key.0[i]) <= tolerance) && p.0[3] > 0
    };
    let mut stack: Vec<(u32, u32)> = Vec::new();
    for x in 0..w {
        stack.push((x, 0));
        stack.push((x, h - 1));
    }
    for y in 0..h {
        stack.push((0, y));
        stack.push((w - 1, y));
    }
    while let Some((x, y)) = stack.pop() {
        let p = *img.get_pixel(x, y);
        if !close(&p) {
            continue;
        }
        img.put_pixel(x, y, Rgba([p.0[0], p.0[1], p.0[2], 0]));
        if x > 0 { stack.push((x - 1, y)); }
        if y > 0 { stack.push((x, y - 1)); }
        if x + 1 < w { stack.push((x + 1, y)); }
        if y + 1 < h { stack.push((x, y + 1)); }
    }
}

fn write_png(root: &Path, rel: &str, img: &RgbaImage) -> Result<(), String> {
    let path = root.join(rel);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    img.save_with_format(&path, image::ImageFormat::Png).map_err(|e| e.to_string())
}

fn make_one(
    job: &AssetJob,
    model: Option<&dyn ImageModel>,
    s: &ImageSettings,
    cancelled: &dyn Fn() -> bool,
) -> Result<RgbaImage, String> {
    let Some(model) = model else {
        return Err("no image backend configured".to_owned());
    };
    let size = match job.kind {
        AssetKindAi::Background => s.background_size,
        AssetKindAi::Sprite => s.sprite_size,
    };
    let req = ImageRequest {
        prompt: job.prompt.clone(),
        negative: s.negative_prompt.clone(),
        width: size.width,
        height: size.height,
        seed: job.seed,
        loras: s.loras.iter().chain(&job.loras).filter(|l| !l.name.is_empty()).cloned().collect(),
    };
    let bytes = model.generate(&req, cancelled).map_err(|e| e.to_string())?;
    let mut img = image::load_from_memory(&bytes)
        .map_err(|e| format!("cannot decode the picture: {e}"))?
        .to_rgba8();
    if job.kind == AssetKindAi::Sprite && s.remove_sprite_background {
        remove_plain_background(&mut img, 24);
    }
    Ok(img)
}

/// Generates every pending job (at most `settings.concurrency` at once),
/// writing PNGs under `root`. A failed picture becomes a placeholder.
/// `on_done(index, jobs)` runs after each picture (progress, saving the
/// state). Returns `false` when cancelled.
pub fn generate_all(
    root: &Path,
    b: &Bible,
    jobs: &mut [AssetJob],
    model: Option<&dyn ImageModel>,
    s: &ImageSettings,
    cancelled: &(dyn Fn() -> bool + Sync),
    on_done: &(dyn Fn(usize, &[AssetJob]) + Sync),
) -> bool {
    let pending: Vec<usize> = jobs
        .iter()
        .enumerate()
        .filter(|(_, j)| j.status == AssetStatus::Pending || !root.join(&j.path).is_file())
        .map(|(i, _)| i)
        .collect();
    let shared = Mutex::new(jobs);
    let next = AtomicUsize::new(0);
    let workers = (s.concurrency.max(1) as usize).min(pending.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    if cancelled() {
                        return;
                    }
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&i) = pending.get(k) else { return };
                    let job = match shared.lock() {
                        Ok(j) => j[i].clone(),
                        Err(_) => return,
                    };
                    let status = match make_one(&job, model, s, cancelled) {
                        _ if cancelled() => return,
                        Ok(img) => match write_png(root, &job.path, &img) {
                            Ok(()) => AssetStatus::Generated,
                            Err(e) => AssetStatus::Placeholder(e),
                        },
                        Err(e) => {
                            let _ = write_png(root, &job.path, &placeholder(&job, b, s));
                            AssetStatus::Placeholder(e)
                        }
                    };
                    if let Ok(mut j) = shared.lock() {
                        j[i].status = status;
                        on_done(i, &j);
                    }
                }
            });
        }
    });
    !cancelled()
}
