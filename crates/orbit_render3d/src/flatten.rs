//! Scene graph -> flat draw/light/camera lists (pure CPU, unit tested).

use std::collections::HashMap;

use orbit_scene::{CameraDef, MaterialDef, Node3d, Node3dKind, Prefab, Scene3d};

use crate::math::{self, Mat4};
use crate::uniforms::WorldLight;

/// Nested prefab instances deeper than this are ignored (cycle guard).
const MAX_PREFAB_DEPTH: usize = 8;

/// One primitive to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawItem {
    pub mesh: String,
    pub primitive: usize,
    /// Resolved material (default PBR white when the reference is missing).
    pub material: MaterialDef,
    pub world: Mat4,
    /// Top-level scene node the draw comes from (the instance node for
    /// prefab contents): the key of its [`orbit_scene::Rig`] pose.
    pub owner: Option<orbit_scene::Node3dId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldCamera {
    pub name: String,
    pub world: Mat4,
    pub def: CameraDef,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlatScene {
    pub draws: Vec<DrawItem>,
    pub lights: Vec<WorldLight>,
    pub cameras: Vec<WorldCamera>,
}

/// Number of primitives of a mesh, used to expand one draw per primitive.
pub trait MeshInfo {
    fn primitive_count(&self, mesh: &str) -> Option<usize>;
}

impl<F: Fn(&str) -> Option<usize>> MeshInfo for F {
    fn primitive_count(&self, mesh: &str) -> Option<usize> {
        self(mesh)
    }
}

/// Default material for missing references: white, rough dielectric.
pub fn default_material() -> MaterialDef {
    let mut m = MaterialDef::new("<default>");
    m.metallic = 0.0;
    m.roughness = 0.8;
    m
}

pub fn flatten(
    scene: &Scene3d,
    prefabs: &HashMap<String, Prefab>,
    meshes: &dyn MeshInfo,
) -> FlatScene {
    let mut out = FlatScene::default();
    let ctx = Ctx { prefabs, meshes };
    ctx.walk(
        &scene.nodes,
        &[&scene.materials],
        &math::IDENTITY,
        0,
        None,
        &mut out,
    );
    out
}

struct Ctx<'a> {
    prefabs: &'a HashMap<String, Prefab>,
    meshes: &'a dyn MeshInfo,
}

impl Ctx<'_> {
    fn walk(
        &self,
        nodes: &[Node3d],
        materials: &[&[MaterialDef]],
        root: &Mat4,
        depth: usize,
        owner: Option<orbit_scene::Node3dId>,
        out: &mut FlatScene,
    ) {
        let index: HashMap<_, _> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        let mut world: Vec<Option<Option<Mat4>>> = vec![None; nodes.len()];
        for i in 0..nodes.len() {
            let Some(w) = world_of(i, nodes, &index, root, &mut world, 0) else {
                continue; // hidden (itself or an ancestor)
            };
            let node = &nodes[i];
            let owner = owner.or(Some(node.id));
            match &node.kind {
                Node3dKind::Empty => {}
                Node3dKind::Mesh {
                    mesh,
                    materials: refs,
                } => {
                    let Some(count) = self.meshes.primitive_count(&mesh.0) else {
                        log::warn!("3d: mesh '{}' not uploaded", mesh.0);
                        continue;
                    };
                    for primitive in 0..count {
                        let material = refs
                            .get(primitive)
                            .or(refs.last())
                            .and_then(|r| find_material(materials, &r.0))
                            .cloned()
                            .unwrap_or_else(default_material);
                        out.draws.push(DrawItem {
                            mesh: mesh.0.clone(),
                            primitive,
                            material,
                            world: w,
                            owner,
                        });
                    }
                }
                Node3dKind::Shape { shape, material } => {
                    let mesh = shape.mesh_key();
                    if self.meshes.primitive_count(&mesh).is_none() {
                        log::warn!("3d: shape '{}' not generated", node.name);
                        continue;
                    }
                    let material = material
                        .as_ref()
                        .and_then(|r| find_material(materials, &r.0))
                        .cloned()
                        .unwrap_or_else(default_material);
                    out.draws.push(DrawItem {
                        mesh,
                        primitive: 0,
                        material,
                        world: w,
                        owner,
                    });
                }
                Node3dKind::Light(light) => out.lights.push(WorldLight {
                    light: light.clone(),
                    position: math::transform_point(&w, [0.0; 3]),
                    direction: math::normalize(math::transform_vector(&w, [0.0, 0.0, -1.0])),
                }),
                Node3dKind::Camera(def) => out.cameras.push(WorldCamera {
                    name: def.name.clone(),
                    world: w,
                    def: def.clone(),
                }),
                Node3dKind::Prefab(r) => {
                    if depth >= MAX_PREFAB_DEPTH {
                        log::warn!("3d: prefab nesting too deep at '{}'", r.0);
                        continue;
                    }
                    let Some(prefab) = self.prefabs.get(&r.0) else {
                        log::warn!("3d: prefab '{}' not registered", r.0);
                        continue;
                    };
                    // Overrides win over imported materials (matched by name).
                    let mut mats: Vec<&[MaterialDef]> =
                        vec![&prefab.material_overrides, &prefab.materials];
                    mats.extend_from_slice(materials);
                    self.walk(&prefab.nodes, &mats, &w, depth + 1, owner, out);
                }
            }
        }
    }
}

fn find_material<'a>(sets: &[&'a [MaterialDef]], name: &str) -> Option<&'a MaterialDef> {
    sets.iter()
        .find_map(|set| set.iter().find(|m| m.name == name))
}

/// World matrix of node `i`, `None` if it or an ancestor is hidden.
/// Memoized in `cache`; parent cycles are cut by a depth guard.
fn world_of(
    i: usize,
    nodes: &[Node3d],
    index: &HashMap<orbit_scene::Node3dId, usize>,
    root: &Mat4,
    cache: &mut [Option<Option<Mat4>>],
    depth: usize,
) -> Option<Mat4> {
    if let Some(w) = cache[i] {
        return w;
    }
    let node = &nodes[i];
    let t = &node.transform;
    let local = math::from_trs(t.translation, t.rotation, t.scale);
    let result = if !node.visible {
        None
    } else {
        let parent = match node.parent.and_then(|p| index.get(&p)) {
            Some(&p) if depth < nodes.len() && p != i => {
                world_of(p, nodes, index, root, cache, depth + 1)
            }
            _ => Some(*root),
        };
        parent.map(|p| math::mul(&p, &local))
    };
    cache[i] = Some(result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_scene::{Light, MaterialRef, MeshRef, PrefabRef, Projection};

    fn mesh_node(name: &str, parent: Option<&Node3d>, mats: &[&str]) -> Node3d {
        Node3d::new(
            name,
            parent.map(|p| p.id),
            Node3dKind::Mesh {
                mesh: MeshRef("cube".into()),
                materials: mats.iter().map(|m| MaterialRef((*m).into())).collect(),
            },
        )
    }

    fn meshes(name: &str) -> Option<usize> {
        (name == "cube").then_some(2)
    }

    #[test]
    fn hierarchy_composes_and_hidden_subtrees_are_skipped() {
        let mut scene = Scene3d::new("s");
        let mut root = Node3d::new("root", None, Node3dKind::Empty);
        root.transform.translation = [10.0, 0.0, 0.0];
        let mut child = mesh_node("child", Some(&root), &["red"]);
        child.transform.translation = [0.0, 1.0, 0.0];
        let mut hidden = Node3d::new("hidden", None, Node3dKind::Empty);
        hidden.visible = false;
        let under_hidden = mesh_node("under", Some(&hidden), &[]);
        // Child listed before its parent on purpose.
        scene.nodes = vec![child, root, under_hidden, hidden];
        let mut red = MaterialDef::new("red");
        red.base_color = [1.0, 0.0, 0.0, 1.0];
        scene.materials.push(red.clone());

        let flat = flatten(&scene, &HashMap::new(), &meshes);
        assert_eq!(
            flat.draws.len(),
            2,
            "one draw per primitive, hidden skipped"
        );
        assert_eq!(flat.draws[0].world[3], [10.0, 1.0, 0.0, 1.0]);
        // The second primitive reuses the last material reference.
        assert_eq!(flat.draws[1].material, red);
    }

    #[test]
    fn prefab_instances_lights_cameras_and_missing_refs() {
        let mut prefab = Prefab {
            name: "chair".into(),
            ..Default::default()
        };
        prefab
            .nodes
            .push(mesh_node("seat", None, &["wood", "nope"]));
        prefab.materials.push(MaterialDef::new("wood"));
        let mut over = MaterialDef::new("wood");
        over.roughness = 0.1;
        prefab.material_overrides.push(over.clone());
        let prefabs = HashMap::from([("chair".to_string(), prefab)]);

        let mut scene = Scene3d::new("s");
        let mut inst = Node3d::new("inst", None, Node3dKind::Prefab(PrefabRef("chair".into())));
        inst.transform.translation = [0.0, 0.0, -5.0];
        let sun = Node3d::new(
            "sun",
            None,
            Node3dKind::Light(Light::Directional {
                color: [1.0; 3],
                intensity: 3.0,
                shadows: false,
            }),
        );
        let cam = Node3d::new(
            "cam",
            None,
            Node3dKind::Camera(CameraDef {
                name: "main".into(),
                projection: Projection::Perspective { fov_y: 1.0 },
                near: 0.1,
                far: 100.0,
            }),
        );
        let missing = Node3d::new("ghost", None, Node3dKind::Prefab(PrefabRef("x".into())));
        scene.nodes = vec![inst, sun, cam, missing];

        let flat = flatten(&scene, &prefabs, &meshes);
        assert_eq!(flat.draws.len(), 2);
        assert_eq!(flat.draws[0].material, over);
        assert_eq!(flat.draws[1].material, default_material());
        assert_eq!(flat.draws[0].world[3], [0.0, 0.0, -5.0, 1.0]);
        assert_eq!(flat.lights.len(), 1);
        assert_eq!(flat.lights[0].direction, [0.0, 0.0, -1.0]);
        assert_eq!(flat.cameras[0].name, "main");
        // Prefab contents are owned by their instance (its rig poses them).
        assert!(flat.draws.iter().all(|d| d.owner == Some(scene.nodes[0].id)));
    }

    #[test]
    fn top_level_meshes_own_their_draws() {
        let root = mesh_node("a", None, &["m"]);
        let child = mesh_node("b", Some(&root), &["m"]);
        let mut scene = Scene3d::new("s");
        scene.nodes = vec![root.clone(), child.clone()];
        let flat = flatten(&scene, &HashMap::new(), &meshes);
        let owners: Vec<_> = flat.draws.iter().map(|d| d.owner).collect();
        assert!(owners.contains(&Some(root.id)) && owners.contains(&Some(child.id)));
    }

    #[test]
    fn parent_cycle_does_not_hang() {
        let mut a = Node3d::new("a", None, Node3dKind::Empty);
        let mut b = Node3d::new("b", Some(a.id), Node3dKind::Empty);
        a.parent = Some(b.id);
        b.transform.translation = [1.0, 0.0, 0.0];
        let mut scene = Scene3d::new("s");
        scene.nodes = vec![a, b];
        let flat = flatten(&scene, &HashMap::new(), &meshes);
        assert!(flat.draws.is_empty());
    }
}
