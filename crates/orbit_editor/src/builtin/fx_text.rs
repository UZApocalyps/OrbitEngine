//! Words for effects and timelines: list rows, summaries, target names.

use orbit_core::{AnimLoop, AnimProp, AnimTarget, Effect, Story, StoryTrack};
use orbit_editor_api::Translator;

pub fn effect_kind_key(effect: &Effect) -> &'static str {
    match effect {
        Effect::Move { .. } => "inspector.fx.kind.move",
        Effect::Shake { .. } => "inspector.fx.kind.shake",
        Effect::Zoom { .. } => "inspector.fx.kind.zoom",
        Effect::Flash { .. } => "inspector.fx.kind.flash",
        Effect::Tint { .. } => "inspector.fx.kind.tint",
    }
}

pub fn prop_key(prop: AnimProp) -> &'static str {
    match prop {
        AnimProp::Offset => "inspector.fx.prop.offset",
        AnimProp::Scale => "inspector.fx.prop.scale",
        AnimProp::Rotation => "inspector.fx.prop.rotation",
        AnimProp::Opacity => "inspector.fx.prop.opacity",
        AnimProp::Tint => "inspector.fx.prop.tint",
        AnimProp::Shake => "inspector.fx.prop.shake",
        AnimProp::Zoom => "inspector.fx.prop.zoom",
        AnimProp::Pan => "inspector.fx.prop.pan",
    }
}

/// "Camera", "Screen", "Background" or the character's name.
pub fn target_name(texts: &Translator, story: &Story, target: &AnimTarget) -> String {
    match target {
        AnimTarget::Camera => texts.get("inspector.fx.target.camera"),
        AnimTarget::Screen => texts.get("inspector.fx.target.screen"),
        AnimTarget::Background => texts.get("inspector.fx.target.background"),
        AnimTarget::Character(c) => story
            .character(*c)
            .and_then(|c| story.text(c.name_id))
            .unwrap_or("?")
            .to_owned(),
    }
}

fn hex(c: orbit_core::Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// `(kind, detail)` of an effect for a row: "Move" and "Alice  ·  +0.20, 0.00  ·  1 s".
pub fn effect_row(texts: &Translator, story: &Story, e: &Effect) -> (String, String) {
    let mut parts: Vec<String> = Vec::new();
    match e {
        Effect::Move { target, dx, dy, .. } => {
            parts.push(target_name(texts, story, target));
            parts.push(format!("{dx:+.2}, {dy:+.2}"));
        }
        Effect::Shake {
            target, amplitude, ..
        } => {
            parts.push(target_name(texts, story, target));
            parts.push(format!("{amplitude:.3}"));
        }
        Effect::Zoom { factor, .. } => parts.push(format!("×{factor:.2}")),
        Effect::Flash { color, .. } => parts.push(hex(*color)),
        Effect::Tint {
            target,
            color,
            strength,
            ..
        } => {
            parts.push(target_name(texts, story, target));
            parts.push(format!("{} {:.0} %", hex(*color), strength * 100.0));
        }
    }
    parts.push(format!("{} s", e.duration()));
    if e.wait() {
        parts.push(texts.get("inspector.fx.wait_short"));
    }
    (texts.get(effect_kind_key(e)), parts.join("  ·  "))
}

/// Detail of a timeline for a row: "3 s  ·  2 tracks  ·  loop ×2".
pub fn timeline_detail(
    texts: &Translator,
    duration: f32,
    looping: AnimLoop,
    wait: bool,
    tracks: &[StoryTrack],
) -> String {
    let mut parts = vec![
        format!("{duration} s"),
        texts.plural("inspector.fx.tracks", tracks.len(), &[]),
    ];
    match looping {
        AnimLoop::Once => {}
        AnimLoop::Loop(n) => parts.push(texts.format("inspector.fx.loop_times", &[("n", &n)])),
        AnimLoop::Forever => parts.push(texts.get("inspector.fx.loop.forever")),
        AnimLoop::PingPong => parts.push(texts.get("inspector.fx.loop.ping_pong")),
    }
    if wait {
        parts.push(texts.get("inspector.fx.wait_short"));
    }
    parts.join("  ·  ")
}
