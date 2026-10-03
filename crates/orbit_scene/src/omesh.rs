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
//! Little endian, every array 4-byte aligned. Each version keeps its own
//! decoder ([`decode`] dispatches on the version and never drops one, plan
//! §1.3); an unknown version is refused with an error, never a panic.

use crate::{MeshAsset, PrimitiveData};

pub const MAGIC: &[u8; 4] = b"OMSH";
/// Version written by [`encode`].
pub const VERSION: u16 = 1;

const NORMALS: u8 = 1;
const UVS: u8 = 2;
const TANGENTS: u8 = 4;

/// Encodes `mesh` (its name is not stored: the pak index names it).
/// Attributes whose length differs from the vertex count are dropped (the
/// renderer regenerates normals, UVs default to zero).
pub fn encode(mesh: &MeshAsset) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
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

    struct Cursor<'a> {
        bytes: &'a [u8],
        at: usize,
    }

    impl<'a> Cursor<'a> {
        fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
            let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len());
            let end = end.ok_or("truncated .omesh data")?;
            let out = &self.bytes[self.at..end];
            self.at = end;
            Ok(out)
        }
        fn u32(&mut self) -> Result<u32, String> {
            let b = self.take(4)?;
            Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }
        fn floats<const N: usize>(&mut self, count: usize) -> Result<Vec<[f32; N]>, String> {
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
        })
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
    fn mismatched_attributes_are_dropped() {
        let mesh = MeshAsset {
            name: "m".into(),
            primitives: vec![PrimitiveData {
                positions: vec![[0.0; 3]; 3],
                normals: vec![[0.0, 1.0, 0.0]; 2],
                indices: vec![0, 1, 2],
                ..Default::default()
            }],
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

    #[test]
    #[ignore = "writes the frozen fixture; run once by hand"]
    fn write_frozen_v1_blob() {
        let shape = Shape::Plane { size: [1.0, 1.0], subdivisions: 1 };
        std::fs::create_dir_all("../../examples/fixtures/compat/omesh_v1").unwrap();
        std::fs::write(
            "../../examples/fixtures/compat/omesh_v1/plane.omesh",
            encode(&shape.mesh()),
        )
        .unwrap();
    }
}
