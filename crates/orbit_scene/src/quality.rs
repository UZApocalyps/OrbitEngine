//! Quality tiers and export validation of 3D scenes (M6-12, ADR 0014
//! section 3). GPU free: the limits are data, shared by the renderer
//! (`orbit_render3d::QualityProfile::for_tier`), the exporter (warnings) and
//! the editor (export panel).
//!
//! A scene over a tier's limits never breaks: extra lights are dropped by the
//! renderer and the exporter only **warns**.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{Light, MaterialDef, Node3d, Node3dKind, Prefab, Scene3d, TextureSlot};

/// A rendering quality tier, chosen per export target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum QualityTier {
    /// WebGL2 (and Android, ADR 0017: the mobile profile is the web one).
    Web,
    DesktopLow,
    DesktopHigh,
}

/// The limits of a tier (ADR 0014 section 3 table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualityLimits {
    /// Dynamic lights rendered; the others are dropped.
    pub max_lights: usize,
    /// Lights casting shadows.
    pub max_shadow_lights: usize,
    pub shadow_map_size: u32,
    /// Largest texture side (WebGL2 guarantees 2048).
    pub max_texture_size: u32,
    /// Triangle budget of one scene (all visible instances).
    pub max_triangles: u64,
    pub bloom: bool,
}

impl QualityTier {
    pub const ALL: [QualityTier; 3] = [Self::Web, Self::DesktopLow, Self::DesktopHigh];

    /// Stable identifier (pak metadata `quality`, CLI, MCP).
    pub fn id(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::DesktopLow => "desktop-low",
            Self::DesktopHigh => "desktop-high",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id.trim())
    }

    pub fn limits(self) -> QualityLimits {
        match self {
            Self::Web => QualityLimits {
                max_lights: 4,
                max_shadow_lights: 1,
                shadow_map_size: 1024,
                max_texture_size: 2048,
                max_triangles: 300_000,
                bloom: false,
            },
            Self::DesktopLow => QualityLimits {
                max_lights: 8,
                max_shadow_lights: 1,
                shadow_map_size: 2048,
                max_texture_size: 4096,
                max_triangles: 1_000_000,
                bloom: true,
            },
            Self::DesktopHigh => QualityLimits {
                max_lights: 16,
                max_shadow_lights: 4,
                shadow_map_size: 2048,
                max_texture_size: 8192,
                max_triangles: 3_000_000,
                bloom: true,
            },
        }
    }
}

/// What the checker needs to know about the assets a scene references.
/// `None` means unknown (not imported): it is reported, not guessed.
pub trait SceneAssets {
    fn prefab(&self, name: &str) -> Option<&Prefab>;
    /// Triangles of one instance of the mesh.
    fn mesh_triangles(&self, name: &str) -> Option<u64>;
    /// `(width, height)` of a texture.
    fn texture_size(&self, name: &str) -> Option<(u32, u32)>;
}

/// No asset known (scene file alone).
pub struct NoAssets;

impl SceneAssets for NoAssets {
    fn prefab(&self, _: &str) -> Option<&Prefab> {
        None
    }
    fn mesh_triangles(&self, _: &str) -> Option<u64> {
        None
    }
    fn texture_size(&self, _: &str) -> Option<(u32, u32)> {
        None
    }
}

/// Assets held in maps (the exporter and tests).
#[derive(Debug, Default, Clone)]
pub struct AssetTable {
    pub prefabs: HashMap<String, Prefab>,
    pub triangles: HashMap<String, u64>,
    pub textures: HashMap<String, (u32, u32)>,
}

impl SceneAssets for AssetTable {
    fn prefab(&self, name: &str) -> Option<&Prefab> {
        self.prefabs.get(name)
    }
    fn mesh_triangles(&self, name: &str) -> Option<u64> {
        self.triangles.get(name).copied()
    }
    fn texture_size(&self, name: &str) -> Option<(u32, u32)> {
        self.textures.get(name).copied()
    }
}

/// Triangles of an imported mesh.
pub fn mesh_triangles(mesh: &crate::MeshAsset) -> u64 {
    mesh.primitives
        .iter()
        .map(|p| {
            if p.indices.is_empty() {
                p.positions.len() as u64 / 3
            } else {
                p.indices.len() as u64 / 3
            }
        })
        .sum()
}

/// What a scene costs, counted over its visible nodes (prefab instances
/// expanded).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneStats {
    pub lights: usize,
    pub shadow_lights: usize,
    pub triangles: u64,
    /// Used textures with a known size, by name.
    pub textures: BTreeMap<String, (u32, u32)>,
    /// Referenced meshes / prefabs / textures that are not known.
    pub unknown: Vec<String>,
}

const MAX_PREFAB_DEPTH: usize = 8;

/// Counts the cost of `scene`.
pub fn scene_stats(scene: &Scene3d, assets: &dyn SceneAssets) -> SceneStats {
    let mut stats = SceneStats::default();
    let mut unknown = HashSet::new();
    let mut textures = HashSet::new();
    if let crate::Background::Sky(t) = &scene.environment.background {
        textures.insert(t.0.clone());
    }
    walk(
        &scene.nodes,
        &[&scene.materials],
        assets,
        0,
        &mut stats,
        &mut unknown,
        &mut textures,
    );
    for name in textures {
        match assets.texture_size(&name) {
            Some(size) => {
                stats.textures.insert(name, size);
            }
            None => {
                unknown.insert(format!("texture `{name}`"));
            }
        }
    }
    stats.unknown = unknown.into_iter().collect();
    stats.unknown.sort();
    stats
}

/// Whether `nodes[i]` and all its ancestors are visible.
fn visible(nodes: &[Node3d], i: usize) -> bool {
    let mut current = &nodes[i];
    for _ in 0..=nodes.len() {
        if !current.visible {
            return false;
        }
        match current.parent.and_then(|p| nodes.iter().find(|n| n.id == p)) {
            Some(parent) => current = parent,
            None => return true,
        }
    }
    true // parent cycle: treated as visible, the validator reports it
}

fn material_textures(m: &MaterialDef, out: &mut HashSet<String>) {
    let slots: [&Option<TextureSlot>; 4] = [
        &m.base_color_texture,
        &m.orm_texture,
        &m.normal_texture,
        &m.emissive_texture,
    ];
    for slot in slots.into_iter().flatten() {
        out.insert(slot.texture.0.clone());
    }
}

fn walk(
    nodes: &[Node3d],
    materials: &[&[MaterialDef]],
    assets: &dyn SceneAssets,
    depth: usize,
    stats: &mut SceneStats,
    unknown: &mut HashSet<String>,
    textures: &mut HashSet<String>,
) {
    for (i, node) in nodes.iter().enumerate() {
        if !visible(nodes, i) {
            continue;
        }
        match &node.kind {
            Node3dKind::Empty | Node3dKind::Camera(_) => {}
            Node3dKind::Light(light) => {
                stats.lights += 1;
                if matches!(light, Light::Directional { shadows: true, .. }) {
                    stats.shadow_lights += 1;
                }
            }
            Node3dKind::Mesh { mesh, materials: refs } => {
                match assets.mesh_triangles(&mesh.0) {
                    Some(t) => stats.triangles += t,
                    None => {
                        unknown.insert(format!("mesh `{}`", mesh.0));
                    }
                }
                for r in refs {
                    // Last set wins (overrides come after the originals).
                    if let Some(m) = materials
                        .iter()
                        .rev()
                        .find_map(|set| set.iter().find(|m| m.name == r.0))
                    {
                        material_textures(m, textures);
                    }
                }
            }
            Node3dKind::Prefab(r) => {
                if depth >= MAX_PREFAB_DEPTH {
                    continue;
                }
                match assets.prefab(&r.0) {
                    Some(prefab) => {
                        let mut sets: Vec<&[MaterialDef]> = materials.to_vec();
                        sets.push(&prefab.materials);
                        sets.push(&prefab.material_overrides);
                        walk(
                            &prefab.nodes,
                            &sets,
                            assets,
                            depth + 1,
                            stats,
                            unknown,
                            textures,
                        );
                    }
                    None => {
                        unknown.insert(format!("prefab `{}`", r.0));
                    }
                }
            }
        }
    }
}

/// One way a scene goes over a tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualityIssue {
    TooManyLights { count: usize, max: usize },
    TooManyShadows { count: usize, max: usize },
    TooManyTriangles { count: u64, max: u64 },
    TextureTooLarge { name: String, width: u32, height: u32, max: u32 },
}

impl std::fmt::Display for QualityIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyLights { count, max } => write!(
                f,
                "{count} lights, {max} at most are rendered (the least important are dropped)"
            ),
            Self::TooManyShadows { count, max } => write!(
                f,
                "{count} lights cast shadows, only {max} will"
            ),
            Self::TooManyTriangles { count, max } => write!(
                f,
                "{count} triangles, over the budget of {max} (slow on modest hardware)"
            ),
            Self::TextureTooLarge {
                name,
                width,
                height,
                max,
            } => write!(
                f,
                "texture `{name}` is {width}x{height}, over {max} pixels a side (may not load)"
            ),
        }
    }
}

/// The ways `stats` exceeds `tier`, most serious first.
pub fn check(stats: &SceneStats, tier: QualityTier) -> Vec<QualityIssue> {
    let l = tier.limits();
    let mut out = Vec::new();
    for (name, &(width, height)) in &stats.textures {
        if width.max(height) > l.max_texture_size {
            out.push(QualityIssue::TextureTooLarge {
                name: name.clone(),
                width,
                height,
                max: l.max_texture_size,
            });
        }
    }
    if stats.lights > l.max_lights {
        out.push(QualityIssue::TooManyLights {
            count: stats.lights,
            max: l.max_lights,
        });
    }
    if stats.shadow_lights > l.max_shadow_lights {
        out.push(QualityIssue::TooManyShadows {
            count: stats.shadow_lights,
            max: l.max_shadow_lights,
        });
    }
    if stats.triangles > l.max_triangles {
        out.push(QualityIssue::TooManyTriangles {
            count: stats.triangles,
            max: l.max_triangles,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MaterialRef, MeshRef, PrefabRef, TextureRef};

    fn light(shadows: bool) -> Node3d {
        Node3d::new(
            "sun",
            None,
            Node3dKind::Light(Light::Directional {
                color: [1.0; 3],
                intensity: 1.0,
                shadows,
            }),
        )
    }

    #[test]
    fn ids_round_trip_and_limits_grow_with_the_tier() {
        for t in QualityTier::ALL {
            assert_eq!(QualityTier::from_id(t.id()), Some(t));
        }
        assert_eq!(QualityTier::from_id("ultra"), None);
        let (w, l, h) = (
            QualityTier::Web.limits(),
            QualityTier::DesktopLow.limits(),
            QualityTier::DesktopHigh.limits(),
        );
        assert_eq!((w.max_lights, w.max_shadow_lights, w.shadow_map_size), (4, 1, 1024));
        assert!(w.max_triangles < l.max_triangles && l.max_triangles < h.max_triangles);
        assert!(w.max_texture_size <= l.max_texture_size && !w.bloom && h.bloom);
    }

    #[test]
    fn a_scene_over_the_web_profile_gets_every_warning() {
        let mut scene = Scene3d::new("big");
        for _ in 0..5 {
            scene.nodes.push(light(true));
        }
        let mut mat = MaterialDef::new("m");
        mat.base_color_texture = Some(TextureSlot::new(TextureRef("huge".into())));
        scene.materials.push(mat);
        scene.nodes.push(Node3d::new(
            "rock",
            None,
            Node3dKind::Mesh {
                mesh: MeshRef("rock".into()),
                materials: vec![MaterialRef("m".into())],
            },
        ));
        let mut assets = AssetTable::default();
        assets.triangles.insert("rock".into(), 400_000);
        assets.textures.insert("huge".into(), (4096, 4096));
        let stats = scene_stats(&scene, &assets);
        assert_eq!((stats.lights, stats.shadow_lights, stats.triangles), (5, 5, 400_000));
        let issues = check(&stats, QualityTier::Web);
        assert_eq!(issues.len(), 4, "{issues:?}");
        assert!(matches!(issues[0], QualityIssue::TextureTooLarge { .. }));
        assert!(issues[0].to_string().contains("huge"));
        // Desktop high takes the same scene but for the shadows.
        let issues = check(&stats, QualityTier::DesktopHigh);
        assert_eq!(
            issues,
            [QualityIssue::TooManyShadows { count: 5, max: 4 }]
        );
    }

    #[test]
    fn prefabs_are_expanded_and_hidden_subtrees_skipped() {
        let mut prefab = Prefab {
            name: "chair".into(),
            ..Default::default()
        };
        prefab.nodes.push(Node3d::new(
            "seat",
            None,
            Node3dKind::Mesh {
                mesh: MeshRef("seat".into()),
                materials: vec![],
            },
        ));
        prefab.nodes.push(light(false));
        let mut scene = Scene3d::new("room");
        for _ in 0..3 {
            scene.nodes.push(Node3d::new(
                "chair",
                None,
                Node3dKind::Prefab(PrefabRef("chair".into())),
            ));
        }
        let mut hidden = Node3d::new("group", None, Node3dKind::Empty);
        hidden.visible = false;
        let child = Node3d::new("lamp", Some(hidden.id), light(true).kind);
        scene.nodes.push(hidden);
        scene.nodes.push(child);
        scene.nodes.push(Node3d::new(
            "ghost",
            None,
            Node3dKind::Prefab(PrefabRef("ghost".into())),
        ));
        let mut assets = AssetTable::default();
        assets.prefabs.insert("chair".into(), prefab);
        assets.triangles.insert("seat".into(), 100);
        let stats = scene_stats(&scene, &assets);
        assert_eq!(stats.triangles, 300);
        assert_eq!((stats.lights, stats.shadow_lights), (3, 0));
        assert_eq!(stats.unknown, ["prefab `ghost`"]);
        assert!(check(&stats, QualityTier::Web).is_empty());
        // Nothing known: everything referenced is unknown, nothing counted.
        let stats = scene_stats(&scene, &NoAssets);
        assert_eq!(stats.triangles, 0);
        assert_eq!(stats.unknown.len(), 2);
    }
}
