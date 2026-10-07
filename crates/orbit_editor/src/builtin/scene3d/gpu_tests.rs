//! Headless GPU read-back of what the 3D editor draws. Skipped (with a
//! message) when no adapter is available.

use orbit_gfx::wgpu;
use orbit_render3d::{CameraSelect, RenderTarget3d, Renderer3d, math};
use orbit_scene::{MaterialDef, Projection};

use super::geom;
use super::model;
use super::panels::{PREVIEW_MESH, preview_scene};

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

fn px(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}

#[test]
fn preview_sphere_and_edit_camera_render() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping the 3D editor render test");
        return;
    };
    let mut r = Renderer3d::new(&device, &queue);
    r.upload_mesh(&device, &model::preview_sphere(PREVIEW_MESH));
    let mut m = MaterialDef::new("Red");
    m.base_color = [1.0, 0.1, 0.1, 1.0];
    m.metallic = 0.0;
    m.roughness = 0.6;
    let scene = preview_scene(&m);
    let target = RenderTarget3d::new_viewable(&device, 64, 64);
    let _gamma = target.gamma_view();
    let cam = orbit_render3d::Camera3d {
        world: math::from_trs([0.0, 0.0, 3.4], [0.0, 0.0, 0.0, 1.0], [1.0; 3]),
        projection: Projection::Perspective {
            fov_y: 30f32.to_radians(),
        },
        near: 0.1,
        far: 20.0,
    };
    r.render(&device, &queue, &scene, CameraSelect::View(cam), &target)
        .unwrap();
    let rgba = target
        .read_rgba_blocking(&device, &queue)
        .expect("read-back");
    let center = px(&rgba, 64, 32, 32);
    let corner = px(&rgba, 64, 2, 2);
    assert!(
        center[0] > center[1] && center[0] > 40,
        "red sphere: {center:?}"
    );
    assert_ne!(center, corner, "the sphere stands out of the background");

    // A fresh scene seen through the edit camera renders (the background shows).
    let doc = model::SceneDoc::new("t");
    let edit = geom::EditCamera::default();
    r.render(
        &device,
        &queue,
        doc.scene(),
        CameraSelect::View(edit.camera3d()),
        &target,
    )
    .unwrap();
    let rgba = target
        .read_rgba_blocking(&device, &queue)
        .expect("read-back");
    assert!(px(&rgba, 64, 5, 5)[2] > 0, "background color drawn");
}

#[test]
fn capture_png_renders_a_named_camera() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping the 3D capture test");
        return;
    };
    let doc = model::SceneDoc::new("t");
    let assets = crate::scene3d_host::ModelAssets::default();
    let png = crate::scene3d_host::capture_png(
        &device,
        &queue,
        doc.scene(),
        &assets,
        Some("main"),
        [64, 48],
    )
    .expect("capture");
    let img = image::load_from_memory(&png).expect("a PNG").to_rgba8();
    assert_eq!(img.dimensions(), (64, 48));
    // The new scene's background colour is drawn.
    assert!(img.get_pixel(3, 3)[2] > 0);
    // A camera the scene does not have is an error, not a black picture.
    let err = crate::scene3d_host::capture_png(
        &device,
        &queue,
        doc.scene(),
        &assets,
        Some("nope"),
        [64, 48],
    )
    .unwrap_err();
    assert!(err.contains("nope"), "{err}");
}

/// The first viewport frame used to build the 3D renderer and compile its
/// shaders on the UI thread; the host now prewarms it on a worker
/// (`Scene3dHost::prewarm`). Frame budget of the first render, prewarmed.
#[test]
fn a_prewarmed_renderer_draws_its_first_frame_within_budget() {
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping the 3D prewarm test");
        return;
    };
    let mut m = MaterialDef::new("Red");
    m.base_color = [1.0, 0.1, 0.1, 1.0];
    let scene = preview_scene(&m);
    let cam = geom::EditCamera::default().camera3d();
    let target = RenderTarget3d::new_viewable(&device, 256, 256);
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    let budget = std::time::Duration::from_millis(if cfg!(debug_assertions) { 150 } else { 50 });

    // Before: everything on the first frame.
    let t = std::time::Instant::now();
    let mut cold = Renderer3d::new(&device, &queue);
    cold.upload_mesh(&device, &model::preview_sphere(PREVIEW_MESH));
    cold.render(&device, &queue, &scene, CameraSelect::View(cam.clone()), &target).unwrap();
    let before = t.elapsed();

    // After: built and prewarmed on a worker, then the first frame.
    let (d, q) = (device.clone(), queue.clone());
    let worker = std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let mut r = Renderer3d::new(&d, &q);
        r.prewarm(&d, &q);
        (r, t.elapsed())
    });
    let (mut warm, prewarm) = worker.join().unwrap();
    let t = std::time::Instant::now();
    warm.upload_mesh(&device, &model::preview_sphere(PREVIEW_MESH));
    warm.render(&device, &queue, &scene, CameraSelect::View(cam), &target).unwrap();
    let after = t.elapsed();
    eprintln!(
        "3D viewport first frame ({}): before {:.1} ms, after {:.1} ms (prewarm on a worker: {:.1} ms)",
        if cfg!(debug_assertions) { "debug" } else { "release" },
        ms(before),
        ms(after),
        ms(prewarm)
    );
    assert!(after <= budget, "first 3D frame took {:.1} ms", ms(after));
}
