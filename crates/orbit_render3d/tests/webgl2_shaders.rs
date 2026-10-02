//! Every shader entry point must validate and translate to GLSL ES 3.00
//! (WebGL2), the backend used by the web profile. No GPU needed.

use naga::back::glsl;
use naga::valid::{Capabilities, ValidationFlags, Validator};

fn check(name: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
    // Downlevel WebGL2: no extra capabilities.
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    let options = glsl::Options {
        version: glsl::Version::Embedded {
            version: 300,
            is_webgl: true,
        },
        ..Default::default()
    };
    assert!(!module.entry_points.is_empty());
    for ep in &module.entry_points {
        let pipeline = glsl::PipelineOptions {
            shader_stage: ep.stage,
            entry_point: ep.name.clone(),
            multiview: None,
        };
        let mut out = String::new();
        let mut writer = glsl::Writer::new(
            &mut out,
            &module,
            &info,
            &options,
            &pipeline,
            naga::proc::BoundsCheckPolicies::default(),
        )
        .unwrap_or_else(|e| panic!("{name}::{}: {e:?}", ep.name));
        writer
            .write()
            .unwrap_or_else(|e| panic!("{name}::{}: {e:?}", ep.name));
        assert!(out.starts_with("#version 300 es"), "{name}::{}", ep.name);
    }
}

#[test]
fn shaders_translate_to_glsl_es_300() {
    check("pbr", include_str!("../src/pbr.wgsl"));
    check("shadow", include_str!("../src/shadow.wgsl"));
    check("post", include_str!("../src/post.wgsl"));
}
