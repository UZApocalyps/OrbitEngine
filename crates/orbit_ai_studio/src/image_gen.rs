//! Image generation: the [`ImageModel`] trait and its backends (ComfyUI,
//! AUTOMATIC1111 / Forge, OpenAI-compatible).

use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value as Json, json};

use crate::settings::{ImageBackend, ImageSettings};
use crate::http::{Request, encode_query};
use crate::text::{ProviderError, join_url, send_json};

/// A text-to-image txt2img workflow in ComfyUI **API format** (Workflow >
/// Export (API) in ComfyUI). String values may hold the placeholders
/// `{{prompt}}`, `{{negative}}`, `{{model}}`, `{{sampler}}`, `{{scheduler}}`; a value that is exactly
/// `"{{seed}}"`, `"{{width}}"`, `"{{height}}"`, `"{{steps}}"` or `"{{cfg}}"`
/// becomes a number.
pub const DEFAULT_COMFY_WORKFLOW: &str = r#"{
  "3": {"class_type": "KSampler", "inputs": {"seed": "{{seed}}", "steps": "{{steps}}", "cfg": "{{cfg}}", "sampler_name": "{{sampler}}", "scheduler": "{{scheduler}}", "denoise": 1.0, "model": ["4", 0], "positive": ["6", 0], "negative": ["7", 0], "latent_image": ["5", 0]}},
  "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "{{model}}"}},
  "5": {"class_type": "EmptyLatentImage", "inputs": {"width": "{{width}}", "height": "{{height}}", "batch_size": 1}},
  "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}", "clip": ["4", 1]}},
  "7": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{negative}}", "clip": ["4", 1]}},
  "8": {"class_type": "VAEDecode", "inputs": {"samples": ["3", 0], "vae": ["4", 2]}},
  "9": {"class_type": "SaveImage", "inputs": {"filename_prefix": "orbit_ai", "images": ["8", 0]}}
}"#;

/// One picture to make.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageRequest {
    pub prompt: String,
    pub negative: String,
    pub width: u32,
    pub height: u32,
    pub seed: u64,
    /// LoRAs for this picture (global ones and the character's).
    pub loras: Vec<crate::sd::LoraPick>,
}

/// Something that turns a prompt into an encoded picture (PNG, JPEG...).
pub trait ImageModel: Send + Sync {
    fn generate(&self, req: &ImageRequest, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>, ProviderError>;
}

/// The backend `settings` asks for; `None` for placeholders only.
pub fn from_settings(settings: &ImageSettings) -> Option<Box<dyn ImageModel>> {
    match settings.backend {
        ImageBackend::Placeholder => None,
        ImageBackend::ComfyUi => Some(Box::new(ComfyUi::new(settings.clone()))),
        ImageBackend::Automatic1111 => Some(Box::new(Automatic1111::new(settings.clone()))),
        ImageBackend::OpenAi => Some(Box::new(OpenAiImages::new(settings.clone()))),
    }
}

fn get_bytes(url: &str, timeout: Duration) -> Result<Vec<u8>, ProviderError> {
    let resp = Request::get(url, timeout).send().map_err(ProviderError::Http)?;
    if !(200..300).contains(&resp.status) {
        return Err(ProviderError::Http(format!("HTTP {} for {url}", resp.status)));
    }
    Ok(resp.body)
}

// ------------------------------------------------------------------ ComfyUI

pub struct ComfyUi {
    settings: ImageSettings,
}

impl ComfyUi {
    pub fn new(settings: ImageSettings) -> Self {
        Self { settings }
    }
}

/// Fills the placeholders of a workflow template.
pub fn fill_workflow(template: &str, req: &ImageRequest, s: &ImageSettings) -> Result<Json, ProviderError> {
    let mut value: Json = serde_json::from_str(template)
        .map_err(|e| ProviderError::BadResponse(format!("ComfyUI workflow is not valid JSON: {e}")))?;
    fn walk(v: &mut Json, req: &ImageRequest, s: &ImageSettings) {
        match v {
            Json::String(text) => {
                let number = match text.as_str() {
                    "{{seed}}" => Some(json!(req.seed)),
                    "{{width}}" => Some(json!(req.width)),
                    "{{height}}" => Some(json!(req.height)),
                    "{{steps}}" => Some(json!(s.steps)),
                    "{{cfg}}" => Some(json!(s.cfg_scale)),
                    _ => None,
                };
                if let Some(n) = number {
                    *v = n;
                } else if text.contains("{{") {
                    let (sampler, karras) = crate::sd::comfy_sampler(&s.sampler);
                    let scheduler = if !s.scheduler.trim().is_empty() {
                        s.scheduler.trim().to_owned()
                    } else {
                        karras.unwrap_or("normal").to_owned()
                    };
                    *text = text
                        .replace("{{prompt}}", &crate::sd::full_prompt(s, &req.prompt, &req.loras, false))
                        .replace("{{negative}}", &req.negative)
                        .replace("{{model}}", &s.model)
                        .replace("{{sampler}}", &sampler)
                        .replace("{{scheduler}}", &scheduler);
                }
            }
            Json::Array(items) => items.iter_mut().for_each(|i| walk(i, req, s)),
            Json::Object(map) => map.values_mut().for_each(|i| walk(i, req, s)),
            _ => {}
        }
    }
    walk(&mut value, req, s);
    crate::sd::inject_comfy(&mut value, &req.loras, &s.vae, s.clip_skip);
    Ok(value)
}

impl ImageModel for ComfyUi {
    fn generate(&self, req: &ImageRequest, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>, ProviderError> {
        let base = &self.settings.base_url;
        let workflow = fill_workflow(&self.settings.comfy_workflow, req, &self.settings)?;
        let short = Duration::from_secs(60);
        let queued = send_json(&Request::post_json(
            &join_url(base, "prompt"),
            &json!({"prompt": workflow, "client_id": "orbit_ai_studio"}),
            short,
        ))?;
        let id = queued["prompt_id"]
            .as_str()
            .ok_or_else(|| ProviderError::BadResponse(format!("no prompt_id: {queued}")))?
            .to_owned();
        let deadline = Instant::now() + Duration::from_secs(self.settings.timeout_s.max(10));
        loop {
            if cancelled() {
                return Err(ProviderError::Cancelled);
            }
            if Instant::now() > deadline {
                return Err(ProviderError::Http("ComfyUI took too long".to_owned()));
            }
            let history = send_json(&Request::get(&join_url(base, &format!("history/{id}")), short))?;
            if let Some(entry) = history.get(&id) {
                let image = entry["outputs"]
                    .as_object()
                    .and_then(|o| o.values().find_map(|out| out["images"].get(0).cloned()));
                if let Some(image) = image {
                    let s = |k: &str| image[k].as_str().unwrap_or("").to_owned();
                    let url = format!(
                        "{}?filename={}&subfolder={}&type={}",
                        join_url(base, "view"),
                        encode_query(&s("filename")),
                        encode_query(&s("subfolder")),
                        encode_query(&s("type")),
                    );
                    return get_bytes(&url, short);
                }
                if entry["status"]["status_str"] == "error" {
                    return Err(ProviderError::BadResponse(format!(
                        "ComfyUI failed: {}",
                        entry["status"]
                    )));
                }
            }
            std::thread::sleep(Duration::from_millis(700));
        }
    }
}

// ------------------------------------------------------- AUTOMATIC1111 / Forge

pub struct Automatic1111 {
    settings: ImageSettings,
}

impl Automatic1111 {
    pub fn new(settings: ImageSettings) -> Self {
        Self { settings }
    }
}

fn decode_b64(data: &str) -> Result<Vec<u8>, ProviderError> {
    // Some servers prefix a data URL.
    let data = data.rsplit_once(',').map_or(data, |(_, d)| d);
    base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| ProviderError::BadResponse(format!("base64: {e}")))
}

impl ImageModel for Automatic1111 {
    fn generate(&self, req: &ImageRequest, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>, ProviderError> {
        if cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let s = &self.settings;
        let mut body = json!({
            "prompt": crate::sd::full_prompt(s, &req.prompt, &req.loras, true),
            "negative_prompt": req.negative,
            "seed": req.seed,
            "width": req.width,
            "height": req.height,
            "steps": s.steps,
            "cfg_scale": s.cfg_scale,
            "sampler_name": s.sampler,
            "batch_size": 1,
            "n_iter": 1,
        });
        if !s.scheduler.trim().is_empty() {
            body["scheduler"] = json!(s.scheduler.trim());
        }
        let mut over = serde_json::Map::new();
        if !s.model.is_empty() {
            over.insert("sd_model_checkpoint".into(), json!(s.model));
        }
        if !s.vae.is_empty() {
            over.insert("sd_vae".into(), json!(s.vae));
        }
        if s.clip_skip > 1 {
            over.insert("CLIP_stop_at_last_layers".into(), json!(s.clip_skip));
        }
        if !over.is_empty() {
            body["override_settings"] = Json::Object(over);
        }
        let url = join_url(&s.base_url, "sdapi/v1/txt2img");
        let json = send_json(&Request::post_json(&url, &body, Duration::from_secs(s.timeout_s.max(10))))?;
        let first = json["images"][0]
            .as_str()
            .ok_or_else(|| ProviderError::BadResponse("no images[0]".to_owned()))?;
        decode_b64(first)
    }
}

// --------------------------------------------------------- OpenAI-compatible

pub struct OpenAiImages {
    settings: ImageSettings,
}

impl OpenAiImages {
    pub fn new(settings: ImageSettings) -> Self {
        Self { settings }
    }
}

impl ImageModel for OpenAiImages {
    fn generate(&self, req: &ImageRequest, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>, ProviderError> {
        if cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let s = &self.settings;
        let p = crate::sd::full_prompt(s, &req.prompt, &[], false);
        let prompt = if req.negative.is_empty() { p } else { format!("{p}\nAvoid: {}", req.negative) };
        let mut body = json!({
            "prompt": prompt,
            "n": 1,
            "size": format!("{}x{}", req.width, req.height),
            "response_format": "b64_json",
        });
        if !s.model.is_empty() {
            body["model"] = json!(s.model);
        }
        let url = join_url(&s.base_url, "images/generations");
        let timeout = Duration::from_secs(s.timeout_s.max(10));
        let json = send_json(&Request::post_json(&url, &body, timeout).bearer(&s.api_key))?;
        let item = &json["data"][0];
        if let Some(b64) = item["b64_json"].as_str() {
            return decode_b64(b64);
        }
        if let Some(url) = item["url"].as_str() {
            return get_bytes(url, timeout);
        }
        Err(ProviderError::BadResponse("no data[0].b64_json or url".to_owned()))
    }
}
