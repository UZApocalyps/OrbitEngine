//! Headless GPU read-back tests. Skipped (with a message) when no adapter is
//! available, e.g. on CI machines without a GPU or software rasterizer.
//! The device is created with WebGL2 downlevel limits to mirror the web.

use orbit_gfx::wgpu;
use orbit_render3d::{CameraSelect, RenderTarget3d, Renderer3d};
use orbit_scene::{
    AlphaMode, Background, CameraDef, Light, MaterialDef, MaterialRef, MeshAsset, MeshRef, Node3d,
    Node3dKind, PrimitiveData, Projection, Scene3d, TextureAsset, TextureRef, TextureSlot,
};

const SIZE: u32 = 32;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("orbit3d test device"),
        required_limits:
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .ok()
}

macro_rules! gpu_or_skip {
    () => {
        match device() {
            Some(d) => d,
            None => {
                eprintln!("no GPU adapter: skipping 3D read-back test");
                return;
            }
        }
    };
}

fn quad() -> MeshAsset {
    MeshAsset {
        name: "quad".into(),
        primitives: vec![PrimitiveData {
            positions: vec![
                [-1.0, -1.0, 0.0],
                [1.0, -1.0, 0.0],
                [1.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            tangents: vec![],
            indices: vec![0, 1, 2, 0, 2, 3],
        }],
    }
}

fn scene_with_camera() -> Scene3d {
    let mut scene = Scene3d::new("test");
    scene.environment.background = Background::Color([0.0, 0.0, 1.0]);
    scene.environment.ambient_intensity = 0.0;
    let mut cam = Node3d::new(
        "cam",
        None,
        Node3dKind::Camera(CameraDef {
            name: "main".into(),
            projection: Projection::Orthographic { height: 4.0 },
            near: 0.1,
            far: 20.0,
        }),
    );
    cam.transform.translation = [0.0, 0.0, 5.0];
    scene.nodes.push(cam);
    scene
}

fn add_quad(scene: &mut Scene3d, material: MaterialDef, z: f32) {
    let mut node = Node3d::new(
        material.name.clone(),
        None,
        Node3dKind::Mesh {
            mesh: MeshRef("quad".into()),
            materials: vec![MaterialRef(material.name.clone())],
        },
    );
    node.transform.translation = [0.0, 0.0, z];
    scene.nodes.push(node);
    scene.materials.push(material);
}

fn unlit(name: &str, color: [f32; 4]) -> MaterialDef {
    let mut m = MaterialDef::new(name);
    m.unlit = true;
    m.base_color = color;
    m
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    r: &mut Renderer3d,
    scene: &Scene3d,
) -> Vec<u8> {
    let target = RenderTarget3d::new(device, SIZE, SIZE);
    r.render(device, queue, scene, CameraSelect::Named("main"), &target)
        .unwrap();
    target.read_rgba_blocking(device, queue).expect("read-back")
}

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}

fn assert_near(actual: [u8; 4], expected: [u8; 4], tol: u8) {
    for k in 0..4 {
        assert!(
            actual[k].abs_diff(expected[k]) <= tol,
            "pixel {actual:?} != {expected:?} (±{tol})"
        );
    }
}

fn renderer(device: &wgpu::Device, queue: &wgpu::Queue) -> Renderer3d {
    let mut r = Renderer3d::new(device, queue);
    r.upload_mesh(device, &quad());
    r
}

#[test]
fn unlit_linear_color_is_srgb_encoded_and_background_clears() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let mut scene = scene_with_camera();
    add_quad(&mut scene, unlit("grey", [0.5, 0.5, 0.5, 1.0]), 0.0);
    let px = render(&device, &queue, &mut r, &scene);
    // Linear 0.5 -> sRGB 188.
    assert_near(pixel(&px, SIZE / 2, SIZE / 2), [188, 188, 188, 255], 2);
    // Quad spans half the 4-unit view: corners show the background.
    assert_near(pixel(&px, 0, 0), [0, 0, 255, 255], 0);
    assert_eq!(r.last_draw_count(), 1);
}

#[test]
fn srgb_texture_roundtrips() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    r.add_texture(TextureAsset {
        name: "t".into(),
        width: 1,
        height: 1,
        rgba8: vec![128, 64, 200, 255],
    });
    let mut scene = scene_with_camera();
    let mut m = unlit("tex", [1.0; 4]);
    m.base_color_texture = Some(TextureSlot::new(TextureRef("t".into())));
    add_quad(&mut scene, m, 0.0);
    let px = render(&device, &queue, &mut r, &scene);
    assert_near(pixel(&px, SIZE / 2, SIZE / 2), [128, 64, 200, 255], 2);
}

#[test]
fn depth_buffer_keeps_nearest_surface_regardless_of_order() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let mut scene = scene_with_camera();
    add_quad(&mut scene, unlit("near", [1.0, 0.0, 0.0, 1.0]), 1.0);
    add_quad(&mut scene, unlit("far", [0.0, 1.0, 0.0, 1.0]), -1.0);
    let px = render(&device, &queue, &mut r, &scene);
    assert_near(pixel(&px, SIZE / 2, SIZE / 2), [255, 0, 0, 255], 1);
}

#[test]
fn alpha_blend_and_mask() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let mut scene = scene_with_camera();
    let mut glass = unlit("glass", [1.0, 0.0, 0.0, 0.5]);
    glass.alpha_mode = AlphaMode::Blend;
    add_quad(&mut scene, glass, 1.0);
    let mut cut = unlit("cut", [0.0, 1.0, 0.0, 0.2]);
    cut.alpha_mode = AlphaMode::Mask;
    add_quad(&mut scene, cut, 0.0);
    let px = render(&device, &queue, &mut r, &scene);
    // Mask discards the green quad; 50% red over blue in linear space.
    assert_near(pixel(&px, SIZE / 2, SIZE / 2), [188, 0, 188, 255], 3);
}

fn lit_scene(light: Light, light_pos: [f32; 3], rotation: [f32; 4]) -> Scene3d {
    let mut scene = scene_with_camera();
    scene.environment.background = Background::Color([0.0; 3]);
    let mut m = MaterialDef::new("white");
    m.metallic = 0.0;
    m.roughness = 1.0;
    add_quad(&mut scene, m, 0.0);
    let mut l = Node3d::new("light", None, Node3dKind::Light(light));
    l.transform.translation = light_pos;
    l.transform.rotation = rotation;
    scene.nodes.push(l);
    scene
}

#[test]
fn directional_light_lights_facing_surfaces_only() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let sun = Light::Directional {
        color: [1.0; 3],
        intensity: std::f32::consts::PI * 0.5,
        shadows: false,
    };
    // Identity rotation: the light travels along -Z, onto the +Z facing quad.
    let lit = render(
        &device,
        &queue,
        &mut r,
        &lit_scene(sun.clone(), [0.0; 3], [0.0, 0.0, 0.0, 1.0]),
    );
    let c = pixel(&lit, SIZE / 2, SIZE / 2);
    // Lambert: albedo/pi * pi/2 ~ 0.5 linear (minus Fresnel, plus a little
    // specular) -> around sRGB 180-195, grey.
    assert!((170..=205).contains(&c[0]), "lit pixel {c:?}");
    assert!(c[0].abs_diff(c[1]) <= 1 && c[0].abs_diff(c[2]) <= 1);
    // Rotated 180 degrees around Y: light travels +Z, from behind.
    let back = render(
        &device,
        &queue,
        &mut r,
        &lit_scene(sun, [0.0; 3], [0.0, 1.0, 0.0, 0.0]),
    );
    assert_near(pixel(&back, SIZE / 2, SIZE / 2), [0, 0, 0, 255], 0);
}

#[test]
fn point_light_attenuates_with_range() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let point = |range| Light::Point {
        color: [1.0; 3],
        intensity: 4.0,
        range,
    };
    let near = render(
        &device,
        &queue,
        &mut r,
        &lit_scene(point(None), [0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]),
    );
    let out_of_range = render(
        &device,
        &queue,
        &mut r,
        &lit_scene(point(Some(0.5)), [0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]),
    );
    let center = pixel(&near, SIZE / 2, SIZE / 2);
    let edge = pixel(&near, SIZE / 2 + SIZE / 4 - 1, SIZE / 2);
    assert!(
        center[0] > edge[0],
        "falloff: center {center:?} edge {edge:?}"
    );
    assert_near(pixel(&out_of_range, SIZE / 2, SIZE / 2), [0, 0, 0, 255], 0);
}

#[test]
fn missing_camera_is_an_error_and_missing_mesh_is_skipped() {
    let (device, queue) = gpu_or_skip!();
    let mut r = Renderer3d::new(&device, &queue);
    let mut scene = scene_with_camera();
    add_quad(&mut scene, unlit("x", [1.0; 4]), 0.0);
    let target = RenderTarget3d::new(&device, SIZE, SIZE);
    assert!(
        r.render(
            &device,
            &queue,
            &scene,
            CameraSelect::Named("nope"),
            &target
        )
        .is_err()
    );
    r.render(&device, &queue, &scene, CameraSelect::First, &target)
        .unwrap();
    assert_eq!(r.last_draw_count(), 0);
}
