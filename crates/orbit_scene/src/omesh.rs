//! `.omesh`: the binary mesh format of the pak (ADR 0014, addendum
//! "Blobs 3D dans le pak").
//!
//! ```text
//! b"OMSH"            magic
//! u16 version        1
//! u16 flags          0 (reserved)
//! u32 primitive_count
//! per primitive:
//!   u32 vertex_count, u32 index_count,
//!   u8 index_format (0 = u16, 1 = u32), u8 attributes (bit 0 normals,
//!   bit 1 uv0, bit 2 tangents), u16 padding
//!   positions [f32; 3] * vertex_count
//!   normals   [f32; 3] * vertex_count   (if bit 0)
//!   uvs       [f32; 2] * vertex_count   (if bit 1)
//!   tangents  [f32; 4] * vertex_count   (if bit 2)
//!   indices   u16 / u32 * index_count, padded to 4 bytes
//! ```
//!
//! Version 2 (skinned meshes only; static meshes are still written as v1)
//! adds attribute bit 3 (`joints [u16; 4]` then `weights [f32; 4]` per
//! vertex, after the tangents) and, when header flag bit 0 is set, a
//! skeleton section after the primitives, then (flag bit 1) the clips:
//!
//! ```text
//! u32 joint_count
//! per joint: str name, i32 parent (-1: root), f32 * 10 rest (t, r, s),
//!            f32 * 16 base, f32 * 16 inverse bind
//! u32 clip_count
//! per clip: str name, f32 duration, u32 channel_count
//!   per channel: u32 joint, u8 path (0 t, 1 r, 2 s), u8 interpolation
//!                (0 step, 1 linear), u16 padding, u32 key_count,
//!                f32 * key_count times, [f32; 4] * key_count values
//! str = u32 byte length, UTF-8 bytes, padded to 4 bytes
//! ```
//!
//! Little endian, every array 4-byte aligned. Each version keeps its own
//! decoder ([`decode`] dispatches on the version and never drops one, plan
//! §1.3); an unknown version is refused with an error, never a panic.

use crate::{MeshAsset, PrimitiveData};

pub const MAGIC: &[u8; 4] = b"OMSH";
/// Newest version written by [`encode`] (v1 for meshes without a skin).
pub const VERSION: u16 = 2;

const NORMALS: u8 = 1;
const UVS: u8 = 2;
const TANGENTS: u8 = 4;
const SKIN: u8 = 8;
const HAS_SKELETON: u16 = 1;
const HAS_CLIPS: u16 = 2;

fn is_skinned(mesh: &MeshAsset) -> bool {
    mesh.skin.is_some()
        || mesh
            .primitives
            .iter()
            .any(|p| !p.joints.is_empty() && p.joints.len() == p.positions.len())
}

/// Encodes `mesh` (its name is not stored: the pak index names it).
/// Attributes whose length differs from the vertex count are dropped (the
/// renderer regenerates normals, UVs default to zero).
pub fn encode(mesh: &MeshAsset) -> Vec<u8> {
    let skinned = is_skinned(mesh);
    let mut flags = 0u16;
    if mesh.skin.is_some() {
        flags |= HAS_SKELETON;
        if !mesh.clips.is_empty() {
            flags |= HAS_CLIPS;
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(if skinned { 2u16 } else { 1u16 }).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(mesh.primitives.len() as u32).to_le_bytes());
    for p in &mesh.primitives {
        let n = p.positions.len();
        let wide = p.indices.iter().any(|&i| i > u32::from(u16::MAX)) || n > usize::from(u16::MAX);
        let mut attributes = 0;
        if p.normals.len() == n {
            attributes |= NORMALS;
        }
        if p.uvs.len() == n {
            attributes |= UVS;
        }
        if p.tangents.len() == n {
            attributes |= TANGENTS;
        }
        if skinned && p.joints.len() == n && p.weights.len() == n {
            attributes |= SKIN;
        }
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&(p.indices.len() as u32).to_le_bytes());
        out.push(u8::from(wide));
        out.push(attributes);
        out.extend_from_slice(&0u16.to_le_bytes());
        let floats = |out: &mut Vec<u8>, values: &mut dyn Iterator<Item = f32>| {
            for v in values {
                out.extend_from_slice(&v.to_le_bytes());
            }
        };
        floats(&mut out, &mut p.positions.iter().flatten().copied());
        if attributes & NORMALS != 0 {
            floats(&mut out, &mut p.normals.iter().flatten().copied());
        }
        if attributes & UVS != 0 {
            floats(&mut out, &mut p.uvs.iter().flatten().copied());
        }
        if attributes & TANGENTS != 0 {
            floats(&mut out, &mut p.tangents.iter().flatten().copied());
        }
        if attributes & SKIN != 0 {
            for j in p.joints.iter().flatten() {
                out.extend_from_slice(&j.to_le_bytes());
            }
            floats(&mut out, &mut p.weights.iter().flatten().copied());
        }
        for &i in &p.indices {
            if wide {
                out.extend_from_slice(&i.to_le_bytes());
            } else {
                out.extend_from_slice(&(i as u16).to_le_bytes());
            }
        }
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    if let Some(skel) = &mesh.skin {
        v2::encode_skeleton(&mut out, skel, &mesh.clips);
    }
    out
}

/// The version of an `.omesh` blob.
pub fn version(bytes: &[u8]) -> Result<u16, String> {
    if !bytes.starts_with(MAGIC) {
        return Err("not an .omesh mesh (bad magic)".into());
    }
    bytes
        .get(4..6)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| "truncated .omesh header".into())
}

/// Decodes any supported version into a mesh called `name`.
pub fn decode(name: &str, bytes: &[u8]) -> Result<MeshAsset, String> {
    match version(bytes)? {
        1 => v1::decode(name, bytes),
        2 => v2::decode(name, bytes),
        v => Err(format!(
            ".omesh version {v} is not supported (this build reads up to {VERSION})"
        )),
    }
}

/// Version 1 (frozen).
mod v1 {
    use super::*;

    /// Upper bound per primitive: refuses absurd counts before allocating.
    const MAX_COUNT: u32 = 1 << 26;

    pub(super) struct Cursor<'a> {
        pub(super) bytes: &'a [u8],
        pub(super) at: usize,
    }

    impl<'a> Cursor<'a> {
        pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
            let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len());
            let end = end.ok_or("truncated .omesh data")?;
            let out = &self.bytes[self.at..end];
            self.at = end;
            Ok(out)
        }
        pub(super) fn u32(&mut self) -> Result<u32, String> {
            let b = self.take(4)?;
            Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }
        pub(super) fn floats<const N: usize>(&mut self, count: usize) -> Result<Vec<[f32; N]>, String> {
            let b = self.take(count * N * 4)?;
            Ok(b.chunks_exact(N * 4)
                .map(|c| std::array::from_fn(|k| f32::from_le_bytes([c[k * 4], c[k * 4 + 1], c[k * 4 + 2], c[k * 4 + 3]])))
                .collect())
        }
    }

    pub(super) fn decode(name: &str, bytes: &[u8]) -> Result<MeshAsset, String> {
        let mut c = Cursor { bytes, at: 8 };
        let count = c.u32()?;
        if count > 4096 {
            return Err(format!("{count} primitives in one .omesh"));
        }
        let mut primitives = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let n = c.u32()?;
            let index_count = c.u32()?;
            if n > MAX_COUNT || index_count > MAX_COUNT {
                return Err("too many vertices or indices in an .omesh primitive".into());
            }
            let head = c.take(4)?;
            let (wide, attributes) = (head[0], head[1]);
            if wide > 1 {
                return Err(format!("unknown .omesh index format {wide}"));
            }
            let n = n as usize;
            let mut p = PrimitiveData {
                positions: c.floats::<3>(n)?,
                ..Default::default()
            };
            if attributes & NORMALS != 0 {
                p.normals = c.floats::<3>(n)?;
            }
            if attributes & UVS != 0 {
                p.uvs = c.floats::<2>(n)?;
            }
            if attributes & TANGENTS != 0 {
                p.tangents = c.floats::<4>(n)?;
            }
            let width = if wide == 1 { 4 } else { 2 };
            let raw = c.take(index_count as usize * width)?;
            p.indices = if wide == 1 {
                raw.chunks_exact(4)
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect()
            } else {
                raw.chunks_exact(2)
                    .map(|b| u32::from(u16::from_le_bytes([b[0], b[1]])))
                    .collect()
            };
            let pad = (4 - c.at % 4) % 4;
            c.take(pad)?;
            primitives.push(p);
        }
        Ok(MeshAsset {
            name: name.to_owned(),
            primitives,
            ..Default::default()
        })
    }
}

/// Version 2: v1 plus skin weights, skeleton and clips.
mod v2 {
    use super::v1::Cursor;
    use super::*;
    use crate::Transform;
    use crate::skin::{Channel, ChannelPath, Clip, Interpolation, Joint, Mat4, Skeleton};

    const MAX_COUNT: u32 = 1 << 26;

    fn put_str(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }

    fn put_f32s(out: &mut Vec<u8>, v: impl IntoIterator<Item = f32>) {
        for x in v {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }

    fn put_mat(out: &mut Vec<u8>, m: &Mat4) {
        put_f32s(out, m.iter().flatten().copied());
    }

    pub(super) fn encode_skeleton(out: &mut Vec<u8>, skel: &Skeleton, clips: &[Clip]) {
        out.extend_from_slice(&(skel.joints.len() as u32).to_le_bytes());
        for j in &skel.joints {
            put_str(out, &j.name);
            let parent = j.parent.map_or(-1i32, |p| p as i32);
            out.extend_from_slice(&parent.to_le_bytes());
            let t = &j.rest;
            put_f32s(out, t.translation.iter().chain(&t.rotation).chain(&t.scale).copied());
            put_mat(out, &j.base);
            put_mat(out, &j.inverse_bind);
        }
        if clips.is_empty() {
            return;
        }
        out.extend_from_slice(&(clips.len() as u32).to_le_bytes());
        for c in clips {
            put_str(out, &c.name);
            put_f32s(out, [c.duration]);
            out.extend_from_slice(&(c.channels.len() as u32).to_le_bytes());
            for ch in &c.channels {
                out.extend_from_slice(&(ch.joint as u32).to_le_bytes());
                out.push(match ch.path {
                    ChannelPath::Translation => 0,
                    ChannelPath::Rotation => 1,
                    ChannelPath::Scale => 2,
                });
                out.push(u8::from(ch.interpolation == Interpolation::Linear));
                out.extend_from_slice(&0u16.to_le_bytes());
                let n = ch.times.len().min(ch.values.len());
                out.extend_from_slice(&(n as u32).to_le_bytes());
                put_f32s(out, ch.times[..n].iter().copied());
                put_f32s(out, ch.values[..n].iter().flatten().copied());
            }
        }
    }

    fn get_str(c: &mut Cursor<'_>) -> Result<String, String> {
        let n = c.u32()?;
        if n > 1 << 16 {
            return Err("name too long in an .omesh".into());
        }
        let s = String::from_utf8(c.take(n as usize)?.to_vec())
            .map_err(|_| "bad UTF-8 name in an .omesh".to_owned())?;
        let pad = (4 - c.at % 4) % 4;
        c.take(pad)?;
        Ok(s)
    }

    fn get_mat(c: &mut Cursor<'_>) -> Result<Mat4, String> {
        let v = c.floats::<4>(4)?;
        Ok([v[0], v[1], v[2], v[3]])
    }

    fn get_f32(c: &mut Cursor<'_>) -> Result<f32, String> {
        Ok(f32::from_bits(c.u32()?))
    }

    pub(super) fn decode(name: &str, bytes: &[u8]) -> Result<MeshAsset, String> {
        let head = bytes.get(6..8).ok_or("truncated .omesh header")?;
        let flags = u16::from_le_bytes([head[0], head[1]]);
        let mut c = Cursor { bytes, at: 8 };
        let count = c.u32()?;
        if count > 4096 {
            return Err(format!("{count} primitives in one .omesh"));
        }
        let mut primitives = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let n = c.u32()?;
            let index_count = c.u32()?;
            if n > MAX_COUNT || index_count > MAX_COUNT {
                return Err("too many vertices or indices in an .omesh primitive".into());
            }
            let head = c.take(4)?;
            let (wide, attributes) = (head[0], head[1]);
            if wide > 1 {
                return Err(format!("unknown .omesh index format {wide}"));
            }
            let n = n as usize;
            let mut p = PrimitiveData {
                positions: c.floats::<3>(n)?,
                ..Default::default()
            };
            if attributes & NORMALS != 0 {
                p.normals = c.floats::<3>(n)?;
            }
            if attributes & UVS != 0 {
                p.uvs = c.floats::<2>(n)?;
            }
            if attributes & TANGENTS != 0 {
                p.tangents = c.floats::<4>(n)?;
            }
            if attributes & SKIN != 0 {
                let raw = c.take(n * 8)?;
                p.joints = raw
                    .chunks_exact(8)
                    .map(|b| std::array::from_fn(|k| u16::from_le_bytes([b[k * 2], b[k * 2 + 1]])))
                    .collect();
                p.weights = c.floats::<4>(n)?;
            }
            let width = if wide == 1 { 4 } else { 2 };
            let raw = c.take(index_count as usize * width)?;
            p.indices = if wide == 1 {
                raw.chunks_exact(4)
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect()
            } else {
                raw.chunks_exact(2)
                    .map(|b| u32::from(u16::from_le_bytes([b[0], b[1]])))
                    .collect()
            };
            let pad = (4 - c.at % 4) % 4;
            c.take(pad)?;
            primitives.push(p);
        }
        let mut mesh = MeshAsset {
            name: name.to_owned(),
            primitives,
            ..Default::default()
        };
        if flags & HAS_SKELETON != 0 {
            let n = c.u32()?;
            if n > 1 << 16 {
                return Err(format!("{n} joints in one .omesh"));
            }
            let mut joints = Vec::with_capacity(n as usize);
            for _ in 0..n {
                let jname = get_str(&mut c)?;
                let parent = c.u32()? as i32;
                let f = c.floats::<10>(1)?[0];
                let rest = Transform {
                    translation: [f[0], f[1], f[2]],
                    rotation: [f[3], f[4], f[5], f[6]],
                    scale: [f[7], f[8], f[9]],
                };
                joints.push(Joint {
                    name: jname,
                    parent: usize::try_from(parent).ok().filter(|&p| p < n as usize),
                    rest,
                    base: get_mat(&mut c)?,
                    inverse_bind: get_mat(&mut c)?,
                });
            }
            mesh.skin = Some(Skeleton { joints });
        }
        if flags & HAS_CLIPS != 0 {
            let n = c.u32()?;
            if n > 4096 {
                return Err(format!("{n} clips in one .omesh"));
            }
            for _ in 0..n {
                let cname = get_str(&mut c)?;
                let duration = get_f32(&mut c)?;
                let channels_n = c.u32()?;
                if channels_n > MAX_COUNT {
                    return Err("too many channels in an .omesh clip".into());
                }
                let mut channels = Vec::new();
                for _ in 0..channels_n {
                    let joint = c.u32()? as usize;
                    let head = c.take(4)?;
                    let path = match head[0] {
                        0 => ChannelPath::Translation,
                        1 => ChannelPath::Rotation,
                        2 => ChannelPath::Scale,
                        k => return Err(format!("unknown .omesh channel path {k}")),
                    };
                    let interpolation = if head[1] == 1 {
                        Interpolation::Linear
                    } else {
                        Interpolation::Step
                    };
                    let keys = c.u32()?;
                    if keys > MAX_COUNT {
                        return Err("too many keys in an .omesh channel".into());
                    }
                    let times = c.floats::<1>(keys as usize)?.into_iter().map(|t| t[0]).collect();
                    let values = c.floats::<4>(keys as usize)?;
                    channels.push(Channel {
                        joint,
                        path,
                        interpolation,
                        times,
                        values,
                    });
                }
                mesh.clips.push(Clip {
                    name: cname,
                    duration,
                    channels,
                });
            }
        }
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Shape;

    #[test]
    fn round_trips_every_attribute_and_both_index_widths() {
        let mut mesh = Shape::default_of("sphere").unwrap().mesh();
        mesh.name = "ball".into();
        let mut big = PrimitiveData {
            positions: vec![[1.0, 2.0, 3.0]; 3],
            indices: vec![0, 1, 2, 70_000],
            ..Default::default()
        };
        big.uvs = vec![[0.5, 0.25]; 3];
        mesh.primitives.push(big);
        // Odd u16 index count: padding.
        mesh.primitives.push(PrimitiveData {
            positions: vec![[0.0; 3]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        });
        let bytes = encode(&mesh);
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(version(&bytes).unwrap(), 1);
        assert_eq!(decode("ball", &bytes).unwrap(), mesh);
    }

    #[test]
    fn skinned_meshes_round_trip_as_v2() {
        use crate::skin::{Channel, ChannelPath, Clip, IDENTITY, Interpolation, Joint, Skeleton};
        let mut mesh = Shape::default_of("cube").unwrap().mesh();
        let n = mesh.primitives[0].positions.len();
        mesh.primitives[0].joints = vec![[0, 1, 0, 0]; n];
        mesh.primitives[0].weights = vec![[0.75, 0.25, 0.0, 0.0]; n];
        mesh.skin = Some(Skeleton {
            joints: vec![
                Joint {
                    name: "hips".into(),
                    parent: None,
                    rest: crate::Transform::IDENTITY,
                    base: IDENTITY,
                    inverse_bind: IDENTITY,
                },
                Joint {
                    name: "spine-é".into(),
                    parent: Some(0),
                    rest: crate::Transform {
                        translation: [0.0, 1.0, 0.0],
                        ..crate::Transform::IDENTITY
                    },
                    base: IDENTITY,
                    inverse_bind: IDENTITY,
                },
            ],
        });
        mesh.clips.push(Clip {
            name: "idle".into(),
            duration: 1.5,
            channels: vec![Channel {
                joint: 1,
                path: ChannelPath::Rotation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.5],
                values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.38, 0.92]],
            }],
        });
        let bytes = encode(&mesh);
        assert_eq!(version(&bytes).unwrap(), 2);
        assert_eq!(decode(&mesh.name, &bytes).unwrap(), mesh);
        for cut in [20, bytes.len() / 2, bytes.len() - 1] {
            assert!(decode("x", &bytes[..cut]).is_err(), "cut at {cut}");
        }
        // Static meshes keep the v1 layout (readable by older players).
        let plain = Shape::default_of("cube").unwrap().mesh();
        assert_eq!(version(&encode(&plain)).unwrap(), 1);
    }

    #[test]
    fn mismatched_attributes_are_dropped() {
        let mesh = MeshAsset {
            name: "m".into(),
            primitives: vec![PrimitiveData {
                positions: vec![[0.0; 3]; 3],
                normals: vec![[0.0, 1.0, 0.0]; 2],
                indices: vec![0, 1, 2],
                ..Default::default()
            }],
            ..Default::default()
        };
        let back = decode("m", &encode(&mesh)).unwrap();
        assert!(back.primitives[0].normals.is_empty());
    }

    #[test]
    fn bad_blobs_are_refused_without_panicking() {
        let good = encode(&Shape::default_of("cube").unwrap().mesh());
        assert!(decode("x", b"nope").is_err());
        assert!(decode("x", b"OMSH").is_err());
        let mut future = good.clone();
        future[4] = 9;
        assert!(decode("x", &future).unwrap_err().contains("version 9"));
        for cut in [9, 13, 30, good.len() - 1] {
            assert!(decode("x", &good[..cut]).is_err(), "cut at {cut}");
        }
        let mut huge = good.clone();
        huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode("x", &huge).is_err());
    }

    /// Frozen v1 blob (a unit plane, written by the first `.omesh` encoder):
    /// later decoders must keep reading it.
    #[test]
    fn a_frozen_v1_blob_still_decodes() {
        let bytes = include_bytes!("../../../examples/fixtures/compat/omesh_v1/plane.omesh");
        let mesh = decode("plane", bytes).unwrap();
        assert_eq!(mesh.primitives.len(), 1);
        let p = &mesh.primitives[0];
        assert_eq!(p.positions.len(), 4);
        assert_eq!(p.indices, vec![0, 1, 3, 0, 3, 2]);
        assert_eq!(p.normals[0], [0.0, 1.0, 0.0]);
        assert_eq!(p.tangents.len(), 4);
    }

    /// Fixture generators run only with `ORBIT_REGENERATE_FIXTURES=1` and refuse
    /// to overwrite an existing file unless it is `force`.
    fn regen_allowed(out: &std::path::Path) -> bool {
        let mode = std::env::var("ORBIT_REGENERATE_FIXTURES").unwrap_or_default();
        if mode != "1" && mode != "force" {
            eprintln!("skipped: set ORBIT_REGENERATE_FIXTURES=1 to regenerate the frozen fixtures");
            return false;
        }
        if out.exists() && mode != "force" {
            eprintln!("skipped: {} exists; ORBIT_REGENERATE_FIXTURES=force overwrites it", out.display());
            return false;
        }
        true
    }

    #[test]
    #[ignore = "writes the frozen fixture; run once by hand"]
    fn write_frozen_v1_blob() {
        if !regen_allowed(&std::path::PathBuf::from("../../examples/fixtures/compat/omesh_v1/plane.omesh")) {
            return;
        }
        let shape = Shape::Plane { size: [1.0, 1.0], subdivisions: 1 };
        std::fs::create_dir_all("../../examples/fixtures/compat/omesh_v1").unwrap();
        std::fs::write(
            "../../examples/fixtures/compat/omesh_v1/plane.omesh",
            encode(&shape.mesh()),
        )
        .unwrap();
    }
}
