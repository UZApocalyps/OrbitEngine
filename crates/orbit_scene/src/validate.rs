//! Structural validation: references, cycles, quaternions, NaN.

use std::collections::{HashMap, HashSet};

use crate::{Light, MaterialDef, Node3d, Node3dKind, Prefab, Projection, Scene3d};
use orbit_core::Node3dId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub path: String,
    pub message: String,
}

fn err(path: &str, message: impl Into<String>) -> Issue {
    Issue {
        severity: Severity::Error,
        path: path.to_owned(),
        message: message.into(),
    }
}

fn finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite())
}

pub fn validate(scene: &Scene3d) -> Vec<Issue> {
    let mut out = Vec::new();
    if scene.scene3d_version != crate::SCENE3D_VERSION {
        out.push(err("scene3d_version", "unsupported scene3d_version"));
    }
    nodes_issues(&scene.nodes, &scene.materials, &mut out);
    let mut cams = HashSet::new();
    for (n, c) in scene.cameras() {
        if !cams.insert(c.name.as_str()) {
            out.push(err(&n.name, format!("duplicate camera name `{}`", c.name)));
        }
    }
    out
}

pub fn validate_prefab(prefab: &Prefab) -> Vec<Issue> {
    let mut out = Vec::new();
    nodes_issues(&prefab.nodes, &prefab.materials, &mut out);
    out
}

fn nodes_issues(nodes: &[Node3d], materials: &[MaterialDef], out: &mut Vec<Issue>) {
    let mut ids: HashMap<Node3dId, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        if ids.insert(n.id, i).is_some() {
            out.push(err(&n.name, "duplicate node id"));
        }
    }
    let mat_names: HashSet<&str> = materials.iter().map(|m| m.name.as_str()).collect();
    for m in materials {
        let vals = [
            m.metallic,
            m.roughness,
            m.normal_scale,
            m.emissive_strength,
            m.occlusion_strength,
            m.alpha_cutoff,
        ];
        if !finite(&m.base_color) || !finite(&m.emissive) || !finite(&vals) {
            out.push(err(&m.name, "material has non-finite values"));
        }
    }
    for n in nodes {
        let t = &n.transform;
        if !(finite(&t.translation) && finite(&t.rotation) && finite(&t.scale)) {
            out.push(err(&n.name, "transform contains NaN or infinity"));
        } else {
            let len = t.rotation.iter().map(|x| x * x).sum::<f32>().sqrt();
            if (len - 1.0).abs() > 1e-3 {
                out.push(err(
                    &n.name,
                    format!("rotation is not normalized (|q| = {len})"),
                ));
            }
            if t.scale.contains(&0.0) {
                out.push(err(&n.name, "scale has a zero component"));
            }
        }
        if let Some(p) = n.parent {
            if p == n.id {
                out.push(err(&n.name, "node is its own parent"));
            } else if !ids.contains_key(&p) {
                out.push(err(&n.name, "parent does not exist"));
            }
        }
        match &n.kind {
            Node3dKind::Mesh {
                materials: refs, ..
            } => {
                for r in refs {
                    if !mat_names.contains(r.0.as_str()) {
                        out.push(err(&n.name, format!("unknown material `{}`", r.0)));
                    }
                }
            }
            Node3dKind::Camera(c) => {
                if !(c.near > 0.0 && c.far > c.near && c.near.is_finite() && c.far.is_finite()) {
                    out.push(err(&n.name, "camera needs 0 < near < far"));
                }
                let ok = match c.projection {
                    Projection::Perspective { fov_y } => {
                        fov_y > 0.0 && fov_y < std::f32::consts::PI
                    }
                    Projection::Orthographic { height } => height > 0.0 && height.is_finite(),
                };
                if !ok {
                    out.push(err(&n.name, "invalid camera projection"));
                }
            }
            Node3dKind::Light(l) => {
                let (c, i) = match l {
                    Light::Directional {
                        color, intensity, ..
                    }
                    | Light::Point {
                        color, intensity, ..
                    }
                    | Light::Spot {
                        color, intensity, ..
                    } => (color, *intensity),
                };
                if !finite(c) || !i.is_finite() || i < 0.0 {
                    out.push(err(&n.name, "invalid light color or intensity"));
                }
            }
            Node3dKind::Shape { shape, material } => {
                if !shape.is_valid() {
                    out.push(err(&n.name, "shape sizes must be positive numbers"));
                }
                if let Some(r) = material
                    && !mat_names.contains(r.0.as_str())
                {
                    out.push(err(&n.name, format!("unknown material `{}`", r.0)));
                }
            }
            _ => {}
        }
    }
    for n in nodes {
        let mut cur = n.parent;
        let mut steps = 0;
        while let Some(p) = cur {
            if p == n.id || steps > nodes.len() {
                out.push(err(&n.name, "parent cycle"));
                break;
            }
            cur = ids.get(&p).and_then(|&i| nodes[i].parent);
            steps += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    #[test]
    fn valid_scene_roundtrips_and_validates() {
        let mut s = Scene3d::new("s");
        let root = Node3d::new("root", None, Node3dKind::Empty);
        let cam = Node3d::new(
            "cam",
            Some(root.id),
            Node3dKind::Camera(CameraDef {
                name: "main".into(),
                projection: Projection::Perspective { fov_y: 0.8 },
                near: 0.1,
                far: 100.0,
            }),
        );
        s.nodes = vec![root, cam];
        assert!(validate(&s).is_empty());
        let back = from_ron(&to_ron(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.cameras().count(), 1);
    }

    #[test]
    fn shapes_roundtrip_and_validate() {
        let mut s = Scene3d::new("s");
        s.materials.push(MaterialDef::new("red"));
        for kind in shapes::SHAPE_KINDS {
            s.nodes.push(Node3d::new(
                kind,
                None,
                Node3dKind::Shape {
                    shape: Shape::default_of(kind).unwrap(),
                    material: Some(MaterialRef("red".into())),
                },
            ));
        }
        assert!(validate(&s).is_empty(), "{:?}", validate(&s));
        let text = to_ron(&s).unwrap();
        assert!(text.contains("Capsule"), "{text}");
        assert_eq!(from_ron(&text).unwrap(), s);
    }

    #[test]
    fn detects_problems() {
        let mut s = Scene3d::new("s");
        let mut a = Node3d::new("a", None, Node3dKind::Empty);
        let mut b = Node3d::new("b", Some(a.id), Node3dKind::Empty);
        a.parent = Some(b.id);
        b.transform.rotation = [0.0, 0.0, 0.0, 2.0];
        let mut c = Node3d::new("c", Some(Node3dId::new()), Node3dKind::Empty);
        c.transform.translation[0] = f32::NAN;
        let d = Node3d::new(
            "d",
            None,
            Node3dKind::Mesh {
                mesh: MeshRef("m".into()),
                materials: vec![MaterialRef("x".into())],
            },
        );
        let e = Node3d::new(
            "e",
            None,
            Node3dKind::Shape {
                shape: Shape::Cube { size: [1.0, 0.0, 1.0] },
                material: None,
            },
        );
        s.nodes = vec![a, b, c, d, e];
        let msgs: Vec<_> = validate(&s).into_iter().map(|i| i.message).collect();
        for needle in [
            "shape sizes",
            "cycle",
            "normalized",
            "NaN",
            "parent does not exist",
            "unknown material",
        ] {
            assert!(msgs.iter().any(|m| m.contains(needle)), "{needle}: {msgs:?}");
        }
    }
}
