// Full-screen transition between two textures. WebGL2-friendly.
//
// mode 0: crossfade (dissolve)           from -> to
// mode 1: fade through a solid color     from -> color -> to
// mode 2: mask dissolve with soft edge   pixels switch from -> to as `progress`
//         passes their mask luminance (dark pixels switch first)

struct Params {
    color: vec4<f32>,
    progress: f32,
    softness: f32,
    mode: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var from_tex: texture_2d<f32>;
@group(0) @binding(2) var to_tex: texture_2d<f32>;
@group(0) @binding(3) var mask_tex: texture_2d<f32>;
@group(0) @binding(4) var samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    // One oversized triangle covering the viewport.
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = pos[index];
    var out: VsOut;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>((p.x + 1.0) * 0.5, 1.0 - (p.y + 1.0) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Sample everything first: textureSample needs uniform control flow.
    let a = textureSample(from_tex, samp, in.uv).rgb;
    let b = textureSample(to_tex, samp, in.uv).rgb;
    let m = textureSample(mask_tex, samp, in.uv).r;
    let t = clamp(params.progress, 0.0, 1.0);

    var rgb: vec3<f32>;
    if (params.mode < 0.5) {
        rgb = mix(a, b, t);
    } else if (params.mode < 1.5) {
        if (t < 0.5) {
            rgb = mix(a, params.color.rgb, t * 2.0);
        } else {
            rgb = mix(params.color.rgb, b, (t - 0.5) * 2.0);
        }
    } else {
        let s = max(params.softness, 0.0001);
        let k = clamp((t * (1.0 + s) - m) / s, 0.0, 1.0);
        rgb = mix(a, b, k);
    }
    return vec4<f32>(rgb, 1.0);
}
