//! Android entry point (ADR 0017, M7-10). `android-activity` (GameActivity)
//! calls `android_main` on its own thread; the game is the pak stored as
//! `assets/game.pak` in the APK, read in place through `AAssetManager`, and
//! saves and preferences go to the app's internal storage.
//!
//! The library is built as a `cdylib` for Android, without changing the crate
//! type of desktop and web builds:
//! `cargo rustc -p orbit_player --lib --release --target aarch64-linux-android
//! --no-default-features --crate-type cdylib` (with the NDK linker set up,
//! e.g. by `cargo ndk`). APK packaging is M7-11.

use std::sync::Arc;

use orbit_assets::{PakLocation, PakRead, register_pak_scheme};
use orbit_platform::android::{self, AndroidApp};

use crate::Launch;

/// Scheme of the pak locations read from the APK `assets/`.
pub(crate) const ASSET_SCHEME: &str = "android-asset";
/// The pak of the game inside `assets/`.
pub(crate) const GAME_PAK: &str = "game.pak";

/// `assets/<name>` read through `AAssetManager`.
struct AssetPak {
    name: String,
}

impl PakRead for AssetPak {
    fn read_at(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        android::read_asset_range(&self.name, offset, len)
    }
}

/// What the Android player plays: the pak of the APK.
fn launch() -> Launch {
    Launch {
        pak: Some(PakLocation::file(format!("{ASSET_SCHEME}:{GAME_PAK}"))),
        data_dir: android::internal_data_dir().map(|p| p.to_string_lossy().into_owned()),
        ..Launch::default()
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android::init(app);
    crate::init_logging();
    register_pak_scheme(
        ASSET_SCHEME,
        Arc::new(|name: &str| {
            if !android::asset_exists(name) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no `assets/{name}` in the APK"),
                ));
            }
            Ok(Arc::new(AssetPak {
                name: name.to_owned(),
            }) as Arc<dyn PakRead>)
        }),
    );
    if let Err(err) = crate::run_launch(launch()) {
        log::error!("player failed: {err}");
    }
}
