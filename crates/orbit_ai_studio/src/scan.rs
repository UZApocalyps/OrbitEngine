//! Models on disk: scans the configured model folders so the editor can offer
//! drop-down lists, merges them with what the backend reports, and detects the
//! standard folder layout of a ComfyUI / A1111 install.

use std::fs;
use std::path::{Path, PathBuf};

use crate::sd::ModelLists;
use crate::settings::{ImageBackend, ModelFolders};

/// Model file extensions.
pub const MODEL_EXTENSIONS: [&str; 6] = ["safetensors", "ckpt", "pt", "pth", "bin", "gguf"];
/// Embeddings are only these.
pub const EMBEDDING_EXTENSIONS: [&str; 2] = ["pt", "safetensors"];

const MAX_DEPTH: usize = 12;
const MAX_FILES: usize = 20_000;

/// What kind of model a folder holds (it decides how names are written).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Checkpoint,
    Lora,
    Vae,
    Embedding,
}

/// A selectable name and where it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub name: String,
    pub on_disk: bool,
    pub on_backend: bool,
}

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Name of `path` (inside `folder`) as the backend expects it: relative path
/// with forward slashes; A1111 LoRAs and embeddings use the bare stem, and
/// embeddings never carry their extension.
pub fn format_name(folder: &Path, path: &Path, kind: Kind, backend: ImageBackend) -> Option<String> {
    let rel = path.strip_prefix(folder).ok()?;
    let rel_str = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    if rel_str.is_empty() {
        return None;
    }
    let stem = || path.file_stem().map(|s| s.to_string_lossy().into_owned());
    match (backend, kind) {
        (ImageBackend::Automatic1111, Kind::Lora | Kind::Embedding) => stem(),
        (_, Kind::Embedding) => Some(match rel_str.rfind('.') {
            Some(i) => rel_str[..i].to_owned(),
            None => rel_str,
        }),
        _ => Some(rel_str),
    }
}

fn walk(dir: &Path, depth: usize, exts: &[&str], out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        // `file_type` does not follow symlinks: no loops.
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            walk(&p, depth + 1, exts, out);
        } else if ft.is_file() && has_ext(&p, exts) {
            out.push(p);
        }
    }
}

/// Lists one folder; empty when `folder` is empty or missing. Sorted, unique.
pub fn scan_folder(folder: &str, kind: Kind, backend: ImageBackend) -> Vec<String> {
    let folder = folder.trim();
    if folder.is_empty() {
        return Vec::new();
    }
    let root = Path::new(folder);
    let exts: &[&str] = if kind == Kind::Embedding { &EMBEDDING_EXTENSIONS } else { &MODEL_EXTENSIONS };
    let mut files = Vec::new();
    walk(root, 0, exts, &mut files);
    let mut names: Vec<String> = files.iter().filter_map(|p| format_name(root, p, kind, backend)).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// Scans the four folders (samplers / schedulers stay empty).
pub fn scan_folders(f: &ModelFolders, backend: ImageBackend) -> ModelLists {
    ModelLists {
        checkpoints: scan_folder(&f.checkpoints, Kind::Checkpoint, backend),
        loras: scan_folder(&f.loras, Kind::Lora, backend),
        vaes: scan_folder(&f.vaes, Kind::Vae, backend),
        embeddings: scan_folder(&f.embeddings, Kind::Embedding, backend),
        samplers: Vec::new(),
        schedulers: Vec::new(),
    }
}

/// Merges disk and backend names: deduped, sorted, with the source marked.
pub fn merge(disk: &[String], backend: &[String]) -> Vec<Choice> {
    let mut out: Vec<Choice> = Vec::new();
    let mut index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (list, is_disk) in [(disk, true), (backend, false)] {
        for n in list {
            match index.get(n.as_str()) {
                Some(&i) => {
                    out[i].on_disk |= is_disk;
                    out[i].on_backend |= !is_disk;
                }
                None => {
                    index.insert(n.as_str(), out.len());
                    out.push(Choice { name: n.clone(), on_disk: is_disk, on_backend: !is_disk });
                }
            }
        }
    }
    out.sort_by_key(|c| c.name.to_lowercase());
    out
}

/// Samplers every ComfyUI knows.
pub const COMFY_SAMPLERS: &[&str] = &[
    "euler", "euler_cfg_pp", "euler_ancestral", "euler_ancestral_cfg_pp", "heun", "heunpp2", "dpm_2",
    "dpm_2_ancestral", "lms", "dpm_fast", "dpm_adaptive", "dpmpp_2s_ancestral", "dpmpp_sde",
    "dpmpp_sde_gpu", "dpmpp_2m", "dpmpp_2m_cfg_pp", "dpmpp_2m_sde", "dpmpp_2m_sde_gpu", "dpmpp_3m_sde",
    "dpmpp_3m_sde_gpu", "ddpm", "lcm", "ipndm", "ipndm_v", "deis", "ddim", "uni_pc", "uni_pc_bh2",
];
/// Schedulers every ComfyUI knows.
pub const COMFY_SCHEDULERS: &[&str] = &[
    "simple", "sgm_uniform", "karras", "exponential", "ddim_uniform", "beta", "normal", "linear_quadratic", "kl_optimal",
];
/// Samplers of AUTOMATIC1111 / Forge.
pub const A1111_SAMPLERS: &[&str] = &[
    "Euler a", "Euler", "LMS", "Heun", "DPM2", "DPM2 a", "DPM++ 2S a", "DPM++ 2M", "DPM++ SDE", "DPM++ 2M SDE",
    "DPM++ 3M SDE", "DPM fast", "DPM adaptive", "LMS Karras", "DPM2 Karras", "DPM2 a Karras",
    "DPM++ 2S a Karras", "DPM++ 2M Karras", "DPM++ SDE Karras", "DPM++ 2M SDE Karras", "DDIM", "PLMS",
    "UniPC", "LCM", "Restart",
];
/// Schedulers of AUTOMATIC1111 / Forge.
pub const A1111_SCHEDULERS: &[&str] = &[
    "Automatic", "Uniform", "Karras", "Exponential", "Polyexponential", "SGM Uniform", "KL Optimal",
    "Align Your Steps", "Simple", "Normal", "DDIM", "Beta",
];

fn builtin(from_backend: &[String], list: &[&str]) -> Vec<String> {
    if from_backend.is_empty() {
        list.iter().map(|s| (*s).to_owned()).collect()
    } else {
        from_backend.to_vec()
    }
}

/// Sampler list: the backend's when read, else the built-in one.
pub fn samplers(backend: ImageBackend, from_backend: &[String]) -> Vec<String> {
    builtin(
        from_backend,
        match backend {
            ImageBackend::ComfyUi => COMFY_SAMPLERS,
            ImageBackend::Automatic1111 => A1111_SAMPLERS,
            _ => &[],
        },
    )
}

/// Scheduler list: the backend's when read, else the built-in one.
pub fn schedulers(backend: ImageBackend, from_backend: &[String]) -> Vec<String> {
    builtin(
        from_backend,
        match backend {
            ImageBackend::ComfyUi => COMFY_SCHEDULERS,
            ImageBackend::Automatic1111 => A1111_SCHEDULERS,
            _ => &[],
        },
    )
}

/// Folders of an install at `root`: ComfyUI (also the portable build, whose
/// models live in `ComfyUI_windows_portable/ComfyUI/models`) or A1111 / Forge.
/// `None` when no model folder is found.
pub fn detect_folders(root: &str, backend: ImageBackend) -> Option<ModelFolders> {
    let root = Path::new(root.trim());
    if root.as_os_str().is_empty() {
        return None;
    }
    let s = |p: PathBuf| p.to_string_lossy().into_owned();
    match backend {
        ImageBackend::ComfyUi => {
            let base = [root.join("models"), root.join("ComfyUI").join("models")]
                .into_iter()
                .find(|p| p.is_dir())?;
            Some(ModelFolders {
                checkpoints: s(base.join("checkpoints")),
                loras: s(base.join("loras")),
                embeddings: s(base.join("embeddings")),
                vaes: s(base.join("vae")),
            })
        }
        ImageBackend::Automatic1111 => {
            let base = [root.to_path_buf(), root.join("stable-diffusion-webui")]
                .into_iter()
                .find(|p| p.join("models").is_dir())?;
            Some(ModelFolders {
                checkpoints: s(base.join("models").join("Stable-diffusion")),
                loras: s(base.join("models").join("Lora")),
                embeddings: s(base.join("embeddings")),
                vaes: s(base.join("models").join("VAE")),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scan_tests").join(name);
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn touch(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    fn norm(s: &str) -> String {
        s.replace('\\', "/")
    }

    #[test]
    fn scans_recursively_with_forward_slashes() {
        let d = tmp("scan");
        touch(&d.join("a.safetensors"));
        touch(&d.join("sdxl/b.ckpt"));
        touch(&d.join("sdxl/deep/C.SAFETENSORS"));
        touch(&d.join("readme.txt"));
        touch(&d.join("x.safetensors.part"));
        let f = d.to_string_lossy().into_owned();
        let c = scan_folder(&f, Kind::Checkpoint, ImageBackend::ComfyUi);
        assert_eq!(c, ["a.safetensors", "sdxl/b.ckpt", "sdxl/deep/C.SAFETENSORS"]);
        assert!(scan_folder("", Kind::Lora, ImageBackend::ComfyUi).is_empty());
        assert!(scan_folder("Z:/no/such/dir", Kind::Lora, ImageBackend::ComfyUi).is_empty());
    }

    #[test]
    fn names_per_backend() {
        let d = tmp("names");
        touch(&d.join("style/x.safetensors"));
        touch(&d.join("e.bin"));
        let f = d.to_string_lossy().into_owned();
        assert_eq!(scan_folder(&f, Kind::Lora, ImageBackend::Automatic1111), ["e", "x"]);
        assert_eq!(scan_folder(&f, Kind::Lora, ImageBackend::ComfyUi), ["e.bin", "style/x.safetensors"]);
        // Embeddings: only .pt / .safetensors, no extension.
        assert_eq!(scan_folder(&f, Kind::Embedding, ImageBackend::ComfyUi), ["style/x"]);
    }

    #[test]
    fn merge_dedupes_and_marks() {
        let m = merge(&["b".into(), "a".into()], &["b".into(), "c".into()]);
        let names: Vec<_> = m.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert!(m[1].on_disk && m[1].on_backend);
        assert!(!m[2].on_disk && m[2].on_backend);
    }

    #[test]
    fn builtin_samplers_when_backend_silent() {
        assert!(samplers(ImageBackend::ComfyUi, &[]).contains(&"euler".to_owned()));
        assert!(samplers(ImageBackend::Automatic1111, &[]).contains(&"Euler a".to_owned()));
        assert_eq!(samplers(ImageBackend::ComfyUi, &["x".into()]), ["x"]);
        assert!(schedulers(ImageBackend::OpenAi, &[]).is_empty());
    }

    #[test]
    fn detects_layouts() {
        let d = tmp("detect_portable");
        fs::create_dir_all(d.join("ComfyUI/models/checkpoints")).unwrap();
        let f = detect_folders(&d.to_string_lossy(), ImageBackend::ComfyUi).unwrap();
        assert!(norm(&f.checkpoints).ends_with("ComfyUI/models/checkpoints"));
        assert!(norm(&f.vaes).ends_with("ComfyUI/models/vae"));
        let d2 = tmp("detect_a1111");
        fs::create_dir_all(d2.join("models")).unwrap();
        let f = detect_folders(&d2.to_string_lossy(), ImageBackend::Automatic1111).unwrap();
        assert!(norm(&f.loras).ends_with("models/Lora"));
        assert!(norm(&f.embeddings).ends_with("/embeddings"));
        assert!(detect_folders(&tmp("empty").to_string_lossy(), ImageBackend::ComfyUi).is_none());
        assert!(detect_folders("", ImageBackend::ComfyUi).is_none());
    }
}
