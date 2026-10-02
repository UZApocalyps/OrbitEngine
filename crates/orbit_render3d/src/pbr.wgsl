// Forward PBR metallic-roughness + unlit (ADR 0014). WebGL2 compatible:
// uniform buffers only, no storage, no compute, combined samplers.
// Output is linear; the target is *Srgb so encoding is done by the GPU.

struct Light {
    position: vec4<f32>,   // xyz world pos, w type (0 dir, 1 point, 2 spot)
    direction: vec4<f32>,  // xyz travel direction, w range (0 = infinite)
    color: vec4<f32>,      // rgb color * intensity
    spot: vec4<f32>,       // x cos(inner), y cos(outer)
};

struct Frame {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    ambient: vec4<f32>,
    counts: vec4<u32>,
    lights: array<Light, 8>,
};

struct Object {
    model: mat4x4<f32>,
    normal: mat4x4<f32>,
};

struct Material {
    base_color: vec4<f32>,
    emissive: vec4<f32>,
    params: vec4<f32>,   // metallic, roughness, alpha cutoff, occlusion strength
    flags: vec4<u32>,    // alpha mode, unlit, double sided
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> object: Object;
@group(2) @binding(0) var<uniform> material: Material;
@group(2) @binding(1) var material_sampler: sampler;
@group(2) @binding(2) var base_color_tex: texture_2d<f32>;
@group(2) @binding(3) var orm_tex: texture_2d<f32>;
@group(2) @binding(4) var emissive_tex: texture_2d<f32>;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let world = object.model * vec4<f32>(in.position, 1.0);
    out.world_pos = world.xyz;
    out.normal = (object.normal * vec4<f32>(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    out.clip = frame.view_proj * world;
    return out;
}

const PI: f32 = 3.14159265;

// `sampled` is fetched by the caller first: every textureSample happens in
// uniform control flow, before any discard.
fn base_color(sampled: vec4<f32>) -> vec4<f32> {
    let c = material.base_color * sampled;
    // Alpha mask: discard below the cutoff, then treat as opaque.
    if (material.flags.x == 1u) {
        if (c.a < material.params.z) {
            discard;
        }
        return vec4<f32>(c.rgb, 1.0);
    }
    if (material.flags.x == 0u) {
        return vec4<f32>(c.rgb, 1.0);
    }
    return c;
}

fn distribution_ggx(n_dot_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / max(PI * d * d, 1e-6);
}

fn visibility_smith(n_dot_v: f32, n_dot_l: f32, alpha: f32) -> f32 {
    // Height-correlated Smith-GGX (glTF spec, appendix B).
    let a2 = alpha * alpha;
    let gv = n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2);
    let gl = n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-6);
}

fn fresnel_schlick(v_dot_h: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - v_dot_h, 5.0);
}

fn range_attenuation(dist: f32, range: f32) -> f32 {
    if (range <= 0.0) {
        return 1.0;
    }
    let r = dist / range;
    let f = clamp(1.0 - r * r * r * r, 0.0, 1.0);
    return f * f;
}

@fragment
fn fs_pbr(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base_sample = textureSample(base_color_tex, material_sampler, in.uv);
    let orm = textureSample(orm_tex, material_sampler, in.uv);
    let emissive_sample = textureSample(emissive_tex, material_sampler, in.uv).rgb;
    let albedo = base_color(base_sample);
    let metallic = clamp(material.params.x * orm.b, 0.0, 1.0);
    let roughness = clamp(material.params.y * orm.g, 0.04, 1.0);
    let occlusion = 1.0 + material.params.w * (orm.r - 1.0);
    let alpha = roughness * roughness;

    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let v = normalize(frame.camera_pos.xyz - in.world_pos);
    let n_dot_v = max(dot(n, v), 1e-4);
    let f0 = mix(vec3<f32>(0.04), albedo.rgb, metallic);
    let diffuse_color = albedo.rgb * (1.0 - metallic);

    var color = vec3<f32>(0.0);
    let count = min(frame.counts.x, 8u);
    for (var i = 0u; i < count; i = i + 1u) {
        let light = frame.lights[i];
        var l: vec3<f32>;
        var attenuation = 1.0;
        if (light.position.w < 0.5) {
            l = -light.direction.xyz;
        } else {
            let to_light = light.position.xyz - in.world_pos;
            let dist = length(to_light);
            l = to_light / max(dist, 1e-4);
            attenuation = range_attenuation(dist, light.direction.w) / max(dist * dist, 1e-4);
            if (light.position.w > 1.5) {
                let cd = dot(light.direction.xyz, -l);
                attenuation = attenuation
                    * smoothstep(light.spot.y, max(light.spot.x, light.spot.y + 1e-4), cd);
            }
        }
        let n_dot_l = dot(n, l);
        if (n_dot_l <= 0.0) {
            continue;
        }
        let h = normalize(l + v);
        let n_dot_h = max(dot(n, h), 0.0);
        let v_dot_h = max(dot(v, h), 0.0);
        let f = fresnel_schlick(v_dot_h, f0);
        let specular = f * distribution_ggx(n_dot_h, alpha) * visibility_smith(n_dot_v, n_dot_l, alpha);
        let diffuse = (vec3<f32>(1.0) - f) * diffuse_color / PI;
        color = color + (diffuse + specular) * light.color.rgb * (n_dot_l * attenuation);
    }

    color = color + frame.ambient.rgb * diffuse_color * occlusion;
    color = color + material.emissive.rgb * emissive_sample;
    // No tonemapping yet (M6-06): clamp so the sRGB target stays valid.
    return vec4<f32>(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), albedo.a);
}

@fragment
fn fs_unlit(in: VsOut) -> @location(0) vec4<f32> {
    let c = base_color(textureSample(base_color_tex, material_sampler, in.uv));
    return vec4<f32>(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
