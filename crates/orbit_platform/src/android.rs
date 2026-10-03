//! Android glue (ADR 0017, `target_os = "android"` only): the `AndroidApp`
//! handed to `android_main` by `android-activity` (GameActivity), the app's
//! internal storage, the APK `assets/` read through `AAssetManager`, and the
//! safe area.
//!
//! The player calls [`init`] first thing in its `android_main`; [`crate::run`]
//! then builds the event loop on that app.

use std::ffi::CString;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::OnceLock;

pub use winit::platform::android::activity::AndroidApp;

use crate::SafeArea;

static APP: OnceLock<AndroidApp> = OnceLock::new();

/// Records the app given to `android_main`. Later calls are ignored (the
/// process keeps one `AndroidApp`).
pub fn init(app: AndroidApp) {
    let _ = APP.set(app);
}

/// The app recorded by [`init`].
pub fn app() -> Option<&'static AndroidApp> {
    APP.get()
}

/// The app's private storage (`/data/data/<package>/files`): saves and
/// preferences live there.
pub fn internal_data_dir() -> Option<PathBuf> {
    app()?.internal_data_path()
}

/// Reads `len` bytes at `offset` of `assets/<name>` in the APK. The asset is
/// opened for each read (an `AAsset` must not be shared between threads, the
/// `AAssetManager` may), so loader threads never share a cursor. Store paks
/// uncompressed in the APK so seeking does not inflate from the start.
pub fn read_asset_range(name: &str, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let app =
        app().ok_or_else(|| std::io::Error::other("orbit_platform::android::init not called"))?;
    let c_name = CString::new(name).map_err(std::io::Error::other)?;
    let mut asset = app.asset_manager().open(&c_name).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, format!("asset `{name}`"))
    })?;
    asset.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; len];
    asset.read_exact(&mut buf)?;
    Ok(buf)
}

/// Whether `assets/<name>` exists in the APK.
pub fn asset_exists(name: &str) -> bool {
    let (Some(app), Ok(c_name)) = (app(), CString::new(name)) else {
        return false;
    };
    app.asset_manager().open(&c_name).is_some()
}

/// The safe area of a `width` x `height` window: what the system bars and the
/// cutout leave of it.
pub fn safe_area(width: u32, height: u32) -> SafeArea {
    let Some(app) = app() else {
        return SafeArea::default();
    };
    let rect = app.content_rect();
    SafeArea::from_content_rect(
        width,
        height,
        (rect.left, rect.top, rect.right, rect.bottom),
    )
}
