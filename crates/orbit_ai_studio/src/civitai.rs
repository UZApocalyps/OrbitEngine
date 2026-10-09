//! Civitai (optional): search models through the public REST API
//! (`/api/v1/models`) and download a version's file into the backend's
//! model folder, checked against the SHA-256 the API publishes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::http::{Request, encode_query};
use crate::settings::CivitaiSettings;
use crate::text::{ProviderError, send_json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelType {
    Checkpoint,
    Lora,
    TextualInversion,
    Vae,
}

impl ModelType {
    pub const ALL: [ModelType; 4] = [ModelType::Checkpoint, ModelType::Lora, ModelType::TextualInversion, ModelType::Vae];

    /// The API's `types` value.
    pub fn api(self) -> &'static str {
        match self {
            ModelType::Checkpoint => "Checkpoint",
            ModelType::Lora => "LORA",
            ModelType::TextualInversion => "TextualInversion",
            ModelType::Vae => "VAE",
        }
    }

    pub fn from_api(s: &str) -> Option<Self> {
        match s {
            "Checkpoint" => Some(ModelType::Checkpoint),
            "LORA" | "LoCon" | "DoRA" => Some(ModelType::Lora),
            "TextualInversion" => Some(ModelType::TextualInversion),
            "VAE" => Some(ModelType::Vae),
            _ => None,
        }
    }

    /// The configured folder this type downloads to.
    pub fn folder(self, c: &CivitaiSettings) -> &str {
        match self {
            ModelType::Checkpoint => &c.folders.checkpoints,
            ModelType::Lora => &c.folders.loras,
            ModelType::TextualInversion => &c.folders.embeddings,
            ModelType::Vae => &c.folders.vaes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelFile {
    pub name: String,
    pub size_kb: f64,
    /// Uppercase hex.
    pub sha256: Option<String>,
    pub download_url: String,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelVersion {
    pub id: u64,
    pub name: String,
    pub base_model: String,
    pub trigger_words: Vec<String>,
    pub preview_url: Option<String>,
    pub files: Vec<ModelFile>,
}

impl ModelVersion {
    /// The file to download: the primary one, else the first.
    pub fn main_file(&self) -> Option<&ModelFile> {
        self.files.iter().find(|f| f.primary).or(self.files.first())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: u64,
    pub name: String,
    pub kind: ModelType,
    pub nsfw: bool,
    pub versions: Vec<ModelVersion>,
}

fn parse_version(v: &Json, show_nsfw: bool) -> ModelVersion {
    let preview_url = v["images"].as_array().and_then(|imgs| {
        imgs.iter()
            .filter(|i| show_nsfw || !image_nsfw(i))
            .find_map(|i| i["url"].as_str().map(str::to_owned))
    });
    ModelVersion {
        id: v["id"].as_u64().unwrap_or(0),
        name: v["name"].as_str().unwrap_or("").to_owned(),
        base_model: v["baseModel"].as_str().unwrap_or("").to_owned(),
        trigger_words: v["trainedWords"]
            .as_array()
            .map(|a| a.iter().filter_map(|w| w.as_str().map(|s| s.trim().to_owned())).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default(),
        preview_url,
        files: v["files"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| {
                        Some(ModelFile {
                            name: f["name"].as_str()?.to_owned(),
                            size_kb: f["sizeKB"].as_f64().unwrap_or(0.0),
                            sha256: f["hashes"]["SHA256"].as_str().map(str::to_ascii_uppercase),
                            download_url: f["downloadUrl"].as_str()?.to_owned(),
                            primary: f["primary"].as_bool().unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// An image is NSFW when flagged so (`nsfw: true` or a level above "None").
fn image_nsfw(i: &Json) -> bool {
    match &i["nsfw"] {
        Json::Bool(b) => *b,
        Json::String(s) => s != "None",
        _ => i["nsfwLevel"].as_u64().is_some_and(|l| l > 1),
    }
}

/// Parses a `/api/v1/models` answer; NSFW models are dropped unless shown.
pub fn parse_search(json: &Json, show_nsfw: bool) -> Vec<ModelInfo> {
    json["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|m| {
                    let nsfw = m["nsfw"].as_bool().unwrap_or(false);
                    if nsfw && !show_nsfw {
                        return None;
                    }
                    Some(ModelInfo {
                        id: m["id"].as_u64()?,
                        name: m["name"].as_str()?.to_owned(),
                        kind: ModelType::from_api(m["type"].as_str()?)?,
                        nsfw,
                        versions: m["modelVersions"]
                            .as_array()
                            .map(|v| v.iter().map(|v| parse_version(v, show_nsfw)).collect())
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Searches models.
pub fn search(c: &CivitaiSettings, query: &str, kind: Option<ModelType>) -> Result<Vec<ModelInfo>, ProviderError> {
    let mut url = format!(
        "{}/api/v1/models?limit=20&query={}&nsfw={}",
        c.base_url.trim_end_matches('/'),
        encode_query(query.trim()),
        c.show_nsfw
    );
    if let Some(k) = kind {
        url.push_str(&format!("&types={}", k.api()));
    }
    let json = send_json(&Request::get(&url, Duration::from_secs(30)).bearer(&c.api_token))?;
    Ok(parse_search(&json, c.show_nsfw))
}

/// Downloads a small file (a preview) into memory.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, ProviderError> {
    let r = Request::get(url, Duration::from_secs(30)).send().map_err(ProviderError::Http)?;
    if (200..300).contains(&r.status) { Ok(r.body) } else { Err(ProviderError::Http(format!("HTTP {}", r.status))) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadOutcome {
    Downloaded(PathBuf),
    /// Already there with the right hash.
    AlreadyPresent(PathBuf),
}

/// Keeps a file name inside its folder.
fn safe_name(name: &str) -> Result<String, ProviderError> {
    let n = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if n.is_empty() || n == "." || n == ".." {
        return Err(ProviderError::BadResponse(format!("bad file name `{name}`")));
    }
    Ok(n.to_owned())
}

/// Downloads `file` into `folder` (resuming nothing: a `.part` file is
/// written then renamed once its SHA-256 matches). `progress(done, total)`
/// is called while it runs; `cancelled` stops it and removes the partial
/// file.
pub fn download(
    c: &CivitaiSettings,
    file: &ModelFile,
    folder: &Path,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> Result<DownloadOutcome, ProviderError> {
    if folder.as_os_str().is_empty() {
        return Err(ProviderError::BadResponse("set the model folder for this type first".to_owned()));
    }
    std::fs::create_dir_all(folder).map_err(|e| ProviderError::Http(e.to_string()))?;
    let dest = folder.join(safe_name(&file.name)?);
    let check = |path: &Path| -> Result<bool, ProviderError> {
        match &file.sha256 {
            None => Ok(true),
            Some(want) => match crate::sha256::file_hex(path, cancelled).map_err(|e| ProviderError::Http(e.to_string()))? {
                None => Err(ProviderError::Cancelled),
                Some(got) => Ok(got.eq_ignore_ascii_case(want)),
            },
        }
    };
    if dest.is_file() && file.sha256.is_some() && check(&dest)? {
        return Ok(DownloadOutcome::AlreadyPresent(dest));
    }
    let part = dest.with_extension("part");
    let total = (file.size_kb * 1024.0) as u64;
    let url = file.download_url.clone();
    let result = if url.starts_with("https://") {
        download_curl(&url, &c.api_token, &part, total, cancelled, progress)
    } else {
        // Plain http (local mirrors, tests): in memory.
        let r = Request::get(&url, Duration::from_secs(3600)).send().map_err(ProviderError::Http)?;
        if !(200..300).contains(&r.status) {
            Err(ProviderError::Http(format!("HTTP {}", r.status)))
        } else {
            progress(r.body.len() as u64, total.max(r.body.len() as u64));
            std::fs::write(&part, &r.body).map_err(|e| ProviderError::Http(e.to_string()))
        }
    };
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    match check(&part) {
        Ok(true) => {
            std::fs::rename(&part, &dest).map_err(|e| ProviderError::Http(e.to_string()))?;
            Ok(DownloadOutcome::Downloaded(dest))
        }
        Ok(false) => {
            let _ = std::fs::remove_file(&part);
            Err(ProviderError::BadResponse("SHA-256 mismatch: the download is corrupt and was removed".to_owned()))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Streams to disk with `curl -L` (redirects to the CDN), polling the file
/// size for progress and killing curl on cancel.
fn download_curl(
    url: &str,
    token: &str,
    part: &Path,
    total: u64,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), ProviderError> {
    let mut cmd = Command::new("curl");
    // curl only sends this header to the original host, not to the CDN it
    // is redirected to.
    if !token.is_empty() {
        cmd.arg("-H").arg(format!("Authorization: Bearer {token}"));
    }
    let mut child = cmd
        .args(["-sS", "-L", "--fail", "-o"])
        .arg(part)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ProviderError::Http(format!("downloads need the `curl` program ({e})")))?;
    loop {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProviderError::Cancelled);
        }
        if let Some(status) = child.try_wait().map_err(|e| ProviderError::Http(e.to_string()))? {
            if status.success() {
                return Ok(());
            }
            let mut err = String::new();
            if let Some(mut s) = child.stderr.take() {
                use std::io::Read;
                let _ = s.read_to_string(&mut err);
            }
            return Err(ProviderError::Http(format!("curl: {}", err.trim())));
        }
        let done = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        progress(done, total);
        std::thread::sleep(Duration::from_millis(300));
    }
}
