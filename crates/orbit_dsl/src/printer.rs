//! IR → text.
//!
//! # Grammar
//!
//! One statement per line; indentation is cosmetic. `#` starts a comment,
//! except `#@<ULID>`, the id annotation that ends a node or scene line. A
//! line without annotation is a new node (or scene). Names (`<name>`) are
//! bare words when they are identifiers that are not keywords, quoted
//! strings otherwise. A reference (`<ref>`) is a name or `@<ULID>`; the
//! printer falls back to `@<ULID>` whenever a name would be ambiguous.
//!
//! ```text
//! scene <name>                                   a scene; nodes follow
//! label <name>                                   NodeKind::Label
//! "text"                                         narrator line (Say)
//! <speaker> "text"                               Say; speaker a bare name
//! say <ref> "text"                               Say, any speaker reference
//! menu                                           Choice; options follow:
//!     "text" -> <label-ref> [if <expr>]
//! jump <label-ref>                               Jump
//! jump_scene <scene-ref> [label <label-ref>]     JumpScene
//! $ <var> = <expr>                               SetVar
//! $ <var> += <operand>  (also -= *= /=)          SetVar, no-code VarOp form
//! if <expr> -> <label-ref> [else <label-ref>]    If
//! bg <asset-ref> [with <transition>]             Background
//! show <char-ref> <expression> at <position>     Show, then any of:
//!     [scale <f>] [scale_y <f>] [flip_x] [flip_y] [with <transition>]
//! hide <char-ref> [with <transition>]            Hide
//! music <asset-ref> [fade <f>] [loop]            PlayMusic
//! stop music [fade <f>]                          StopMusic
//! sound <asset-ref>                              PlaySound
//! wait <f>                                       Wait
//! call <label-ref>                               Call
//! return | end                                   Return / End
//! custom "<plugin>" <kind> [version <n>] [data <value>]   Custom
//! scene3d <asset-ref> [camera "<name>"] [with <transition>]   ShowScene3d
//! camera3d "<name>" [with <transition>]          Camera3d
//! hide_scene3d [with <transition>]               HideScene3d
//! open screen "<id>" [wait] [-> <var>] [with <p> = <expr>, ...]   OpenScreen
//! close screen "<id>"                            CloseScreen
//! move <target> by (<dx>, <dy>) over <s> [ease <curve>] [wait]      Effect::Move
//! shake <target> <amplitude> over <s> [ease <curve>] [wait]         Effect::Shake
//! zoom <factor> over <s> [ease <curve>] [wait]                      Effect::Zoom
//! flash "#rrggbb" over <s> [ease <curve>] [wait]                    Effect::Flash
//! tint <target> "#rrggbb" <strength> over <s> [ease <curve>] [wait] Effect::Tint
//! timeline over <s> [loop <n> | forever | pingpong] [wait]          Timeline; tracks follow:
//!     track <target> <prop> <t> <value> [ease <curve>], <t> <value> ...
//! bg_layers [with <transition>]                  BackgroundLayers (parallax); layers follow:
//!     layer <asset-ref> depth <f> [repeat_x]
//! particles "<name>" <preset> [<key> <value> ...]   Particles (an emitter)
//! stop particles ["<name>"]                      StopParticles
//! script "<rhai source>" [branch <name> -> <label-ref> ...] [on_error <label-ref>]   Script
//! ```
//!
//! A Rhai condition is the expression `rhai "<code>"` (`if rhai "…" -> l`,
//! menu option `… if rhai "…"`). Script sources are one quoted string:
//! their newlines are escaped (`\n`), like every DSL string.
//!
//! `particles` keys, all optional after the preset (`rain`, `snow`, `petals`,
//! `dust`, `sparks` or `custom`, whose values they override), and always
//! written by the printer: `seed <n> rate <f> lifetime <f> jitter <f>
//! spawn (x0, y0, x1, y1) direction <deg> spread <deg> speed (min, max)
//! gravity (x, y) sway (amplitude, hz) size (min, max) stretch <f> spin <f>
//! color "#rrggbb" to "#rrggbb" opacity <f> fade <f> [sprite <asset-ref>]
//! front|behind prewarm|cold`.
//!
//! `<target>` is `camera`, `screen`, `background` or a character reference.
//! `<prop>` is `offset`, `scale`, `pan` (values `(x, y)`), `rotation` (degrees),
//! `opacity`, `shake`, `zoom` (numbers) or `tint` (`rgba(r, g, b, a)`). A key's
//! `ease` shapes the segment towards the next key.
//! `<curve>` is a preset name (`OutQuad`), `bezier(x1, y1, x2, y2)` or a
//! quoted named curve.
//!
//! `<position>` is `left`, `center`, `right`, a quoted custom position name
//! or `(x, y)`. `<transition>` is `dissolve(<seconds>[, ease <curve>])` or
//! `fade(<seconds>, "#rrggbb"[, ease <curve>])`. Characters are named by their display
//! name, assets by their manifest path, scenes by their name and labels by
//! their name within the scene. Custom data is JSON-like (`null`, `true`,
//! numbers, strings, `[…]`, `{"key": …}`; floats always carry a `.` or an
//! exponent).

use std::collections::HashMap;

use orbit_core::{
    Angle, AnimLoop, AnimTarget, AnimValue as Value, AssetId, CharacterId, Color, CustomData,
    Curve, Effect, Expr, Node, NodeId, NodeKind, OpKind, Position, Rgba, Scene, SceneId, Story,
    StoryTrack, TrackData, Transition, TransitionKind, Vec2, VarOp,
};

use crate::expr::{fmt_f32, fmt_f64, print_expr};
use crate::lexer::{is_ident, quote};

/// Words with a meaning at the start of a line or inside a statement. A name
/// equal to one of them is always quoted.
pub const KEYWORDS: &[&str] = &[
    "scene",
    "label",
    "say",
    "menu",
    "jump",
    "jump_scene",
    "if",
    "else",
    "bg",
    "show",
    "hide",
    "music",
    "stop",
    "sound",
    "wait",
    "call",
    "return",
    "end",
    "custom",
    "script",
    "scene3d",
    "camera3d",
    "hide_scene3d",
    "open",
    "close",
    "at",
    "with",
    "scale",
    "scale_y",
    "flip_x",
    "flip_y",
    "fade",
    "loop",
    "version",
    "data",
    "left",
    "center",
    "right",
    "dissolve",
    "true",
    "false",
    "null",
    "move",
    "shake",
    "zoom",
    "flash",
    "tint",
    "timeline",
    "track",
    "bg_layers",
    "layer",
    "particles",
    "by",
    "over",
    "ease",
    "forever",
    "pingpong",
    "camera",
    "screen",
    "background",
    "bezier",
    "rgba",
];

/// A name as written in the DSL: bare when possible, quoted otherwise.
pub(crate) fn fmt_name(s: &str) -> String {
    if is_ident(s) && !KEYWORDS.contains(&s) {
        s.to_owned()
    } else {
        quote(s)
    }
}

/// Name lookups shared by the printer and the parser. A name maps to every
/// id carrying it; only a name with exactly one owner is usable.
#[derive(Default)]
pub(crate) struct Names {
    pub characters: HashMap<String, Vec<CharacterId>>,
    pub assets: HashMap<String, Vec<AssetId>>,
}

impl Names {
    pub fn of(story: &Story) -> Self {
        let mut names = Names::default();
        for c in &story.characters {
            if let Some(name) = story.text(c.name_id) {
                names
                    .characters
                    .entry(name.to_owned())
                    .or_default()
                    .push(c.id);
            }
        }
        for a in &story.assets.assets {
            names.assets.entry(a.path.clone()).or_default().push(a.id);
        }
        names
    }
}

pub(crate) fn unique<T: Copy>(map: &HashMap<String, Vec<T>>, name: &str) -> Option<T> {
    match map.get(name).map(Vec::as_slice) {
        Some([one]) => Some(*one),
        _ => None,
    }
}

/// Label names of a scene.
pub(crate) fn labels_of(nodes: &[Node]) -> HashMap<String, Vec<NodeId>> {
    let mut map: HashMap<String, Vec<NodeId>> = HashMap::new();
    for n in nodes {
        if let NodeKind::Label { name } = &n.kind {
            map.entry(name.clone()).or_default().push(n.id);
        }
    }
    map
}

pub(crate) fn scene_names<'a>(
    scenes: impl Iterator<Item = (&'a str, SceneId)>,
) -> HashMap<String, Vec<SceneId>> {
    let mut map: HashMap<String, Vec<SceneId>> = HashMap::new();
    for (name, id) in scenes {
        map.entry(name.to_owned()).or_default().push(id);
    }
    map
}

struct Ctx<'a> {
    story: &'a Story,
    names: Names,
    scenes: HashMap<String, Vec<SceneId>>,
    /// Labels per scene id.
    labels: HashMap<SceneId, HashMap<String, Vec<NodeId>>>,
}

impl<'a> Ctx<'a> {
    fn new(story: &'a Story) -> Self {
        Ctx {
            story,
            names: Names::of(story),
            scenes: scene_names(story.scenes.iter().map(|s| (s.name.as_str(), s.id))),
            labels: story
                .scenes
                .iter()
                .map(|s| (s.id, labels_of(&s.nodes)))
                .collect(),
        }
    }

    fn label_ref(&self, scene: SceneId, target: NodeId) -> String {
        if let Some(Node {
            kind: NodeKind::Label { name },
            ..
        }) = self
            .story
            .scene(scene)
            .and_then(|s| s.nodes.iter().find(|n| n.id == target))
            && let Some(labels) = self.labels.get(&scene)
            && unique(labels, name) == Some(target)
        {
            return fmt_name(name);
        }
        format!("@{target}")
    }

    fn scene_ref(&self, id: SceneId) -> String {
        match self.story.scene(id) {
            Some(s) if unique(&self.scenes, &s.name) == Some(id) => fmt_name(&s.name),
            _ => format!("@{id}"),
        }
    }

    fn character_name(&self, id: CharacterId) -> Option<&str> {
        let c = self.story.character(id)?;
        let name = self.story.text(c.name_id)?;
        (unique(&self.names.characters, name) == Some(id)).then_some(name)
    }

    fn character_ref(&self, id: CharacterId) -> String {
        self.character_name(id)
            .map_or_else(|| format!("@{id}"), fmt_name)
    }

    fn asset_ref(&self, id: AssetId) -> String {
        match self.story.assets.get(id) {
            Some(a) if unique(&self.names.assets, &a.path) == Some(id) => fmt_name(&a.path),
            _ => format!("@{id}"),
        }
    }

    fn text(&self, id: orbit_core::StringId) -> String {
        // A dangling string id prints as an empty text; the parser then
        // keeps the id of an existing node (see `parse`).
        quote(self.story.text(id).unwrap_or_default())
    }
}

/// Prints every scene of the story.
pub fn print_story(story: &Story) -> String {
    let ctx = Ctx::new(story);
    let mut out = String::new();
    for (i, scene) in story.scenes.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        write_scene(&ctx, scene, &mut out);
    }
    out
}

/// Prints one scene (`None` if the story has no such scene).
pub fn print_scene(story: &Story, scene: SceneId) -> Option<String> {
    let ctx = Ctx::new(story);
    let mut out = String::new();
    write_scene(&ctx, story.scene(scene)?, &mut out);
    Some(out)
}

fn write_scene(ctx: &Ctx<'_>, scene: &Scene, out: &mut String) {
    out.push_str(&format!("scene {} #@{}\n", fmt_name(&scene.name), scene.id));
    for node in &scene.nodes {
        write_node(ctx, scene.id, node, out);
    }
}

fn transition(t: &Option<Transition>) -> String {
    match t {
        None => String::new(),
        Some(Transition {
            kind: TransitionKind::Dissolve,
            duration,
            ease,
        }) => format!(" with dissolve({}{})", fmt_f32(*duration), ease_arg(ease)),
        Some(Transition {
            kind: TransitionKind::Fade(c),
            duration,
            ease,
        }) => format!(
            " with fade({}, \"#{:02x}{:02x}{:02x}\"{})",
            fmt_f32(*duration),
            c.r,
            c.g,
            c.b,
            ease_arg(ease)
        ),
    }
}

/// `, ease <curve>` inside a transition, nothing for a linear one.
fn ease_arg(ease: &Option<Curve>) -> String {
    match ease {
        None => String::new(),
        Some(c) => format!(", ease {}", curve(c)),
    }
}

/// A curve: a preset name (`OutQuad`), `bezier(x1, y1, x2, y2)` or a quoted
/// named curve.
pub(crate) fn curve(c: &Curve) -> String {
    match c {
        Curve::Preset(e) => e.name().to_owned(),
        Curve::Bezier { p1, p2 } => format!(
            "bezier({}, {}, {}, {})",
            fmt_f32(p1[0]),
            fmt_f32(p1[1]),
            fmt_f32(p2[0]),
            fmt_f32(p2[1])
        ),
        Curve::Named(name) => quote(name),
    }
}

fn hex(c: Color) -> String {
    format!("\"#{:02x}{:02x}{:02x}\"", c.r, c.g, c.b)
}

/// ` over <seconds> [ease <curve>] [wait]`, the tail of an effect line.
fn over(duration: f32, ease: &Option<Curve>, wait: bool) -> String {
    let mut s = format!(" over {}", fmt_f32(duration));
    if let Some(c) = ease {
        s.push_str(&format!(" ease {}", curve(c)));
    }
    if wait {
        s.push_str(" wait");
    }
    s
}

fn anim_target(ctx: &Ctx<'_>, t: &AnimTarget) -> String {
    match t {
        AnimTarget::Camera => "camera".into(),
        AnimTarget::Screen => "screen".into(),
        AnimTarget::Background => "background".into(),
        AnimTarget::Character(c) => ctx.character_ref(*c),
    }
}

fn value_text(v: &Value) -> Option<String> {
    Some(match v {
        Value::Scalar(x) => fmt_f32(*x),
        Value::Vec2(v) => format!("({}, {})", fmt_f32(v.x), fmt_f32(v.y)),
        Value::Angle(a) => fmt_f32(a.0),
        Value::Color(c) => format!(
            "rgba({}, {}, {}, {})",
            fmt_f32(c.r),
            fmt_f32(c.g),
            fmt_f32(c.b),
            fmt_f32(c.a)
        ),
        Value::Name(_) => return None,
    })
}

/// `track <target> <prop> <t> <value> [ease <curve>], ...`; `None` for a track
/// that has no text form (a discrete track no property accepts).
fn track_line(ctx: &Ctx<'_>, t: &StoryTrack) -> Option<String> {
    let mut keys = Vec::new();
    macro_rules! keys_of {
        ($track:expr, $wrap:expr) => {
            for k in &$track.keys {
                let mut s = format!("{} {}", fmt_f32(k.t), value_text(&$wrap(&k.value))?);
                if k.ease != Curve::default() {
                    s.push_str(&format!(" ease {}", curve(&k.ease)));
                }
                keys.push(s);
            }
        };
    }
    match &t.data {
        TrackData::Scalar(x) => keys_of!(x, |v: &f32| Value::Scalar(*v)),
        TrackData::Vec2(x) => keys_of!(x, |v: &Vec2| Value::Vec2(*v)),
        TrackData::Color(x) => keys_of!(x, |v: &Rgba| Value::Color(*v)),
        TrackData::Angle(x) => keys_of!(x, |v: &Angle| Value::Angle(*v)),
        TrackData::Name(_) => return None,
    }
    Some(format!(
        "track {} {} {}",
        anim_target(ctx, &t.target),
        t.prop.name(),
        keys.join(", ")
    ))
}

fn position(p: &Position) -> String {
    match p {
        Position::Left => "left".into(),
        Position::Center => "center".into(),
        Position::Right => "right".into(),
        Position::Custom(name) => quote(name),
        Position::Free { x, y } => format!("({}, {})", fmt_f32(*x), fmt_f32(*y)),
    }
}

/// The `$` statement of a `SetVar`, using the compound no-code form when the
/// expression is one (`VarOp::from_expr`).
fn set_var(var: &str, value: &Expr) -> String {
    let lhs = if is_ident(var) && var != "true" && var != "false" {
        var.to_owned()
    } else {
        quote(var)
    };
    let compound = match VarOp::from_expr(var, value).map(|op| op.op) {
        Some(OpKind::Add) => Some("+="),
        Some(OpKind::Sub) => Some("-="),
        Some(OpKind::Mul) => Some("*="),
        Some(OpKind::Div) => Some("/="),
        _ => None,
    };
    match (compound, value) {
        (Some(sym), Expr::Binary(_, _, rhs)) => format!("$ {lhs} {sym} {}", print_expr(rhs)),
        _ => format!("$ {lhs} = {}", print_expr(value)),
    }
}

pub(crate) fn custom_data(d: &CustomData) -> String {
    match d {
        CustomData::Null => "null".into(),
        CustomData::Bool(b) => b.to_string(),
        CustomData::Int(i) => i.to_string(),
        CustomData::Float(f) if f.is_nan() => "nan".into(),
        CustomData::Float(f) if f.is_infinite() => {
            if *f > 0.0 {
                "inf".into()
            } else {
                "-inf".into()
            }
        }
        CustomData::Float(f) => fmt_f64(*f),
        CustomData::Str(s) => quote(s),
        CustomData::List(items) => {
            let inner: Vec<String> = items.iter().map(custom_data).collect();
            format!("[{}]", inner.join(", "))
        }
        CustomData::Map(entries) => {
            let inner: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("{}: {}", quote(k), custom_data(v)))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
    }
}

fn write_node(ctx: &Ctx<'_>, scene: SceneId, node: &Node, out: &mut String) {
    let label = |id: NodeId| ctx.label_ref(scene, id);
    let body = match &node.kind {
        NodeKind::Label { name } => format!("label {}", fmt_name(name)),
        NodeKind::Say { speaker, text_id } => match speaker {
            None => ctx.text(*text_id),
            Some(c) => match ctx.character_name(*c) {
                Some(name) if is_ident(name) && !KEYWORDS.contains(&name) => {
                    format!("{name} {}", ctx.text(*text_id))
                }
                _ => format!("say {} {}", ctx.character_ref(*c), ctx.text(*text_id)),
            },
        },
        NodeKind::Choice { options } => {
            out.push_str(&format!("    menu #@{}\n", node.id));
            for o in options {
                out.push_str(&format!(
                    "        {} -> {}",
                    ctx.text(o.text_id),
                    label(o.target)
                ));
                if let Some(c) = &o.condition {
                    out.push_str(&format!(" if {}", print_expr(c)));
                }
                out.push('\n');
            }
            return;
        }
        NodeKind::Jump { target } => format!("jump {}", label(*target)),
        NodeKind::JumpScene {
            scene: to,
            label: l,
        } => {
            let mut s = format!("jump_scene {}", ctx.scene_ref(*to));
            if let Some(l) = l {
                s.push_str(&format!(" label {}", ctx.label_ref(*to, *l)));
            }
            s
        }
        NodeKind::SetVar { var, value } => set_var(var, value),
        NodeKind::If {
            condition,
            then_target,
            else_target,
        } => {
            let mut s = format!("if {} -> {}", print_expr(condition), label(*then_target));
            if let Some(e) = else_target {
                s.push_str(&format!(" else {}", label(*e)));
            }
            s
        }
        NodeKind::Background {
            asset,
            transition: t,
        } => {
            format!("bg {}{}", ctx.asset_ref(*asset), transition(t))
        }
        NodeKind::Show {
            character,
            expression,
            position: p,
            transition: t,
            scale,
            scale_y,
            flip_x,
            flip_y,
        } => {
            let mut s = format!(
                "show {} {} at {}",
                ctx.character_ref(*character),
                fmt_name(expression),
                position(p)
            );
            if let Some(v) = scale {
                s.push_str(&format!(" scale {}", fmt_f32(*v)));
            }
            if let Some(v) = scale_y {
                s.push_str(&format!(" scale_y {}", fmt_f32(*v)));
            }
            if *flip_x {
                s.push_str(" flip_x");
            }
            if *flip_y {
                s.push_str(" flip_y");
            }
            s.push_str(&transition(t));
            s
        }
        NodeKind::Hide {
            character,
            transition: t,
        } => {
            format!("hide {}{}", ctx.character_ref(*character), transition(t))
        }
        NodeKind::PlayMusic {
            asset,
            fade_in,
            looping,
        } => {
            let mut s = format!("music {}", ctx.asset_ref(*asset));
            if *fade_in != 0.0 {
                s.push_str(&format!(" fade {}", fmt_f32(*fade_in)));
            }
            if *looping {
                s.push_str(" loop");
            }
            s
        }
        NodeKind::StopMusic { fade_out } => {
            if *fade_out != 0.0 {
                format!("stop music fade {}", fmt_f32(*fade_out))
            } else {
                "stop music".into()
            }
        }
        NodeKind::PlaySound { asset } => format!("sound {}", ctx.asset_ref(*asset)),
        NodeKind::Wait { seconds } => format!("wait {}", fmt_f32(*seconds)),
        NodeKind::Call { label: l } => format!("call {}", label(*l)),
        NodeKind::ShowScene3d {
            scene,
            camera,
            transition: t,
        } => {
            let mut s = format!("scene3d {}", ctx.asset_ref(*scene));
            if let Some(c) = camera {
                s.push_str(&format!(" camera {}", quote(c)));
            }
            s.push_str(&transition(t));
            s
        }
        NodeKind::Camera3d {
            camera,
            transition: t,
        } => format!("camera3d {}{}", quote(camera), transition(t)),
        NodeKind::HideScene3d { transition: t } => format!("hide_scene3d{}", transition(t)),
        NodeKind::Return => "return".into(),
        NodeKind::End => "end".into(),
        NodeKind::Custom {
            plugin_id,
            kind,
            version,
            data,
        } => {
            let mut s = format!("custom {} {}", quote(plugin_id), fmt_name(kind));
            if *version != 0 {
                s.push_str(&format!(" version {version}"));
            }
            if *data != CustomData::Null {
                s.push_str(&format!(" data {}", custom_data(data)));
            }
            s
        }
        NodeKind::Effect(effect) => match effect {
            Effect::Move {
                target,
                dx,
                dy,
                duration,
                ease,
                wait,
            } => format!(
                "move {} by ({}, {}){}",
                anim_target(ctx, target),
                fmt_f32(*dx),
                fmt_f32(*dy),
                over(*duration, ease, *wait)
            ),
            Effect::Shake {
                target,
                amplitude,
                duration,
                ease,
                wait,
            } => format!(
                "shake {} {}{}",
                anim_target(ctx, target),
                fmt_f32(*amplitude),
                over(*duration, ease, *wait)
            ),
            Effect::Zoom {
                factor,
                duration,
                ease,
                wait,
            } => format!(
                "zoom {}{}",
                fmt_f32(*factor),
                over(*duration, ease, *wait)
            ),
            Effect::Flash {
                color,
                duration,
                ease,
                wait,
            } => format!("flash {}{}", hex(*color), over(*duration, ease, *wait)),
            Effect::Tint {
                target,
                color,
                strength,
                duration,
                ease,
                wait,
            } => format!(
                "tint {} {} {}{}",
                anim_target(ctx, target),
                hex(*color),
                fmt_f32(*strength),
                over(*duration, ease, *wait)
            ),
        },
        NodeKind::OpenScreen {
            screen,
            wait,
            result,
            params,
        } => {
            let mut s = format!("open screen {}", quote(screen));
            if *wait {
                s.push_str(" wait");
            }
            if let Some(r) = result {
                s.push_str(&format!(" -> {}", fmt_name(r)));
            }
            if !params.is_empty() {
                let list: Vec<String> = params
                    .iter()
                    .map(|p| format!("{} = {}", fmt_name(&p.name), print_expr(&p.value.to_expr())))
                    .collect();
                s.push_str(&format!(" with {}", list.join(", ")));
            }
            s
        }
        NodeKind::CloseScreen { screen } => format!("close screen {}", quote(screen)),
        NodeKind::BackgroundLayers { layers, transition: t } => {
            out.push_str(&format!("    bg_layers{} #@{}\n", transition(t), node.id));
            for l in layers {
                out.push_str(&format!(
                    "        layer {} depth {}{}\n",
                    ctx.asset_ref(l.asset),
                    fmt_f32(l.depth),
                    if l.repeat_x { " repeat_x" } else { "" }
                ));
            }
            return;
        }
        NodeKind::Particles(e) => {
            let pair = |p: [f32; 2]| format!("({}, {})", fmt_f32(p[0]), fmt_f32(p[1]));
            let mut s = format!(
                "particles {} {} seed {} rate {} lifetime {} jitter {} spawn ({}, {}, {}, {}) direction {} spread {} speed {} gravity {} sway {} size {} stretch {} spin {} color {} to {} opacity {} fade {}",
                quote(&e.name),
                e.preset.name(),
                e.seed,
                fmt_f32(e.rate),
                fmt_f32(e.lifetime),
                fmt_f32(e.lifetime_jitter),
                fmt_f32(e.spawn[0]),
                fmt_f32(e.spawn[1]),
                fmt_f32(e.spawn[2]),
                fmt_f32(e.spawn[3]),
                fmt_f32(e.direction),
                fmt_f32(e.spread),
                pair(e.speed),
                pair(e.gravity),
                pair(e.sway),
                pair(e.size),
                fmt_f32(e.stretch),
                fmt_f32(e.spin),
                hex(e.color),
                hex(e.color_end),
                fmt_f32(e.opacity),
                fmt_f32(e.fade),
            );
            if let Some(sprite) = e.sprite {
                s.push_str(&format!(" sprite {}", ctx.asset_ref(sprite)));
            }
            s.push_str(if e.front { " front" } else { " behind" });
            s.push_str(if e.prewarm { " prewarm" } else { " cold" });
            s
        }
        NodeKind::Script {
            source,
            branches,
            on_error,
        } => {
            let mut s = format!("script {}", quote(source));
            for b in branches {
                s.push_str(&format!(" branch {} -> {}", fmt_name(&b.name), label(b.target)));
            }
            if let Some(e) = on_error {
                s.push_str(&format!(" on_error {}", label(*e)));
            }
            s
        }
        NodeKind::StopParticles { name } if name.is_empty() => "stop particles".into(),
        NodeKind::StopParticles { name } => format!("stop particles {}", quote(name)),
        NodeKind::Timeline {
            duration,
            looping,
            wait,
            tracks,
        } => {
            let mut s = format!("timeline over {}", fmt_f32(*duration));
            match looping {
                AnimLoop::Once => {}
                AnimLoop::Loop(n) => s.push_str(&format!(" loop {n}")),
                AnimLoop::Forever => s.push_str(" forever"),
                AnimLoop::PingPong => s.push_str(" pingpong"),
            }
            if *wait {
                s.push_str(" wait");
            }
            out.push_str(&format!("    {s} #@{}\n", node.id));
            for t in tracks {
                if let Some(line) = track_line(ctx, t) {
                    out.push_str(&format!("        {line}\n"));
                }
            }
            return;
        }
    };
    out.push_str(&format!("    {body} #@{}\n", node.id));
}
