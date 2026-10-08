//! The 3D scene document of the 3D editor: node-tree operations, materials,
//! picking, undo / redo (snapshot stack, like the interface editor) and
//! save / load of a `.scene3d.ron`. No egui, no GPU: unit tested headlessly.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use orbit_render3d::math::{self, Mat4};
use orbit_scene::{
    CameraDef, Light, MaterialDef, MaterialRef, MeshAsset, Node3d, Node3dId, Node3dKind, Prefab,
    PrefabRef, Projection, Scene3d, TextureAsset, Transform,
};

use super::geom::{self, Bounds, Ray};

const MAX_UNDO: usize = 200;
/// Material given to new shapes (created in the scene on first use).
pub const DEFAULT_MATERIAL: &str = "Default";

/// Local bounds of a procedural shape.
pub fn shape_bounds(shape: &orbit_scene::Shape) -> Bounds {
    Bounds::of_points(&shape.primitive().positions).unwrap_or(Bounds::cube(0.5))
}

/// Half height of a shape: lifting it by this puts it on the ground.
fn shape_half_height(shape: &orbit_scene::Shape) -> f32 {
    let b = shape_bounds(shape);
    -b.min[1]
}
const MAX_DEPTH: usize = 64;


// ------------------------------------------------------------ model library

/// Imported glTF assets available to the scene (meshes, prefabs, textures),
/// loaded from the `models/` folder next to the scene file.
#[derive(Default, Clone)]
pub struct ModelLib {
    pub meshes: HashMap<String, MeshAsset>,
    pub prefabs: HashMap<String, Prefab>,
    pub textures: Vec<TextureAsset>,
    /// Bumped on every change; the host re-uploads the assets when it moves.
    pub generation: u64,
}

impl ModelLib {
    pub fn add_imported(&mut self, imp: orbit_scene::import::Imported) {
        for m in imp.meshes {
            self.meshes.insert(m.name.clone(), m);
        }
        self.textures.extend(imp.textures);
        self.prefabs.insert(imp.prefab.name.clone(), imp.prefab);
        self.generation += 1;
    }

    /// Names of the textures a material slot or the sky can use, sorted.
    pub fn texture_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.textures.iter().map(|t| t.name.clone()).collect();
        names.sort();
        names.dedup();
        names
    }

    /// Decodes a PNG / JPEG into a texture called `name` (replacing a
    /// texture of the same name).
    pub fn add_texture_file(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        // KTX2 (M6-13): uncompressed RGBA / RGB, zstd or zlib supercompressed.
        if orbit_scene::ktx2::is_ktx2(bytes) {
            let texture = orbit_scene::ktx2::decode_rgba8(name, bytes)?;
            if texture.width > 8192 || texture.height > 8192 {
                return Err(format!("unsupported size {}x{}", texture.width, texture.height));
            }
            self.textures.retain(|t| t.name != name);
            self.textures.push(texture);
            self.generation += 1;
            return Ok(());
        }
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        let (width, height) = (img.width(), img.height());
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err(format!("unsupported size {width}x{height}"));
        }
        self.textures.retain(|t| t.name != name);
        self.textures.push(TextureAsset {
            name: name.to_owned(),
            width,
            height,
            rgba8: img.to_rgba8().into_raw(),
        });
        self.generation += 1;
        Ok(())
    }

    /// Imports every `.glb` / `.gltf` of `dir` and loads its `.png` / `.jpg`
    /// files as textures (named after the file). Returns the library and the
    /// files that could not be imported.
    pub fn load_dir(dir: &Path) -> (Self, Vec<String>) {
        let mut lib = Self::default();
        let mut errors = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return (lib, errors);
        };
        let mut files: Vec<PathBuf> = rd
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| {
                        matches!(
                            e.to_ascii_lowercase().as_str(),
                            "glb" | "gltf" | "png" | "jpg" | "jpeg" | "ktx2"
                        )
                    })
            })
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("model")
                .to_owned();
            let is_image = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| !e.to_ascii_lowercase().starts_with("gl"));
            if is_image {
                if let Err(e) = std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| lib.add_texture_file(&name, &b))
                {
                    errors.push(format!("{}: {e}", path.display()));
                }
                continue;
            }
            match orbit_scene::import::import_gltf_file(&path) {
                Ok(imp) => lib.add_imported(imp),
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
        (lib, errors)
    }

    fn mesh_bounds(&self, name: &str) -> Option<Bounds> {
        let mesh = self.meshes.get(name)?;
        let mut out: Option<Bounds> = None;
        for p in &mesh.primitives {
            if let Some(b) = Bounds::of_points(&p.positions) {
                match &mut out {
                    Some(o) => o.union(&b),
                    None => out = Some(b),
                }
            }
        }
        out
    }

    fn prefab_bounds(&self, name: &str) -> Option<Bounds> {
        let prefab = self.prefabs.get(name)?;
        let mut out: Option<Bounds> = None;
        for n in &prefab.nodes {
            let Node3dKind::Mesh { mesh, .. } = &n.kind else {
                continue;
            };
            let Some(b) = self.mesh_bounds(&mesh.0) else {
                continue;
            };
            let w = world_in(&prefab.nodes, n.id).unwrap_or(math::IDENTITY);
            let b = b.transformed(&w);
            match &mut out {
                Some(o) => o.union(&b),
                None => out = Some(b),
            }
        }
        out
    }

    /// Local-space bounds of what a node draws (`None`: markers).
    pub fn kind_bounds(&self, kind: &Node3dKind) -> Option<Bounds> {
        match kind {
            Node3dKind::Mesh { mesh, .. } => {
                Some(self.mesh_bounds(&mesh.0).unwrap_or(Bounds::cube(0.5)))
            }
            Node3dKind::Prefab(p) => Some(self.prefab_bounds(&p.0).unwrap_or(Bounds::cube(0.5))),
            Node3dKind::Shape { shape, .. } => Some(shape_bounds(shape)),
            _ => None,
        }
    }
}

// ----------------------------------------------------------- tree helpers

fn local_matrix(t: &Transform) -> Mat4 {
    math::from_trs(t.translation, t.rotation, t.scale)
}

/// World matrix of `id` inside `nodes` (cycle / missing parent safe).
pub fn world_in(nodes: &[Node3d], id: Node3dId) -> Option<Mat4> {
    let by_id: HashMap<Node3dId, &Node3d> = nodes.iter().map(|n| (n.id, n)).collect();
    let mut chain = Vec::new();
    let mut cur = Some(id);
    while let Some(c) = cur {
        if chain.len() > MAX_DEPTH {
            return None;
        }
        let n = by_id.get(&c)?;
        chain.push(n.transform);
        cur = n.parent;
    }
    let mut m = math::IDENTITY;
    for t in chain.iter().rev() {
        m = math::mul(&m, &local_matrix(t));
    }
    Some(m)
}

/// TRS decomposition of an affine matrix.
pub fn decompose(m: &Mat4) -> Transform {
    let sx = geom::len([m[0][0], m[0][1], m[0][2]]);
    let sy = geom::len([m[1][0], m[1][1], m[1][2]]);
    let sz = geom::len([m[2][0], m[2][1], m[2][2]]);
    Transform {
        translation: [m[3][0], m[3][1], m[3][2]],
        rotation: geom::mat_rotation(m),
        scale: [sx, sy, sz],
    }
}

/// Local transform after rotating a node about the world axis by `delta`.
pub fn rotated_local(base: &Transform, parent_world: &Mat4, delta: [f32; 4]) -> Transform {
    let p = geom::mat_rotation(parent_world);
    // L' = P^-1 * D * P * L
    let l = geom::quat_mul(
        geom::quat_mul(geom::quat_conj(p), geom::quat_mul(delta, p)),
        base.rotation,
    );
    Transform {
        rotation: geom::quat_normalize(l),
        ..*base
    }
}

/// Local transform that puts a node at world position `world_pos`.
pub fn moved_local(base: &Transform, parent_world: &Mat4, world_pos: [f32; 3]) -> Transform {
    let inv = math::inverse(parent_world).unwrap_or(math::IDENTITY);
    Transform {
        translation: math::transform_point(&inv, world_pos),
        ..*base
    }
}

// --------------------------------------------------------------- document

/// What kind of node to add.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    Empty,
    Directional,
    Point,
    Spot,
    Camera,
    /// A procedural primitive (`orbit_scene::shapes::SHAPE_KINDS` id).
    Shape(&'static str),
}

pub struct SceneDoc {
    pub path: Option<PathBuf>,
    scene: Scene3d,
    saved: Scene3d,
    undo: Vec<Scene3d>,
    redo: Vec<Scene3d>,
    open_key: Option<String>,
    /// Bumped on every change (incl. undo / redo): the viewport redraws.
    pub revision: u64,
}

impl SceneDoc {
    pub fn new(name: &str) -> Self {
        let mut scene = Scene3d::new(name);
        // A fresh scene is usable at once: a sun and a camera.
        let mut sun = Node3d::new(
            "Sun",
            None,
            Node3dKind::Light(Light::Directional {
                color: [1.0; 3],
                intensity: 3.0,
                shadows: true,
            }),
        );
        sun.transform.rotation = geom::quat_from_euler_deg([-50.0, 30.0, 0.0]);
        sun.transform.translation = [0.0, 4.0, 0.0];
        let mut cam = Node3d::new(
            "Camera",
            None,
            Node3dKind::Camera(CameraDef {
                name: "main".into(),
                projection: Projection::Perspective {
                    fov_y: 50f32.to_radians(),
                },
                near: 0.1,
                far: 200.0,
            }),
        );
        cam.transform.translation = [0.0, 1.5, 5.0];
        scene.nodes.push(sun);
        scene.nodes.push(cam);
        scene.environment.background = orbit_scene::Background::Color([0.12, 0.13, 0.16]);
        scene.environment.ambient_intensity = 0.3;
        Self::from_scene(scene, None, true)
    }

    fn from_scene(scene: Scene3d, path: Option<PathBuf>, dirty: bool) -> Self {
        let mut saved = scene.clone();
        if dirty {
            // Never saved: always dirty until the first save.
            saved.name.push('\u{0}');
        }
        Self {
            path,
            saved,
            scene,
            undo: Vec::new(),
            redo: Vec::new(),
            open_key: None,
            revision: 1,
        }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let scene = orbit_scene::from_ron(&text)?;
        Ok(Self::from_scene(scene, Some(path.to_owned()), false))
    }

    pub fn save(&mut self) -> Result<(), String> {
        let path = self.path.clone().ok_or("no path")?;
        self.save_as(&path)
    }

    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        let text = orbit_scene::to_ron(&self.scene)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())?;
        self.path = Some(path.to_owned());
        self.saved = self.scene.clone();
        Ok(())
    }

    pub fn dirty(&self) -> bool {
        self.scene != self.saved
    }

    pub fn scene(&self) -> &Scene3d {
        &self.scene
    }

    // ---- history

    /// Applies `f` as one undoable step. Consecutive edits with the same
    /// `key` (a drag) are merged until [`SceneDoc::end_gesture`].
    pub fn edit<R>(&mut self, key: Option<&str>, f: impl FnOnce(&mut Scene3d) -> R) -> R {
        let merge = key.is_some() && self.open_key.as_deref() == key;
        if !merge {
            self.undo.push(self.scene.clone());
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.open_key = key.map(str::to_owned);
        let r = f(&mut self.scene);
        self.revision += 1;
        r
    }

    /// Ends a merged edit (the pointer was released).
    pub fn end_gesture(&mut self) {
        self.open_key = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.scene, prev));
        self.open_key = None;
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.scene, next));
        self.open_key = None;
        self.revision += 1;
        true
    }

    // ---- queries

    pub fn node(&self, id: Node3dId) -> Option<&Node3d> {
        self.scene.nodes.iter().find(|n| n.id == id)
    }

    pub fn children(&self, parent: Option<Node3dId>) -> Vec<Node3dId> {
        self.scene
            .nodes
            .iter()
            .filter(|n| n.parent == parent)
            .map(|n| n.id)
            .collect()
    }

    pub fn world_matrix(&self, id: Node3dId) -> Option<Mat4> {
        world_in(&self.scene.nodes, id)
    }

    pub fn parent_world(&self, id: Node3dId) -> Mat4 {
        self.node(id)
            .and_then(|n| n.parent)
            .and_then(|p| self.world_matrix(p))
            .unwrap_or(math::IDENTITY)
    }

    fn is_descendant(&self, id: Node3dId, ancestor: Node3dId) -> bool {
        let mut cur = self.node(id).and_then(|n| n.parent);
        let mut guard = 0;
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            guard += 1;
            if guard > MAX_DEPTH {
                return true;
            }
            cur = self.node(c).and_then(|n| n.parent);
        }
        false
    }

    /// A node is shown when it and all its ancestors are visible.
    pub fn effectively_visible(&self, id: Node3dId) -> bool {
        let mut cur = Some(id);
        let mut guard = 0;
        while let Some(c) = cur {
            let Some(n) = self.node(c) else { return false };
            if !n.visible {
                return false;
            }
            guard += 1;
            if guard > MAX_DEPTH {
                return false;
            }
            cur = n.parent;
        }
        true
    }

    /// Bounds of a node in world space; markers (lights, cameras, empties)
    /// get a box of half-size `marker_half`.
    pub fn world_bounds(&self, id: Node3dId, lib: &ModelLib, marker_half: f32) -> Option<Bounds> {
        let n = self.node(id)?;
        let w = self.world_matrix(id)?;
        match lib.kind_bounds(&n.kind) {
            Some(b) => Some(b.transformed(&w)),
            None => {
                let c = [w[3][0], w[3][1], w[3][2]];
                Some(Bounds {
                    min: [c[0] - marker_half, c[1] - marker_half, c[2] - marker_half],
                    max: [c[0] + marker_half, c[1] + marker_half, c[2] + marker_half],
                })
            }
        }
    }

    /// Bounds of the whole scene (for "frame all").
    pub fn scene_bounds(&self, lib: &ModelLib) -> Option<Bounds> {
        let mut out: Option<Bounds> = None;
        for n in &self.scene.nodes {
            if let Some(b) = self.world_bounds(n.id, lib, 0.25) {
                match &mut out {
                    Some(o) => o.union(&b),
                    None => out = Some(b),
                }
            }
        }
        out
    }

    /// Nearest visible node under `ray` (CPU ray against oriented bounds;
    /// markers use a box of half-size `marker_half`).
    pub fn pick(&self, ray: &Ray, lib: &ModelLib, marker_half: f32) -> Option<Node3dId> {
        let mut best: Option<(f32, Node3dId)> = None;
        for n in &self.scene.nodes {
            if !self.effectively_visible(n.id) {
                continue;
            }
            let Some(w) = self.world_matrix(n.id) else {
                continue;
            };
            let t = match lib.kind_bounds(&n.kind) {
                Some(local) => geom::ray_oriented_bounds(ray, &w, &local),
                None => {
                    // Marker: world-space box around the node origin.
                    let c = [w[3][0], w[3][1], w[3][2]];
                    let b = Bounds {
                        min: [c[0] - marker_half, c[1] - marker_half, c[2] - marker_half],
                        max: [c[0] + marker_half, c[1] + marker_half, c[2] + marker_half],
                    };
                    geom::ray_aabb(ray, &b)
                }
            };
            if let Some(t) = t
                && best.is_none_or(|(bt, _)| t < bt)
            {
                best = Some((t, n.id));
            }
        }
        best.map(|(_, id)| id)
    }

    // ---- node operations

    fn unique_name(&self, base: &str) -> String {
        let names: HashSet<&str> = self.scene.nodes.iter().map(|n| n.name.as_str()).collect();
        if !names.contains(base) {
            return base.to_owned();
        }
        (2..)
            .map(|i| format!("{base} {i}"))
            .find(|n| !names.contains(n.as_str()))
            .unwrap_or_else(|| base.to_owned())
    }

    fn unique_camera_name(&self, base: &str) -> String {
        let names: HashSet<&str> = self.scene.cameras().map(|(_, c)| c.name.as_str()).collect();
        if !names.contains(base) {
            return base.to_owned();
        }
        (2..)
            .map(|i| format!("{base}{i}"))
            .find(|n| !names.contains(n.as_str()))
            .unwrap_or_else(|| base.to_owned())
    }

    pub fn add_node(&mut self, kind: NewKind, parent: Option<Node3dId>) -> Node3dId {
        let parent = parent.filter(|p| self.node(*p).is_some());
        if let NewKind::Shape(shape_kind) = kind {
            return self.add_shape(shape_kind, parent);
        }
        let (name, node_kind) = match kind {
            NewKind::Shape(_) => unreachable!("handled above"),
            NewKind::Empty => ("Empty", Node3dKind::Empty),
            NewKind::Directional => (
                "Directional light",
                Node3dKind::Light(Light::Directional {
                    color: [1.0; 3],
                    intensity: 3.0,
                    shadows: false,
                }),
            ),
            NewKind::Point => (
                "Point light",
                Node3dKind::Light(Light::Point {
                    color: [1.0; 3],
                    intensity: 10.0,
                    range: None,
                }),
            ),
            NewKind::Spot => (
                "Spot light",
                Node3dKind::Light(Light::Spot {
                    color: [1.0; 3],
                    intensity: 20.0,
                    range: None,
                    inner_angle: 20f32.to_radians(),
                    outer_angle: 35f32.to_radians(),
                }),
            ),
            NewKind::Camera => (
                "Camera",
                Node3dKind::Camera(CameraDef {
                    name: self.unique_camera_name("camera"),
                    projection: Projection::Perspective {
                        fov_y: 50f32.to_radians(),
                    },
                    near: 0.1,
                    far: 200.0,
                }),
            ),
        };
        let node = Node3d::new(self.unique_name(name), parent, node_kind);
        let id = node.id;
        self.edit(None, |s| s.nodes.push(node));
        id
    }

    pub fn add_prefab_instance(&mut self, prefab: &str, parent: Option<Node3dId>) -> Node3dId {
        self.add_prefab_instance_at(prefab, parent, [0.0; 3])
    }

    /// Adds a prefab instance at world position `at` (one undo step).
    pub fn add_prefab_instance_at(
        &mut self,
        prefab: &str,
        parent: Option<Node3dId>,
        at: [f32; 3],
    ) -> Node3dId {
        let parent = parent.filter(|p| self.node(*p).is_some());
        let mut node = Node3d::new(
            self.unique_name(prefab),
            parent,
            Node3dKind::Prefab(PrefabRef(prefab.to_owned())),
        );
        if at != [0.0; 3] {
            let pw = parent.and_then(|p| self.world_matrix(p)).unwrap_or(math::IDENTITY);
            node.transform = moved_local(&node.transform, &pw, at);
        }
        let id = node.id;
        self.edit(None, |s| s.nodes.push(node));
        id
    }

    #[cfg(test)]
    pub fn add_mesh(
        &mut self,
        mesh: &str,
        material: Option<&str>,
        parent: Option<Node3dId>,
    ) -> Node3dId {
        let node = Node3d::new(
            self.unique_name(mesh),
            parent.filter(|p| self.node(*p).is_some()),
            Node3dKind::Mesh {
                mesh: orbit_scene::MeshRef(mesh.to_owned()),
                materials: material.map(|m| vec![MaterialRef(m.to_owned())]).unwrap_or_default(),
            },
        );
        let id = node.id;
        self.edit(None, |s| s.nodes.push(node));
        id
    }

    /// Deletes a node and its descendants.
    pub fn delete_node(&mut self, id: Node3dId) -> bool {
        if self.node(id).is_none() {
            return false;
        }
        let mut doomed = HashSet::from([id]);
        loop {
            let before = doomed.len();
            for n in &self.scene.nodes {
                if n.parent.is_some_and(|p| doomed.contains(&p)) {
                    doomed.insert(n.id);
                }
            }
            if doomed.len() == before {
                break;
            }
        }
        self.edit(None, |s| {
            s.nodes.retain(|n| !doomed.contains(&n.id));
            s.rigs.retain(|r| !doomed.contains(&r.node));
        });
        true
    }

    /// World bounds of a node and its descendants (markers excluded unless
    /// the subtree has nothing else).
    pub fn subtree_bounds(&self, id: Node3dId, lib: &ModelLib, marker_half: f32) -> Option<Bounds> {
        let mut out: Option<Bounds> = None;
        let mut stack = vec![id];
        let mut guard = 0;
        while let Some(cur) = stack.pop() {
            guard += 1;
            if guard > 100_000 {
                break;
            }
            if let Some(n) = self.node(cur)
                && lib.kind_bounds(&n.kind).is_some()
                && let Some(b) = self.world_bounds(cur, lib, marker_half)
            {
                match &mut out {
                    Some(o) => o.union(&b),
                    None => out = Some(b),
                }
            }
            stack.extend(self.children(Some(cur)));
        }
        out.or_else(|| self.world_bounds(id, lib, marker_half))
    }

    /// Moves a node vertically so that its subtree rests on the ground
    /// (`y = 0`). Returns false when nothing moved.
    pub fn snap_to_ground(&mut self, id: Node3dId, lib: &ModelLib) -> bool {
        let (Some(b), Some(node), Some(w)) = (
            self.subtree_bounds(id, lib, 0.0),
            self.node(id).cloned(),
            self.world_matrix(id),
        ) else {
            return false;
        };
        let dy = -b.min[1];
        if dy.abs() < 1e-5 {
            return false;
        }
        let pos = [w[3][0], w[3][1] + dy, w[3][2]];
        let t = moved_local(&node.transform, &self.parent_world(id), pos);
        self.set_transform(id, "ground", t);
        true
    }

    /// Places node `id` so that its world matrix becomes `world` (rotation
    /// and position; its scale is kept). Used by "align camera to view".
    pub fn align_to(&mut self, id: Node3dId, world: &Mat4) -> bool {
        let Some(node) = self.node(id).cloned() else {
            return false;
        };
        let inv = math::inverse(&self.parent_world(id)).unwrap_or(math::IDENTITY);
        let local = decompose(&math::mul(&inv, world));
        let t = Transform {
            translation: local.translation,
            rotation: local.rotation,
            scale: node.transform.scale,
        };
        self.set_transform(id, "align", t);
        true
    }

    /// Duplicates a node with its subtree (new ids, new unique names).
    pub fn duplicate_node(&mut self, id: Node3dId) -> Option<Node3dId> {
        self.node(id)?;
        let mut subtree = vec![id];
        let mut i = 0;
        while i < subtree.len() {
            let cur = subtree[i];
            subtree.extend(self.children(Some(cur)));
            i += 1;
        }
        let mut map: HashMap<Node3dId, Node3dId> = HashMap::new();
        let mut copies = Vec::new();
        for (k, old) in subtree.iter().enumerate() {
            let mut n = self.node(*old)?.clone();
            let new_id = Node3dId::new();
            map.insert(*old, new_id);
            n.id = new_id;
            if k == 0 {
                n.name = self.unique_name(&format!("{} copy", n.name));
            } else {
                n.parent = n.parent.and_then(|p| map.get(&p).copied());
            }
            if let Node3dKind::Camera(c) = &mut n.kind {
                c.name = self.unique_camera_name(&c.name);
            }
            copies.push(n);
        }
        let root = copies.first()?.id;
        let rigs: Vec<orbit_scene::Rig> = self
            .scene
            .rigs
            .iter()
            .filter_map(|r| {
                let mut r = r.clone();
                r.node = *map.get(&r.node)?;
                Some(r)
            })
            .collect();
        self.edit(None, |s| {
            s.nodes.extend(copies);
            s.rigs.extend(rigs);
        });
        Some(root)
    }

    /// Moves a node under `new_parent` (`None`: root), keeping its world
    /// transform. Refused when it would create a cycle.
    pub fn reparent(&mut self, id: Node3dId, new_parent: Option<Node3dId>) -> bool {
        if self.node(id).is_none() {
            return false;
        }
        if let Some(p) = new_parent
            && (p == id || self.node(p).is_none() || self.is_descendant(p, id))
        {
            return false;
        }
        let world = self.world_matrix(id).unwrap_or(math::IDENTITY);
        let parent_world = new_parent
            .and_then(|p| self.world_matrix(p))
            .unwrap_or(math::IDENTITY);
        let local = math::inverse(&parent_world)
            .map(|inv| decompose(&math::mul(&inv, &world)))
            .unwrap_or_else(|| self.node(id).map_or(Transform::IDENTITY, |n| n.transform));
        self.edit(None, |s| {
            if let Some(n) = s.nodes.iter_mut().find(|n| n.id == id) {
                n.parent = new_parent;
                n.transform = local;
            }
        });
        true
    }

    pub fn rename(&mut self, id: Node3dId, name: &str) {
        let name = name.to_owned();
        self.edit(Some(&format!("rename:{id:?}")), |s| {
            if let Some(n) = s.nodes.iter_mut().find(|n| n.id == id) {
                n.name = name;
            }
        });
    }

    pub fn set_visible(&mut self, id: Node3dId, visible: bool) {
        self.edit(None, |s| {
            if let Some(n) = s.nodes.iter_mut().find(|n| n.id == id) {
                n.visible = visible;
            }
        });
    }

    /// Edits one node (merged while `key` stays the same).
    pub fn edit_node(&mut self, id: Node3dId, key: &str, f: impl FnOnce(&mut Node3d)) {
        self.edit(Some(&format!("{key}:{id:?}")), |s| {
            if let Some(n) = s.nodes.iter_mut().find(|n| n.id == id) {
                f(n);
            }
        });
    }

    pub fn set_transform(&mut self, id: Node3dId, key: &str, t: Transform) {
        self.edit_node(id, key, |n| n.transform = t);
    }

    // ---- materials

    pub fn add_material(&mut self) -> String {
        let names: HashSet<String> = self.scene.materials.iter().map(|m| m.name.clone()).collect();
        let name = (1..)
            .map(|i| format!("Material {i}"))
            .find(|n| !names.contains(n))
            .unwrap_or_default();
        let mut m = MaterialDef::new(name.clone());
        m.metallic = 0.0;
        m.roughness = 0.5;
        self.edit(None, |s| s.materials.push(m));
        name
    }

    pub fn remove_material(&mut self, name: &str) {
        self.edit(None, |s| {
            s.materials.retain(|m| m.name != name);
            for n in &mut s.nodes {
                match &mut n.kind {
                    Node3dKind::Mesh { materials, .. } => materials.retain(|r| r.0 != name),
                    Node3dKind::Shape { material, .. }
                        if material.as_ref().is_some_and(|r| r.0 == name) =>
                    {
                        *material = None;
                    }
                    _ => {}
                }
            }
        });
    }

    /// Renames a material and every reference to it. Refused (false) when
    /// the new name is empty or taken.
    pub fn rename_material(&mut self, old: &str, new: &str) -> bool {
        if new.is_empty()
            || old == new
            || self.scene.materials.iter().any(|m| m.name == new)
            || !self.scene.materials.iter().any(|m| m.name == old)
        {
            return false;
        }
        self.edit(None, |s| {
            for m in &mut s.materials {
                if m.name == old {
                    m.name = new.to_owned();
                }
            }
            for n in &mut s.nodes {
                let refs: Vec<&mut MaterialRef> = match &mut n.kind {
                    Node3dKind::Mesh { materials, .. } => materials.iter_mut().collect(),
                    Node3dKind::Shape { material, .. } => material.iter_mut().collect(),
                    _ => Vec::new(),
                };
                for r in refs {
                    if r.0 == old {
                        r.0 = new.to_owned();
                    }
                }
            }
        });
        true
    }

    pub fn edit_material(&mut self, name: &str, f: impl FnOnce(&mut MaterialDef)) {
        self.edit(Some(&format!("mat:{name}")), |s| {
            if let Some(m) = s.materials.iter_mut().find(|m| m.name == name) {
                f(m);
            }
        });
    }

    /// Adds a procedural shape (`kind`: `orbit_scene::shapes::SHAPE_KINDS`)
    /// with its default parameters and the scene's default material
    /// (created on first use, white and rough, editable like any other).
    /// One undo step.
    pub fn add_shape(&mut self, kind: &str, parent: Option<Node3dId>) -> Node3dId {
        let shape = orbit_scene::Shape::default_of(kind)
            .unwrap_or(orbit_scene::Shape::Cube { size: [1.0; 3] });
        let label = match shape.kind() {
            "cube" => "Cube",
            "sphere" => "Sphere",
            "plane" => "Plane",
            "cylinder" => "Cylinder",
            "cone" => "Cone",
            "capsule" => "Capsule",
            _ => "Torus",
        };
        let mut node = Node3d::new(
            self.unique_name(label),
            parent,
            Node3dKind::Shape {
                shape,
                material: Some(MaterialRef(DEFAULT_MATERIAL.to_owned())),
            },
        );
        // A plane lies on the ground; other shapes stand on it.
        if !matches!(shape, orbit_scene::Shape::Plane { .. }) {
            node.transform.translation[1] = shape_half_height(&shape);
        }
        let id = node.id;
        self.edit(None, |s| {
            if !s.materials.iter().any(|m| m.name == DEFAULT_MATERIAL) {
                let mut m = MaterialDef::new(DEFAULT_MATERIAL);
                m.metallic = 0.0;
                m.roughness = 0.8;
                s.materials.push(m);
            }
            s.nodes.push(node);
        });
        id
    }

    /// Replaces the shape (kind and parameters) of a shape node.
    pub fn set_shape(&mut self, node: Node3dId, shape: orbit_scene::Shape) {
        self.edit(Some(&format!("shape:{node:?}")), |s| {
            if let Some(n) = s.nodes.iter_mut().find(|n| n.id == node)
                && let Node3dKind::Shape { shape: old, .. } = &mut n.kind
            {
                *old = shape;
            }
        });
    }

    pub fn assign_material(&mut self, node: Node3dId, slot: usize, material: &str) {
        self.edit(None, |s| {
            let Some(n) = s.nodes.iter_mut().find(|n| n.id == node) else {
                return;
            };
            match &mut n.kind {
                Node3dKind::Mesh { materials, .. } => {
                    while materials.len() <= slot {
                        materials.push(MaterialRef(material.to_owned()));
                    }
                    materials[slot] = MaterialRef(material.to_owned());
                }
                Node3dKind::Shape { material: m, .. } => {
                    *m = (!material.is_empty()).then(|| MaterialRef(material.to_owned()));
                }
                _ => {}
            }
        });
    }
}

/// UV sphere used by the material preview.
pub fn preview_sphere(name: &str) -> MeshAsset {
    let (rings, segs) = (24u32, 48u32);
    let mut p = orbit_scene::PrimitiveData::default();
    for r in 0..=rings {
        let v = r as f32 / rings as f32;
        let phi = v * std::f32::consts::PI;
        for s in 0..=segs {
            let u = s as f32 / segs as f32;
            let th = u * std::f32::consts::TAU;
            let n = [phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin()];
            p.positions.push(n);
            p.normals.push(n);
            p.uvs.push([u, v]);
        }
    }
    for r in 0..rings {
        for s in 0..segs {
            let a = r * (segs + 1) + s;
            let b = a + segs + 1;
            p.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    MeshAsset {
        name: name.to_owned(),
        primitives: vec![p],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orbit_scene3d_{name}_{}",
            Node3dId::new()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ktx2_textures_load_from_the_models_folder() {
        let dir = tmp("ktx2");
        let px = vec![200u8; 2 * 2 * 4];
        let file = orbit_scene::ktx2::encode_simple(
            orbit_scene::ktx2::Format::R8G8B8A8_SRGB,
            2,
            2,
            &px,
            Some(orbit_scene::ktx2::SupercompressionScheme::Zstandard),
        );
        std::fs::write(dir.join("wall.ktx2"), file).unwrap();
        let (lib, errors) = ModelLib::load_dir(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(lib.texture_names(), ["wall"]);
        let bc7 = orbit_scene::ktx2::encode_simple(orbit_scene::ktx2::Format::BC7_UNORM_BLOCK, 4, 4, &[0; 16], None);
        std::fs::write(dir.join("packed.ktx2"), bc7).unwrap();
        let (_, errors) = ModelLib::load_dir(&dir);
        assert_eq!(errors.len(), 1, "{errors:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A multi-file `.gltf` (external `.bin`) is copied into `models/` with
    /// its files under `<name>.files/` and loads again from there; a
    /// broken one is refused with a message and writes nothing.
    #[test]
    fn multi_file_gltf_is_copied_and_reloads() {
        let dir = tmp("multi_gltf");
        let src = dir.join("download");
        std::fs::create_dir_all(&src).unwrap();
        let mut bin = Vec::new();
        for c in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&c.to_le_bytes());
        }
        std::fs::write(src.join("scene.bin"), &bin).unwrap();
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"name":"n","mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
            "buffers":[{"uri":"scene.bin","byteLength":36}],
            "bufferViews":[{"buffer":0,"byteLength":36}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}]}"#;
        std::fs::write(src.join("scene.gltf"), json).unwrap();
        let models = dir.join("models");
        let imp = super::super::copy_model_into(&src.join("scene.gltf"), "girl", &models).unwrap();
        assert_eq!(imp.meshes.len(), 1);
        assert!(models.join("girl.files/scene.bin").is_file());
        let (lib, errors) = ModelLib::load_dir(&models);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(lib.prefabs.contains_key("girl"));

        std::fs::remove_file(src.join("scene.bin")).unwrap();
        let err = super::super::copy_model_into(&src.join("scene.gltf"), "other", &models)
            .unwrap_err();
        assert!(err.contains("scene.bin"), "{err}");
        assert!(!models.join("other.gltf").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shapes_are_added_with_the_default_material_and_edited() {
        let mut doc = SceneDoc::new("Test");
        let lib = ModelLib::default();
        let cube = doc.add_node(NewKind::Shape("cube"), None);
        let ball = doc.add_shape("sphere", None);
        assert!(orbit_scene::validate(doc.scene()).is_empty());
        // One shared default material, created once.
        assert_eq!(
            doc.scene().materials.iter().filter(|m| m.name == DEFAULT_MATERIAL).count(),
            1
        );
        let node = doc.node(cube).unwrap();
        assert_eq!(node.name, "Cube");
        assert!((node.transform.translation[1] - 0.5).abs() < 1e-5, "stands on the ground");
        assert!(lib.kind_bounds(&node.kind).is_some());

        doc.set_shape(ball, orbit_scene::Shape::Torus { radius: 1.0, tube: 0.2, segments: 8, sides: 6 });
        doc.rename_material(DEFAULT_MATERIAL, "Clay");
        let Node3dKind::Shape { shape, material } = &doc.node(ball).unwrap().kind else {
            panic!("not a shape");
        };
        assert_eq!(shape.kind(), "torus");
        assert_eq!(material.as_ref().unwrap().0, "Clay");
        doc.assign_material(ball, 0, "");
        doc.remove_material("Clay");
        assert!(orbit_scene::validate(doc.scene()).is_empty());
        let text = orbit_scene::to_ron(doc.scene()).unwrap();
        assert_eq!(orbit_scene::from_ron(&text).unwrap(), *doc.scene());
        doc.undo();
        assert!(doc.scene().materials.iter().any(|m| m.name == "Clay"));
    }

    #[test]
    fn new_scene_is_valid_and_dirty() {
        let doc = SceneDoc::new("Test");
        assert!(orbit_scene::validate(doc.scene()).is_empty());
        assert!(doc.dirty());
        assert_eq!(doc.scene().cameras().count(), 1);
    }

    #[test]
    fn add_delete_duplicate_with_children() {
        let mut doc = SceneDoc::new("T");
        let a = doc.add_node(NewKind::Empty, None);
        let b = doc.add_node(NewKind::Empty, Some(a));
        let c = doc.add_node(NewKind::Camera, Some(b));
        let n0 = doc.scene().nodes.len();
        let copy = doc.duplicate_node(a).unwrap();
        assert_eq!(doc.scene().nodes.len(), n0 + 3);
        assert_ne!(copy, a);
        assert_eq!(doc.children(Some(copy)).len(), 1);
        // Duplicated cameras get a unique name (validation stays clean).
        assert!(orbit_scene::validate(doc.scene()).is_empty());
        assert!(doc.node(c).is_some());
        assert!(doc.delete_node(a));
        assert!(doc.node(b).is_none() && doc.node(c).is_none());
        assert_eq!(doc.scene().nodes.len(), n0);
        assert!(!doc.delete_node(a));
    }

    #[test]
    fn reparent_keeps_world_and_refuses_cycles() {
        let mut doc = SceneDoc::new("T");
        let a = doc.add_node(NewKind::Empty, None);
        let b = doc.add_node(NewKind::Empty, None);
        doc.edit_node(a, "t", |n| {
            n.transform.translation = [10.0, 0.0, 0.0];
            n.transform.scale = [2.0; 3];
        });
        doc.edit_node(b, "t", |n| n.transform.translation = [12.0, 0.0, 0.0]);
        let before = doc.world_matrix(b).unwrap();
        assert!(doc.reparent(b, Some(a)));
        let after = doc.world_matrix(b).unwrap();
        for c in 0..4 {
            for r in 0..4 {
                assert!(close(before[c][r], after[c][r]), "{before:?} {after:?}");
            }
        }
        // Local translation is (12-10)/2 = 1.
        assert!(close(doc.node(b).unwrap().transform.translation[0], 1.0));
        assert!(!doc.reparent(a, Some(b)), "cycle");
        assert!(!doc.reparent(a, Some(a)), "self");
        assert!(doc.reparent(b, None));
    }

    #[test]
    fn undo_redo_and_gesture_merging() {
        let mut doc = SceneDoc::new("T");
        let a = doc.add_node(NewKind::Empty, None);
        for i in 1..=5 {
            doc.edit_node(a, "move", |n| n.transform.translation[0] = i as f32);
        }
        doc.end_gesture();
        assert!(close(doc.node(a).unwrap().transform.translation[0], 5.0));
        // The whole drag is one undo step.
        assert!(doc.undo());
        assert!(close(doc.node(a).unwrap().transform.translation[0], 0.0));
        assert!(doc.redo());
        assert!(close(doc.node(a).unwrap().transform.translation[0], 5.0));
        assert!(doc.undo() && doc.undo());
        assert!(doc.node(a).is_none());
        assert!(doc.redo());
        // A new edit clears the redo stack.
        doc.add_node(NewKind::Empty, None);
        assert!(!doc.can_redo());
        assert!(!SceneDoc::new("x").undo());
    }

    #[test]
    fn save_load_roundtrip_and_dirty_tracking() {
        let dir = tmp("save");
        let path = dir.join("a.scene3d.ron");
        let mut doc = SceneDoc::new("Roundtrip");
        doc.add_node(NewKind::Point, None);
        let m = doc.add_material();
        doc.edit_material(&m, |m| m.base_color = [1.0, 0.0, 0.0, 1.0]);
        assert!(doc.dirty());
        doc.save_as(&path).unwrap();
        assert!(!doc.dirty());
        doc.add_node(NewKind::Empty, None);
        assert!(doc.dirty());
        assert!(doc.undo());
        assert!(!doc.dirty(), "undo back to the saved state is clean");
        let loaded = SceneDoc::load(&path).unwrap();
        assert_eq!(loaded.scene(), doc.scene());
        assert!(!loaded.dirty());
        assert!(SceneDoc::load(&dir.join("missing.ron")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn picking_nearest_visible_node_with_hierarchy() {
        let mut doc = SceneDoc::new("T");
        doc.edit(None, |s| s.nodes.clear());
        let mut lib = ModelLib::default();
        lib.meshes.insert("box".into(), {
            let mut m = preview_sphere("box");
            // Replace by a unit cube footprint through its points.
            m.primitives[0].positions = vec![[-0.5; 3], [0.5; 3]];
            m
        });
        let near = doc.add_mesh("box", None, None);
        let far = doc.add_mesh("box", None, None);
        doc.edit_node(near, "t", |n| n.transform.translation = [0.0, 0.0, 0.0]);
        doc.edit_node(far, "t", |n| n.transform.translation = [0.0, 0.0, -5.0]);
        let ray = Ray {
            origin: [0.0, 0.0, 10.0],
            dir: [0.0, 0.0, -1.0],
        };
        assert_eq!(doc.pick(&ray, &lib, 0.2), Some(near));
        doc.set_visible(near, false);
        assert_eq!(doc.pick(&ray, &lib, 0.2), Some(far));
        // A hidden parent hides its children.
        doc.set_visible(near, true);
        let parent = doc.add_node(NewKind::Empty, None);
        doc.reparent(far, Some(parent));
        doc.set_visible(parent, false);
        doc.set_visible(near, false);
        assert_eq!(doc.pick(&ray, &lib, 0.2), None);
        // Markers are picked by their small box.
        let light = doc.add_node(NewKind::Point, None);
        doc.edit_node(light, "t", |n| n.transform.translation = [0.0, 0.0, 3.0]);
        assert_eq!(doc.pick(&ray, &lib, 0.2), Some(light));
        let off = Ray {
            origin: [1.0, 0.0, 10.0],
            dir: [0.0, 0.0, -1.0],
        };
        assert_eq!(doc.pick(&off, &lib, 0.2), None);
    }

    #[test]
    fn gizmo_transforms_respect_the_parent() {
        let parent = math::from_trs(
            [5.0, 0.0, 0.0],
            geom::quat_axis_angle([0.0, 1.0, 0.0], std::f32::consts::FRAC_PI_2),
            [1.0; 3],
        );
        let base = Transform::IDENTITY;
        // Put the child at world (5, 0, -2): parent +Y quarter turn maps
        // local +X to world -Z, so local (2, 0, 0).
        let moved = moved_local(&base, &parent, [5.0, 0.0, -2.0]);
        assert!(close(moved.translation[0], 2.0), "{moved:?}");
        // Rotate by 90 degrees about world Y: the child's world rotation is
        // parent * local = 180 degrees about Y.
        let rot = rotated_local(
            &base,
            &parent,
            geom::quat_axis_angle([0.0, 1.0, 0.0], std::f32::consts::FRAC_PI_2),
        );
        let world = geom::quat_mul(geom::mat_rotation(&parent), rot.rotation);
        let v = geom::quat_rotate(world, [1.0, 0.0, 0.0]);
        assert!(close(v[0], -1.0), "{v:?}");
    }

    #[test]
    fn decompose_inverts_from_trs() {
        let q = geom::quat_from_euler_deg([20.0, 30.0, 40.0]);
        let m = math::from_trs([1.0, 2.0, 3.0], q, [2.0, 3.0, 4.0]);
        let t = decompose(&m);
        assert!(close(t.scale[1], 3.0) && close(t.translation[2], 3.0));
        let d: f32 = (0..4).map(|k| q[k] * t.rotation[k]).sum::<f32>().abs();
        assert!(d > 0.9999);
    }

    #[test]
    fn materials_rename_remove_assign() {
        let mut doc = SceneDoc::new("T");
        let m = doc.add_material();
        let n = doc.add_mesh("m", Some(&m), None);
        assert!(doc.rename_material(&m, "Gold"));
        assert!(!doc.rename_material("Gold", "Gold"));
        assert!(!doc.rename_material("Gold", ""));
        let m2 = doc.add_material();
        assert!(!doc.rename_material(&m2, "Gold"), "name taken");
        let Node3dKind::Mesh { materials, .. } = &doc.node(n).unwrap().kind else {
            panic!()
        };
        assert_eq!(materials[0].0, "Gold");
        doc.assign_material(n, 2, &m2);
        let Node3dKind::Mesh { materials, .. } = &doc.node(n).unwrap().kind else {
            panic!()
        };
        assert_eq!(materials.len(), 3);
        doc.remove_material("Gold");
        let Node3dKind::Mesh { materials, .. } = &doc.node(n).unwrap().kind else {
            panic!()
        };
        assert!(materials.iter().all(|r| r.0 != "Gold"));
        assert!(doc.scene().materials.iter().all(|m| m.name != "Gold"));
    }

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn images_next_to_the_scene_load_as_named_textures() {
        let dir = tmp("textures");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sky.png"), png_bytes(8, 4)).unwrap();
        std::fs::write(dir.join("wall.PNG"), png_bytes(2, 2)).unwrap();
        std::fs::write(dir.join("broken.png"), b"nope").unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        let (lib, errors) = ModelLib::load_dir(&dir);
        assert_eq!(lib.texture_names(), ["sky", "wall"]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("broken.png"));
        let sky = lib.textures.iter().find(|t| t.name == "sky").unwrap();
        assert_eq!((sky.width, sky.height, sky.rgba8.len()), (8, 4, 8 * 4 * 4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adding_a_texture_twice_replaces_it() {
        let mut lib = ModelLib::default();
        lib.add_texture_file("a", &png_bytes(2, 2)).unwrap();
        lib.add_texture_file("a", &png_bytes(4, 4)).unwrap();
        assert_eq!(lib.textures.len(), 1);
        assert_eq!(lib.textures[0].width, 4);
        assert!(lib.add_texture_file("b", b"x").is_err());
    }

    #[test]
    fn preview_sphere_is_a_closed_indexed_mesh() {
        let s = preview_sphere("s");
        let p = &s.primitives[0];
        assert!(p.indices.iter().all(|i| (*i as usize) < p.positions.len()));
        assert!(p.indices.len().is_multiple_of(3) && !p.indices.is_empty());
        let b = Bounds::of_points(&p.positions).unwrap();
        assert!(close(b.max[0], 1.0) && close(b.min[1], -1.0));
    }

    #[test]
    fn world_in_survives_cycles_and_missing_parents() {
        let mut a = Node3d::new("a", None, Node3dKind::Empty);
        let mut b = Node3d::new("b", Some(a.id), Node3dKind::Empty);
        a.parent = Some(b.id);
        let (ia, ib) = (a.id, b.id);
        let nodes = vec![a, b.clone()];
        assert!(world_in(&nodes, ia).is_none());
        b.parent = Some(Node3dId::new());
        assert!(world_in(&[b], ib).is_none());
    }

    #[test]
    fn snap_to_ground_rests_the_subtree_on_y0() {
        let mut doc = SceneDoc::new("s");
        let lib = ModelLib::default();
        let cube = doc.add_shape("cube", None);
        doc.set_transform(
            cube,
            "t",
            Transform {
                translation: [1.0, 5.0, 0.0],
                ..Transform::IDENTITY
            },
        );
        assert!(doc.snap_to_ground(cube, &lib));
        let b = doc.subtree_bounds(cube, &lib, 0.0).unwrap();
        assert!(close(b.min[1], 0.0), "{b:?}");
        assert!(close(doc.node(cube).unwrap().transform.translation[0], 1.0));
        assert!(!doc.snap_to_ground(cube, &lib), "already on the ground");
        assert!(doc.undo());
    }

    #[test]
    fn align_to_puts_a_camera_on_the_view() {
        let mut doc = SceneDoc::new("s");
        let cam = doc.scene().cameras().next().unwrap().0.id;
        let mut edit = geom::EditCamera::default();
        edit.target = [1.0, 0.0, -2.0];
        let w = edit.world();
        assert!(doc.align_to(cam, &w));
        let got = doc.world_matrix(cam).unwrap();
        for c in 0..4 {
            for r in 0..3 {
                assert!(close(got[c][r], w[c][r]), "{got:?} vs {w:?}");
            }
        }
    }

    #[test]
    fn rigs_follow_duplicate_and_delete() {
        let mut doc = SceneDoc::new("s");
        let id = doc.add_prefab_instance_at("hero", None, [2.0, 0.0, 1.0]);
        assert_eq!(doc.node(id).unwrap().transform.translation, [2.0, 0.0, 1.0]);
        doc.edit(None, |s| s.rig_mut(id).pose.set("arm", Transform::IDENTITY));
        let copy = doc.duplicate_node(id).unwrap();
        assert_eq!(doc.scene().rig(copy).unwrap().pose, doc.scene().rig(id).unwrap().pose);
        doc.delete_node(id);
        assert!(doc.scene().rig(id).is_none());
        assert!(doc.scene().rig(copy).is_some());
        // Saved and reloaded with the poses.
        let text = orbit_scene::to_ron(doc.scene()).unwrap();
        assert_eq!(&orbit_scene::from_ron(&text).unwrap(), doc.scene());
    }
}
