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
            ..Default::default()
        }],
        ..Default::default()
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

/// A skinned mesh is drawn through the pose of its scene node's rig, and a
/// pose change is picked up on the next render (CPU skinning cache).
#[test]
fn skinned_mesh_follows_the_rig_pose() {
    use orbit_scene::skin::{IDENTITY, Joint, Skeleton};
    let (device, queue) = gpu_or_skip!();
    let mut r = Renderer3d::new(&device, &queue);
    let mut mesh = quad();
    mesh.name = "skinquad".into();
    mesh.primitives[0].joints = vec![[0, 0, 0, 0]; 4];
    mesh.primitives[0].weights = vec![[1.0, 0.0, 0.0, 0.0]; 4];
    mesh.skin = Some(Skeleton {
        joints: vec![Joint {
            name: "root".into(),
            parent: None,
            rest: orbit_scene::Transform::IDENTITY,
            base: IDENTITY,
            inverse_bind: IDENTITY,
        }],
    });
    r.upload_mesh(&device, &mesh);
    let mut scene = scene_with_camera();
    let node = Node3d::new(
        "hero",
        None,
        Node3dKind::Mesh {
            mesh: MeshRef("skinquad".into()),
            materials: vec![MaterialRef("grey".into())],
        },
    );
    let id = node.id;
    scene.nodes.push(node);
    scene.materials.push(unlit("grey", [0.5, 0.5, 0.5, 1.0]));

    let px = render(&device, &queue, &mut r, &scene);
    assert_near(pixel(&px, SIZE / 2, SIZE / 2), [188, 188, 188, 255], 2);
    // Root bone moved out of the view: the background shows.
    scene.rig_mut(id).pose.set(
        "root",
        orbit_scene::Transform {
            translation: [10.0, 0.0, 0.0],
            ..orbit_scene::Transform::IDENTITY
        },
    );
    let px = render(&device, &queue, &mut r, &scene);
    assert_near(pixel(&px, SIZE / 2, SIZE / 2), [0, 0, 255, 255], 0);
    // Back to rest.
    scene.rig_mut(id).pose = Default::default();
    let px = render(&device, &queue, &mut r, &scene);
    assert_near(pixel(&px, SIZE / 2, SIZE / 2), [188, 188, 188, 255], 2);
    assert_eq!(r.last_draw_count(), 1);
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

// ---------------------------------------------------------------------------
// M6-04 shadows, M6-05 environment, M6-06 post-process.

use orbit_render3d::quality::{self, aces};
use orbit_render3d::{BloomSettings, PostSettings, QualityProfile, Tonemap};
use orbit_scene::Fog;

/// Whether the test adapter renders `Rgba16Float` (HDR profiles).
fn hdr_ok() -> bool {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .is_ok_and(|a| quality::hdr_supported(&a))
}

fn linear_to_srgb8(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

/// Directional sun travelling along (1, 0, -1)/sqrt(2): node rotated -45
/// degrees around Y.
const SUN_ROTATION: [f32; 4] = [0.0, -0.382_683_4, 0.0, 0.923_879_5];

fn shadow_scene(shadows: bool, occluder: MaterialDef) -> Scene3d {
    let mut scene = scene_with_camera();
    scene.environment.background = Background::Color([0.0; 3]);
    let mut white = MaterialDef::new("receiver");
    white.metallic = 0.0;
    white.roughness = 1.0;
    let mut receiver = Node3d::new(
        "receiver",
        None,
        Node3dKind::Mesh {
            mesh: MeshRef("quad".into()),
            materials: vec![MaterialRef("receiver".into())],
        },
    );
    receiver.transform.scale = [2.0; 3];
    scene.nodes.push(receiver);
    scene.materials.push(white);
    // Occluder covers x in [-1.5, -0.5] at z = 1; its shadow lands on
    // x in [-0.5, 0.5] of the receiver, visible to the camera.
    let mut occ = Node3d::new(
        "occluder",
        None,
        Node3dKind::Mesh {
            mesh: MeshRef("quad".into()),
            materials: vec![MaterialRef(occluder.name.clone())],
        },
    );
    occ.transform.translation = [-1.0, 0.0, 1.0];
    occ.transform.scale = [0.5; 3];
    scene.nodes.push(occ);
    scene.materials.push(occluder);
    let mut sun = Node3d::new(
        "sun",
        None,
        Node3dKind::Light(Light::Directional {
            color: [1.0; 3],
            intensity: std::f32::consts::PI,
            shadows,
        }),
    );
    sun.transform.rotation = SUN_ROTATION;
    scene.nodes.push(sun);
    scene
}

#[test]
fn directional_shadow_darkens_receiver() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let occluder = unlit("occ", [1.0, 0.0, 0.0, 1.0]);
    let shadowed = render(
        &device,
        &queue,
        &mut r,
        &shadow_scene(true, occluder.clone()),
    );
    assert!(r.last_shadow_active());
    let center = pixel(&shadowed, SIZE / 2, SIZE / 2);
    let lit = pixel(&shadowed, 26, SIZE / 2);
    assert!(center[0] < 20, "shadowed pixel {center:?}");
    assert!(lit[0] > 150, "lit pixel {lit:?}");
    // Same scene, light without shadows: the center is lit.
    let plain = render(
        &device,
        &queue,
        &mut r,
        &shadow_scene(false, occluder.clone()),
    );
    assert!(!r.last_shadow_active());
    assert!(pixel(&plain, SIZE / 2, SIZE / 2)[0] > 150);
    // Profile without shadow maps.
    r.set_quality(QualityProfile {
        shadows: None,
        ..QualityProfile::default()
    });
    let off = render(&device, &queue, &mut r, &shadow_scene(true, occluder));
    assert!(pixel(&off, SIZE / 2, SIZE / 2)[0] > 150);
}

#[test]
fn web_profile_shadow_with_pcf_and_blend_does_not_cast() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let mut web = QualityProfile::web();
    // Isolate shadows: neutral post.
    web.post = PostSettings::default();
    r.set_quality(web);
    let px = render(
        &device,
        &queue,
        &mut r,
        &shadow_scene(true, unlit("occ", [1.0, 0.0, 0.0, 1.0])),
    );
    assert!(pixel(&px, SIZE / 2, SIZE / 2)[0] < 20);
    // Shadow edge at x = 0.5 (pixel 20): a monotonic ramp into the light.
    let ramp: Vec<u8> = (17..24).map(|x| pixel(&px, x, SIZE / 2)[0]).collect();
    assert!(ramp.windows(2).all(|w| w[0] <= w[1]), "ramp {ramp:?}");
    let mut glass = unlit("glass", [1.0, 0.0, 0.0, 0.5]);
    glass.alpha_mode = AlphaMode::Blend;
    let px = render(&device, &queue, &mut r, &shadow_scene(true, glass));
    assert!(pixel(&px, SIZE / 2, SIZE / 2)[0] > 150);
}

const FACE_COLORS: [[u8; 4]; 6] = [
    [255, 0, 0, 255],   // +X
    [0, 255, 0, 255],   // -X
    [0, 0, 255, 255],   // +Y
    [255, 255, 0, 255], // -Y
    [255, 0, 255, 255], // +Z
    [0, 255, 255, 255], // -Z
];

fn face(name: &str, c: [u8; 4], size: u32) -> TextureAsset {
    TextureAsset {
        name: name.into(),
        width: size,
        height: size,
        rgba8: c.repeat((size * size) as usize),
    }
}

fn sky_scene(sky: &str, rotation: [f32; 4]) -> Scene3d {
    let mut scene = Scene3d::new("sky");
    scene.environment.background = Background::Sky(TextureRef(sky.into()));
    let mut cam = Node3d::new(
        "cam",
        None,
        Node3dKind::Camera(CameraDef {
            name: "main".into(),
            projection: Projection::Perspective { fov_y: 0.5 },
            near: 0.1,
            far: 20.0,
        }),
    );
    cam.transform.rotation = rotation;
    scene.nodes.push(cam);
    scene
}

const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// Rotated +90 degrees around X: looks towards +Y.
const LOOK_UP: [f32; 4] = [H, 0.0, 0.0, H];
/// Rotated -90 degrees around Y: looks towards +X.
const LOOK_RIGHT: [f32; 4] = [0.0, -H, 0.0, H];

#[test]
fn cubemap_sky_follows_view_direction() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let faces = std::array::from_fn(|i| face("f", FACE_COLORS[i], 4));
    assert!(r.add_cubemap(&device, &queue, "cube", faces));
    let c = SIZE / 2;
    let identity = [0.0, 0.0, 0.0, 1.0];
    let forward = render(&device, &queue, &mut r, &sky_scene("cube", identity));
    assert_near(pixel(&forward, c, c), FACE_COLORS[5], 1);
    let up = render(&device, &queue, &mut r, &sky_scene("cube", LOOK_UP));
    assert_near(pixel(&up, c, c), FACE_COLORS[2], 1);
    let right = render(&device, &queue, &mut r, &sky_scene("cube", LOOK_RIGHT));
    assert_near(pixel(&right, c, c), FACE_COLORS[0], 1);
    // Invalid faces are refused.
    let mut bad = std::array::from_fn(|i| face("f", FACE_COLORS[i], 4));
    bad[3] = face("f", [0; 4], 2);
    assert!(!r.add_cubemap(&device, &queue, "bad", bad));
}

#[test]
fn cubemap_face_orientation() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    // -Z face: top half red, bottom half green. Looking forward, the top of
    // the image must be red (rows are not flipped on any backend).
    let mut faces = std::array::from_fn(|_| face("f", [0, 0, 0, 255], 8));
    faces[5].rgba8 = (0..64)
        .flat_map(|i| {
            if i < 32 {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            }
        })
        .collect();
    r.add_cubemap(&device, &queue, "o", faces);
    let mut scene = sky_scene("o", [0.0, 0.0, 0.0, 1.0]);
    let px = render(&device, &queue, &mut r, &scene);
    assert_near(pixel(&px, SIZE / 2, 2), [255, 0, 0, 255], 1);
    assert_near(pixel(&px, SIZE / 2, SIZE - 3), [0, 255, 0, 255], 1);
    // Left/right: standard cubemap convention (GL / Vulkan / D3D spec
    // table, as exported by cubemap tools): the -Z face's s = 0 column is
    // +X, i.e. the face is seen mirrored from inside. `face_direction`
    // (used by the equirect conversion) must agree with the GPU.
    let mut faces = std::array::from_fn(|_| face("f", [0, 0, 0, 255], 8));
    faces[5].rgba8 = (0..64)
        .flat_map(|i| {
            if i % 8 < 4 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        })
        .collect();
    r.add_cubemap(&device, &queue, "lr", faces);
    scene.environment.background = Background::Sky(TextureRef("lr".into()));
    let px = render(&device, &queue, &mut r, &scene);
    let left_dir = orbit_render3d::environment::face_direction(5, 0.25, 0.5);
    assert!(left_dir[0] > 0.0);
    assert_near(pixel(&px, SIZE - 3, SIZE / 2), [255, 0, 0, 255], 1);
    assert_near(pixel(&px, 2, SIZE / 2), [0, 0, 255, 255], 1);
}

#[test]
fn equirect_panorama_is_not_mirrored() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    // Right half of the panorama's center (u in 0.5..0.75) red, rest blue:
    // looking forward (-Z), red must be on the right of the image.
    let (w, h) = (64u32, 32u32);
    let mut rgba8 = Vec::new();
    for _ in 0..h {
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            rgba8.extend_from_slice(if (0.5..0.75).contains(&u) {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 255]
            });
        }
    }
    r.add_texture(TextureAsset {
        name: "pano".into(),
        width: w,
        height: h,
        rgba8,
    });
    let px = render(
        &device,
        &queue,
        &mut r,
        &sky_scene("pano", [0.0, 0.0, 0.0, 1.0]),
    );
    assert_near(pixel(&px, SIZE - 3, SIZE / 2), [255, 0, 0, 255], 2);
    assert_near(pixel(&px, 2, SIZE / 2), [0, 0, 255, 255], 2);
}

#[test]
fn equirect_sky_is_converted_and_missing_sky_is_black() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    // Top half red sky, bottom half green ground.
    let mut rgba8 = Vec::new();
    for y in 0..16 {
        for _ in 0..32 {
            rgba8.extend_from_slice(if y < 8 {
                &[255, 0, 0, 255]
            } else {
                &[0, 255, 0, 255]
            });
        }
    }
    r.add_texture(TextureAsset {
        name: "pano".into(),
        width: 32,
        height: 16,
        rgba8,
    });
    let up = render(&device, &queue, &mut r, &sky_scene("pano", LOOK_UP));
    assert_near(pixel(&up, SIZE / 2, SIZE / 2), [255, 0, 0, 255], 2);
    let missing = render(&device, &queue, &mut r, &sky_scene("nope", LOOK_UP));
    assert_near(pixel(&missing, SIZE / 2, SIZE / 2), [0, 0, 0, 255], 0);
}

#[test]
fn sky_sh_ambient_lights_diffuse_surfaces() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let grey = std::array::from_fn(|_| face("g", [188, 188, 188, 255], 4));
    r.add_cubemap(&device, &queue, "grey", grey);
    let mut scene = scene_with_camera();
    scene.environment.background = Background::Sky(TextureRef("grey".into()));
    scene.environment.ambient_intensity = 1.0;
    let mut m = MaterialDef::new("white");
    m.metallic = 0.0;
    m.roughness = 1.0;
    add_quad(&mut scene, m, 0.0);
    let px = render(&device, &queue, &mut r, &scene);
    // Uniform sky of radiance L: diffuse ambient = albedo * L.
    assert_near(pixel(&px, SIZE / 2, SIZE / 2), [188, 188, 188, 255], 3);
    // The ambient color tints it.
    scene.environment.ambient_color = [1.0, 0.0, 0.0];
    let px = render(&device, &queue, &mut r, &scene);
    assert_near(pixel(&px, SIZE / 2, SIZE / 2), [188, 0, 0, 255], 3);
}

#[test]
fn linear_and_exponential_fog() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let mut scene = scene_with_camera();
    // Quad at distance 5 from the camera.
    add_quad(&mut scene, unlit("red", [1.0, 0.0, 0.0, 1.0]), 0.0);
    let green = [0.0, 1.0, 0.0];
    let c = SIZE / 2;
    scene.environment.fog = Some(Fog::Linear {
        color: green,
        start: 0.0,
        end: 1.0,
    });
    let px = render(&device, &queue, &mut r, &scene);
    assert_near(pixel(&px, c, c), [0, 255, 0, 255], 1);
    // Halfway: 50% linear mix.
    scene.environment.fog = Some(Fog::Linear {
        color: green,
        start: 4.0,
        end: 6.0,
    });
    let px = render(&device, &queue, &mut r, &scene);
    assert_near(pixel(&px, c, c), [188, 188, 0, 255], 2);
    // Exponential: 1 - exp(-d * density) = 0.5.
    scene.environment.fog = Some(Fog::Exponential {
        color: green,
        density: 2.0f32.ln() / 5.0,
    });
    let px = render(&device, &queue, &mut r, &scene);
    assert_near(pixel(&px, c, c), [188, 188, 0, 255], 2);
    // The background is not fogged.
    assert_near(pixel(&px, 0, 0), [0, 0, 255, 255], 0);
}

fn grey_quad_scene(level: f32) -> Scene3d {
    let mut scene = scene_with_camera();
    scene.environment.background = Background::Color([0.0; 3]);
    add_quad(&mut scene, unlit("grey", [level, level, level, 1.0]), 0.0);
    scene
}

fn post_profile(hdr: bool, post: PostSettings) -> QualityProfile {
    QualityProfile {
        hdr,
        post,
        ..QualityProfile::default()
    }
}

#[test]
fn aces_exposure_direct_and_hdr_paths_agree() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let post = PostSettings {
        tonemap: Tonemap::Aces,
        exposure: 2.0,
        bloom: None,
        vignette: 0.0,
    };
    let e = linear_to_srgb8(aces(0.5 * 2.0));
    let c = SIZE / 2;
    r.set_quality(post_profile(false, post));
    let direct = render(&device, &queue, &mut r, &grey_quad_scene(0.5));
    assert_near(pixel(&direct, c, c), [e, e, e, 255], 2);
    if !hdr_ok() {
        eprintln!("no Rgba16Float rendering: skipping HDR half");
        return;
    }
    r.set_quality(post_profile(true, post));
    let hdr = render(&device, &queue, &mut r, &grey_quad_scene(0.5));
    assert_near(pixel(&hdr, c, c), [e, e, e, 255], 2);
    // HDR keeps values above 1 until tonemapping.
    let bright = render(&device, &queue, &mut r, &grey_quad_scene(4.0));
    let one = render(&device, &queue, &mut r, &grey_quad_scene(1.0));
    assert!(pixel(&bright, c, c)[0] > pixel(&one, c, c)[0]);
}

#[test]
fn bloom_spreads_bright_pixels() {
    let (device, queue) = gpu_or_skip!();
    if !hdr_ok() {
        eprintln!("no Rgba16Float rendering: skipping bloom test");
        return;
    }
    let mut r = renderer(&device, &queue);
    let mut scene = scene_with_camera();
    scene.environment.background = Background::Color([0.0; 3]);
    let mut node = Node3d::new(
        "spark",
        None,
        Node3dKind::Mesh {
            mesh: MeshRef("quad".into()),
            materials: vec![MaterialRef("spark".into())],
        },
    );
    node.transform.scale = [0.25; 3];
    scene.nodes.push(node);
    scene.materials.push(unlit("spark", [8.0, 8.0, 8.0, 1.0]));
    let mut post = PostSettings {
        tonemap: Tonemap::Aces,
        exposure: 1.0,
        bloom: None,
        vignette: 0.0,
    };
    r.set_quality(post_profile(true, post));
    let plain = render(&device, &queue, &mut r, &scene);
    post.bloom = Some(BloomSettings {
        threshold: 1.0,
        intensity: 1.0,
    });
    r.set_quality(post_profile(true, post));
    let bloom = render(&device, &queue, &mut r, &scene);
    let (x, y) = (SIZE / 2 + 4, SIZE / 2);
    assert_eq!(pixel(&plain, x, y)[0], 0);
    assert!(
        pixel(&bloom, x, y)[0] > 20,
        "halo {:?}",
        pixel(&bloom, x, y)
    );
    assert_eq!(pixel(&bloom, 0, 0)[0], 0);
}

#[test]
fn vignette_darkens_corners_only() {
    let (device, queue) = gpu_or_skip!();
    let mut r = renderer(&device, &queue);
    let mut scene = scene_with_camera();
    scene.environment.background = Background::Color([1.0; 3]);
    let post = PostSettings {
        vignette: 0.5,
        ..PostSettings::default()
    };
    let mut profiles = vec![post_profile(false, post)];
    if hdr_ok() {
        profiles.push(post_profile(true, post));
    }
    for p in profiles {
        r.set_quality(p);
        let px = render(&device, &queue, &mut r, &scene);
        assert!(pixel(&px, SIZE / 2, SIZE / 2)[0] >= 250, "hdr {}", p.hdr);
        // 1 - 0.5 * r^2 with r^2 ~ 0.94 -> ~0.53 linear -> ~190 sRGB.
        let corner = pixel(&px, 0, 0)[0];
        assert!(
            (175..=205).contains(&corner),
            "hdr {} corner {corner}",
            p.hdr
        );
    }
}

/// Procedural shapes (`scene3d_version` 2) are generated by the renderer,
/// face the camera with their outer side (lit from the front; culled or
/// inverted faces would read black) and use their material.
#[test]
fn procedural_shapes_render_lit_from_the_front() {
    let (device, queue) = gpu_or_skip!();
    let mut r = Renderer3d::new(&device, &queue);
    for kind in orbit_scene::shapes::SHAPE_KINDS {
        let mut scene = scene_with_camera();
        scene.environment.background = Background::Color([0.0; 3]);
        let mut m = MaterialDef::new("white");
        m.metallic = 0.0;
        m.roughness = 1.0;
        scene.materials.push(m);
        let shape = orbit_scene::Shape::default_of(kind).unwrap();
        let mut node = Node3d::new(
            kind,
            None,
            Node3dKind::Shape {
                shape,
                material: Some(MaterialRef("white".into())),
            },
        );
        if kind == "plane" {
            // Stand the plane up (+Y -> +Z) so it faces the camera.
            let h = std::f32::consts::FRAC_1_SQRT_2;
            node.transform.rotation = [h, 0.0, 0.0, h];
        }
        scene.nodes.push(node);
        let sun = Light::Directional {
            color: [1.0; 3],
            intensity: std::f32::consts::PI * 0.5,
            shadows: false,
        };
        scene.nodes.push(Node3d::new("sun", None, Node3dKind::Light(sun)));
        let px = render(&device, &queue, &mut r, &scene);
        let c = pixel(&px, SIZE / 2, SIZE / 2);
        assert!(c[0] > 100, "{kind}: centre {c:?} is not lit");
        assert_near(pixel(&px, 0, 0), [0, 0, 0, 255], 0);
        assert!(r.has_mesh(&shape.mesh_key()), "{kind}");
    }
}
