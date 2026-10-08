//! Rigged characters in the 3D editor: finding the skeleton of a node,
//! bone matrices for the overlay and the gizmo, clip playback, and the
//! inspector section (bone tree, named poses, clips).
//!
//! The pose lives in the scene ([`orbit_scene::Rig`], undoable like any
//! edit); playback only overrides it in the picture sent to the renderer,
//! so playing a clip never touches the document.

use egui::{RichText, Ui};
use orbit_editor_api::{EditorContext, icons};
use orbit_render3d::math::{self, Mat4};
use orbit_scene::skin::{Skeleton, mat_mul};
use orbit_scene::{MeshAsset, NamedPose, Node3dId, Node3dKind, Pose, PoseClip, Transform};

use super::State;
use super::geom;
use super::model::{ModelLib, SceneDoc, world_in};

/// The skinned mesh a scene node poses.
#[derive(Debug, Clone)]
pub struct RigTarget {
    /// Mesh asset name (in [`ModelLib::meshes`]).
    pub mesh: String,
    /// World matrix of the skinned mesh node.
    pub mesh_world: Mat4,
}

/// The skinned mesh posed by node `id`: the node itself (a `Mesh` node)
/// or the first skinned mesh of its prefab.
pub fn target(doc: &SceneDoc, lib: &ModelLib, id: Node3dId) -> Option<RigTarget> {
    let node = doc.node(id)?;
    let world = doc.world_matrix(id)?;
    let skinned = |m: &str| lib.meshes.get(m).is_some_and(|m| m.skin.is_some());
    match &node.kind {
        Node3dKind::Mesh { mesh, .. } if skinned(&mesh.0) => Some(RigTarget {
            mesh: mesh.0.clone(),
            mesh_world: world,
        }),
        Node3dKind::Prefab(p) => {
            let prefab = lib.prefabs.get(&p.0)?;
            prefab.nodes.iter().find_map(|n| match &n.kind {
                Node3dKind::Mesh { mesh, .. } if skinned(&mesh.0) => Some(RigTarget {
                    mesh: mesh.0.clone(),
                    mesh_world: math::mul(&world, &world_in(&prefab.nodes, n.id)?),
                }),
                _ => None,
            })
        }
        _ => None,
    }
}

/// World matrix of every bone.
pub fn bone_worlds(skel: &Skeleton, pose: &Pose, mesh_world: &Mat4) -> Vec<Mat4> {
    skel.globals(&skel.locals(pose))
        .iter()
        .map(|g| mat_mul(mesh_world, g))
        .collect()
}

/// World matrix of the space bone `i`'s local transform lives in.
pub fn bone_parent_world(skel: &Skeleton, worlds: &[Mat4], mesh_world: &Mat4, i: usize) -> Mat4 {
    match skel.joints[i].parent {
        Some(p) if p < worlds.len() => worlds[p],
        _ => mat_mul(mesh_world, &skel.joints[i].base),
    }
}

/// Local transform of `joint` in `pose` (its rest transform when unposed).
pub fn local_of(skel: &Skeleton, pose: &Pose, i: usize) -> Transform {
    let j = &skel.joints[i];
    pose.get(&j.name).copied().unwrap_or(j.rest)
}

/// Nearest screen point within `tolerance` of `p`.
pub fn pick_point(points: &[Option<[f32; 2]>], p: [f32; 2], tolerance: f32) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .filter_map(|(i, q)| {
            let q = (*q)?;
            let d = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
            (d <= tolerance).then_some((i, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

// --------------------------------------------------------------- playback

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A glTF animation of the mesh.
    Gltf(String),
    /// An authored [`PoseClip`] of the rig.
    Authored(String),
}

/// A clip being previewed on a node.
#[derive(Debug, Clone, PartialEq)]
pub struct Play {
    pub node: Node3dId,
    pub source: Source,
    pub t: f32,
    pub playing: bool,
}

/// Duration of a clip source.
pub fn duration(mesh: &MeshAsset, rig_clips: &[PoseClip], source: &Source) -> f32 {
    match source {
        Source::Gltf(n) => mesh
            .clips
            .iter()
            .find(|c| &c.name == n)
            .map_or(0.0, |c| c.duration),
        Source::Authored(n) => rig_clips
            .iter()
            .find(|c| &c.name == n)
            .map_or(0.0, PoseClip::duration),
    }
}

/// The pose of `play` at its time (looping).
pub fn sample(mesh: &MeshAsset, rig_clips: &[PoseClip], play: &Play) -> Option<Pose> {
    let skel = mesh.skin.as_ref()?;
    let rest = |n: &str| {
        skel.index_of(n)
            .map_or(Transform::IDENTITY, |i| skel.joints[i].rest)
    };
    match &play.source {
        Source::Gltf(n) => {
            let clip = mesh.clips.iter().find(|c| &c.name == n)?;
            let t = if clip.duration > 0.0 {
                play.t.rem_euclid(clip.duration)
            } else {
                0.0
            };
            Some(clip.sample(skel, t))
        }
        Source::Authored(n) => {
            let clip = rig_clips.iter().find(|c| &c.name == n)?;
            let mut c = clip.clone();
            c.looping = true;
            Some(c.sample(play.t, &rest))
        }
    }
}

/// The pose shown for node `id`: the previewed clip, else the scene pose.
pub fn shown_pose(st: &State, id: Node3dId, mesh: &MeshAsset) -> Pose {
    let doc = st.doc.as_ref();
    let rig = doc.and_then(|d| d.scene().rig(id));
    if let Some(play) = st.play.as_ref().filter(|p| p.node == id)
        && let Some(p) = sample(mesh, rig.map_or(&[][..], |r| &r.clips[..]), play)
    {
        return p;
    }
    rig.map(|r| r.pose.clone()).unwrap_or_default()
}

/// The scene to render: the document with the previewed pose applied.
pub fn scene_for_render(st: &State) -> Option<orbit_scene::Scene3d> {
    let doc = st.doc.as_ref()?;
    let mut scene = doc.scene().clone();
    if let Some(play) = &st.play
        && let Some(t) = target(doc, &st.lib, play.node)
        && let Some(mesh) = st.lib.meshes.get(&t.mesh)
    {
        let clips = doc
            .scene()
            .rig(play.node)
            .map(|r| r.clips.clone())
            .unwrap_or_default();
        if let Some(pose) = sample(mesh, &clips, play) {
            scene.rig_mut(play.node).pose = pose;
        }
    }
    Some(scene)
}

// -------------------------------------------------------------- inspector

fn joint_tree(
    ui: &mut Ui,
    skel: &Skeleton,
    pose: &Pose,
    parent: Option<usize>,
    sel: &mut Option<String>,
) {
    for i in skel.children(parent) {
        let j = &skel.joints[i];
        let posed = pose.get(&j.name).is_some();
        let label = format!(
            "{} {}{}",
            icons::BONE,
            j.name,
            if posed { " •" } else { "" }
        );
        let selected = sel.as_deref() == Some(j.name.as_str());
        let kids = skel.children(Some(i));
        if kids.is_empty() {
            if ui.selectable_label(selected, label).clicked() {
                *sel = Some(j.name.clone());
            }
        } else {
            let id = ui.make_persistent_id(("s3d_bone", &j.name));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    if ui.selectable_label(selected, label).clicked() {
                        *sel = Some(j.name.clone());
                    }
                })
                .body(|ui| joint_tree(ui, skel, pose, Some(i), sel));
        }
    }
}

/// The "Rig" section of the node inspector (skinned nodes only).
pub fn inspector(ui: &mut Ui, ctx: &EditorContext<'_>, st: &mut State, id: Node3dId) {
    let Some(doc) = st.doc.as_ref() else { return };
    let Some(t) = target(doc, &st.lib, id) else {
        return;
    };
    let Some(mesh) = st.lib.meshes.get(&t.mesh).cloned() else {
        return;
    };
    let Some(skel) = mesh.skin.clone() else {
        return;
    };
    let rig = doc
        .scene()
        .rig(id)
        .cloned()
        .unwrap_or_else(|| orbit_scene::Rig::new(id));

    crate::builtin::ui_util::section_title(ui, &ctx.t("s3d.rig"));
    ui.horizontal(|ui| {
        ui.toggle_value(
            &mut st.pose_mode,
            format!("{}  {}", icons::BONE, ctx.t("s3d.pose_mode")),
        )
        .on_hover_text(ctx.t("s3d.pose_mode_hint"));
        if ui
            .add_enabled(
                !rig.pose.is_rest(),
                egui::Button::new(ctx.t("s3d.reset_pose")),
            )
            .clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            doc.edit(None, |s| s.rig_mut(id).pose = Pose::default());
        }
    });
    ui.label(
        RichText::new(ctx.tf("s3d.bones_count", &[("n", &skel.joints.len())]))
            .small()
            .color(super::weak(ui)),
    );

    // Bone tree.
    let mut sel = st.bone.clone();
    egui::ScrollArea::vertical()
        .id_salt("s3d_bones")
        .max_height(180.0)
        .show(ui, |ui| joint_tree(ui, &skel, &rig.pose, None, &mut sel));
    if sel != st.bone {
        st.bone = sel;
        st.pose_mode = true;
    }

    // Selected bone.
    if let Some(name) = st.bone.clone()
        && let Some(i) = skel.index_of(&name)
    {
        ui.add_space(4.0);
        ui.label(RichText::new(format!("{} {}", icons::BONE, name)).strong());
        let mut local = local_of(&skel, &rig.pose, i);
        let mut euler = geom::quat_to_euler_deg(local.rotation);
        let mut changed = false;
        if super::panels::drag3(ui, &ctx.t("s3d.rotation"), &mut euler, 0.5, "°") {
            local.rotation = geom::quat_from_euler_deg(euler);
            changed = true;
        }
        if skel.joints[i].parent.is_none()
            && super::panels::drag3(ui, &ctx.t("s3d.position"), &mut local.translation, 0.01, "")
        {
            changed = true;
        }
        if changed && let Some(doc) = st.doc.as_mut() {
            doc.edit(Some("bone-inspector"), |s| {
                s.rig_mut(id).pose.set(&name, local)
            });
        }
        if ui.small_button(ctx.t("s3d.reset_bone")).clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            doc.edit(None, |s| s.rig_mut(id).pose.reset(&name));
        }
    }

    // Named poses.
    ui.add_space(6.0);
    ui.label(RichText::new(ctx.t("s3d.poses")).strong());
    for p in &rig.poses {
        ui.horizontal(|ui| {
            ui.label(&p.name);
            if ui.small_button(ctx.t("s3d.apply")).clicked()
                && let Some(doc) = st.doc.as_mut()
            {
                let pose = p.pose.clone();
                doc.edit(None, |s| s.rig_mut(id).pose = pose);
                st.play = None;
            }
            if ui
                .small_button(icons::TRASH)
                .on_hover_text(ctx.t("s3d.delete"))
                .clicked()
                && let Some(doc) = st.doc.as_mut()
            {
                let n = p.name.clone();
                doc.edit(None, |s| s.rig_mut(id).poses.retain(|q| q.name != n));
            }
        });
    }
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut st.pose_name)
                .hint_text(ctx.t("s3d.pose_name"))
                .desired_width(120.0),
        );
        let name = st.pose_name.trim().to_owned();
        if ui
            .add_enabled(!name.is_empty(), egui::Button::new(ctx.t("s3d.save_pose")))
            .clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            let pose = rig.pose.clone();
            doc.edit(None, |s| save_named_pose(s.rig_mut(id), &name, pose));
            st.pose_name.clear();
        }
    });

    // Clips.
    ui.add_space(6.0);
    ui.label(RichText::new(ctx.t("s3d.clips")).strong());
    let mut sources: Vec<(Source, String)> = mesh
        .clips
        .iter()
        .map(|c| {
            (
                Source::Gltf(c.name.clone()),
                format!("{} {}", icons::FILM_STRIP, c.name),
            )
        })
        .collect();
    sources.extend(rig.clips.iter().map(|c| {
        (
            Source::Authored(c.name.clone()),
            format!("{} {}", icons::PENCIL_SIMPLE, c.name),
        )
    }));
    if sources.is_empty() {
        ui.label(
            RichText::new(ctx.t("s3d.no_clips"))
                .small()
                .color(super::weak(ui)),
        );
    }
    for (source, label) in &sources {
        let active = st
            .play
            .as_ref()
            .is_some_and(|p| p.node == id && &p.source == source);
        ui.horizontal(|ui| {
            let playing = active && st.play.as_ref().is_some_and(|p| p.playing);
            if ui
                .small_button(if playing { icons::PAUSE } else { icons::PLAY })
                .on_hover_text(ctx.t(if playing { "s3d.pause" } else { "s3d.play" }))
                .clicked()
            {
                match st.play.as_mut().filter(|_| active) {
                    Some(p) => p.playing = !p.playing,
                    None => {
                        st.play = Some(Play {
                            node: id,
                            source: source.clone(),
                            t: 0.0,
                            playing: true,
                        })
                    }
                }
            }
            ui.label(label);
            if let Source::Authored(n) = source
                && ui
                    .small_button(icons::TRASH)
                    .on_hover_text(ctx.t("s3d.delete"))
                    .clicked()
                && let Some(doc) = st.doc.as_mut()
            {
                let n = n.clone();
                doc.edit(None, |s| s.rig_mut(id).clips.retain(|c| c.name != n));
                if active {
                    st.play = None;
                }
            }
        });
    }
    if let Some(play) = st.play.clone().filter(|p| p.node == id) {
        let d = duration(&mesh, &rig.clips, &play.source).max(0.01);
        let mut t = play.t.rem_euclid(d);
        ui.horizontal(|ui| {
            if ui
                .add(egui::Slider::new(&mut t, 0.0..=d).suffix(" s"))
                .changed()
                && let Some(p) = st.play.as_mut()
            {
                p.t = t;
                p.playing = false;
            }
            if ui
                .small_button(icons::STOP)
                .on_hover_text(ctx.t("s3d.stop"))
                .clicked()
            {
                st.play = None;
            }
        });
        ui.horizontal(|ui| {
            if ui
                .button(ctx.t("s3d.bake_pose"))
                .on_hover_text(ctx.t("s3d.bake_pose_hint"))
                .clicked()
                && let Some(pose) = sample(&mesh, &rig.clips, &play)
                && let Some(doc) = st.doc.as_mut()
            {
                doc.edit(None, |s| s.rig_mut(id).pose = pose);
                st.play = None;
            }
            if let Source::Authored(n) = &play.source
                && ui
                    .button(ctx.t("s3d.key_pose"))
                    .on_hover_text(ctx.t("s3d.key_pose_hint"))
                    .clicked()
                && let Some(doc) = st.doc.as_mut()
            {
                let (n, pose) = (n.clone(), rig.pose.clone());
                doc.edit(None, |s| {
                    if let Some(c) = s.rig_mut(id).clips.iter_mut().find(|c| c.name == n) {
                        c.set_key(t, pose);
                    }
                });
            }
        });
    }
    // New authored clip, keyed with the current pose at 0 s.
    if ui
        .small_button(format!("{}  {}", icons::PLUS, ctx.t("s3d.new_clip")))
        .clicked()
        && let Some(doc) = st.doc.as_mut()
    {
        let pose = rig.pose.clone();
        let name = doc.edit(None, |s| new_clip(s.rig_mut(id), pose));
        st.play = Some(Play {
            node: id,
            source: Source::Authored(name),
            t: 0.0,
            playing: false,
        });
    }
}

/// Saves (or overwrites) a named pose.
pub fn save_named_pose(rig: &mut orbit_scene::Rig, name: &str, pose: Pose) {
    match rig.poses.iter_mut().find(|p| p.name == name) {
        Some(p) => p.pose = pose,
        None => rig.poses.push(NamedPose {
            name: name.to_owned(),
            pose,
        }),
    }
}

/// Adds a clip `Clip N` with one key (the current pose at 0 s).
pub fn new_clip(rig: &mut orbit_scene::Rig, pose: Pose) -> String {
    let mut n = rig.clips.len() + 1;
    let name = loop {
        let candidate = format!("Clip {n}");
        if !rig.clips.iter().any(|c| c.name == candidate) {
            break candidate;
        }
        n += 1;
    };
    let mut clip = PoseClip {
        name: name.clone(),
        looping: true,
        ..Default::default()
    };
    clip.set_key(0.0, pose);
    rig.clips.push(clip);
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_scene::skin::{IDENTITY, Joint};

    fn arm() -> Skeleton {
        Skeleton {
            joints: vec![
                Joint {
                    name: "shoulder".into(),
                    parent: None,
                    rest: Transform::IDENTITY,
                    base: IDENTITY,
                    inverse_bind: IDENTITY,
                },
                Joint {
                    name: "elbow".into(),
                    parent: Some(0),
                    rest: Transform {
                        translation: [1.0, 0.0, 0.0],
                        ..Transform::IDENTITY
                    },
                    base: IDENTITY,
                    inverse_bind: IDENTITY,
                },
            ],
        }
    }

    #[test]
    fn bone_worlds_follow_the_node_and_the_pose() {
        let s = arm();
        let mut node = math::IDENTITY;
        node[3] = [0.0, 2.0, 0.0, 1.0];
        let w = bone_worlds(&s, &Pose::default(), &node);
        assert_eq!([w[1][3][0], w[1][3][1]], [1.0, 2.0]);
        let mut pose = Pose::default();
        let h = std::f32::consts::FRAC_PI_4;
        pose.set(
            "shoulder",
            Transform {
                rotation: [0.0, 0.0, h.sin(), h.cos()],
                ..Transform::IDENTITY
            },
        );
        let w = bone_worlds(&s, &pose, &node);
        assert!((w[1][3][0]).abs() < 1e-5 && (w[1][3][1] - 3.0).abs() < 1e-5);
        assert_eq!(bone_parent_world(&s, &w, &node, 1), w[0]);
        assert_eq!(bone_parent_world(&s, &w, &node, 0), node);
    }

    #[test]
    fn picks_the_nearest_bone_in_reach() {
        let pts = [Some([0.0, 0.0]), None, Some([10.0, 0.0]), Some([4.0, 0.0])];
        assert_eq!(pick_point(&pts, [5.0, 0.0], 6.0), Some(3));
        assert_eq!(pick_point(&pts, [50.0, 0.0], 6.0), None);
    }

    #[test]
    fn named_poses_and_clips_are_added_once() {
        let mut rig = orbit_scene::Rig::new(Node3dId::new());
        let mut p = Pose::default();
        p.set("elbow", Transform::IDENTITY);
        save_named_pose(&mut rig, "wave", Pose::default());
        save_named_pose(&mut rig, "wave", p.clone());
        assert_eq!(rig.poses.len(), 1);
        assert_eq!(rig.poses[0].pose, p);
        assert_eq!(new_clip(&mut rig, p.clone()), "Clip 1");
        assert_eq!(new_clip(&mut rig, p), "Clip 2");
        assert_eq!(rig.clips[0].keys.len(), 1);
    }
}
