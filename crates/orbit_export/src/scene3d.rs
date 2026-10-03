//! 3D scenes at export (M6-12 / M6-13): quality-tier checks and template
//! choice.
//!
//! A scene `<dir>/<name>.scene3d.ron` finds its models and textures in
//! `<dir>/models/` (`.glb` / `.gltf`, `.png` / `.jpg`, `.ktx2`), like the
//! editor's 3D panel. Each exported scene is measured (lights, shadows,
//! triangles, texture sizes, see `orbit_scene::quality`) and compared with
//! the tier of every export target: web and Android use the web tier, the
//! desktop players the tier of the profile. Going over is a **warning**,
//! never an error (ADR 0014 section 3).

use std::collections::BTreeSet;
use std::path::Path;

use orbit_core::{AssetKind, Story};
use orbit_project::{ExportProfile, ExportTarget};
use orbit_scene::quality::{AssetTable, QualityIssue, QualityTier, SceneStats, check, scene_stats};

/// Whether the shipped story holds a 3D scene (it then needs the 3D web
/// template and gets the quality metadata).
pub fn uses_3d(story: &Story) -> bool {
    story
        .assets
        .assets
        .iter()
        .any(|a| a.kind == AssetKind::Scene3d)
}

/// The quality tier of an export target.
pub fn tier_of(target: ExportTarget, profile: &ExportProfile) -> QualityTier {
    match target {
        ExportTarget::Web | ExportTarget::Android => QualityTier::Web,
        ExportTarget::Windows | ExportTarget::Linux => {
            QualityTier::from_id(profile.desktop_quality.tier_id()).unwrap_or(QualityTier::DesktopLow)
        }
    }
}

/// One measured scene.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneReport {
    /// Manifest path of the scene.
    pub path: String,
    pub stats: SceneStats,
    /// Why the scene or some of its models could not be read.
    pub errors: Vec<String>,
}

impl SceneReport {
    pub fn issues(&self, tier: QualityTier) -> Vec<QualityIssue> {
        check(&self.stats, tier)
    }
}

/// Reads the models and textures of a `models/` folder.
pub fn load_models(dir: &Path) -> (AssetTable, Vec<String>) {
    let mut table = AssetTable::default();
    let mut errors = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (table, errors);
    };
    let mut files: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
    files.sort();
    for path in files {
        let Some(ext) = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase)
        else {
            continue;
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model")
            .to_owned();
        let bytes = match ext.as_str() {
            "glb" | "gltf" | "png" | "jpg" | "jpeg" | "ktx2" => match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    errors.push(format!("{}: {e}", path.display()));
                    continue;
                }
            },
            _ => continue,
        };
        match ext.as_str() {
            "glb" | "gltf" => match orbit_scene::import::import_gltf_in(
                &name,
                &bytes,
                dir,
            ) {
                Ok(imported) => {
                    for mesh in &imported.meshes {
                        table
                            .triangles
                            .insert(mesh.name.clone(), orbit_scene::quality::mesh_triangles(mesh));
                    }
                    for t in &imported.textures {
                        table.textures.insert(t.name.clone(), (t.width, t.height));
                    }
                    table
                        .prefabs
                        .insert(imported.prefab.name.clone(), imported.prefab);
                }
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            },
            "ktx2" => match orbit_scene::ktx2::info(&bytes) {
                Ok(info) => {
                    table.textures.insert(name, (info.width, info.height));
                }
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            },
            _ => match image_size(&bytes) {
                Some(size) => {
                    table.textures.insert(name, size);
                }
                None => errors.push(format!("{}: not a PNG or JPEG image", path.display())),
            },
        }
    }
    (table, errors)
}

/// `(width, height)` of a PNG or baseline/progressive JPEG, from its header.
fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if let Some(size) = crate::compile::png_size(bytes) {
        return Some(size);
    }
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        // SOF0..SOF15 except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// Measures every 3D scene of the shipped story.
pub fn analyze(root: &Path, story: &Story) -> Vec<SceneReport> {
    let mut out = Vec::new();
    let mut libraries: Vec<(std::path::PathBuf, AssetTable, Vec<String>)> = Vec::new();
    for entry in story.assets.assets.iter().filter(|a| a.kind == AssetKind::Scene3d) {
        let file = root.join(&entry.path);
        let mut errors = Vec::new();
        let scene = std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|text| orbit_scene::from_ron(&text));
        let scene = match scene {
            Ok(s) => s,
            Err(e) => {
                out.push(SceneReport {
                    path: entry.path.clone(),
                    stats: SceneStats::default(),
                    errors: vec![e],
                });
                continue;
            }
        };
        let models = file
            .parent()
            .map(|d| d.join("models"))
            .unwrap_or_default();
        let index = match libraries.iter().position(|(d, _, _)| *d == models) {
            Some(i) => i,
            None => {
                let (table, errs) = load_models(&models);
                libraries.push((models, table, errs));
                libraries.len() - 1
            }
        };
        errors.extend(libraries[index].2.iter().cloned());
        out.push(SceneReport {
            path: entry.path.clone(),
            stats: scene_stats(&scene, &libraries[index].1),
            errors,
        });
    }
    out
}

/// The export warnings of `reports` for the tiers of `targets` (one line per
/// scene, tier and issue).
pub fn warnings(
    reports: &[SceneReport],
    targets: &[ExportTarget],
    profile: &ExportProfile,
) -> Vec<String> {
    let tiers: BTreeSet<QualityTier> = targets.iter().map(|t| tier_of(*t, profile)).collect();
    let mut out = Vec::new();
    for report in reports {
        for error in &report.errors {
            out.push(format!("3D scene `{}`: {error}", report.path));
        }
        for &tier in &tiers {
            for issue in report.issues(tier) {
                out.push(format!(
                    "3D scene `{}` exceeds the {} quality profile: {issue}",
                    report.path,
                    tier.id()
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use orbit_core::{AssetEntry, AssetId};
    use orbit_project::DesktopQuality;
    use orbit_scene::{Light, Node3d, Node3dKind, Scene3d};

    fn lamp() -> Node3d {
        Node3d::new(
            "lamp",
            None,
            Node3dKind::Light(Light::Point {
                color: [1.0; 3],
                intensity: 1.0,
                range: None,
            }),
        )
    }

    #[test]
    fn a_scene_over_the_web_profile_warns_for_web_only() {
        let dir = TempDir::new("s3d_check");
        let mut scene = Scene3d::new("room");
        for _ in 0..6 {
            scene.nodes.push(lamp());
        }
        std::fs::create_dir_all(dir.path().join("assets/3d/models")).unwrap();
        std::fs::write(
            dir.path().join("assets/3d/room.scene3d.ron"),
            orbit_scene::to_ron(&scene).unwrap(),
        )
        .unwrap();
        // A texture over the web limit, read from its header only.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&4096u32.to_be_bytes());
        png.extend_from_slice(&4096u32.to_be_bytes());
        std::fs::write(dir.path().join("assets/3d/models/huge.png"), &png).unwrap();

        let mut story = Story::new("t");
        assert!(!uses_3d(&story));
        story.assets.assets.push(AssetEntry {
            id: AssetId::new(),
            kind: AssetKind::Scene3d,
            path: "assets/3d/room.scene3d.ron".into(),
        });
        assert!(uses_3d(&story));
        let reports = analyze(dir.path(), &story);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].stats.lights, 6);
        assert!(reports[0].errors.is_empty(), "{:?}", reports[0].errors);

        let mut profile = ExportProfile::new("release", "Game");
        let web = warnings(&reports, &[ExportTarget::Web], &profile);
        assert_eq!(web.len(), 1, "{web:?}");
        assert!(web[0].contains("web quality profile") && web[0].contains("6 lights"));
        assert!(warnings(&reports, &[ExportTarget::Windows], &profile).is_empty());
        profile.desktop_quality = DesktopQuality::High;
        assert_eq!(tier_of(ExportTarget::Linux, &profile), QualityTier::DesktopHigh);
        assert_eq!(tier_of(ExportTarget::Android, &profile), QualityTier::Web);
        // Unused texture: not counted, so no texture warning.
        let (table, errors) = load_models(&dir.path().join("assets/3d/models"));
        assert_eq!(table.textures.get("huge"), Some(&(4096, 4096)));
        assert!(errors.is_empty());
    }

    #[test]
    fn an_unreadable_scene_is_reported_not_fatal() {
        let dir = TempDir::new("s3d_bad");
        std::fs::write(dir.path().join("bad.scene3d.ron"), "not ron").unwrap();
        let mut story = Story::new("t");
        story.assets.assets.push(AssetEntry {
            id: AssetId::new(),
            kind: AssetKind::Scene3d,
            path: "bad.scene3d.ron".into(),
        });
        let reports = analyze(dir.path(), &story);
        let w = warnings(&reports, &[ExportTarget::Web], &ExportProfile::new("r", "G"));
        assert_eq!(w.len(), 1);
        assert!(w[0].starts_with("3D scene `bad.scene3d.ron`"));
    }

    #[test]
    fn jpeg_sizes_are_read_from_the_frame_header() {
        // SOI, APP0 (len 4), SOF0 (len 11): 8 bit, h 300, w 500.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 1, 44, 1, 244, 3, 0, 0, 0,
            0, 0,
        ];
        assert_eq!(image_size(&jpeg), Some((500, 300)));
        assert_eq!(image_size(b"GIF89a"), None);
    }
}
