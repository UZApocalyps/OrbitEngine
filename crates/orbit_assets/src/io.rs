//! Platform I/O: worker threads natively, `fetch` on the web.

use crate::handle::Slot;
use crate::{AssetError, Source};

/// The pak entry for a requested path.
fn pak_path(prefix: &str, path: &str) -> String {
    if prefix.is_empty() {
        path.to_owned()
    } else {
        format!("{prefix}/{path}")
    }
}

fn decode_error(path: &str, message: String) -> AssetError {
    AssetError::Decode {
        path: path.to_owned(),
        message,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn_load<T, F>(source: Source, path: String, decode: F, slot: Slot<T>)
where
    T: Send + Sync + 'static,
    F: FnOnce(&str, Vec<u8>) -> Result<T, String> + Send + 'static,
{
    let job_path = path.clone();
    let spawned = std::thread::Builder::new()
        .name("orbit-asset-loader".to_owned())
        .spawn(move || {
            let path = job_path;
            let read = match &source {
                Source::Root(root) => {
                    let full = std::path::Path::new(&**root).join(&path);
                    std::fs::read(&full).map_err(|err| AssetError::Io {
                        path: path.clone(),
                        message: err.to_string(),
                    })
                }
                Source::Pak { pak, prefix } => pak.read_blocking(&pak_path(prefix, &path)),
            };
            match read {
                Err(err) => slot.fail(err),
                Ok(bytes) => match decode(&path, bytes) {
                    Ok(value) => slot.ready(value),
                    Err(msg) => slot.fail(decode_error(&path, msg)),
                },
            }
        });
    if let Err(err) = spawned {
        log::error!("could not spawn the asset loader thread for `{path}`: {err}");
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn_load<T, F>(source: Source, path: String, decode: F, slot: Slot<T>)
where
    T: Send + Sync + 'static,
    F: FnOnce(&str, Vec<u8>) -> Result<T, String> + Send + 'static,
{
    wasm_bindgen_futures::spawn_local(async move {
        let read = match &source {
            Source::Root(root) => {
                let url = format!("{}/{}", root.trim_end_matches('/'), path);
                fetch_bytes(&url).await.map_err(|message| AssetError::Io {
                    path: path.clone(),
                    message,
                })
            }
            Source::Pak { pak, prefix } => pak.read_async(&pak_path(prefix, &path)).await,
        };
        match read {
            Err(err) => slot.fail(err),
            Ok(bytes) => match decode(&path, bytes) {
                Ok(value) => slot.ready(value),
                Err(msg) => slot.fail(decode_error(&path, msg)),
            },
        }
    });
}

#[cfg(target_arch = "wasm32")]
async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let js_err = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let resp = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(js_err)?;
    let resp: web_sys::Response = resp.dyn_into().map_err(js_err)?;
    if !resp.ok() {
        return Err(format!("HTTP {} for {url}", resp.status()));
    }
    let buf = JsFuture::from(resp.array_buffer().map_err(js_err)?)
        .await
        .map_err(js_err)?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// Web only: like `spawn_load` for an image, but JPEG is decoded by the
/// browser (the Rust JPEG decoder is left out of the wasm to keep the bundle
/// small); PNG goes through `decode_image`.
#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn_load_image(source: Source, path: String, slot: Slot<crate::ImageData>) {
    wasm_bindgen_futures::spawn_local(async move {
        let read = match &source {
            Source::Root(root) => {
                let url = format!("{}/{}", root.trim_end_matches('/'), path);
                fetch_bytes(&url).await.map_err(|message| AssetError::Io {
                    path: path.clone(),
                    message,
                })
            }
            Source::Pak { pak, prefix } => pak.read_async(&pak_path(prefix, &path)).await,
        };
        match read {
            Err(err) => slot.fail(err),
            Ok(bytes) => {
                let decoded = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                    browser_decode(&bytes).await
                } else {
                    crate::decode_image(&bytes)
                };
                match decoded {
                    Ok(value) => slot.ready(value),
                    Err(msg) => slot.fail(decode_error(&path, msg)),
                }
            }
        }
    });
}

/// Decodes an encoded image with `createImageBitmap` and reads the pixels
/// back as non-premultiplied RGBA8 through an offscreen 2D canvas.
#[cfg(target_arch = "wasm32")]
async fn browser_decode(bytes: &[u8]) -> Result<crate::ImageData, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let js_err = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::of1(&array);
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).map_err(js_err)?;
    let options = web_sys::ImageBitmapOptions::new();
    options.set_premultiply_alpha(web_sys::PremultiplyAlpha::None);
    options.set_color_space_conversion(web_sys::ColorSpaceConversion::None);
    let bitmap = JsFuture::from(
        window
            .create_image_bitmap_with_blob_and_image_bitmap_options(&blob, &options)
            .map_err(js_err)?,
    )
    .await
    .map_err(js_err)?;
    let bitmap: web_sys::ImageBitmap = bitmap.dyn_into().map_err(js_err)?;
    let (width, height) = (bitmap.width(), bitmap.height());
    let canvas = web_sys::OffscreenCanvas::new(width, height).map_err(js_err)?;
    let ctx = canvas
        .get_context("2d")
        .map_err(js_err)?
        .ok_or("no 2d context")?
        .dyn_into::<web_sys::OffscreenCanvasRenderingContext2d>()
        .map_err(|o| format!("{o:?}"))?;
    ctx.draw_image_with_image_bitmap(&bitmap, 0.0, 0.0)
        .map_err(js_err)?;
    let data = ctx
        .get_image_data(0.0, 0.0, f64::from(width), f64::from(height))
        .map_err(js_err)?;
    bitmap.close();
    Ok(crate::ImageData {
        width,
        height,
        rgba: data.data().0,
    })
}
