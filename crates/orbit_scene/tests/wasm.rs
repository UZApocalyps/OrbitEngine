//! wasm smoke test (M6-13): the GPU-free 3D data path of the web player
//! (scene parsing, quality check, KTX2 decoding) runs in wasm32.
//!
//! ```text
//! CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!   cargo test -p orbit_scene --target wasm32-unknown-unknown \
//!   --no-default-features --features ktx2 --test wasm
//! ```
//!
//! Runs under Node (no browser needed); the CI `wasm` job does it.
#![cfg(target_arch = "wasm32")]

use orbit_scene::ktx2::{decode_rgba8, encode_simple};
use orbit_scene::quality::{AssetTable, QualityTier, check, scene_stats};
use orbit_scene::{Light, Node3d, Node3dKind, Scene3d, from_ron, to_ron};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn scene_round_trip_and_web_quality_check() {
    let mut scene = Scene3d::new("room");
    for _ in 0..6 {
        scene.nodes.push(Node3d::new(
            "lamp",
            None,
            Node3dKind::Light(Light::Directional {
                color: [1.0; 3],
                intensity: 1.0,
                shadows: true,
            }),
        ));
    }
    let text = to_ron(&scene).unwrap();
    let back = from_ron(&text).unwrap();
    assert_eq!(back, scene);
    let stats = scene_stats(&back, &AssetTable::default());
    assert_eq!(check(&stats, QualityTier::Web).len(), 2);
}

#[wasm_bindgen_test]
fn ktx2_zstd_decodes_in_wasm() {
    let px: Vec<u8> = (0..4 * 4 * 4).map(|i| i as u8).collect();
    let file = encode_simple(
        orbit_scene::ktx2::Format::R8G8B8A8_UNORM,
        4,
        4,
        &px,
        Some(orbit_scene::ktx2::SupercompressionScheme::Zstandard),
    );
    assert_eq!(decode_rgba8("t", &file).unwrap().rgba8, px);
}
