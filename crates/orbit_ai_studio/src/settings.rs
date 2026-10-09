//! Provider settings. They live in the **user** config folder
//! (`ai_studio.ron` next to the editor's `config.ron`), never in the project:
//! API keys must not end up in a repository or an exported game.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Settings file name, inside the user config folder.
pub const SETTINGS_FILE: &str = "ai_studio.ron";

/// An OpenAI-compatible `chat/completions` endpoint: Ollama, LM Studio,
/// llama.cpp server, KoboldCpp, vLLM, OpenRouter, OpenAI... The model is
/// whatever the user picks; the studio adds no filtering of its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextSettings {
    /// Base URL, up to `/v1` (`http://localhost:11434/v1` for Ollama).
    pub base_url: String,
    pub model: String,
    /// Bearer key; empty for local servers.
    pub api_key: String,
    pub temperature: f32,
    pub max_tokens: u32,
    /// Seconds before a request gives up (local models can be slow).
    pub timeout_s: u64,
    /// Attempts to repair invalid output (JSON, DSL) before giving up.
    pub max_repairs: u32,
}

impl Default for TextSettings {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434/v1".to_owned(),
            model: "llama3.1".to_owned(),
            api_key: String::new(),
            temperature: 0.8,
            max_tokens: 4096,
            timeout_s: 600,
            max_repairs: 3,
        }
    }
}

/// Which image backend to call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageBackend {
    /// No generation: every picture is a placeholder (replace them later).
    #[default]
    Placeholder,
    /// ComfyUI HTTP API with a workflow template.
    ComfyUi,
    /// AUTOMATIC1111 / Forge `/sdapi/v1/txt2img`.
    Automatic1111,
    /// OpenAI-compatible `images/generations`.
    OpenAi,
}

impl ImageBackend {
    pub const ALL: [ImageBackend; 4] = [
        ImageBackend::Placeholder,
        ImageBackend::ComfyUi,
        ImageBackend::Automatic1111,
        ImageBackend::OpenAi,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ImageBackend::Placeholder => "Placeholder",
            ImageBackend::ComfyUi => "ComfyUI",
            ImageBackend::Automatic1111 => "AUTOMATIC1111 / Forge",
            ImageBackend::OpenAi => "OpenAI-compatible",
        }
    }
}

/// A `(width, height)` in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageSettings {
    pub backend: ImageBackend,
    /// ComfyUI / A1111 server, or the OpenAI-compatible base URL (`.../v1`).
    pub base_url: String,
    /// Key for the OpenAI-compatible backend.
    pub api_key: String,
    /// Model name (OpenAI-compatible backend; A1111 checkpoint override when
    /// set).
    pub model: String,
    /// ComfyUI workflow in **API format** with the placeholders `{{prompt}}`,
    /// `{{negative}}`, `{{seed}}`, `{{width}}`, `{{height}}` (see
    /// [`crate::image_gen::DEFAULT_COMFY_WORKFLOW`]).
    pub comfy_workflow: String,
    pub steps: u32,
    pub cfg_scale: f32,
    pub sampler: String,
    /// Prepended to every prompt (art style).
    pub style_prefix: String,
    pub negative_prompt: String,
    pub background_size: Size,
    pub sprite_size: Size,
    /// Base seed; each character / location derives a stable seed from it,
    /// so a character keeps the same look across its expressions.
    pub seed: u64,
    /// Pictures generated at once.
    pub concurrency: u32,
    /// Sprites: turn the plain background around the character transparent
    /// (local flood fill from the borders), for backends without background
    /// removal.
    pub remove_sprite_background: bool,
    pub timeout_s: u64,
    /// Model family the sizes and quality tags were set for.
    pub preset: crate::sd::SdPreset,
    /// Prepended to every prompt (`masterpiece, best quality`, Pony scores...).
    pub quality_tags: String,
    /// LoRAs applied to every picture (per-character ones add to them).
    pub loras: Vec<crate::sd::LoraPick>,
    /// VAE override; empty for the checkpoint's own.
    pub vae: String,
    /// Scheduler (`karras`, `normal`...); empty for the backend default.
    pub scheduler: String,
    /// CLIP skip (1 = none, 2 for most anime / Pony models).
    pub clip_skip: u32,
}

impl Default for ImageSettings {
    fn default() -> Self {
        Self {
            backend: ImageBackend::Placeholder,
            base_url: "http://127.0.0.1:8188".to_owned(),
            api_key: String::new(),
            model: String::new(),
            comfy_workflow: crate::image_gen::DEFAULT_COMFY_WORKFLOW.to_owned(),
            steps: 28,
            cfg_scale: 6.5,
            sampler: "Euler a".to_owned(),
            style_prefix: "anime visual novel art, clean lineart, soft colors".to_owned(),
            negative_prompt: "lowres, bad anatomy, extra fingers, text, watermark, signature, blurry"
                .to_owned(),
            background_size: Size {
                width: 1280,
                height: 720,
            },
            sprite_size: Size {
                width: 768,
                height: 1152,
            },
            seed: 1234,
            concurrency: 2,
            remove_sprite_background: true,
            timeout_s: 600,
            preset: crate::sd::SdPreset::Custom,
            quality_tags: String::new(),
            loras: Vec::new(),
            vae: String::new(),
            scheduler: String::new(),
            clip_skip: 1,
        }
    }
}

/// Everything the studio remembers per user.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StudioSettings {
    pub text: TextSettings,
    pub image: ImageSettings,
    pub civitai: CivitaiSettings,
}

/// Where downloaded models go: the backend's own model folders.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelFolders {
    /// `ComfyUI/models/checkpoints` or `stable-diffusion-webui/models/Stable-diffusion`.
    pub checkpoints: String,
    /// `ComfyUI/models/loras` or `.../models/Lora`.
    pub loras: String,
    /// `ComfyUI/models/embeddings` or `.../embeddings`.
    pub embeddings: String,
    /// `ComfyUI/models/vae` or `.../models/VAE`.
    pub vaes: String,
}

/// Civitai access (optional).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CivitaiSettings {
    pub base_url: String,
    /// API token (some downloads require one). User config only.
    pub api_token: String,
    /// Show models / previews flagged NSFW in the search results.
    pub show_nsfw: bool,
    pub folders: ModelFolders,
}

impl Default for CivitaiSettings {
    fn default() -> Self {
        Self {
            base_url: "https://civitai.com".to_owned(),
            api_token: String::new(),
            show_nsfw: false,
            folders: ModelFolders::default(),
        }
    }
}

impl StudioSettings {
    pub fn path_in(config_dir: &Path) -> PathBuf {
        config_dir.join(SETTINGS_FILE)
    }

    /// Reads the settings of `config_dir`; defaults when missing or broken.
    pub fn load(config_dir: &Path) -> Self {
        fs::read_to_string(Self::path_in(config_dir))
            .ok()
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, config_dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(config_dir)?;
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(std::io::Error::other)?;
        let path = Self::path_in(config_dir);
        let tmp = path.with_extension("ron.tmp");
        fs::write(&tmp, text)?;
        fs::rename(tmp, path)
    }
}
