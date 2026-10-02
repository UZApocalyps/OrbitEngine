// Depth-only directional shadow pass (M6-04). WebGL2 compatible.

struct ShadowFrame {
    light_view_proj: mat4x4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    normal: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> shadow: ShadowFrame;
@group(1) @binding(0) var<uniform> object: Object;

@vertex
fn vs_shadow(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return shadow.light_view_proj * object.model * vec4<f32>(position, 1.0);
}
