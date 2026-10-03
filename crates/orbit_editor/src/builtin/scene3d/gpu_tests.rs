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
