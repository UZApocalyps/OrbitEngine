//! Interleaved vertex building from [`orbit_scene::PrimitiveData`].

use bytemuck::{Pod, Zeroable};
use orbit_scene::PrimitiveData;

use crate::math;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

pub const VERTEX_ATTRIBUTES: [orbit_gfx::wgpu::VertexAttribute; 3] = orbit_gfx::wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x2,
];

/// Builds vertices and indices. Missing normals are generated (smooth,
/// area weighted), missing UVs are zero, a missing index list means a plain
/// triangle list, and out-of-range indices drop their triangle.
pub fn build(p: &PrimitiveData) -> (Vec<Vertex>, Vec<u32>) {
    let n = p.positions.len();
    let mut indices: Vec<u32> = if p.indices.is_empty() {
        (0..n as u32).collect()
    } else {
        p.indices.clone()
    };
    indices.truncate(indices.len() / 3 * 3);
    let indices: Vec<u32> = indices
        .chunks_exact(3)
        .filter(|t| t.iter().all(|&i| (i as usize) < n))
        .flatten()
        .copied()
        .collect();

    let normals = if p.normals.len() == n {
        p.normals.clone()
    } else {
        generate_normals(&p.positions, &indices)
    };
    let vertices = (0..n)
        .map(|i| Vertex {
            position: p.positions[i],
            normal: normals[i],
            uv: p.uvs.get(i).copied().unwrap_or([0.0; 2]),
        })
        .collect();
    (vertices, indices)
}

fn generate_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut acc = vec![[0.0f32; 3]; positions.len()];
    for t in indices.chunks_exact(3) {
        let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
        let face = math::cross(
            math::sub(positions[b], positions[a]),
            math::sub(positions[c], positions[a]),
        );
        for i in [a, b, c] {
            for k in 0..3 {
                acc[i][k] += face[k];
            }
        }
    }
    acc.into_iter()
        .map(|v| {
            let v = math::normalize(v);
            if v == [0.0; 3] { [0.0, 1.0, 0.0] } else { v }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_normals_and_drops_bad_triangles() {
        let p = PrimitiveData {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2, 0, 1, 9, 0],
            ..Default::default()
        };
        let (v, i) = build(&p);
        assert_eq!(i, vec![0, 1, 2]);
        assert_eq!(v[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(v[2].uv, [0.0, 0.0]);
        assert_eq!(size_of::<Vertex>(), 32);
    }

    #[test]
    fn non_indexed_primitive_gets_sequential_indices() {
        let p = PrimitiveData {
            positions: vec![[0.0; 3]; 3],
            normals: vec![[1.0, 0.0, 0.0]; 3],
            uvs: vec![[0.5, 0.5]; 3],
            ..Default::default()
        };
        let (v, i) = build(&p);
        assert_eq!(i, vec![0, 1, 2]);
        assert_eq!(v[1].normal, [1.0, 0.0, 0.0]);
        // Degenerate triangle: fallback normal is kept when provided.
        assert_eq!(v[1].uv, [0.5, 0.5]);
    }
}
