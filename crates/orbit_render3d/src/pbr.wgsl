// Forward PBR metallic-roughness + unlit (ADR 0014). WebGL2 compatible:
// uniform buffers only, no storage, no compute, combined samplers.
// Output is linear; the target is *Srgb so encoding is done by the GPU.
// M6-04 directional shadow map + PCF, M6-05 sky / SH9 ambient / fog,
// M6-06 in-shader exposure + tonemap + vignette when not rendering HDR.

struct Light {
    position: vec4<f32>,   // xyz world pos, w type (0 dir, 1 point, 2 spot)
    direction: vec4<f32>,  // xyz travel direction, w range (0 = infinite)
    color: vec4<f32>,      // rgb color * intensity
    spot: vec4<f32>,       // x cos(inner), y cos(outer)
};

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    background: vec4<f32>, // rgb linear color, w 1 = sky cubemap
    counts: vec4<u32>,     // x lights, y shadow light index, z PCF radius
    shadow: vec4<f32>,     // x bias, y texel size, z enabled
    fog_color: vec4<f32>,  // rgb, w mode (0 none, 1 linear, 2 exponential)
    fog: vec4<f32>,        // start, end, density
    post: vec4<f32>,       // exposure, mode (0 raw HDR, 1 clamp, 2 ACES), vignette
    viewport: vec4<f32>,   // 1/w, 1/h
    sh: array<vec4<f32>, 9>,
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
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var env_cube: texture_cube<f32>;
@group(0) @binding(4) var env_sampler: sampler;
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

// Diffuse ambient: SH9 irradiance / pi (a flat ambient color is band 0).
fn sh_irradiance(n: vec3<f32>) -> vec3<f32> {
    var e = frame.sh[0].rgb * 0.282095;
    e = e + frame.sh[1].rgb * (0.488603 * n.y);
    e = e + frame.sh[2].rgb * (0.488603 * n.z);
    e = e + frame.sh[3].rgb * (0.488603 * n.x);
    e = e + frame.sh[4].rgb * (1.092548 * n.x * n.y);
    e = e + frame.sh[5].rgb * (1.092548 * n.y * n.z);
    e = e + frame.sh[6].rgb * (0.315392 * (3.0 * n.z * n.z - 1.0));
    e = e + frame.sh[7].rgb * (1.092548 * n.x * n.z);
    e = e + frame.sh[8].rgb * (0.546274 * (n.x * n.x - n.y * n.y));
    return max(e, vec3<f32>(0.0));
}

// Visibility of the shadow light, PCF over (2r+1)^2 taps. Uses the
// explicit-level compare (no derivatives, valid in non-uniform flow).
fn shadow_visibility(world_pos: vec3<f32>, n_dot_l: f32) -> f32 {
    let clip = frame.light_view_proj * vec4<f32>(world_pos, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }
    // Slope-scaled bias: grazing surfaces need more.
    let bias = frame.shadow.x * (1.0 + 2.0 * (1.0 - clamp(n_dot_l, 0.0, 1.0)));
    let depth = ndc.z - bias;
    let r = i32(min(frame.counts.z, 3u));
    var sum = 0.0;
    var taps = 0.0;
    for (var y = -r; y <= r; y = y + 1) {
        for (var x = -r; x <= r; x = x + 1) {
            let o = vec2<f32>(f32(x), f32(y)) * frame.shadow.y;
            sum = sum + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + o, depth);
            taps = taps + 1.0;
        }
    }
    return sum / taps;
}

fn apply_fog(c: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let mode = frame.fog_color.w;
    if (mode < 0.5) {
        return c;
    }
    let d = distance(frame.camera_pos.xyz, world_pos);
    var f: f32;
    if (mode < 1.5) {
        f = clamp((d - frame.fog.x) / max(frame.fog.y - frame.fog.x, 1e-4), 0.0, 1.0);
    } else {
        f = 1.0 - exp(-max(frame.fog.z, 0.0) * d);
    }
    return mix(c, frame.fog_color.rgb, f);
}

// ACES filmic (Narkowicz fit), mirrored by `quality::aces`.
fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Exposure, tonemapping and vignette when rendering straight to the LDR
// target; raw (non negative) HDR when a post pass follows.
fn finish(c: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    var x = max(c, vec3<f32>(0.0)) * frame.post.x;
    if (frame.post.y < 0.5) {
        return x;
    }
    if (frame.post.y > 1.5) {
        x = aces(x);
    }
    x = clamp(x, vec3<f32>(0.0), vec3<f32>(1.0));
    let d = (frag * frame.viewport.xy - vec2<f32>(0.5)) * 1.4142136;
    return x * clamp(1.0 - frame.post.z * dot(d, d), 0.0, 1.0);
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

    var visibility = 1.0;
    if (frame.shadow.z > 0.5) {
        let sl = frame.lights[min(frame.counts.y, 7u)];
        visibility = shadow_visibility(in.world_pos, dot(n, -sl.direction.xyz));
    }

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
        if (frame.shadow.z > 0.5 && i == frame.counts.y) {
            attenuation = attenuation * visibility;
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

    color = color + sh_irradiance(n) * diffuse_color * occlusion;
    color = color + material.emissive.rgb * emissive_sample;
    color = apply_fog(color, in.world_pos);
    return vec4<f32>(finish(color, in.clip.xy), albedo.a);
}

@fragment
fn fs_unlit(in: VsOut) -> @location(0) vec4<f32> {
    let c = base_color(textureSample(base_color_tex, material_sampler, in.uv));
    return vec4<f32>(finish(apply_fog(c.rgb, in.world_pos), in.clip.xy), c.a);
}

// Background: fullscreen triangle at the far plane, solid color or sky
// cubemap looked up by view direction.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = 1.0 - f32(vi & 2u) * 2.0;
    var out: SkyOut;
    out.clip = vec4<f32>(x, y, 1.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let far = frame.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let near = frame.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let dir = normalize(far.xyz / far.w - near.xyz / near.w);
    let sky = textureSampleLevel(env_cube, env_sampler, dir, 0.0).rgb;
    let c = select(frame.background.rgb, sky, frame.background.w > 0.5);
    return vec4<f32>(finish(c, in.clip.xy), 1.0);
}
