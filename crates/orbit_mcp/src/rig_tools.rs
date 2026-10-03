//! MCP tools for 2D skeletal rigs (M5B-07, ADR 0013): list, read and sample
//! the `.rig.ron` assets of the project, generate meshes and weights from a
//! picture, and make a character expression play a rig clip.
//!
//! A rig is a file outside the story, so (like the 3D scenes) its content is
//! read and written directly and is not covered by undo; the expression that
//! plays it goes through `Command::SetCharacter` and is undoable. Rig units
//! are the pixels of the poster image of the expression, the origin being
//! the bottom-center anchor of the character.

use std::path::PathBuf;

use orbit_anim::rig::{Attachment, PreparedRig, Rig, Severity, save_rig};
use orbit_core::{AssetId, AssetKind, LoopMode, Motion, RiggedMotion};
use orbit_project::Command;
use orbit_project::rigmesh::{meshgen, weights};
use orbit_project::rigs::{RigSummary, list_rigs, read_rig};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::convert::{find_asset, find_character};
use crate::error::ToolError;
use crate::tools::ToolCx;

fn out(value: Json) -> Result<String, ToolError> {
    Ok(serde_json::to_string_pretty(&value).unwrap_or_default())
}

fn io_err(e: impl std::fmt::Display) -> ToolError {
    ToolError::new(crate::ErrorKind::Io, e.to_string())
}

// ------------------------------------------------------------------ files

struct RigFile {
    asset: Option<AssetId>,
    rel: String,
    abs: PathBuf,
}

fn file_stem(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.strip_suffix(".rig.ron")
        .or_else(|| file.rsplit_once('.').map(|(s, _)| s))
        .unwrap_or(file)
}

fn assets_root(cx: &ToolCx<'_>) -> PathBuf {
    orbit_project::find_assets_root(cx.sandbox.root(), cx.project.story())
}

/// A rig by asset id, manifest path, file name or name; also a `.rig.ron`
/// file of the rigs folder that no asset declares yet.
fn resolve(cx: &ToolCx<'_>, key: &str) -> Result<RigFile, ToolError> {
    let story = cx.project.story();
    let root = assets_root(cx);
    let found = find_asset(story, key, Some(AssetKind::Rig)).or_else(|first| {
        find_asset(story, &format!("{key}.rig"), Some(AssetKind::Rig)).map_err(|_| first)
    });
    let (asset, rel) = match found {
        Ok(id) => {
            let rel = story
                .assets
                .get(id)
                .map(|a| a.path.clone())
                .ok_or_else(|| ToolError::not_found(format!("no rig `{key}`")))?;
            (Some(id), rel)
        }
        Err(e) => {
            if e.kind != crate::ErrorKind::NotFound {
                return Err(e);
            }
            let norm = key.replace('\\', "/");
            let hit = list_rigs(&root, story).into_iter().find(|s| {
                s.asset.is_none() && {
                    let file = s.path.rsplit('/').next().unwrap_or(&s.path);
                    s.path == norm || file == norm || file_stem(&s.path) == norm
                }
            });
            match hit {
                Some(s) => (None, s.path),
                None => return Err(e),
            }
        }
    };
    if !orbit_project::ops::is_valid_asset_path(&rel) {
        return Err(ToolError::invalid(format!("`{rel}` is not a project path")));
    }
    let abs = root.join(&rel);
    Ok(RigFile { asset, rel, abs })
}

fn load(file: &RigFile, root: &std::path::Path) -> Result<Rig, ToolError> {
    read_rig(root, &file.rel)
        .map_err(|e| ToolError::invalid(format!("`{}` cannot be used as a rig: {e}", file.rel)))
}

fn issues_json(rig: &Rig) -> Vec<Json> {
    let mut issues: Vec<(Severity, String)> = rig
        .validate()
        .into_iter()
        .map(|i| (i.severity, i.to_string()))
        .collect();
    issues.sort_by_key(|(s, _)| *s != Severity::Error);
    issues
        .into_iter()
        .map(|(s, m)| {
            json!({"severity": if s == Severity::Error { "error" } else { "warning" }, "message": m})
        })
        .collect()
}

fn loop_str(m: LoopMode) -> &'static str {
    match m {
        LoopMode::Loop => "loop",
        LoopMode::Once => "once",
        LoopMode::PingPong => "ping_pong",
    }
}

/// The rigged motion of an expression as JSON (used by the character views).
pub fn motion_json(story: &orbit_core::Story, m: &RiggedMotion) -> Json {
    json!({
        "rig": m.rig.to_string(),
        "path": story.asset_path(m.rig),
        "skin": m.skin,
        "clip": m.clip,
        "loop_mode": loop_str(m.loop_mode),
    })
}

// ------------------------------------------------------------------ args

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RigArg {
    /// The rig: asset id, manifest path, file name or name without extension
    /// (see `rig_list`).
    pub rig: String,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoopArg {
    Loop,
    Once,
    PingPong,
}

impl From<LoopArg> for LoopMode {
    fn from(l: LoopArg) -> Self {
        match l {
            LoopArg::Loop => LoopMode::Loop,
            LoopArg::Once => LoopMode::Once,
            LoopArg::PingPong => LoopMode::PingPong,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SampleArgs {
    /// The rig (see `rig_list`).
    pub rig: String,
    /// Clip name; default: the first clip.
    pub clip: Option<String>,
    /// Seconds since the clip started (default 0). Wrapped like the runtime
    /// does with `loop_mode`.
    pub time: Option<f32>,
    /// What happens past the end of the clip: loop (default), once or
    /// ping_pong (the same as the expression's loop_mode).
    pub loop_mode: Option<LoopArg>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshAutoArgs {
    /// The rig file (see `rig_list`).
    pub rig: String,
    /// The attachment (region or mesh) to turn into a mesh: its id, as shown
    /// by `rig_read` (key of the skin's attachments).
    pub attachment: String,
    /// Skin holding the attachment; default: the skin named `default`, else
    /// the first one.
    pub skin: Option<String>,
    /// Project path of the picture to trace (png/jpg); default: the picture
    /// the attachment already uses.
    pub image: Option<String>,
    /// Outline simplification tolerance in pixels (bigger: fewer vertices).
    pub tolerance: Option<f32>,
    /// Distance between interior vertices in pixels (0: outline only).
    pub spacing: Option<f32>,
    /// Pixels with alpha above this (0..254) belong to the shape (default 8).
    pub alpha_threshold: Option<u8>,
    /// Pixels the shape is grown by before tracing (default 2).
    pub padding: Option<u32>,
    /// Automatic skin weights from the bones (default true).
    pub auto_weights: Option<bool>,
    /// Weight falloff (bigger: sharper joints; default 3).
    pub falloff: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetRigArgs {
    /// The character: id or name.
    pub character: String,
    /// Name of the expression to change.
    pub expression: String,
    /// The rig (asset id, path or name; must be a rig). Absent: the
    /// expression goes back to its still image.
    pub rig: Option<String>,
    /// Skin (outfit) of the rig; default: the rig's default skin.
    pub skin: Option<String>,
    /// Clip to play (required with `rig`).
    pub clip: Option<String>,
    /// What happens at the end of the clip (default loop).
    pub loop_mode: Option<LoopArg>,
}

// ------------------------------------------------------------------ list

fn summary_json(cx: &ToolCx<'_>, s: &RigSummary) -> Json {
    let mut v = json!({
        "id": s.asset.map(|a| a.to_string()),
        "path": s.path,
        "declared": s.asset.is_some(),
    });
    let story = cx.project.story();
    let used_by: Vec<Json> = story
        .characters
        .iter()
        .flat_map(|c| {
            c.expressions
                .iter()
                .filter(|e| e.rigged().is_some_and(|m| Some(m.rig) == s.asset))
                .map(|e| json!({"character": c.id.to_string(), "expression": e.name}))
                .collect::<Vec<_>>()
        })
        .collect();
    v["used_by"] = Json::Array(used_by);
    match &s.content {
        Ok(c) => {
            v["bones"] = json!(c.bones);
            v["slots"] = json!(c.slots);
            v["skins"] = json!(c.skins);
            v["clips"] = Json::Array(
                c.clips
                    .iter()
                    .map(|(n, d)| json!({"name": n, "duration": d}))
                    .collect(),
            );
            v["images"] = json!(c.images);
            v["missing_images"] = json!(c.missing_images);
            v["ik_count"] = json!(c.ik);
            v["issues"] = json!(c.issues);
        }
        Err(e) => v["error"] = json!(e),
    }
    v
}

pub fn rig_list(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    let root = assets_root(cx);
    let rigs = list_rigs(&root, cx.project.story());
    let items: Vec<Json> = rigs.iter().map(|s| summary_json(cx, s)).collect();
    out(json!({"count": items.len(), "rigs": items}))
}

// ------------------------------------------------------------------ read

fn rig_json(rig: &Rig) -> Json {
    let bone_name = |id: &orbit_anim::rig::BoneId| {
        rig.bones
            .iter()
            .find(|b| &b.id == id)
            .map_or_else(|| id.to_string(), |b| b.name.clone())
    };
    json!({
        "rig_version": rig.rig_version,
        "bones": rig.bones.iter().map(|b| json!({
            "id": b.id.to_string(), "name": b.name,
            "parent": b.parent.as_ref().map(bone_name),
            "rest": serde_json::to_value(b.rest).unwrap_or(Json::Null),
            "length": b.length,
        })).collect::<Vec<_>>(),
        "slots": rig.slots.iter().map(|s| json!({
            "id": s.id.to_string(), "name": s.name, "bone": bone_name(&s.bone),
            "attachment": s.attachment.as_ref().map(ToString::to_string),
        })).collect::<Vec<_>>(),
        "skins": rig.skins.iter().map(|k| json!({
            "name": k.name,
            "attachments": k.attachments.iter().map(|(id, a)| match a {
                Attachment::Region(r) => json!({"id": id.to_string(), "type": "region", "image": r.image, "size": r.size}),
                Attachment::Mesh(m) => json!({
                    "id": id.to_string(), "type": "mesh", "image": m.image,
                    "vertices": m.vertices.len(), "triangles": m.triangles.len(),
                    "weighted": !m.weights.is_empty(),
                }),
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "clips": rig.clips.iter().map(|(n, c)| json!({
            "name": n, "duration": c.duration, "looping": format!("{:?}", c.looping),
            "tracks": c.tracks.len(),
        })).collect::<Vec<_>>(),
        "ik": rig.ik.iter().map(|i| json!({
            "bones": i.bones.iter().map(bone_name).collect::<Vec<_>>(),
            "target": bone_name(&i.target), "bend_positive": i.bend_positive, "mix": i.mix,
        })).collect::<Vec<_>>(),
    })
}

pub fn rig_read(cx: &mut ToolCx<'_>, a: RigArg) -> Result<String, ToolError> {
    let root = assets_root(cx);
    let file = resolve(cx, &a.rig)?;
    let rig = load(&file, &root)?;
    out(json!({
        "id": file.asset.map(|i| i.to_string()),
        "path": file.rel,
        "declared": file.asset.is_some(),
        "rig": rig_json(&rig),
        "issues": issues_json(&rig),
        "ron": save_rig(&rig).map_err(ToolError::invalid)?,
    }))
}

// ---------------------------------------------------------------- sample

/// Same wrapping as the runtime (`orbit_runtime::rigs::clip_time`).
fn clip_time(duration: f32, mode: LoopMode, elapsed: f32) -> f32 {
    if !(duration.is_finite() && duration > 0.0 && elapsed.is_finite()) || elapsed <= 0.0 {
        return 0.0;
    }
    match mode {
        LoopMode::Loop => elapsed % duration,
        LoopMode::Once => elapsed.min(duration),
        LoopMode::PingPong => {
            let p = elapsed % (2.0 * duration);
            if p <= duration { p } else { 2.0 * duration - p }
        }
    }
}

fn prepare(rig: Rig) -> Result<PreparedRig, ToolError> {
    PreparedRig::new(rig).map_err(|issues| {
        ToolError::invalid(format!(
            "the rig cannot be evaluated: {}",
            issues.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
        ))
    })
}

pub fn rig_sample(cx: &mut ToolCx<'_>, a: SampleArgs) -> Result<String, ToolError> {
    let root = assets_root(cx);
    let file = resolve(cx, &a.rig)?;
    let prepared = prepare(load(&file, &root)?)?;
    let rig = prepared.rig();
    let (name, clip) = match &a.clip {
        Some(n) => rig.clips.get_key_value(n).ok_or_else(|| {
            ToolError::not_found(format!(
                "no clip `{n}` (clips: {})",
                rig.clips.keys().cloned().collect::<Vec<_>>().join(", ")
            ))
        })?,
        None => rig
            .clips
            .iter()
            .next()
            .ok_or_else(|| ToolError::not_found("the rig has no clip"))?,
    };
    let mode: LoopMode = a.loop_mode.map_or(LoopMode::Loop, Into::into);
    let elapsed = a.time.unwrap_or(0.0);
    let local = clip_time(clip.duration, mode, elapsed);
    let pose = {
        let story = cx.project.story();
        prepared.sample_clip_local_with(clip, local, |n| story.curves.get(n))
    };
    let world = prepared.world(&pose);
    let round = |v: f32| (f64::from(v) * 1000.0).round() / 1000.0;
    let bones: Vec<Json> = rig
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let l = pose.bones[i];
            let w = world.bones[i];
            let o = w.origin();
            json!({
                "name": b.name,
                "local": {"x": round(l.x), "y": round(l.y), "rotation": round(l.rotation), "sx": round(l.sx), "sy": round(l.sy)},
                "world": {
                    "x": round(o[0]), "y": round(o[1]),
                    "rotation": round(w.b.atan2(w.a).to_degrees()),
                    "matrix": [round(w.a), round(w.b), round(w.c), round(w.d), round(w.tx), round(w.ty)],
                },
            })
        })
        .collect();
    let slots: Vec<Json> = rig
        .slots
        .iter()
        .zip(&pose.slots)
        .map(|(s, p)| {
            json!({
                "name": s.name,
                "attachment": p.attachment.as_ref().map(ToString::to_string),
                "color": serde_json::to_value(p.color).unwrap_or(Json::Null),
            })
        })
        .collect();
    out(json!({
        "clip": name, "duration": clip.duration, "loop_mode": loop_str(mode),
        "elapsed": elapsed, "clip_time": round(local),
        "bones": bones, "slots": slots,
        "draw_order": pose.draw_order.iter().filter_map(|i| rig.slots.get(*i)).map(|s| s.name.clone()).collect::<Vec<_>>(),
    }))
}

// ------------------------------------------------------------- mesh auto

pub fn rig_mesh_auto(cx: &mut ToolCx<'_>, a: MeshAutoArgs) -> Result<String, ToolError> {
    let root = assets_root(cx);
    let file = resolve(cx, &a.rig)?;
    let mut rig = load(&file, &root)?;

    let skin_idx = match &a.skin {
        Some(n) => rig
            .skins
            .iter()
            .position(|s| &s.name == n)
            .ok_or_else(|| ToolError::not_found(format!("the rig has no skin `{n}`")))?,
        None => rig
            .skins
            .iter()
            .position(|s| s.name == "default")
            .or(if rig.skins.is_empty() { None } else { Some(0) })
            .ok_or_else(|| ToolError::not_found("the rig has no skin"))?,
    };
    let att_key = orbit_anim::rig::AttachmentId::new(a.attachment.clone());
    let current = rig.skins[skin_idx]
        .attachments
        .get(&att_key)
        .cloned()
        .ok_or_else(|| {
            ToolError::not_found(format!(
                "skin `{}` has no attachment `{}` (see rig_read)",
                rig.skins[skin_idx].name, a.attachment
            ))
        })?;
    let image = a.image.clone().unwrap_or_else(|| current.image().to_owned());
    if !orbit_project::ops::is_valid_asset_path(&image) {
        return Err(ToolError::invalid(format!("`{image}` is not a project path")));
    }
    let bytes = std::fs::read(root.join(&image))
        .map_err(|e| ToolError::not_found(format!("cannot read picture `{image}`: {e}")))?;
    let img = orbit_assets::decode_image(&bytes)
        .map_err(|e| ToolError::invalid(format!("cannot decode `{image}`: {e}")))?;

    // Picture pixels -> rig space (bind pose), as the editor does.
    let uv_to_rig = match &current {
        Attachment::Region(r) => {
            let slot = rig
                .slots
                .iter()
                .find(|s| s.attachment.as_ref() == Some(&att_key))
                .ok_or_else(|| {
                    ToolError::refused(format!(
                        "no slot shows `{}` in the setup pose: make it a slot's attachment first",
                        a.attachment
                    ))
                })?;
            let prepared = prepare(rig.clone())?;
            let bind = prepared.world(&prepared.setup_pose());
            let bone = prepared
                .bone_index(slot.bone.as_str())
                .ok_or_else(|| ToolError::invalid("slot bone not found"))?;
            meshgen::region_uv_to_rig(&bind.bones[bone].mul(&r.xform.matrix()), r.size)
        }
        Attachment::Mesh(m) => meshgen::uv_map(m)
            .and_then(|u| u.inverse())
            .ok_or_else(|| ToolError::invalid("the existing mesh has no usable UV map"))?,
    };

    let mut params = meshgen::MeshParams::for_image(img.width, img.height);
    if let Some(v) = a.tolerance {
        params.tolerance = v.clamp(0.5, 64.0);
    }
    if let Some(v) = a.spacing {
        params.spacing = v.clamp(0.0, 1024.0);
    }
    if let Some(v) = a.alpha_threshold {
        params.alpha_threshold = v.min(254);
    }
    if let Some(v) = a.padding {
        params.padding = v.min(32);
    }
    let g = meshgen::generate(img.width, img.height, &img.rgba, &params).map_err(|e| {
        ToolError::refused(format!("cannot generate a mesh from `{image}`: {e:?}"))
    })?;
    let mut mesh = meshgen::to_attachment(&image, &g, img.width, img.height, &uv_to_rig);
    let weighted = a.auto_weights.unwrap_or(true) && !rig.bones.is_empty();
    if weighted {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for v in &mesh.vertices {
            lo = [lo[0].min(v[0]), lo[1].min(v[1])];
            hi = [hi[0].max(v[0]), hi[1].max(v[1])];
        }
        let diag = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
        mesh.weights = weights::auto_weights(
            &mesh.vertices,
            &weights::bone_segments(&rig),
            a.falloff.unwrap_or(3.0),
            (diag / 20.0).max(1.0),
        );
    }
    let (nv, nt) = (mesh.vertices.len(), mesh.triangles.len());
    rig.skins[skin_idx]
        .attachments
        .insert(att_key, Attachment::Mesh(mesh));

    let errors: Vec<String> = rig
        .validate()
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.to_string())
        .collect();
    if !errors.is_empty() {
        return Err(ToolError::refused(format!(
            "the result would leave errors in the rig, nothing written: {}",
            errors.join("; ")
        )));
    }
    let warnings: Vec<String> = match PreparedRig::new(rig.clone()) {
        Ok(p) => p.warnings.iter().map(ToString::to_string).collect(),
        Err(_) => Vec::new(),
    };
    let text = save_rig(&rig).map_err(ToolError::invalid)?;
    let tmp = file.abs.with_extension("ron.tmp");
    std::fs::write(&tmp, text).map_err(io_err)?;
    std::fs::rename(&tmp, &file.abs).map_err(io_err)?;
    out(json!({
        "rig": file.rel, "skin": rig.skins[skin_idx].name, "attachment": a.attachment,
        "image": image, "vertices": nv, "triangles": nt, "weighted": weighted,
        "skipped_outline_edges": g.skipped,
        "warnings": warnings,
        "note": "The rig file was rewritten (not covered by undo).",
    }))
}

// ------------------------------------------------------ set_expression_rig

pub fn set_expression_rig(cx: &mut ToolCx<'_>, a: SetRigArgs) -> Result<String, ToolError> {
    let story = cx.project.story();
    let id = find_character(story, &a.character)?;
    let mut character = story
        .character(id)
        .cloned()
        .ok_or_else(|| ToolError::not_found("unknown character"))?;
    let idx = character
        .expressions
        .iter()
        .position(|e| e.name == a.expression)
        .ok_or_else(|| {
            ToolError::not_found(format!("character has no expression `{}`", a.expression))
        })?;
    let Some(key) = a.rig.as_deref() else {
        match &character.expressions[idx].motion {
            Some(Motion::Rigged(_)) => character.expressions[idx].motion = None,
            Some(Motion::Animation(_)) => {
                return Err(ToolError::refused(
                    "this expression plays a frame animation, not a rig; nothing to remove",
                ));
            }
            None => {}
        }
        cx.project
            .execute(Command::SetCharacter { id, character: Some(character), index: None })?;
        return out(json!({"expression": a.expression, "motion": null}));
    };
    let clip = a
        .clip
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| ToolError::invalid("`clip` is required with `rig`"))?
        .to_owned();
    let root = assets_root(cx);
    let file = resolve(cx, key)?;
    let rig = load(&file, &root)?;
    if !rig.clips.contains_key(&clip) {
        return Err(ToolError::refused(format!(
            "the rig has no clip `{clip}` (clips: {})",
            rig.clips.keys().cloned().collect::<Vec<_>>().join(", ")
        )));
    }
    if let Some(skin) = &a.skin
        && !rig.skins.iter().any(|s| &s.name == skin)
    {
        return Err(ToolError::refused(format!(
            "the rig has no skin `{skin}` (skins: {})",
            rig.skins.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join(", ")
        )));
    }
    let asset = match file.asset {
        Some(id) => id,
        None => orbit_project::ops::add_asset(cx.project, AssetKind::Rig, &file.rel)?,
    };
    let loop_mode: LoopMode = a.loop_mode.map_or(LoopMode::Loop, Into::into);
    character.expressions[idx].motion = Some(Motion::Rigged(RiggedMotion {
        rig: asset,
        skin: a.skin.clone(),
        clip: clip.clone(),
        loop_mode,
    }));
    cx.project
        .execute(Command::SetCharacter { id, character: Some(character), index: None })?;
    out(json!({
        "expression": a.expression,
        "rig": asset.to_string(), "path": file.rel,
        "skin": a.skin, "clip": clip, "loop_mode": loop_str(loop_mode),
    }))
}
