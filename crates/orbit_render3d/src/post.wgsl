// HDR post-process (M6-06): bloom bright pass, separable blur, composite
// (exposure, ACES, vignette) into the sRGB target. WebGL2 compatible.

struct Post {
    // xy: 1 / source size.
    texel: vec4<f32>,
    // x threshold, y bloom intensity, z exposure, w tonemap mode (1 clamp, 2 aces).
    params: vec4<f32>,
    // x vignette, yz blur direction.
    extra: vec4<f32>,
};

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var post_sampler: sampler;
@group(0) @binding(2) var tex_a: texture_2d<f32>;
@group(0) @binding(3) var tex_b: texture_2d<f32>;

struct FsIn {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> FsIn {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var out: FsIn;
    out.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fs_bright(in: FsIn) -> @location(0) vec4<f32> {
    let o = post.texel.xy * 0.5;
    var c = textureSample(tex_a, post_sampler, in.uv + vec2<f32>(-o.x, -o.y)).rgb;
    c = c + textureSample(tex_a, post_sampler, in.uv + vec2<f32>(o.x, -o.y)).rgb;
    c = c + textureSample(tex_a, post_sampler, in.uv + vec2<f32>(-o.x, o.y)).rgb;
    c = c + textureSample(tex_a, post_sampler, in.uv + vec2<f32>(o.x, o.y)).rgb;
    c = max(c * 0.25 * post.params.z, vec3<f32>(0.0));
    let l = luminance(c);
    let keep = max(l - post.params.x, 0.0) / max(l, 1e-4);
    return vec4<f32>(c * keep, 1.0);
}

@fragment
fn fs_blur(in: FsIn) -> @location(0) vec4<f32> {
    let step = post.extra.yz * post.texel.xy;
    var c = textureSample(tex_a, post_sampler, in.uv).rgb * 0.227027;
    c = c + textureSample(tex_a, post_sampler, in.uv + step * 1.384615).rgb * 0.316216;
    c = c + textureSample(tex_a, post_sampler, in.uv - step * 1.384615).rgb * 0.316216;
    c = c + textureSample(tex_a, post_sampler, in.uv + step * 3.230769).rgb * 0.070270;
    c = c + textureSample(tex_a, post_sampler, in.uv - step * 3.230769).rgb * 0.070270;
    return vec4<f32>(c, 1.0);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_composite(in: FsIn) -> @location(0) vec4<f32> {
    let hdr = textureSample(tex_a, post_sampler, in.uv).rgb;
    let bloom = textureSample(tex_b, post_sampler, in.uv).rgb;
    var c = max(hdr * post.params.z + bloom * post.params.y, vec3<f32>(0.0));
    if (post.params.w > 1.5) {
        c = aces(c);
    }
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let d = (in.uv - vec2<f32>(0.5)) * 1.4142136;
    c = c * clamp(1.0 - post.extra.x * dot(d, d), 0.0, 1.0);
    return vec4<f32>(c, 1.0);
}
