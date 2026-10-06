//! MCP tools that build and animate a 2D rig without the editor UI:
//! create a rig from a picture, add parts, bones, bind, slots, skins, IK,
//! clips and keys. Every edit is validated (orbit_anim `validate` and the web
//! budget; errors refuse the call and nothing is written), atomic, and goes
//! through the open rig editor when it has the rig (undo, refresh): see
//! [`crate::rig_tools::LiveRig`].
//!
//! Bones, slots, skins and clips are named with an id or a name. Rig units
//! are pixels, the origin is the character's bottom-center, Y points down,
//! rotations are degrees clockwise on screen.


use orbit_anim::rig::{Attachment, Rig};
use orbit_anim::{Curve, Loop};
use orbit_core::AssetKind;
use orbit_project::rigops::{self, BoneSpec, KeyTarget, KeyValue, OpError, OpErrorKind, SlotPatch, TrackKind};
use orbit_project::rigmesh::workflow::Picture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::error::ToolError;
use crate::rig_tools::{RigFile, Stored, assets_root, io_err, issues_json, load, out, resolve, store};
use crate::tools::{ImportAssetArgs, ToolCx, import_asset};

fn op_err(e: OpError) -> ToolError {
    match e.kind {
        OpErrorKind::NotFound => ToolError::not_found(e.message),
        OpErrorKind::Invalid => ToolError::invalid(e.message),
        OpErrorKind::Refused => ToolError::refused(e.message),
    }
}

// ---------------------------------------------------------------- summary

fn summary(rig: &Rig) -> Json {
    let name = |id: &orbit_anim::rig::BoneId| {
        rig.bones.iter().find(|b| &b.id == id).map_or_else(|| id.to_string(), |b| b.name.clone())
    };
    json!({
        "bones": rig.bones.iter().map(|b| json!({
            "id": b.id.to_string(), "name": b.name, "parent": b.parent.as_ref().map(name),
            "x": b.rest.x, "y": b.rest.y, "rotation": b.rest.rotation, "length": b.length,
        })).collect::<Vec<_>>(),
        "slots": rig.slots.iter().map(|s| json!({
            "id": s.id.to_string(), "name": s.name, "bone": name(&s.bone),
            "attachment": s.attachment.as_ref().map(ToString::to_string),
        })).collect::<Vec<_>>(),
        "skins": rig.skins.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        "clips": rig.clips.iter().map(|(n, c)| json!({"name": n, "duration": c.duration, "tracks": c.tracks.len()})).collect::<Vec<_>>(),
        "ik": rig.ik.len(),
    })
}

fn respond(file: &RigFile, rig: &Rig, stored: &Stored, mut result: Json) -> Result<String, ToolError> {
    result["rig"] = json!(file.rel);
    result["id"] = json!(file.asset.map(|a| a.to_string()));
    result["summary"] = summary(rig);
    result["warnings"] = json!(stored.warnings);
    result["note"] = json!(if stored.live {
        "Applied in the open rig editor (Ctrl+Z undoes it)."
    } else {
        "The rig file was rewritten (not covered by undo)."
    });
    out(result)
}

/// Loads the rig, runs `f` on a copy, validates and stores the result.
fn edit<R>(
    cx: &mut ToolCx<'_>,
    key: &str,
    label: &str,
    f: impl FnOnce(&mut Rig) -> Result<R, ToolError>,
) -> Result<(RigFile, Rig, R, Stored), ToolError> {
    let root = assets_root(cx);
    let file = resolve(cx, key)?;
    let before = load(&file, &root)?;
    let mut rig = before.clone();
    let result = f(&mut rig)?;
    let stored = store(&file, Some(&before), &rig, label)?;
    Ok((file, rig, result, stored))
}

// ------------------------------------------------------- expressions in use

/// The character expressions that play `rig` (asset id) in `clip` / `skin`.
fn users(cx: &ToolCx<'_>, file: &RigFile, clip: Option<&str>, skin: Option<&str>) -> Vec<String> {
    let Some(asset) = file.asset else { return Vec::new() };
    cx.project
        .story()
        .characters
        .iter()
        .flat_map(|c| {
            c.expressions
                .iter()
                .filter_map(|e| e.rigged().map(|m| (c, e, m)))
                .filter(|(_, _, m)| {
                    m.rig == asset
                        && clip.is_none_or(|k| m.clip == k)
                        && skin.is_none_or(|k| m.skin.as_deref() == Some(k))
                })
                .map(|(c, e, _)| format!("{}:{}", c.id, e.name))
                .collect::<Vec<_>>()
        })
        .collect()
}

// ----------------------------------------------------------------- pictures

fn picture(cx: &mut ToolCx<'_>, image: &str) -> Result<Picture, ToolError> {
    let root = assets_root(cx);
    let imported = import_asset(cx, ImportAssetArgs { source: image.to_owned(), kind: None, name: None })?;
    let v: Json = serde_json::from_str(&imported).unwrap_or(Json::Null);
    let path = v["path"].as_str().unwrap_or(image).to_owned();
    if v["kind"].as_str().is_some_and(|k| k != "image") {
        return Err(ToolError::invalid(format!("`{image}` is not a picture")));
    }
    let bytes = std::fs::read(root.join(&path)).map_err(|e| ToolError::not_found(format!("cannot read picture `{path}`: {e}")))?;
    let img = orbit_assets::decode_image(&bytes).map_err(|e| ToolError::invalid(format!("cannot decode `{path}`: {e}")))?;
    Ok(Picture { path, width: img.width, height: img.height, rgba: img.rgba })
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).collect();
    s.trim_matches('_').to_owned()
}

// ------------------------------------------------------------------- args

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateArgs {
    /// Name of the new rig: the file is `assets/rigs/<name>.rig.ron`
    /// (default: the picture's file name, else `rig`). An existing file is
    /// never overwritten.
    pub name: Option<String>,
    /// Picture of the character (png / jpg), a path in the project: it is
    /// imported as an image asset if it is not one yet. The rig gets a
    /// `root` bone at the feet (bottom-center, pointing up), a slot named
    /// after the file and a mesh traced from the picture's alpha, standing on
    /// the origin. Absent: an empty rig (no bone: see `rig_bone_add`).
    pub image: Option<String>,
    /// Declare the rig as a project asset right away (default false; see
    /// `rig_add_to_project`).
    pub add_to_project: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddPartArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// Picture to add, a path in the project (imported if needed).
    pub image: String,
    /// Bone the part follows (id or name); default: the first root bone.
    pub bone: Option<String>,
    /// Slot name (default: the picture's file name).
    pub name: Option<String>,
    /// Rig-space position `[x, y]` of the picture's center. Default: aligned
    /// on an existing picture of the same size (layers from one canvas), else
    /// standing on the origin.
    pub center: Option<[f32; 2]>,
    /// Skin receiving the picture (default `default`).
    pub skin: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Space {
    /// Relative to the parent bone (default).
    Parent,
    /// Rig space: origin at the character's feet.
    World,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoneAddArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// Bone name (default `bone`, `bone2`...). Unique in the rig.
    pub name: Option<String>,
    /// Parent bone (id or name). Absent: a root bone.
    pub parent: Option<String>,
    /// Position of the bone's head (x, right) in `space` (default 0).
    pub x: Option<f32>,
    /// Position of the bone's head (y, down) in `space` (default 0).
    pub y: Option<f32>,
    /// Rotation in degrees clockwise on screen (0 = pointing right, -90 =
    /// up) in `space`.
    pub rotation: Option<f32>,
    pub scale_x: Option<f32>,
    pub scale_y: Option<f32>,
    /// Length along the bone's X axis (default 50; used by IK, auto weights
    /// and the editor).
    pub length: Option<f32>,
    /// How x / y / rotation are read: `parent` (default) or `world`.
    pub space: Option<Space>,
    /// Alternative to x, y, rotation, length: the head `[x, y]` in rig space.
    pub head: Option<[f32; 2]>,
    /// With `head`: the tip `[x, y]` in rig space; gives the rotation and
    /// the length (what click-drag draws in the editor).
    pub tip: Option<[f32; 2]>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoneUpdateArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// The bone (id or name).
    pub bone: String,
    /// New name.
    pub name: Option<String>,
    /// New parent (id or name); `""` makes it a root. The bone keeps its
    /// place in the rig (its local pose is recomputed), then x / y / rotation
    /// apply.
    pub parent: Option<String>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub rotation: Option<f32>,
    pub scale_x: Option<f32>,
    pub scale_y: Option<f32>,
    pub length: Option<f32>,
    /// How x / y / rotation are read: `parent` (default) or `world`.
    pub space: Option<Space>,
    /// Alternative: new head `[x, y]` in rig space.
    pub head: Option<[f32; 2]>,
    /// With `head`: new tip `[x, y]` in rig space (rotation and length).
    pub tip: Option<[f32; 2]>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoneArg {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// The bone (id or name).
    pub bone: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReparentArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// The bone to move (id or name), with its descendants.
    pub bone: String,
    /// The new parent (id or name); absent or `""`: the bone becomes a root.
    pub parent: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// Weight falloff (bigger: sharper joints; default 3).
    pub falloff: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotUpdateArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// The slot (id or name).
    pub slot: String,
    pub name: Option<String>,
    /// Attachment shown in the setup pose: an attachment id of a skin (see
    /// `rig_read`); `""` shows nothing.
    pub attachment: Option<String>,
    /// Tint `[r, g, b, a]`, each 0..1 (or `"#rrggbb"` / `"#rrggbbaa"`).
    pub color: Option<Json>,
    /// Bone the slot follows (id or name).
    pub bone: Option<String>,
    /// Position in the draw order: 0 = behind, last = in front.
    pub draw_order: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkinArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// The skin name.
    pub skin: String,
    /// `rig_skin_duplicate`: name of the copy (default `<skin> copy`).
    pub name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IkSetArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// 1 bone (aims at the target) or 2 bones `[parent, child]` (bends the
    /// pair so the child's tip reaches the target; the child must be a child
    /// of the parent). Ids or names.
    pub bones: Vec<String>,
    /// Bone whose origin is the goal (not part of the chain).
    pub target: String,
    /// Two-bone only: the child bends with a positive (clockwise) angle.
    pub bend_positive: Option<bool>,
    /// 0 = no effect, 1 = full IK (default 1).
    pub mix: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IkRemoveArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// Index of the constraint (see `rig_read`).
    pub index: Option<usize>,
    /// Or its chain (bones, ids or names).
    pub bones: Option<Vec<String>>,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoopKind {
    Once,
    Forever,
    PingPong,
    /// Plays `times` times, then stops.
    Times,
}

fn loop_of(kind: LoopKind, times: Option<u32>) -> Result<Loop, ToolError> {
    Ok(match kind {
        LoopKind::Once => Loop::Once,
        LoopKind::Forever => Loop::Forever,
        LoopKind::PingPong => Loop::PingPong,
        LoopKind::Times => Loop::Loop(times.filter(|t| *t > 0).ok_or_else(|| ToolError::invalid("`times` (at least 1) is required with looping `times`"))?),
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipCreateArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// Clip name (unique in the rig).
    pub name: String,
    /// Length in seconds (above 0, at most 3600).
    pub duration: f32,
    /// What happens at the end: once (default), forever, ping_pong, times.
    pub looping: Option<LoopKind>,
    /// With looping `times`: how many passes.
    pub times: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipUpdateArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// The clip.
    pub name: String,
    /// New length in seconds; shortening below the last key is refused.
    pub duration: Option<f32>,
    pub looping: Option<LoopKind>,
    pub times: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipArg {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// The clip.
    pub name: String,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackArg {
    /// value: degrees (number), added to the bone's setup rotation.
    BoneRotate,
    /// value: `[dx, dy]`, added to the setup position.
    BoneTranslate,
    /// value: `[sx, sy]`, multiplies the setup scale.
    BoneScale,
    /// value: an attachment id, or null to hide (stepped, no blend).
    SlotAttachment,
    /// value: `[r, g, b, a]` 0..1 or `"#rrggbb[aa]"`.
    SlotColor,
    /// value: `[[dx, dy], ...]`, one offset per vertex of the mesh.
    Deform,
    /// value: slot names / ids, behind to front (stepped; no target).
    DrawOrder,
}

impl From<TrackArg> for TrackKind {
    fn from(t: TrackArg) -> Self {
        match t {
            TrackArg::BoneRotate => TrackKind::BoneRotate,
            TrackArg::BoneTranslate => TrackKind::BoneTranslate,
            TrackArg::BoneScale => TrackKind::BoneScale,
            TrackArg::SlotAttachment => TrackKind::SlotAttachment,
            TrackArg::SlotColor => TrackKind::SlotColor,
            TrackArg::Deform => TrackKind::Deform,
            TrackArg::DrawOrder => TrackKind::DrawOrder,
        }
    }
}

/// Easing of the segment from this key to the next.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum EaseArg {
    /// A preset (`Linear`, `InQuad`, `OutCubic`, `InOutSine`, `OutBack`,
    /// `OutBounce`, `Step`... in any case, `in_out_quad` works too) or the
    /// name of a story curve (see `list_curves`).
    Name(String),
    /// A CSS cubic-bezier: `{"bezier": [x1, y1, x2, y2]}`.
    Bezier { bezier: [f32; 4] },
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeySetArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// The clip (see `rig_clip_create`).
    pub clip: String,
    /// Which property is animated; also gives the shape of `value`.
    pub track: TrackArg,
    /// The bone (bone_* tracks) or the slot (slot_* and deform tracks), id or
    /// name. Not used by draw_order.
    pub target: Option<String>,
    /// Deform: the mesh attachment id (default: the slot's own).
    pub attachment: Option<String>,
    /// Seconds from the clip start, 0 to the clip's duration. A key already
    /// at that time (within a millisecond) is replaced.
    pub time: f32,
    /// The value (see `track`).
    pub value: Json,
    /// Easing toward the next key (default: keep the existing key's, linear
    /// for a new key).
    pub ease: Option<EaseArg>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyDeleteArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    pub clip: String,
    pub track: TrackArg,
    /// The bone or slot, as in `rig_clip_key_set`.
    pub target: Option<String>,
    /// Deform: the mesh attachment id (default: the slot's own).
    pub attachment: Option<String>,
    /// Time of the key (within a millisecond).
    pub time: f32,
}

// ---------------------------------------------------------------- parsing

fn num(v: &Json, what: &str) -> Result<f32, ToolError> {
    v.as_f64().map(|f| f as f32).ok_or_else(|| ToolError::invalid(format!("{what}: expected a number")))
}

fn pair(v: &Json, what: &str) -> Result<[f32; 2], ToolError> {
    match v {
        Json::Array(a) if a.len() == 2 => Ok([num(&a[0], what)?, num(&a[1], what)?]),
        Json::Object(o) if o.contains_key("x") && o.contains_key("y") => Ok([num(&o["x"], what)?, num(&o["y"], what)?]),
        _ => Err(ToolError::invalid(format!("{what}: expected [x, y]"))),
    }
}

fn color(v: &Json) -> Result<[f32; 4], ToolError> {
    match v {
        Json::String(s) => {
            let h = s.trim_start_matches('#');
            let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("zz"), 16).map(|b| f32::from(b) / 255.0);
            let bad = || ToolError::invalid(format!("`{s}` is not a color (#rrggbb or #rrggbbaa)"));
            match h.len() {
                6 => Ok([byte(0).map_err(|_| bad())?, byte(2).map_err(|_| bad())?, byte(4).map_err(|_| bad())?, 1.0]),
                8 => Ok([byte(0).map_err(|_| bad())?, byte(2).map_err(|_| bad())?, byte(4).map_err(|_| bad())?, byte(6).map_err(|_| bad())?]),
                _ => Err(bad()),
            }
        }
        Json::Array(a) if a.len() == 3 || a.len() == 4 => {
            let r = num(&a[0], "color")?;
            let g = num(&a[1], "color")?;
            let b = num(&a[2], "color")?;
            let al = if a.len() == 4 { num(&a[3], "color")? } else { 1.0 };
            Ok([r, g, b, al])
        }
        _ => Err(ToolError::invalid("a color is [r, g, b, a] (0..1) or \"#rrggbb[aa]\"")),
    }
}

fn key_value(track: TrackKind, v: &Json) -> Result<KeyValue, ToolError> {
    Ok(match track {
        TrackKind::BoneRotate => KeyValue::Angle(num(v, "value")?),
        TrackKind::BoneTranslate | TrackKind::BoneScale => KeyValue::Vec2(pair(v, "value")?),
        TrackKind::SlotAttachment => match v {
            Json::Null => KeyValue::Attachment(None),
            Json::String(s) => KeyValue::Attachment(Some(s.clone())),
            _ => return Err(ToolError::invalid("value: an attachment id or null")),
        },
        TrackKind::SlotColor => KeyValue::Color(color(v)?),
        TrackKind::Deform => match v {
            Json::Array(a) => KeyValue::Deform(a.iter().map(|p| pair(p, "value")).collect::<Result<_, _>>()?),
            _ => return Err(ToolError::invalid("value: [[dx, dy], ...]")),
        },
        TrackKind::DrawOrder => match v {
            Json::Array(a) => KeyValue::Order(
                a.iter()
                    .map(|s| s.as_str().map(str::to_owned).ok_or_else(|| ToolError::invalid("value: slot names")))
                    .collect::<Result<_, _>>()?,
            ),
            _ => return Err(ToolError::invalid("value: [slot, ...] behind to front")),
        },
    })
}

fn curve_of(cx: &ToolCx<'_>, e: &EaseArg) -> Result<Curve, ToolError> {
    match e {
        EaseArg::Bezier { bezier } => {
            if bezier.iter().any(|v| !v.is_finite()) {
                return Err(ToolError::invalid("bezier values must be finite"));
            }
            Ok(Curve::Bezier { p1: [bezier[0], bezier[1]], p2: [bezier[2], bezier[3]] })
        }
        EaseArg::Name(n) => {
            if let Some(c) = rigops::ease_by_name(n) {
                return Ok(c);
            }
            if cx.project.story().curves.contains_key(n) {
                return Ok(Curve::Named(n.clone()));
            }
            let known: Vec<&String> = cx.project.story().curves.keys().collect();
            Err(ToolError::not_found(format!(
                "`{n}` is neither an easing preset (Linear, InQuad, OutCubic, InOutSine, OutBack, OutBounce, Step...) nor a story curve (curves: {})",
                if known.is_empty() { "none".to_owned() } else { known.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") }
            )))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn bone_spec(
    name: Option<String>,
    x: Option<f32>,
    y: Option<f32>,
    rotation: Option<f32>,
    scale: (Option<f32>, Option<f32>),
    length: Option<f32>,
    space: Option<Space>,
    head: Option<[f32; 2]>,
    tip: Option<[f32; 2]>,
) -> Result<BoneSpec, ToolError> {
    let mut spec = BoneSpec {
        name,
        x,
        y,
        rotation,
        scale_x: scale.0,
        scale_y: scale.1,
        length,
        world: matches!(space, Some(Space::World)),
    };
    if let Some(h) = head {
        if x.is_some() || y.is_some() || rotation.is_some() {
            return Err(ToolError::invalid("use either `head` / `tip` or `x` / `y` / `rotation`"));
        }
        spec.world = true;
        spec.x = Some(h[0]);
        spec.y = Some(h[1]);
        if let Some(t) = tip {
            let (dx, dy) = (t[0] - h[0], t[1] - h[1]);
            let len = dx.hypot(dy);
            if len > 1e-6 {
                spec.rotation = Some(dy.atan2(dx).to_degrees());
                spec.length = Some(len);
            }
        }
    } else if tip.is_some() {
        return Err(ToolError::invalid("`tip` needs `head`"));
    }
    Ok(spec)
}

// ------------------------------------------------------------------ tools

pub fn rig_create(cx: &mut ToolCx<'_>, a: CreateArgs) -> Result<String, ToolError> {
    let root = assets_root(cx);
    let (rig, ids) = match &a.image {
        Some(image) => {
            let pic = picture(cx, image)?;
            let (rig, root_bone, slot) = rigops::rig_from_picture(&pic).map_err(op_err)?;
            (rig, json!({"root_bone": root_bone.to_string(), "slot": slot.to_string(), "image": pic.path, "size": [pic.width, pic.height]}))
        }
        None => (Rig::default(), json!({})),
    };
    let stem = match (&a.name, &a.image) {
        (Some(n), _) => sanitize(n),
        (None, Some(i)) => sanitize(i.rsplit(['/', '\\']).next().unwrap_or(i).split('.').next().unwrap_or("rig")),
        (None, None) => String::new(),
    };
    let stem = if stem.is_empty() { "rig".to_owned() } else { stem };
    let rel = format!("{}/{stem}.rig.ron", orbit_project::rigs::RIG_DIR);
    let abs = root.join(&rel);
    if abs.exists() {
        return Err(ToolError::refused(format!("`{rel}` already exists: pick another `name` (rig_delete removes a rig)")));
    }
    let file = RigFile { asset: None, rel: rel.clone(), abs };
    let stored = store(&file, None, &rig, "rig_create")?;
    let asset = if a.add_to_project.unwrap_or(false) {
        Some(orbit_project::ops::add_asset(cx.project, AssetKind::Rig, &rel)?)
    } else {
        None
    };
    let file = RigFile { asset, ..file };
    respond(&file, &rig, &stored, ids)
}

pub fn rig_add_image_part(cx: &mut ToolCx<'_>, a: AddPartArgs) -> Result<String, ToolError> {
    let pic = picture(cx, &a.image)?;
    let (file, rig, (slot, att), stored) = edit(cx, &a.rig, "rig_add_image_part", |rig| {
        rigops::add_image_part(rig, &pic, a.bone.as_deref(), a.name.as_deref(), a.center, a.skin.as_deref()).map_err(op_err)
    })?;
    let note = "The new mesh is not weighted yet (it follows its bone rigidly): call rig_bind once the bones are in place.";
    respond(&file, &rig, &stored, json!({"slot": slot.to_string(), "attachment": att.to_string(), "image": pic.path, "hint": note}))
}

pub fn rig_delete(cx: &mut ToolCx<'_>, a: DeleteArgs) -> Result<String, ToolError> {
    let file = resolve(cx, &a.rig)?;
    let used = users(cx, &file, None, None);
    if !used.is_empty() {
        return Err(ToolError::refused(format!(
            "character expressions still play this rig ({}): change them first (set_expression_rig without `rig`)",
            used.join(", ")
        )));
    }
    if let Some(id) = file.asset {
        orbit_project::ops::remove_asset(cx.project, id)?;
    }
    std::fs::remove_file(&file.abs).map_err(io_err)?;
    out(json!({"deleted": file.rel, "was_declared": file.asset.is_some()}))
}

pub fn rig_bone_add(cx: &mut ToolCx<'_>, a: BoneAddArgs) -> Result<String, ToolError> {
    let spec = bone_spec(a.name, a.x, a.y, a.rotation, (a.scale_x, a.scale_y), a.length, a.space, a.head, a.tip)?;
    let parent = a.parent.clone().filter(|p| !p.is_empty());
    let (file, rig, id, stored) =
        edit(cx, &a.rig, "rig_bone_add", |rig| rigops::bone_add(rig, parent.as_deref(), &spec).map_err(op_err))?;
    let name = rig.bones.iter().find(|b| b.id == id).map(|b| b.name.clone());
    respond(&file, &rig, &stored, json!({"bone": id.to_string(), "name": name}))
}

pub fn rig_bone_update(cx: &mut ToolCx<'_>, a: BoneUpdateArgs) -> Result<String, ToolError> {
    let spec = bone_spec(a.name, a.x, a.y, a.rotation, (a.scale_x, a.scale_y), a.length, a.space, a.head, a.tip)?;
    let parent = a.parent.as_deref().map(|p| if p.is_empty() { None } else { Some(p) });
    let (file, rig, id, stored) =
        edit(cx, &a.rig, "rig_bone_update", |rig| rigops::bone_update(rig, &a.bone, parent, &spec).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"bone": id.to_string()}))
}

pub fn rig_bone_delete(cx: &mut ToolCx<'_>, a: BoneArg) -> Result<String, ToolError> {
    let (file, rig, gone, stored) = edit(cx, &a.rig, "rig_bone_delete", |rig| rigops::bone_delete(rig, &a.bone).map_err(op_err))?;
    respond(
        &file,
        &rig,
        &stored,
        json!({"deleted_bones": gone.bones, "deleted_slots": gone.slots, "deleted_ik": gone.ik, "deleted_tracks": gone.tracks}),
    )
}

pub fn rig_bone_reparent(cx: &mut ToolCx<'_>, a: ReparentArgs) -> Result<String, ToolError> {
    let parent = a.parent.clone().filter(|p| !p.is_empty());
    let (file, rig, id, stored) =
        edit(cx, &a.rig, "rig_bone_reparent", |rig| rigops::bone_reparent(rig, &a.bone, parent.as_deref()).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"bone": id.to_string()}))
}

pub fn rig_bind(cx: &mut ToolCx<'_>, a: BindArgs) -> Result<String, ToolError> {
    let (file, rig, (meshes, vertices), stored) = edit(cx, &a.rig, "rig_bind", |rig| rigops::bind(rig, a.falloff).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"meshes_bound": meshes, "vertices": vertices}))
}

pub fn rig_slot_update(cx: &mut ToolCx<'_>, a: SlotUpdateArgs) -> Result<String, ToolError> {
    let patch = SlotPatch {
        name: a.name.clone(),
        attachment: a.attachment.clone(),
        color: a.color.as_ref().map(color).transpose()?,
        bone: a.bone.clone(),
        draw_order: a.draw_order,
    };
    let (file, rig, id, stored) = edit(cx, &a.rig, "rig_slot_update", |rig| rigops::slot_update(rig, &a.slot, &patch).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"slot": id.to_string()}))
}

pub fn rig_skin_add(cx: &mut ToolCx<'_>, a: SkinArgs) -> Result<String, ToolError> {
    let (file, rig, name, stored) = edit(cx, &a.rig, "rig_skin_add", |rig| rigops::skin_add(rig, &a.skin).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"skin": name, "hint": "The new skin is empty: slots fall back on the default skin's pictures. Use rig_skin_duplicate to start from an existing one."}))
}

pub fn rig_skin_duplicate(cx: &mut ToolCx<'_>, a: SkinArgs) -> Result<String, ToolError> {
    let (file, rig, name, stored) =
        edit(cx, &a.rig, "rig_skin_duplicate", |rig| rigops::skin_duplicate(rig, &a.skin, a.name.as_deref()).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"skin": name, "copied_from": a.skin}))
}

pub fn rig_skin_delete(cx: &mut ToolCx<'_>, a: SkinArgs) -> Result<String, ToolError> {
    let file = resolve(cx, &a.rig)?;
    let used = users(cx, &file, None, Some(&a.skin));
    if !used.is_empty() {
        return Err(ToolError::refused(format!("expressions use the skin `{}` ({}): change them first", a.skin, used.join(", "))));
    }
    let (file, rig, (), stored) = edit(cx, &a.rig, "rig_skin_delete", |rig| rigops::skin_delete(rig, &a.skin).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"deleted_skin": a.skin}))
}

pub fn rig_ik_set(cx: &mut ToolCx<'_>, a: IkSetArgs) -> Result<String, ToolError> {
    let (file, rig, index, stored) =
        edit(cx, &a.rig, "rig_ik_set", |rig| rigops::ik_set(rig, &a.bones, &a.target, a.bend_positive, a.mix).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"ik_index": index}))
}

pub fn rig_ik_remove(cx: &mut ToolCx<'_>, a: IkRemoveArgs) -> Result<String, ToolError> {
    let (file, rig, index, stored) =
        edit(cx, &a.rig, "rig_ik_remove", |rig| rigops::ik_remove(rig, a.index, a.bones.as_deref()).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"removed_ik_index": index}))
}

pub fn rig_clip_create(cx: &mut ToolCx<'_>, a: ClipCreateArgs) -> Result<String, ToolError> {
    let looping = loop_of(a.looping.unwrap_or(LoopKind::Once), a.times)?;
    let (file, rig, (), stored) =
        edit(cx, &a.rig, "rig_clip_create", |rig| rigops::clip_create(rig, &a.name, a.duration, looping).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"clip": a.name, "duration": a.duration}))
}

pub fn rig_clip_update(cx: &mut ToolCx<'_>, a: ClipUpdateArgs) -> Result<String, ToolError> {
    let looping = a.looping.map(|k| loop_of(k, a.times)).transpose()?;
    if a.duration.is_none() && looping.is_none() {
        return Err(ToolError::invalid("nothing to change: give `duration` and / or `looping`"));
    }
    let (file, rig, (), stored) =
        edit(cx, &a.rig, "rig_clip_update", |rig| rigops::clip_update(rig, &a.name, a.duration, looping).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"clip": a.name}))
}

pub fn rig_clip_delete(cx: &mut ToolCx<'_>, a: ClipArg) -> Result<String, ToolError> {
    let file = resolve(cx, &a.rig)?;
    let used = users(cx, &file, Some(&a.name), None);
    if !used.is_empty() {
        return Err(ToolError::refused(format!("expressions play the clip `{}` ({}): change them first", a.name, used.join(", "))));
    }
    let (file, rig, (), stored) = edit(cx, &a.rig, "rig_clip_delete", |rig| rigops::clip_delete(rig, &a.name).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"deleted_clip": a.name}))
}

pub fn rig_clip_key_set(cx: &mut ToolCx<'_>, a: KeySetArgs) -> Result<String, ToolError> {
    let kind: TrackKind = a.track.into();
    let value = key_value(kind, &a.value)?;
    let ease = a.ease.as_ref().map(|e| curve_of(cx, e)).transpose()?;
    let at = KeyTarget { clip: &a.clip, kind, target: a.target.as_deref(), attachment: a.attachment.as_deref() };
    let (file, rig, report, stored) =
        edit(cx, &a.rig, "rig_clip_key_set", |rig| rigops::clip_key_set(rig, &at, a.time, &value, ease).map_err(op_err))?;
    respond(
        &file,
        &rig,
        &stored,
        json!({"clip": report.clip, "track_index": report.track, "keys_on_track": report.keys, "replaced_existing_key": report.existing}),
    )
}

pub fn rig_clip_key_delete(cx: &mut ToolCx<'_>, a: KeyDeleteArgs) -> Result<String, ToolError> {
    let kind: TrackKind = a.track.into();
    let at = KeyTarget { clip: &a.clip, kind, target: a.target.as_deref(), attachment: a.attachment.as_deref() };
    let (file, rig, report, stored) =
        edit(cx, &a.rig, "rig_clip_key_delete", |rig| rigops::clip_key_delete(rig, &at, a.time).map_err(op_err))?;
    respond(&file, &rig, &stored, json!({"clip": report.clip, "keys_left_on_track": report.keys, "track_removed": report.keys == 0}))
}

pub fn rig_add_to_project(cx: &mut ToolCx<'_>, a: DeleteArgs) -> Result<String, ToolError> {
    let root = assets_root(cx);
    let file = resolve(cx, &a.rig)?;
    let rig = load(&file, &root)?;
    let already = file.asset.is_some();
    let id = match file.asset {
        Some(id) => id,
        None => orbit_project::ops::add_asset(cx.project, AssetKind::Rig, &file.rel)?,
    };
    out(json!({
        "id": id.to_string(), "path": file.rel, "already_declared": already,
        "issues": issues_json(&rig),
        "next": "set_expression_rig makes a character expression play a clip of this rig.",
    }))
}

// -------------------------------------------------------- layered import

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportLayersArgs {
    /// The layered file, a path in the project: Photoshop `.psd`, Krita
    /// `.kra` or OpenRaster `.ora`.
    pub source: String,
    /// An existing rig to add the layers to (see `rig_list`). Absent: a new
    /// rig is created (named by `name`, else after the file).
    pub rig: Option<String>,
    /// Name of the new rig (ignored with `rig`). Never overwrites.
    pub name: Option<String>,
    /// One bone per layer group (named after it, nested like the groups);
    /// the layers of a group follow its bone. Default false: every layer
    /// follows the root bone.
    pub groups_as_bones: Option<bool>,
    /// Import hidden layers too, as slots with no active attachment
    /// (alternate parts such as "eyes closed"; show one with
    /// `rig_slot_update` or an attachment track). Default true.
    pub include_hidden: Option<bool>,
    /// Declare a new rig as a project asset right away (default false).
    pub add_to_project: Option<bool>,
}

pub fn rig_import_layers(cx: &mut ToolCx<'_>, a: ImportLayersArgs) -> Result<String, ToolError> {
    let source = cx.sandbox.resolve(&a.source)?;
    let meta = std::fs::metadata(&source).map_err(|_| ToolError::not_found(format!("no file `{}` in the project", a.source)))?;
    if !meta.is_file() || meta.len() > 512 * 1024 * 1024 {
        return Err(ToolError::invalid(format!("`{}` is not a usable layered file", a.source)));
    }
    let bytes = std::fs::read(&source).map_err(io_err)?;
    let doc = orbit_layers::parse(&bytes).map_err(|e| ToolError::invalid(format!("cannot read `{}`: {e}", a.source)))?;
    drop(bytes);
    let root = assets_root(cx);
    let (file, before, mut rig, is_new) = match &a.rig {
        Some(key) => {
            let file = resolve(cx, key)?;
            let before = load(&file, &root)?;
            let rig = before.clone();
            (file, Some(before), rig, false)
        }
        None => {
            let stem = a.name.clone().unwrap_or_else(|| source.file_stem().and_then(|s| s.to_str()).unwrap_or("rig").to_owned());
            let stem = sanitize(&stem);
            let stem = if stem.is_empty() { "rig".to_owned() } else { stem };
            let rel = format!("{}/{stem}.rig.ron", orbit_project::rigs::RIG_DIR);
            let abs = root.join(&rel);
            if abs.exists() {
                return Err(ToolError::refused(format!("`{rel}` already exists: pick another `name`, or pass `rig` to add to it")));
            }
            (RigFile { asset: None, rel, abs }, None, Rig::default(), true)
        }
    };
    let folder = sanitize(file.rel.rsplit('/').next().unwrap_or("rig").trim_end_matches(".rig.ron"));
    let dir = root.join("images").join(if folder.is_empty() { "layers" } else { &folder });
    std::fs::create_dir_all(&dir).map_err(io_err)?;
    let mut index = 0usize;
    let opts = orbit_layers::ImportOptions {
        groups_as_bones: a.groups_as_bones.unwrap_or(false),
        include_hidden: a.include_hidden.unwrap_or(true),
    };
    let report = {
        let mut save = |name: &str, w: u32, h: u32, rgba: &[u8]| -> Result<String, String> {
            index += 1;
            let stem = sanitize(name);
            let abs = dir.join(format!("{index:02}_{}.png", if stem.is_empty() { "layer" } else { &stem }));
            std::fs::write(&abs, orbit_layers::encode_png(w, h, rgba)?).map_err(|e| e.to_string())?;
            let rel = cx.sandbox.relative(&abs).ok_or("layer file is outside the project")?;
            let imported = import_asset(cx, ImportAssetArgs { source: rel, kind: None, name: None }).map_err(|e| e.to_string())?;
            let v: Json = serde_json::from_str(&imported).unwrap_or(Json::Null);
            v["path"].as_str().map(str::to_owned).ok_or_else(|| "the layer picture was not imported".to_owned())
        };
        orbit_layers::import_into_rig(&mut rig, &doc, &opts, &mut save).map_err(ToolError::invalid)?
    };
    let stored = store(&file, before.as_ref(), &rig, "rig_import_layers")?;
    let asset = if is_new && a.add_to_project.unwrap_or(false) {
        Some(orbit_project::ops::add_asset(cx.project, AssetKind::Rig, &file.rel)?)
    } else {
        file.asset
    };
    let file = RigFile { asset, ..file };
    respond(
        &file,
        &rig,
        &stored,
        json!({
            "canvas": [doc.width, doc.height],
            "slots": report.slots, "hidden_slots": report.hidden, "bones_created": report.bones,
            "import_warnings": report.warnings,
            "hint": "Meshes are unweighted: add bones (rig_bone_add) or use the group bones, then rig_bind.",
        }),
    )
}

// ---------------------------------------------------------------- cut

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CutArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// The slot whose picture is cut (id or name).
    pub slot: String,
    /// Closed polygon `[[x, y], ...]` (at least 3 points) in the picture's
    /// pixel coordinates (origin top-left, Y down): the pixels whose center
    /// is inside (even-odd rule) are cut.
    pub polygon: Vec<[f32; 2]>,
    /// Name of the new slot (default `<slot>_cut`); also names the new picture.
    pub name: Option<String>,
    /// Bone the new part follows (id or name; default: the source slot's bone).
    pub bone: Option<String>,
    /// Draw the new part just behind the source instead of just in front.
    pub below: Option<bool>,
    /// Fill the hole in the source picture from its surroundings (default
    /// true). The filled source is written as a NEW picture (the original
    /// file is kept) and the rig points to it. `false`: the original stays
    /// underneath, unchanged.
    pub fill_hole: Option<bool>,
    /// Skin of the picture (default: the default skin).
    pub skin: Option<String>,
}

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, ToolError> {
    use image::ImageEncoder;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba, w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| ToolError::invalid(format!("cannot encode PNG: {e}")))?;
    Ok(png)
}

/// `<dir>/<stem>.png` next to `beside`, not existing yet on disk.
fn free_picture_path(root: &std::path::Path, beside: &str, stem: &str, taken: &[&str]) -> String {
    let dir = beside.rsplit_once('/').map_or("", |(d, _)| d);
    let base = if dir.is_empty() { stem.to_owned() } else { format!("{dir}/{stem}") };
    let mut n = 1;
    loop {
        let rel = if n == 1 { format!("{base}.png") } else { format!("{base}_{n}.png") };
        if !root.join(&rel).exists() && !taken.contains(&rel.as_str()) {
            return rel;
        }
        n += 1;
    }
}

pub fn rig_cut(cx: &mut ToolCx<'_>, a: CutArgs) -> Result<String, ToolError> {
    let root = assets_root(cx);
    let file = resolve(cx, &a.rig)?;
    let current = load(&file, &root)?;
    let si = rigops::find_slot(&current, &a.slot).map_err(op_err)?;
    let slot = current.slots[si].clone();
    let att_id =
        slot.attachment.clone().ok_or_else(|| ToolError::refused(format!("slot `{}` shows no picture", slot.name)))?;
    let image = current
        .skins
        .iter()
        .filter(|k| a.skin.as_deref().is_none_or(|s| s == k.name))
        .find_map(|k| k.attachments.get(&att_id))
        .map(|att| att.image().to_owned())
        .ok_or_else(|| ToolError::not_found("the slot shows no picture in that skin"))?;
    if a.polygon.len() < 3 {
        return Err(ToolError::invalid("`polygon` needs at least 3 points"));
    }
    let bytes = std::fs::read(root.join(&image))
        .map_err(|e| ToolError::not_found(format!("cannot read picture `{image}`: {e}")))?;
    let img = orbit_assets::decode_image(&bytes).map_err(|e| ToolError::invalid(format!("cannot decode `{image}`: {e}")))?;
    let src = Picture { path: image.clone(), width: img.width, height: img.height, rgba: img.rgba };
    let mask = orbit_project::rigmesh::pixels::polygon_mask(src.width, src.height, &a.polygon);
    let fill = a.fill_hole.unwrap_or(true);
    let src_stem = src.stem();
    let part_stem = sanitize(a.name.as_deref().unwrap_or(&format!("{src_stem}_cut")));
    let part_path = free_picture_path(&root, &image, if part_stem.is_empty() { "part" } else { &part_stem }, &[]);
    let filled_path = free_picture_path(&root, &image, &format!("{src_stem}_filled"), &[&part_path]);
    let mut written: Vec<String> = Vec::new();
    let result = edit(cx, &a.rig, "rig_cut", |rig| {
        let spec = rigops::CutSpec {
            slot: &a.slot,
            skin: a.skin.as_deref(),
            new_image: &part_path,
            name: a.name.as_deref(),
            bone: a.bone.as_deref(),
            below: a.below.unwrap_or(false),
            fill_hole: fill,
        };
        let r = rigops::cut_part(rig, &src, &mask, &spec).map_err(op_err)?;
        let mut write = |rel: &str, w: u32, h: u32, rgba: &[u8]| -> Result<(), ToolError> {
            let abs = root.join(rel);
            if let Some(dir) = abs.parent() {
                std::fs::create_dir_all(dir).map_err(|e| io_err(format!("{}: {e}", abs.display())))?;
            }
            std::fs::write(&abs, encode_png(w, h, rgba)?).map_err(|e| io_err(format!("{}: {e}", abs.display())))?;
            written.push(rel.to_owned());
            Ok(())
        };
        write(&part_path, r.part.width, r.part.height, &r.part.rgba)?;
        if let Some(px) = &r.source {
            write(&filled_path, src.width, src.height, px)?;
            for k in &mut rig.skins {
                for att in k.attachments.values_mut() {
                    match att {
                        Attachment::Mesh(m) if m.image == image => m.image = filled_path.clone(),
                        Attachment::Region(g) if g.image == image => g.image = filled_path.clone(),
                        _ => {}
                    }
                }
            }
        }
        Ok(r)
    });
    let (file, rig, r, stored) = match result {
        Ok(v) => v,
        Err(e) => {
            for rel in &written {
                let _ = std::fs::remove_file(root.join(rel));
            }
            return Err(e);
        }
    };
    for rel in &written {
        orbit_project::ops::add_asset(cx.project, AssetKind::Image, rel)?;
    }
    let source_image = if r.source.is_some() { filled_path } else { image };
    respond(
        &file,
        &rig,
        &stored,
        json!({
            "slot": r.slot.to_string(), "attachment": r.attachment.to_string(),
            "image": part_path, "size": [r.part.width, r.part.height], "offset": r.offset,
            "source_image": source_image, "source_remeshed": r.remeshed,
            "hint": "The new part is rigid on its bone; rig_bind weights it with the others. Hide or swap it in a clip with a slot_attachment key.",
        }),
    )
}

// ------------------------------------------- duplicate, copy clips, recenter

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuplicateArgs {
    /// The rig to copy (see `rig_list`).
    pub rig: String,
    /// Name of the copy: the file is `assets/rigs/<name>.rig.ron`. An
    /// existing file is never overwritten.
    pub name: String,
    /// Also copy the pictures into `images/<name>/` so the copy owns them
    /// (default false: both rigs share the same pictures).
    pub copy_pictures: Option<bool>,
}

pub fn rig_duplicate(cx: &mut ToolCx<'_>, a: DuplicateArgs) -> Result<String, ToolError> {
    let root = assets_root(cx);
    let file = resolve(cx, &a.rig)?;
    let rig = load(&file, &root)?;
    let done = rigops::duplicate_files(&root, &rig, &a.name, a.copy_pictures.unwrap_or(false)).map_err(op_err)?;
    let id = orbit_project::ops::add_asset(cx.project, AssetKind::Rig, &done.rig)?;
    for (_, new) in &done.pictures {
        orbit_project::ops::add_asset(cx.project, AssetKind::Image, new)?;
    }
    out(json!({
        "rig": done.rig, "id": id.to_string(), "copied_from": file.rel,
        "pictures": done.pictures.iter().map(|(o, n)| json!({"from": o, "to": n})).collect::<Vec<_>>(),
        "summary": summary(&rig),
    }))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CopyClipsArgs {
    /// The rig receiving the clips (see `rig_list`).
    pub rig: String,
    /// The rig the clips come from.
    pub source: String,
    /// Clips to copy (default: all of the source's). A name already used in
    /// `rig` gets a number (`walk2`): nothing is overwritten.
    pub clips: Option<Vec<String>>,
    /// Match bones and slots by their position in the hierarchy (same child
    /// index from the root) instead of by name (default false).
    pub by_hierarchy: Option<bool>,
    /// Rescale translation keys by the ratio of the matched bones' lengths
    /// (default false).
    pub rescale: Option<bool>,
}

pub fn rig_copy_clips(cx: &mut ToolCx<'_>, a: CopyClipsArgs) -> Result<String, ToolError> {
    let root = assets_root(cx);
    let src_file = resolve(cx, &a.source)?;
    let src = load(&src_file, &root)?;
    let spec = rigops::CopyClipsSpec {
        clips: a.clips.clone().unwrap_or_default(),
        by_hierarchy: a.by_hierarchy.unwrap_or(false),
        rescale: a.rescale.unwrap_or(false),
    };
    let (file, rig, r, stored) = edit(cx, &a.rig, "rig_copy_clips", |rig| rigops::copy_clips(rig, &src, &spec).map_err(op_err))?;
    respond(
        &file,
        &rig,
        &stored,
        json!({
            "copied": r.copied.iter().map(|(from, to)| json!({"from": from, "to": to})).collect::<Vec<_>>(),
            "tracks_copied": r.tracks_copied, "tracks_dropped": r.tracks_dropped,
            "unmatched_bones": r.unmatched_bones, "unmatched_slots": r.unmatched_slots,
            "unmatched_attachments": r.unmatched_attachments, "dropped_deform": r.dropped_deform,
        }),
    )
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecenterArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// Skin the drawing is measured on (default: the default skin).
    pub skin: Option<String>,
    /// Measure the pictures of every skin together (default false).
    pub all_skins: Option<bool>,
}

pub fn rig_recenter(cx: &mut ToolCx<'_>, a: RecenterArgs) -> Result<String, ToolError> {
    let (file, rig, r, stored) = edit(cx, &a.rig, "rig_recenter", |rig| {
        rigops::recenter(rig, a.skin.as_deref(), a.all_skins.unwrap_or(false)).map_err(op_err)
    })?;
    respond(&file, &rig, &stored, json!({"bounds_before": r.bounds, "shift": r.shift}))
}
