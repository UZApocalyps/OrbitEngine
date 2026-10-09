//! Providers, Stable Diffusion model management and Civitai against a local
//! mock HTTP server (127.0.0.1, no network).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use orbit_ai_studio::civitai::{self, DownloadOutcome, ModelType};
use orbit_ai_studio::image_gen::{Automatic1111, ComfyUi, ImageModel, ImageRequest, fill_workflow};
use orbit_ai_studio::sd::{self, LoraPick, SdPreset};
use orbit_ai_studio::settings::{CivitaiSettings, ImageBackend, ImageSettings, TextSettings};
use orbit_ai_studio::text::{Message, OpenAiChat, TextModel};
use serde_json::{Value as Json, json};

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<String>,
    body: String,
}

type Routes = BTreeMap<String, (u16, Vec<u8>, bool)>;

/// A mock server: `path (without query) -> (status, body, chunked)`.
struct Mock {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

fn mock(routes: Routes) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut r = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or("").to_owned();
            let path = parts.next().unwrap_or("").to_owned();
            let mut headers = Vec::new();
            let mut len = 0;
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                headers.push(h.trim().to_owned());
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            log.lock().unwrap().push(Seen { method, path: path.clone(), headers, body: String::from_utf8_lossy(&body).into() });
            let key = path.split('?').next().unwrap().to_owned();
            let (status, data, chunked) = routes.get(&key).cloned().unwrap_or((404, b"{}".to_vec(), false));
            let mut out = format!("HTTP/1.1 {status} X\r\nConnection: close\r\n").into_bytes();
            if chunked {
                out.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
                for chunk in data.chunks(7) {
                    out.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                    out.extend_from_slice(chunk);
                    out.extend_from_slice(b"\r\n");
                }
                out.extend_from_slice(b"0\r\n\r\n");
            } else {
                out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", data.len()).as_bytes());
                out.extend_from_slice(&data);
            }
            let _ = stream.write_all(&out);
        }
    });
    Mock { base, seen }
}

fn route(path: &str, body: Json) -> (String, (u16, Vec<u8>, bool)) {
    (path.to_owned(), (200, body.to_string().into_bytes(), false))
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn request() -> ImageRequest {
    ImageRequest {
        prompt: "a girl".into(),
        negative: "bad".into(),
        width: 64,
        height: 96,
        seed: 7,
        loras: vec![LoraPick { name: "heroine.safetensors".into(), weight: 0.8, trigger_words: vec!["hrn_girl".into()] }],
    }
}

#[test]
fn openai_chat_with_chunked_answer() {
    let answer = json!({"choices": [{"message": {"content": "<think>hmm</think>\nHello there"}}]});
    let mut routes = Routes::new();
    routes.insert("/v1/chat/completions".into(), (200, answer.to_string().into_bytes(), true));
    routes.extend([route("/v1/models", json!({"data": [{"id": "qwen"}, {"id": "llama"}]}))]);
    let m = mock(routes);
    let chat = OpenAiChat::new(TextSettings { base_url: format!("{}/v1", m.base), api_key: "sk-test".into(), model: "qwen".into(), ..TextSettings::default() });
    assert_eq!(chat.complete(&[Message::user("hi")]).unwrap(), "Hello there");
    assert_eq!(chat.list_models().unwrap(), ["qwen", "llama"]);
    let seen = m.seen.lock().unwrap();
    assert!(seen[0].headers.iter().any(|h| h == "Authorization: Bearer sk-test"));
    let body: Json = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["model"], "qwen");
    assert_eq!(body["messages"][0]["content"], "hi");
}

#[test]
fn http_error_status_is_reported() {
    let mut routes = Routes::new();
    routes.insert("/v1/chat/completions".into(), (500, b"model not loaded".to_vec(), false));
    let m = mock(routes);
    let chat = OpenAiChat::new(TextSettings { base_url: format!("{}/v1", m.base), ..TextSettings::default() });
    let err = chat.complete(&[Message::user("hi")]).unwrap_err().to_string();
    assert!(err.contains("500") && err.contains("model not loaded"), "{err}");
}

#[test]
fn automatic1111_sends_loras_vae_clip_skip() {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png());
    let m = mock(Routes::from([route("/sdapi/v1/txt2img", json!({"images": [b64]}))]));
    let mut s = ImageSettings { backend: ImageBackend::Automatic1111, base_url: m.base.clone(), model: "ponyXL.safetensors".into(), vae: "sdxl_vae.safetensors".into(), ..ImageSettings::default() };
    sd::apply_preset(&mut s, SdPreset::Pony);
    let bytes = Automatic1111::new(s).generate(&request(), &|| false).unwrap();
    assert_eq!(bytes, png());
    let seen = m.seen.lock().unwrap();
    let body: Json = serde_json::from_str(&seen[0].body).unwrap();
    let prompt = body["prompt"].as_str().unwrap();
    assert!(prompt.starts_with("score_9, score_8_up"), "{prompt}");
    assert!(prompt.contains("hrn_girl"), "trigger word: {prompt}");
    assert!(prompt.ends_with("<lora:heroine:0.8>"), "{prompt}");
    assert_eq!(body["override_settings"]["sd_model_checkpoint"], "ponyXL.safetensors");
    assert_eq!(body["override_settings"]["sd_vae"], "sdxl_vae.safetensors");
    assert_eq!(body["override_settings"]["CLIP_stop_at_last_layers"], 2);
    assert_eq!(body["width"], 64);
}

#[test]
fn comfy_workflow_gets_lora_chain_and_vae() {
    let s = ImageSettings { model: "illustrious.safetensors".into(), vae: "vae.safetensors".into(), clip_skip: 2, sampler: "DPM++ 2M Karras".into(), ..ImageSettings::default() };
    let wf = fill_workflow(&s.comfy_workflow, &request(), &s).unwrap();
    assert_eq!(wf["3"]["inputs"]["seed"], 7);
    assert_eq!(wf["3"]["inputs"]["sampler_name"], "dpmpp_2m");
    assert_eq!(wf["3"]["inputs"]["scheduler"], "karras");
    assert_eq!(wf["4"]["inputs"]["ckpt_name"], "illustrious.safetensors");
    assert_eq!(wf["orbit_lora_0"]["inputs"]["lora_name"], "heroine.safetensors");
    assert_eq!(wf["orbit_lora_0"]["inputs"]["model"], json!(["4", 0]));
    // The sampler takes the LoRA model; the text encoders the clip-skipped LoRA clip.
    assert_eq!(wf["3"]["inputs"]["model"], json!(["orbit_lora_0", 0]));
    assert_eq!(wf["orbit_clip_skip"]["inputs"]["clip"], json!(["orbit_lora_0", 1]));
    assert_eq!(wf["6"]["inputs"]["clip"], json!(["orbit_clip_skip", 0]));
    assert_eq!(wf["8"]["inputs"]["vae"], json!(["orbit_vae", 0]));
    assert!(wf["6"]["inputs"]["text"].as_str().unwrap().contains("hrn_girl"));
    assert!(!wf["6"]["inputs"]["text"].as_str().unwrap().contains("<lora:"));
}

#[test]
fn comfy_round_trip_and_model_lists() {
    let history = json!({"abc": {"outputs": {"9": {"images": [{"filename": "o 1.png", "subfolder": "", "type": "output"}]}}}});
    let mut routes = Routes::from([
        route("/prompt", json!({"prompt_id": "abc"})),
        route("/history/abc", history),
        route("/system_stats", json!({})),
        route("/embeddings", json!(["easynegative"])),
        route("/object_info/CheckpointLoaderSimple", json!({"CheckpointLoaderSimple": {"input": {"required": {"ckpt_name": [["a.safetensors", "b.safetensors"]]}}}})),
        route("/object_info/LoraLoader", json!({"LoraLoader": {"input": {"required": {"lora_name": [["heroine.safetensors"]]}}}})),
        route("/object_info/KSampler", json!({"KSampler": {"input": {"required": {"sampler_name": [["euler"]], "scheduler": [["karras"]]}}}})),
    ]);
    routes.insert("/view".into(), (200, png(), false));
    let m = mock(routes);
    let s = ImageSettings { backend: ImageBackend::ComfyUi, base_url: m.base.clone(), ..ImageSettings::default() };
    assert_eq!(ComfyUi::new(s.clone()).generate(&request(), &|| false).unwrap(), png());
    {
        let seen = m.seen.lock().unwrap();
        assert!(seen.iter().any(|r| r.path == "/view?filename=o%201.png&subfolder=&type=output"));
        let posted: Json = serde_json::from_str(&seen[0].body).unwrap();
        assert!(posted["prompt"]["orbit_lora_0"].is_object());
    }
    let lists = sd::list_models(&s).unwrap();
    assert_eq!(lists.checkpoints, ["a.safetensors", "b.safetensors"]);
    assert_eq!(lists.loras, ["heroine.safetensors"]);
    assert!(lists.vaes.is_empty(), "missing endpoint -> empty");
    assert_eq!(lists.embeddings, ["easynegative"]);
    assert_eq!(lists.schedulers, ["karras"]);
}

#[test]
fn a1111_model_lists() {
    let m = mock(Routes::from([
        route("/sdapi/v1/sd-models", json!([{"title": "pony.safetensors [abc]", "model_name": "pony"}])),
        route("/sdapi/v1/loras", json!([{"name": "heroine"}])),
        route("/sdapi/v1/sd-vae", json!([{"model_name": "sdxl_vae.safetensors"}])),
        route("/sdapi/v1/embeddings", json!({"loaded": {"easynegative": {}}})),
    ]));
    let s = ImageSettings { backend: ImageBackend::Automatic1111, base_url: m.base.clone(), ..ImageSettings::default() };
    let l = sd::list_models(&s).unwrap();
    assert_eq!(l.checkpoints, ["pony.safetensors [abc]"]);
    assert_eq!(l.loras, ["heroine"]);
    assert_eq!(l.vaes, ["sdxl_vae.safetensors"]);
    assert_eq!(l.embeddings, ["easynegative"]);
    sd::refresh_models(&s).unwrap();
    assert!(m.seen.lock().unwrap().iter().any(|r| r.method == "POST" && r.path == "/sdapi/v1/refresh-loras"));
}

fn civitai_answer(file_url: &str, sha: &str) -> Json {
    json!({"items": [
        {"id": 1, "name": "Heroine LoRA", "type": "LORA", "nsfw": false, "modelVersions": [{
            "id": 11, "name": "v1", "baseModel": "Pony", "trainedWords": ["hrn_girl", " "],
            "images": [{"url": "http://x/nsfw.png", "nsfw": "X"}, {"url": "http://x/safe.png", "nsfw": "None"}],
            "files": [{"name": "heroine.safetensors", "sizeKB": 0.01, "primary": true,
                       "hashes": {"SHA256": sha}, "downloadUrl": file_url}]}]},
        {"id": 2, "name": "Spicy", "type": "Checkpoint", "nsfw": true, "modelVersions": []},
        {"id": 3, "name": "Workflow", "type": "Workflows", "nsfw": false, "modelVersions": []}
    ]})
}

fn tmp(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ai_studio_civitai").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn civitai_search_and_verified_download() {
    let payload = b"fake safetensors".to_vec();
    let sha = orbit_ai_studio::sha256::digest_hex(&payload);
    let mut routes = Routes::new();
    routes.insert("/files/heroine".into(), (200, payload.clone(), false));
    let m = mock(routes);
    let file_url = format!("{}/files/heroine", m.base);
    // Second server: the API (needs the file URL of the first).
    let api = mock(Routes::from([route("/api/v1/models", civitai_answer(&file_url, &sha.to_lowercase()))]));
    let dir = tmp("dl");
    let mut c = CivitaiSettings { base_url: api.base.clone(), api_token: "tok".into(), ..CivitaiSettings::default() };
    c.folders.loras = dir.join("loras").to_string_lossy().into_owned();

    let found = civitai::search(&c, "heroine girl", Some(ModelType::Lora)).unwrap();
    assert_eq!(found.len(), 1, "NSFW and unknown types hidden");
    let v = &found[0].versions[0];
    assert_eq!(v.trigger_words, ["hrn_girl"]);
    assert_eq!(v.base_model, "Pony");
    assert_eq!(v.preview_url.as_deref(), Some("http://x/safe.png"));
    {
        let seen = api.seen.lock().unwrap();
        assert_eq!(seen[0].path, "/api/v1/models?limit=20&query=heroine%20girl&nsfw=false&types=LORA");
        assert!(seen[0].headers.iter().any(|h| h == "Authorization: Bearer tok"));
    }
    c.show_nsfw = true;
    let all = civitai::search(&c, "x", None).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].versions[0].preview_url.as_deref(), Some("http://x/nsfw.png"));

    let file = v.main_file().unwrap().clone();
    let folder = PathBuf::from(ModelType::Lora.folder(&c));
    let progress = Mutex::new(0u64);
    let out = civitai::download(&c, &file, &folder, &|| false, &|d, _| *progress.lock().unwrap() = d).unwrap();
    assert_eq!(out, DownloadOutcome::Downloaded(folder.join("heroine.safetensors")));
    assert_eq!(std::fs::read(folder.join("heroine.safetensors")).unwrap(), payload);
    assert_eq!(*progress.lock().unwrap(), payload.len() as u64);
    // Present with the right hash: skipped, no new request.
    let before = m.seen.lock().unwrap().len();
    let again = civitai::download(&c, &file, &folder, &|| false, &|_, _| {}).unwrap();
    assert!(matches!(again, DownloadOutcome::AlreadyPresent(_)));
    assert_eq!(m.seen.lock().unwrap().len(), before);

    // A wrong hash: the file is refused and removed.
    let mut bad = file.clone();
    bad.name = "other.safetensors".into();
    bad.sha256 = Some("00".repeat(32));
    let err = civitai::download(&c, &bad, &folder, &|| false, &|_, _| {}).unwrap_err();
    assert!(err.to_string().contains("SHA-256"), "{err}");
    assert!(!folder.join("other.safetensors").exists());
    assert!(!folder.join("other.part").exists());
    // Path tricks stay in the folder.
    let mut sneaky = file;
    sneaky.name = "../../evil.safetensors".into();
    sneaky.sha256 = Some(sha);
    let out = civitai::download(&c, &sneaky, &folder, &|| false, &|_, _| {}).unwrap();
    assert_eq!(out, DownloadOutcome::Downloaded(folder.join("evil.safetensors")));
}

#[test]
fn presets_and_trigger_words() {
    let mut s = ImageSettings::default();
    sd::apply_preset(&mut s, SdPreset::Sd15);
    assert_eq!((s.sprite_size.width, s.sprite_size.height), (512, 768));
    assert_eq!(s.clip_skip, 2);
    let loras = [LoraPick { name: "x".into(), weight: 1.0, trigger_words: vec!["already".into(), "new_word".into()] }];
    let p = sd::full_prompt(&s, "girl, already", &loras, false);
    assert_eq!(p, "masterpiece, best quality, girl, already, new_word");
}
