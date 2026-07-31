use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn extents(&self) -> Vec3 {
        self.max - self.min
    }

    pub fn center(&self) -> Vec3 {
        (self.max + self.min) * 0.5
    }

    pub fn contains(&self, p: Vec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }
}

/// Interleaved-agnostic triangle mesh: positions + normals + uvs + indices.
/// The exchange format between `artificer_assets` builders and render adapters.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn bounds(&self) -> Option<Aabb> {
        if self.positions.is_empty() {
            return None;
        }
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        for p in &self.positions {
            let v = Vec3::from_array(*p);
            min = min.min(v);
            max = max.max(v);
        }
        Some(Aabb { min, max })
    }

    /// Structural sanity: matching attribute counts, indices in range,
    /// index count divisible by 3, finite positions.
    pub fn validate(&self) -> Result<(), String> {
        let n = self.positions.len();
        if n == 0 {
            return Err("mesh has no vertices".into());
        }
        if self.normals.len() != n {
            return Err(format!(
                "normal count {} != vertex count {}",
                self.normals.len(),
                n
            ));
        }
        if self.uvs.len() != n {
            return Err(format!("uv count {} != vertex count {}", self.uvs.len(), n));
        }
        if self.indices.is_empty() || !self.indices.len().is_multiple_of(3) {
            return Err(format!(
                "index count {} not a triangle list",
                self.indices.len()
            ));
        }
        if let Some(&bad) = self.indices.iter().find(|&&i| i as usize >= n) {
            return Err(format!("index {} out of range ({} vertices)", bad, n));
        }
        for p in &self.positions {
            if !p.iter().all(|c| c.is_finite()) {
                return Err("non-finite vertex position".into());
            }
        }
        Ok(())
    }

    /// Recompute smooth (area-weighted) vertex normals from triangles.
    pub fn recompute_normals(&mut self) {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for tri in self.indices.chunks_exact(3) {
            let a = Vec3::from_array(self.positions[tri[0] as usize]);
            let b = Vec3::from_array(self.positions[tri[1] as usize]);
            let c = Vec3::from_array(self.positions[tri[2] as usize]);
            let face = (b - a).cross(c - a); // magnitude = 2x area (weighting)
            for &i in tri {
                normals[i as usize] += face;
            }
        }
        self.normals = normals
            .into_iter()
            .map(|n| n.normalize_or_zero().to_array())
            .collect();
    }

    /// A minimal valid mesh for unit tests.
    pub fn unit_test_triangle() -> MeshData {
        MeshData {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_validates() {
        assert!(MeshData::unit_test_triangle().validate().is_ok());
    }

    #[test]
    fn bad_index_rejected() {
        let mut m = MeshData::unit_test_triangle();
        m.indices = vec![0, 1, 9];
        assert!(m.validate().is_err());
    }

    #[test]
    fn bounds_cover_vertices() {
        let m = MeshData::unit_test_triangle();
        let b = m.bounds().unwrap();
        assert_eq!(b.min, Vec3::ZERO);
        assert_eq!(b.max, Vec3::new(1.0, 1.0, 0.0));
    }

    #[test]
    fn recomputed_normals_point_out_of_plane() {
        let mut m = MeshData::unit_test_triangle();
        m.normals = vec![[0.0; 3]; 3];
        m.recompute_normals();
        for n in &m.normals {
            assert!((Vec3::from_array(*n) - Vec3::Z).length() < 1e-5);
        }
    }
}
