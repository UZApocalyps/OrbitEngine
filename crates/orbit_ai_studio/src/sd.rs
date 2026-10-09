//! Stable Diffusion specifics: model families (presets), LoRA / VAE / clip
//! skip injection for ComfyUI workflows and A1111 prompts, and listing the
//! models a backend has (checkpoints, LoRAs, VAEs, embeddings, samplers).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json, json};

use crate::http::Request;
use crate::settings::{ImageBackend, ImageSettings, Size};
use crate::text::{ProviderError, join_url, send_json};

/// A LoRA applied to a picture.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoraPick {
    /// Name as the backend knows it (A1111: the LoRA name; ComfyUI: the file
    /// name under `models/loras`, e.g. `myStyle.safetensors`).
    pub name: String,
    pub weight: f32,
    /// Added to the prompt when the LoRA is used (Civitai "trained words").
    pub trigger_words: Vec<String>,
}

/// Model families: sizes and quality tags that suit them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SdPreset {
    #[default]
    Custom,
    Sd15,
    Sdxl,
    Pony,
    Illustrious,
}

pub struct PresetValues {
    pub background: Size,
    pub sprite: Size,
    pub quality: &'static str,
    pub negative: &'static str,
    pub clip_skip: u32,
}

impl SdPreset {
    pub const ALL: [SdPreset; 5] = [SdPreset::Custom, SdPreset::Sd15, SdPreset::Sdxl, SdPreset::Pony, SdPreset::Illustrious];

    pub fn name(self) -> &'static str {
        match self {
            SdPreset::Custom => "Custom",
            SdPreset::Sd15 => "SD 1.5",
            SdPreset::Sdxl => "SDXL",
            SdPreset::Pony => "Pony",
            SdPreset::Illustrious => "Illustrious / NoobAI",
        }
    }

    pub fn values(self) -> Option<PresetValues> {
        let xl_bg = Size { width: 1344, height: 768 };
        let xl_sprite = Size { width: 832, height: 1216 };
        Some(match self {
            SdPreset::Custom => return None,
            SdPreset::Sd15 => PresetValues {
                background: Size { width: 768, height: 512 },
                sprite: Size { width: 512, height: 768 },
                quality: "masterpiece, best quality",
                negative: "worst quality, low quality, lowres, bad anatomy, bad hands, text, watermark",
                clip_skip: 2,
            },
            SdPreset::Sdxl => PresetValues {
                background: xl_bg,
                sprite: xl_sprite,
                quality: "high quality, highly detailed",
                negative: "lowres, bad anatomy, bad hands, text, watermark, blurry",
                clip_skip: 1,
            },
            SdPreset::Pony => PresetValues {
                background: xl_bg,
                sprite: xl_sprite,
                quality: "score_9, score_8_up, score_7_up",
                negative: "score_4, score_5, score_6, lowres, bad anatomy, text, watermark",
                clip_skip: 2,
            },
            SdPreset::Illustrious => PresetValues {
                background: xl_bg,
                sprite: xl_sprite,
                quality: "masterpiece, best quality, very aesthetic, absurdres",
                negative: "worst quality, low quality, lowres, bad anatomy, bad hands, text, watermark",
                clip_skip: 2,
            },
        })
    }
}

/// Applies a preset to the settings (sizes, quality tags, negative, clip
/// skip); the user can still change each value after.
pub fn apply_preset(s: &mut ImageSettings, preset: SdPreset) {
    s.preset = preset;
    if let Some(v) = preset.values() {
        s.background_size = v.background;
        s.sprite_size = v.sprite;
        s.quality_tags = v.quality.to_owned();
        s.negative_prompt = v.negative.to_owned();
        s.clip_skip = v.clip_skip;
    }
}

/// The prompt text of a picture: quality tags, the prompt, LoRA trigger
/// words, and for A1111 the `<lora:name:weight>` tags.
pub fn full_prompt(s: &ImageSettings, prompt: &str, loras: &[LoraPick], a1111_tags: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !s.quality_tags.trim().is_empty() {
        parts.push(s.quality_tags.trim().to_owned());
    }
    parts.push(prompt.trim().to_owned());
    for l in loras {
        for w in &l.trigger_words {
            if !w.trim().is_empty() && !prompt.contains(w.trim()) {
                parts.push(w.trim().to_owned());
            }
        }
    }
    let mut out = parts.join(", ");
    if a1111_tags {
        for l in loras.iter().filter(|l| !l.name.is_empty()) {
            let name = l.name.trim_end_matches(".safetensors").trim_end_matches(".pt");
            out.push_str(&format!(" <lora:{name}:{}>", l.weight));
        }
    }
    out
}

/// A1111 sampler names → ComfyUI `(sampler, scheduler)`.
pub fn comfy_sampler(name: &str) -> (String, Option<&'static str>) {
    let n = name.trim();
    let karras = n.ends_with(" Karras");
    let base = n.trim_end_matches(" Karras");
    let mapped = match base {
        "Euler a" => "euler_ancestral",
        "Euler" => "euler",
        "DPM++ 2M" => "dpmpp_2m",
        "DPM++ 2M SDE" => "dpmpp_2m_sde",
        "DPM++ SDE" => "dpmpp_sde",
        "DPM++ 2S a" => "dpmpp_2s_ancestral",
        "DPM2" => "dpm_2",
        "Heun" => "heun",
        "LMS" => "lms",
        "DDIM" => "ddim",
        "UniPC" => "uni_pc",
        other => return (other.to_owned(), None),
    };
    (mapped.to_owned(), karras.then_some("karras"))
}

fn link_is(v: &Json, node: &str, slot: u64) -> bool {
    v.as_array().is_some_and(|a| a.len() == 2 && a[0] == node && a[1] == slot)
}

/// Rewires every input pointing at `(from, slot)` to `to`, except in the
/// nodes listed in `skip`.
fn rewire(wf: &mut Map<String, Json>, from: &str, slot: u64, to: Json, skip: &[String]) {
    for (id, node) in wf.iter_mut() {
        if skip.contains(id) {
            continue;
        }
        if let Some(inputs) = node.get_mut("inputs").and_then(Json::as_object_mut) {
            for v in inputs.values_mut() {
                if link_is(v, from, slot) {
                    *v = to.clone();
                }
            }
        }
    }
}

/// Adds LoRA loaders, a VAE loader and a clip skip to a ComfyUI workflow
/// (API format), after its first `CheckpointLoaderSimple`.
pub fn inject_comfy(wf: &mut Json, loras: &[LoraPick], vae: &str, clip_skip: u32) {
    let Some(map) = wf.as_object_mut() else { return };
    let Some(ckpt) = map
        .iter()
        .find(|(_, n)| n["class_type"] == "CheckpointLoaderSimple")
        .map(|(k, _)| k.clone())
    else {
        return;
    };
    let (mut model, mut clip) = (json!([ckpt, 0]), json!([ckpt, 1]));
    let mut added: Vec<String> = Vec::new();
    for (i, l) in loras.iter().filter(|l| !l.name.is_empty()).enumerate() {
        let id = format!("orbit_lora_{i}");
        map.insert(
            id.clone(),
            json!({"class_type": "LoraLoader", "inputs": {
                "lora_name": l.name, "strength_model": l.weight, "strength_clip": l.weight,
                "model": model, "clip": clip}}),
        );
        model = json!([id, 0]);
        clip = json!([id, 1]);
        added.push(id);
    }
    if clip_skip > 1 {
        let id = "orbit_clip_skip".to_owned();
        map.insert(
            id.clone(),
            json!({"class_type": "CLIPSetLastLayer", "inputs": {"stop_at_clip_layer": -(clip_skip as i64), "clip": clip}}),
        );
        clip = json!([id, 0]);
        added.push(id);
    }
    rewire(map, &ckpt, 0, model, &added);
    rewire(map, &ckpt, 1, clip, &added);
    if !vae.trim().is_empty() {
        let id = "orbit_vae".to_owned();
        map.insert(id.clone(), json!({"class_type": "VAELoader", "inputs": {"vae_name": vae}}));
        rewire(map, &ckpt, 2, json!([id, 0]), std::slice::from_ref(&id));
    }
}

/// What a backend offers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelLists {
    pub checkpoints: Vec<String>,
    pub loras: Vec<String>,
    pub vaes: Vec<String>,
    pub embeddings: Vec<String>,
    pub samplers: Vec<String>,
    pub schedulers: Vec<String>,
}

fn names(v: &Json, key: &str) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x[key].as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

fn get(url: &str) -> Result<Json, ProviderError> {
    send_json(&Request::get(url, Duration::from_secs(30)))
}

/// Comfy `object_info/<class>` → the choices of one input.
fn comfy_choices(base: &str, class: &str, input: &str) -> Vec<String> {
    get(&join_url(base, &format!("object_info/{class}")))
        .ok()
        .and_then(|j| {
            j[class]["input"]["required"][input][0]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
        })
        .unwrap_or_default()
}

/// Lists the models of the configured backend. Missing endpoints give empty
/// lists; only an unreachable server is an error.
pub fn list_models(s: &ImageSettings) -> Result<ModelLists, ProviderError> {
    let base = s.base_url.as_str();
    match s.backend {
        ImageBackend::Automatic1111 => {
            let checkpoints = get(&join_url(base, "sdapi/v1/sd-models"))?;
            let opt = |p: &str| get(&join_url(base, p)).unwrap_or(Json::Null);
            let embeddings = opt("sdapi/v1/embeddings");
            Ok(ModelLists {
                checkpoints: names(&checkpoints, "title"),
                loras: names(&opt("sdapi/v1/loras"), "name"),
                vaes: names(&opt("sdapi/v1/sd-vae"), "model_name"),
                embeddings: embeddings["loaded"]
                    .as_object()
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default(),
                samplers: names(&opt("sdapi/v1/samplers"), "name"),
                schedulers: names(&opt("sdapi/v1/schedulers"), "name"),
            })
        }
        ImageBackend::ComfyUi => {
            // Reachability first, for a clear error.
            get(&join_url(base, "system_stats"))?;
            let embeddings = get(&join_url(base, "embeddings"))
                .ok()
                .and_then(|j| j.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()))
                .unwrap_or_default();
            Ok(ModelLists {
                checkpoints: comfy_choices(base, "CheckpointLoaderSimple", "ckpt_name"),
                loras: comfy_choices(base, "LoraLoader", "lora_name"),
                vaes: comfy_choices(base, "VAELoader", "vae_name"),
                embeddings,
                samplers: comfy_choices(base, "KSampler", "sampler_name"),
                schedulers: comfy_choices(base, "KSampler", "scheduler"),
            })
        }
        _ => Ok(ModelLists::default()),
    }
}

/// Asks the backend to rescan its model folders (after a download).
/// ComfyUI rescans by itself on each `object_info`.
pub fn refresh_models(s: &ImageSettings) -> Result<(), ProviderError> {
    if s.backend == ImageBackend::Automatic1111 {
        for p in ["sdapi/v1/refresh-checkpoints", "sdapi/v1/refresh-loras", "sdapi/v1/refresh-vae"] {
            // Older servers lack some of them: best effort.
            let _ = send_json(&Request::post_json(&join_url(&s.base_url, p), &json!({}), Duration::from_secs(120)));
        }
    }
    Ok(())
}
