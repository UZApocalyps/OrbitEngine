//! glTF / GLB import into a [`Prefab`] (M6-02, ADR 0014 section 4).
//!
//! Node ids are mapped by name path (`root/arm/hand`) so that [`reimport`]
//! keeps the ULIDs, visibility and material overrides of a previous import.
//! Animations, skins, morph targets and glTF cameras/lights are ignored with
//! a warning. Tangent generation (mikktspace) is not done yet: tangents are
//! only carried when present in the file.

use std::collections::{HashMap, HashSet};

use gltf::image::Format;

use crate::{
    AlphaMode, MaterialDef, MaterialRef, MeshRef, Node3d, Node3dId, Node3dKind, Prefab,
    TextureRef, TextureSlot, Transform,
};

pub use crate::assets::{MeshAsset, PrimitiveData, TextureAsset};

#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub prefab: Prefab,
    pub meshes: Vec<MeshAsset>,
    pub textures: Vec<TextureAsset>,
    pub warnings: Vec<String>,
}

/// Imports a `.glb` / self-contained `.gltf` (embedded or data-URI buffers).
/// A file referencing external buffers or images is refused with a message:
/// use [`import_gltf_file`] / [`import_gltf_in`] for those.
pub fn import_gltf(name: &str, bytes: &[u8]) -> Result<Imported, String> {
    import_gltf_impl(name, bytes, None)
}

/// Imports a `.gltf` / `.glb` file, resolving its relative `uri`s (external
/// `.bin` buffers, images) from the file's folder.
pub fn import_gltf_file(path: &std::path::Path) -> Result<Imported, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
    let base = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    import_gltf_in(name, &bytes, base)
}

/// Imports `bytes`, resolving relative `uri`s inside `base` only (see
/// [`external_files`]).
pub fn import_gltf_in(name: &str, bytes: &[u8], base: &std::path::Path) -> Result<Imported, String> {
    import_gltf_impl(name, bytes, Some(base))
}

/// Percent-decodes a URI path (`%20` -> space); bad escapes stay as is.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Checks a relative `uri` and returns it percent-decoded, normalised
/// (`a/./b` -> `a/b`). Refused: absolute paths, schemes other than
/// `data:`, backslashes, and anything that leaves the folder (`..`).
pub fn safe_relative_uri(uri: &str) -> Result<String, String> {
    let decoded = percent_decode(uri);
    let refuse = || format!("external file `{uri}` is outside the model's folder");
    if decoded.starts_with('/') || decoded.contains('\\') || decoded.contains(':') {
        return Err(refuse());
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or_else(refuse)?;
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(refuse());
    }
    Ok(parts.join("/"))
}

/// The external files (buffers, images) a glTF references, as safe
/// relative paths (data URIs and GLB chunks are not listed).
pub fn external_files(bytes: &[u8]) -> Result<Vec<String>, String> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| format!("invalid glTF: {e}"))?;
    external_uris(&gltf.document)
        .into_iter()
        .map(|u| safe_relative_uri(&u))
        .collect()
}

fn external_uris(doc: &gltf::Document) -> Vec<String> {
    let mut out = Vec::new();
    for b in doc.buffers() {
        if let gltf::buffer::Source::Uri(u) = b.source()
            && !u.starts_with("data:")
        {
            out.push(u.to_owned());
        }
    }
    for i in doc.images() {
        if let gltf::image::Source::Uri { uri, .. } = i.source()
            && !uri.starts_with("data:")
        {
            out.push(uri.to_owned());
        }
    }
    out.dedup();
    out
}

/// Rewrites the relative `uri`s of a `.gltf` (JSON) under `prefix`
/// (`scene.bin` -> `<prefix>/scene.bin`), used to copy a multi-file model
/// into `models/` without name clashes. Returns the new JSON and the
/// `(original, rewritten)` relative paths. A `.glb` is returned unchanged
/// (its external references, if any, are rewritten too).
pub fn relocate_gltf(bytes: &[u8], prefix: &str) -> Result<(Vec<u8>, Vec<(String, String)>), String> {
    if bytes.starts_with(b"glTF") {
        let files = external_files(bytes)?;
        if files.is_empty() {
            return Ok((bytes.to_vec(), Vec::new()));
        }
        return Err("a .glb that references external files is not supported".into());
    }
    let mut json: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid glTF: {e}"))?;
    let mut moved = Vec::new();
    for list in ["buffers", "images"] {
        let Some(items) = json.get_mut(list).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        for item in items {
            let Some(uri) = item.get("uri").and_then(|u| u.as_str()) else {
                continue;
            };
            if uri.starts_with("data:") {
                continue;
            }
            let rel = safe_relative_uri(uri)?;
            let new = format!("{prefix}/{rel}");
            // Written back unencoded but with spaces escaped (valid URI).
            item["uri"] = serde_json::Value::String(new.replace('%', "%25").replace(' ', "%20"));
            if !moved.iter().any(|(o, _): &(String, String)| *o == rel) {
                moved.push((rel, new));
            }
        }
    }
    let out = serde_json::to_vec_pretty(&json).map_err(|e| e.to_string())?;
    Ok((out, moved))
}

fn import_gltf_impl(
    name: &str,
    bytes: &[u8],
    base: Option<&std::path::Path>,
) -> Result<Imported, String> {
    let gltf::Gltf { document, blob } =
        gltf::Gltf::from_slice(bytes).map_err(|e| format!("invalid glTF: {e}"))?;
    let uris = external_uris(&document);
    if base.is_none()
        && let Some(u) = uris.first()
    {
        return Err(format!(
            "the model references the external file `{u}`: import the .gltf from its folder"
        ));
    }
    for u in &uris {
        let rel = safe_relative_uri(u)?;
        if let Some(base) = base
            && !base.join(&rel).is_file()
        {
            return Err(format!("missing file `{rel}` next to the model"));
        }
    }
    let buffers = gltf::import_buffers(&document, base, blob)
        .map_err(|e| format!("cannot read the model's buffers: {e}"))?;
    let images = gltf::import_images(&document, base, &buffers)
        .map_err(|e| format!("cannot read the model's images: {e}"))?;
    import_document(name, &document, &buffers, &images)
}

fn import_document(
    name: &str,
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
) -> Result<Imported, String> {
    let mut warnings = Vec::new();
    if doc.animations().len() > 0 {
        warnings.push(format!(
            "{} animation(s) ignored",
            doc.animations().len()
        ));
    }
    if doc.skins().len() > 0 {
        warnings.push(format!("{} skin(s) ignored", doc.skins().len()));
    }

    // Textures.
    let mut textures = Vec::new();
    let mut tex_names: HashMap<usize, TextureRef> = HashMap::new();
    let mut used_names = HashSet::new();
    for tex in doc.textures() {
        let img = &images[tex.source().index()];
        let base = tex
            .name()
            .or_else(|| tex.source().name())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("texture{}", tex.index()));
        let tname = unique(&mut used_names, base);
        textures.push(TextureAsset {
            name: tname.clone(),
            width: img.width,
            height: img.height,
            rgba8: to_rgba8(img, &mut warnings, &tname),
        });
        tex_names.insert(tex.index(), TextureRef(tname));
    }

    // Materials.
    let mut materials = Vec::new();
    let mut mat_names: HashMap<Option<usize>, MaterialRef> = HashMap::new();
    let mut used = HashSet::new();
    for m in doc.materials() {
        let base = m
            .name()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("material{}", m.index().unwrap_or(0)));
        let mname = unique(&mut used, base);
        let mut def = MaterialDef::new(mname.clone());
        let pbr = m.pbr_metallic_roughness();
        def.unlit = m.unlit();
        def.base_color = pbr.base_color_factor();
        def.metallic = pbr.metallic_factor();
        def.roughness = pbr.roughness_factor();
        def.emissive = m.emissive_factor();
        def.emissive_strength = m.emissive_strength().unwrap_or(1.0);
        def.alpha_cutoff = m.alpha_cutoff().unwrap_or(0.5);
        def.double_sided = m.double_sided();
        def.alpha_mode = match m.alpha_mode() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask,
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
        };
        def.base_color_texture = pbr
            .base_color_texture()
            .map(|i| slot(&tex_names, i.texture().index(), i.tex_coord(), i.texture_transform()));
        def.orm_texture = pbr.metallic_roughness_texture().map(|i| {
            slot(&tex_names, i.texture().index(), i.tex_coord(), i.texture_transform())
        });
        if let Some(sg) = m.pbr_specular_glossiness() {
            // KHR_materials_pbrSpecularGlossiness (archived, common in
            // Sketchfab exports): standard approximation to
            // metallic-roughness. Diffuse -> base colour, glossiness ->
            // roughness = 1 - g, dielectric (metallic 0). The
            // specular-glossiness texture has no metallic-roughness
            // equivalent and is dropped.
            def.base_color = sg.diffuse_factor();
            def.base_color_texture = sg.diffuse_texture().map(|i| {
                slot(&tex_names, i.texture().index(), i.tex_coord(), i.texture_transform())
            });
            def.metallic = 0.0;
            def.roughness = (1.0 - sg.glossiness_factor()).clamp(0.0, 1.0);
            if sg.specular_glossiness_texture().is_some() {
                // The factor only scales the (dropped) texture: without it a
                // default factor of 1 would give a mirror. Mid roughness.
                def.roughness = def.roughness.max(0.5);
            }
            def.orm_texture = None;
            warnings.push(format!(
                "material `{mname}`: specular-glossiness converted to metallic-roughness (approximation{})",
                if sg.specular_glossiness_texture().is_some() {
                    ", specular-glossiness texture ignored"
                } else {
                    ""
                }
            ));
        }
        if let Some(n) = m.normal_texture() {
            def.normal_scale = n.scale();
            def.normal_texture = Some(slot(&tex_names, n.texture().index(), n.tex_coord(), None));
        }
        def.emissive_texture = m
            .emissive_texture()
            .map(|i| slot(&tex_names, i.texture().index(), i.tex_coord(), i.texture_transform()));
        if let Some(o) = m.occlusion_texture() {
            def.occlusion_strength = o.strength();
            let same = def
                .orm_texture
                .as_ref()
                .is_some_and(|s| tex_names.get(&o.texture().index()) == Some(&s.texture));
            if !same {
                warnings.push(format!(
                    "material `{mname}`: separate occlusion texture ignored (only combined ORM is supported)"
                ));
            }
        }
        mat_names.insert(m.index(), MaterialRef(mname));
        materials.push(def);
    }
    // glTF default material, created lazily.
    let default_name = {
        let n = unique(&mut used, "default".to_owned());
        MaterialRef(n)
    };
    let mut default_used = false;

    // Meshes.
    let mut meshes = Vec::new();
    let mut mesh_info: HashMap<usize, Option<(MeshRef, Vec<MaterialRef>)>> = HashMap::new();
    let mut used = HashSet::new();
    for mesh in doc.meshes() {
        let base = mesh
            .name()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("mesh{}", mesh.index()));
        let mname = unique(&mut used, base);
        let mut prims = Vec::new();
        let mut mats = Vec::new();
        for p in mesh.primitives() {
            if p.mode() != gltf::mesh::Mode::Triangles {
                warnings.push(format!(
                    "mesh `{mname}`: non-triangle primitive ({:?}) skipped",
                    p.mode()
                ));
                continue;
            }
            if p.morph_targets().len() > 0 {
                warnings.push(format!("mesh `{mname}`: morph targets ignored"));
            }
            let r = p.reader(|b| Some(&buffers[b.index()]));
            let Some(pos) = r.read_positions() else {
                warnings.push(format!("mesh `{mname}`: primitive without positions skipped"));
                continue;
            };
            let positions: Vec<[f32; 3]> = pos.collect();
            let indices: Vec<u32> = match r.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            prims.push(PrimitiveData {
                normals: r.read_normals().map(|n| n.collect()).unwrap_or_default(),
                uvs: r
                    .read_tex_coords(0)
                    .map(|t| t.into_f32().collect())
                    .unwrap_or_default(),
                tangents: r.read_tangents().map(|t| t.collect()).unwrap_or_default(),
                positions,
                indices,
            });
            mats.push(match p.material().index() {
                Some(i) => mat_names[&Some(i)].clone(),
                None => {
                    default_used = true;
                    default_name.clone()
                }
            });
        }
        if prims.is_empty() {
            mesh_info.insert(mesh.index(), None);
        } else {
            mesh_info.insert(mesh.index(), Some((MeshRef(mname.clone()), mats)));
            meshes.push(MeshAsset {
                name: mname,
                primitives: prims,
            });
        }
    }
    if default_used {
        materials.push(MaterialDef::new(default_name.0.clone()));
    }

    // Node hierarchy of the default scene.
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or("glTF contains no scene")?;
    let mut nodes: Vec<Node3d> = Vec::new();
    let mut sibling_names: HashMap<Option<Node3dId>, HashSet<String>> = HashMap::new();
    let mut stack: Vec<(gltf::Node, Option<Node3dId>)> =
        scene.nodes().map(|n| (n, None)).collect::<Vec<_>>().into_iter().rev().collect();
    while let Some((n, parent)) = stack.pop() {
        let base = n
            .name()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("node{}", n.index()));
        let name = unique(sibling_names.entry(parent).or_default(), base);
        let kind = match n.mesh() {
            Some(m) => match &mesh_info[&m.index()] {
                Some((mesh, mats)) => Node3dKind::Mesh {
                    mesh: mesh.clone(),
                    materials: mats.clone(),
                },
                None => Node3dKind::Empty,
            },
            None => Node3dKind::Empty,
        };
        if n.camera().is_some() {
            warnings.push(format!("node `{name}`: glTF camera ignored"));
        }
        if n.skin().is_some() {
            warnings.push(format!("node `{name}`: skin ignored, mesh rendered unskinned"));
        }
        let (t, r, s) = n.transform().decomposed();
        let len = r.iter().map(|x| x * x).sum::<f32>().sqrt();
        let rotation = if len > 0.0 {
            [r[0] / len, r[1] / len, r[2] / len, r[3] / len]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        };
        let mut node = Node3d::new(name, parent, kind);
        node.transform = Transform {
            translation: t,
            rotation,
            scale: s,
        };
        let id = node.id;
        nodes.push(node);
        for c in n.children().collect::<Vec<_>>().into_iter().rev() {
            stack.push((c, Some(id)));
        }
    }

    Ok(Imported {
        prefab: Prefab {
            name: name.to_owned(),
            nodes,
            materials,
            material_overrides: Vec::new(),
        },
        meshes,
        textures,
        warnings,
    })
}

/// Name path of each node (`root/arm/hand`), in node order.
pub fn node_paths(nodes: &[Node3d]) -> Vec<String> {
    let by_id: HashMap<Node3dId, &Node3d> = nodes.iter().map(|n| (n.id, n)).collect();
    nodes
        .iter()
        .map(|n| {
            let mut parts = vec![n.name.as_str()];
            let mut cur = n.parent;
            let mut guard = 0;
            while let Some(p) = cur.and_then(|p| by_id.get(&p)) {
                parts.push(p.name.as_str());
                cur = p.parent;
                guard += 1;
                if guard > nodes.len() {
                    break;
                }
            }
            parts.reverse();
            parts.join("/")
        })
        .collect()
}

/// Carries ids, visibility and material overrides from the `old` prefab to a
/// freshly imported one, matching nodes by name path. Returns the number of
/// nodes whose identity was preserved.
pub fn reimport(old: &Prefab, new: &mut Prefab) -> usize {
    let old_paths = node_paths(&old.nodes);
    let old_by_path: HashMap<&str, &Node3d> = old_paths
        .iter()
        .map(String::as_str)
        .zip(old.nodes.iter())
        .collect();
    let new_paths = node_paths(&new.nodes);
    // Fresh id -> preserved id, applied to parents afterwards.
    let mut remap: HashMap<Node3dId, Node3dId> = HashMap::new();
    let mut kept = 0;
    for (n, path) in new.nodes.iter_mut().zip(&new_paths) {
        if let Some(o) = old_by_path.get(path.as_str()) {
            remap.insert(n.id, o.id);
            n.id = o.id;
            n.visible = o.visible;
            kept += 1;
        }
    }
    for n in &mut new.nodes {
        if let Some(p) = n.parent.and_then(|p| remap.get(&p)) {
            n.parent = Some(*p);
        }
    }
    let names: HashSet<&str> = new.materials.iter().map(|m| m.name.as_str()).collect();
    new.material_overrides = old
        .material_overrides
        .iter()
        .filter(|m| names.contains(m.name.as_str()))
        .cloned()
        .collect();
    kept
}

fn unique(used: &mut HashSet<String>, base: String) -> String {
    let mut name = base.clone();
    let mut n = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}#{n}");
        n += 1;
    }
    name
}

fn slot(
    names: &HashMap<usize, TextureRef>,
    index: usize,
    uv_set: u32,
    tt: Option<gltf::texture::TextureTransform>,
) -> TextureSlot {
    let mut s = TextureSlot::new(names[&index].clone());
    s.uv_set = uv_set;
    if let Some(t) = tt {
        s.offset = t.offset();
        s.rotation = t.rotation();
        s.scale = t.scale();
        if let Some(c) = t.tex_coord() {
            s.uv_set = c;
        }
    }
    s
}

fn to_rgba8(img: &gltf::image::Data, warnings: &mut Vec<String>, name: &str) -> Vec<u8> {
    let (ch, wide) = match img.format {
        Format::R8 => (1, false),
        Format::R8G8 => (2, false),
        Format::R8G8B8 => (3, false),
        Format::R8G8B8A8 => (4, false),
        Format::R16 => (1, true),
        Format::R16G16 => (2, true),
        Format::R16G16B16 => (3, true),
        Format::R16G16B16A16 => (4, true),
        _ => {
            warnings.push(format!("texture `{name}`: unsupported pixel format, left white"));
            return vec![255; (img.width * img.height * 4) as usize];
        }
    };
    let step = if wide { 2 } else { 1 };
    let mut out = Vec::with_capacity((img.width * img.height * 4) as usize);
    for px in img.pixels.chunks_exact(ch * step) {
        // 16-bit samples are little endian: keep the high byte.
        let c = |i: usize| px[i * step + (step - 1)];
        let rgba = match ch {
            1 => [c(0), c(0), c(0), 255],
            2 => [c(0), c(0), c(0), c(1)],
            3 => [c(0), c(1), c(2), 255],
            _ => [c(0), c(1), c(2), c(3)],
        };
        out.extend_from_slice(&rgba);
    }
    out
}
