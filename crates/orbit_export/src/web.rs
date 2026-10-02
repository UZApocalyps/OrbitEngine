//! Web export: the wasm player template, the pak, a page with a loader.
//!
//! ```text
//! <game>-<version>-web/
//!   index.html            page + loader (real progress bar)
//!   orbit_player.js       wasm-bindgen glue          \  the player template
//!   orbit_player_bg.wasm  the player                 /
//!   game.pak              the game: index first, assets read on demand
//!   icon.png favicon.png orbit-logo.png
//!   *.br *.gz             precompressed index.html, .js, .wasm (optional)
//!   HOSTING.txt
//!   orbit-bundle.json     files written (so that a re-export cleans up)
//! ```
//!
//! The player reads `game.pak` with HTTP range requests: the index first,
//! then the assets of the scene on screen and of the scenes ahead, so a long
//! game starts after a few hundred KB (see ADR 0010).

use std::io::Write;
use std::path::{Path, PathBuf};

use orbit_pak::crc32;
use orbit_project::ExportProfile;

use crate::error::{ExportError, Result};
use crate::pakbuild::BuiltPak;
use crate::report::Reporter;
use crate::templates::WebTemplate;

const INDEX_HTML: &str = include_str!("../web/index.html");
const LOGO_DARK: &[u8] = include_bytes!("../../../assets/brand/logo-horizontal-dark.png");
const DEFAULT_ICON: &[u8] = include_bytes!("../../../assets/brand/icon-256.png");
const DEFAULT_FAVICON: &[u8] = include_bytes!("../../../assets/brand/icon-64.png");

const HOSTING: &str = "\
Hosting this game
=================

Upload the whole folder to any static web host (itch.io HTML5 zips, GitHub Pages,
Netlify, S3 + a CDN, your own nginx...). Open index.html through http(s):// :
browsers refuse WebAssembly and fetch() on file:// pages.

Requirements
* .wasm files are served as application/wasm (every common host does).
* The server answers HTTP Range requests for game.pak (they are what makes the
  loading lazy). A server that ignores them still works: the player then
  downloads the whole game.pak once.
* Optional: index.html, orbit_player.js and orbit_player_bg.wasm come with
  precompressed .br and .gz versions. Hosts that serve them for
  Accept-Encoding (nginx brotli_static / gzip_static, Caddy precompressed...)
  save bandwidth; otherwise delete the .br and .gz files.

Test locally: orbit export web --project <dir> --out <dir> --serve
";

/// File names of the bundle.
pub(crate) const MANIFEST: &str = "orbit-bundle.json";

/// `&`, `<`, `>`, `\"` escaped for HTML text and attributes.
pub(crate) fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `text` as a JavaScript string literal body (no quotes).
fn js_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn render_index(profile: &ExportProfile, pak_crc: u32, js_crc: u32, wasm_crc: u32, wasm_bytes: usize) -> String {
    let lang = profile
        .default_language
        .as_deref()
        .filter(|l| !l.trim().is_empty())
        .unwrap_or("en");
    INDEX_HTML
        .replace("@@LANG@@", &escape_html(lang))
        .replace("@@TITLE@@", &escape_html(&profile.game_name))
        .replace("@@FAVICON@@", "favicon.png")
        .replace("@@ICON@@", "icon.png")
        .replace("@@LOGO@@", "orbit-logo.png")
        .replace("@@PAK@@", &js_string(&format!("game.pak?v={pak_crc:08x}")))
        .replace("@@JS@@", &format!("orbit_player.js?v={js_crc:08x}"))
        .replace("@@WASM@@", &format!("orbit_player_bg.wasm?v={wasm_crc:08x}"))
        .replace("@@WASM_BYTES@@", &wasm_bytes.to_string())
}

/// Brotli (`.br`) of `data`.
pub(crate) fn brotli_bytes(data: &[u8]) -> Result<Vec<u8>> {
    let params = brotli::enc::BrotliEncoderParams {
        quality: 11,
        lgwin: 24,
        ..Default::default()
    };
    let mut out = Vec::with_capacity(data.len() / 3);
    brotli::BrotliCompress(&mut &data[..], &mut out, &params)
        .map_err(|e| ExportError::Package(format!("brotli: {e}")))?;
    Ok(out)
}

/// Gzip (`.gz`) of `data`.
pub(crate) fn gzip_bytes(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(data)
        .and_then(|()| encoder.finish())
        .map_err(|e| ExportError::Package(format!("gzip: {e}")))
}

pub(crate) struct WebWritten {
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
    pub wasm_bytes: u64,
    pub wasm_brotli_bytes: Option<u64>,
}

/// Name of the bundle folder.
pub(crate) fn bundle_dir_name(profile: &ExportProfile) -> String {
    format!(
        "{}-{}-web",
        profile.game_id,
        crate::native::file_safe(&profile.version).replace(' ', "_")
    )
}

/// Removes the files a previous export wrote in `dir` (listed in its
/// manifest); anything else in the folder is left alone.
fn clean_previous(dir: &Path) {
    let Ok(text) = std::fs::read_to_string(dir.join(MANIFEST)) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    for name in value["files"].as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
        if !name.contains("..") && !name.starts_with('/') && !name.contains('\\') {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
}

pub(crate) fn export(
    pak: &BuiltPak,
    icon: Option<&[u8]>,
    profile: &ExportProfile,
    template: &WebTemplate,
    out_dir: &Path,
    rep: &mut Reporter<'_>,
) -> Result<WebWritten> {
    let dir = out_dir.join(bundle_dir_name(profile));
    std::fs::create_dir_all(&dir).map_err(ExportError::io(&dir))?;
    clean_previous(&dir);

    rep.step(0.0, "Reading the web player template");
    let js = std::fs::read(&template.js).map_err(ExportError::io(&template.js))?;
    let wasm = std::fs::read(&template.wasm).map_err(ExportError::io(&template.wasm))?;
    let html = render_index(profile, pak.crc, crc32(&js), crc32(&wasm), wasm.len());

    // (name, bytes, precompress)
    let favicon = icon.unwrap_or(DEFAULT_FAVICON);
    let files: Vec<(&str, Vec<u8>, bool)> = vec![
        ("index.html", html.into_bytes(), true),
        ("orbit_player.js", js, true),
        ("orbit_player_bg.wasm", wasm, true),
        ("game.pak", pak.bytes.clone(), false),
        ("icon.png", icon.unwrap_or(DEFAULT_ICON).to_vec(), false),
        ("favicon.png", favicon.to_vec(), false),
        ("orbit-logo.png", LOGO_DARK.to_vec(), false),
        ("HOSTING.txt", HOSTING.as_bytes().to_vec(), false),
    ];
    let mut written: Vec<PathBuf> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut wasm_bytes = 0;
    let mut wasm_brotli_bytes = None;
    let total = files.len();
    for (i, (name, bytes, precompress)) in files.iter().enumerate() {
        rep.check_cancelled()?;
        let base = i as f32 / total as f32;
        rep.step(base, &format!("Writing {name}"));
        let path = dir.join(name);
        std::fs::write(&path, bytes).map_err(ExportError::io(&path))?;
        written.push(path);
        names.push((*name).to_owned());
        if *name == "orbit_player_bg.wasm" {
            wasm_bytes = bytes.len() as u64;
        }
        if *precompress && profile.web_precompress {
            rep.step(base, &format!("Compressing {name} (brotli, gzip)"));
            for (ext, packed) in [("br", brotli_bytes(bytes)?), ("gz", gzip_bytes(bytes)?)] {
                let packed_path = dir.join(format!("{name}.{ext}"));
                std::fs::write(&packed_path, &packed).map_err(ExportError::io(&packed_path))?;
                if *name == "orbit_player_bg.wasm" && ext == "br" {
                    wasm_brotli_bytes = Some(packed.len() as u64);
                }
                written.push(packed_path);
                names.push(format!("{name}.{ext}"));
            }
        }
    }
    let manifest = serde_json::json!({
        "format": 1,
        "game": profile.game_name,
        "id": profile.game_id,
        "version": profile.version,
        "files": names,
    });
    let manifest_path = dir.join(MANIFEST);
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap_or_default())
        .map_err(ExportError::io(&manifest_path))?;
    written.push(manifest_path);
    rep.step(1.0, "Web bundle written");
    Ok(WebWritten {
        dir,
        files: written,
        wasm_bytes,
        wasm_brotli_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_is_filled_in_and_escaped() {
        let mut profile = ExportProfile::new("release", "Tom & <Jerry> \"v2\"");
        profile.default_language = Some("fr".into());
        let html = render_index(&profile, 0xdead_beef, 1, 2, 12345);
        assert!(html.contains("<title>Tom &amp; &lt;Jerry&gt; &quot;v2&quot;</title>"));
        assert!(html.contains("lang=\"fr\""));
        assert!(html.contains("game.pak?v=deadbeef"));
        assert!(html.contains("const WASM_BYTES = 12345;"));
        assert!(!html.contains("@@"), "a placeholder was left in the page");
    }

    #[test]
    fn compression_round_trips() {
        let data = b"hello hello hello hello hello".repeat(50);
        let gz = gzip_bytes(&data).unwrap();
        assert!(gz.len() < data.len());
        let mut back = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&gz[..]), &mut back).unwrap();
        assert_eq!(back, data);
        let br = brotli_bytes(&data).unwrap();
        assert!(br.len() < data.len());
        let mut back = Vec::new();
        brotli::BrotliDecompress(&mut &br[..], &mut back).unwrap();
        assert_eq!(back, data);
    }
}
