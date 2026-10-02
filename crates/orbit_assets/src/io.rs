//! Platform I/O: worker threads natively, `fetch` on the web.

use std::sync::Arc;

use crate::AssetError;
use crate::handle::Slot;

fn decode_error(path: &str, message: String) -> AssetError {
    AssetError::Decode {
        path: path.to_owned(),
        message,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn_load<T, F>(root: Arc<str>, path: String, decode: F, slot: Slot<T>)
where
    T: Send + Sync + 'static,
    F: FnOnce(&str, Vec<u8>) -> Result<T, String> + Send + 'static,
{
    let job_path = path.clone();
    let spawned = std::thread::Builder::new()
        .name("orbit-asset-loader".to_owned())
        .spawn(move || {
            let path = job_path;
            let full = std::path::Path::new(&*root).join(&path);
            match std::fs::read(&full) {
                Err(err) => slot.fail(AssetError::Io {
                    path,
                    message: err.to_string(),
                }),
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
pub(crate) fn spawn_load<T, F>(root: Arc<str>, path: String, decode: F, slot: Slot<T>)
where
    T: Send + Sync + 'static,
    F: FnOnce(&str, Vec<u8>) -> Result<T, String> + Send + 'static,
{
    wasm_bindgen_futures::spawn_local(async move {
        let url = format!("{}/{}", root.trim_end_matches('/'), path);
        match fetch_bytes(&url).await {
            Err(message) => slot.fail(AssetError::Io { path, message }),
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
