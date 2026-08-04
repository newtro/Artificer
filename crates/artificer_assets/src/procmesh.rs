//! Procedural mesh builders. All geometry is Y-up, -Z forward, meters.
//! Pivot is the object's logical center unless documented otherwise.

use artificer_scene::{MeshData, TransformDesc};
use glam::{Mat3, Vec2, Vec3};
use std::f32::consts::{PI, TAU};

/// Axis-aligned box with per-face normals and box-projected UVs.
pub fn cuboid(width: f32, height: f32, depth: f32) -> MeshData {
    let (hx, hy, hz) = (width * 0.5, height * 0.5, depth * 0.5);
    // (normal, four corners CCW when viewed from outside)
    let faces: [(Vec3, [Vec3; 4]); 6] = [
        (
            Vec3::Z,
            [
                Vec3::new(-hx, -hy, hz),
                Vec3::new(hx, -hy, hz),
                Vec3::new(hx, hy, hz),
                Vec3::new(-hx, hy, hz),
            ],
        ),
        (
            Vec3::NEG_Z,
            [
                Vec3::new(hx, -hy, -hz),
                Vec3::new(-hx, -hy, -hz),
                Vec3::new(-hx, hy, -hz),
                Vec3::new(hx, hy, -hz),
            ],
        ),
        (
            Vec3::X,
            [
                Vec3::new(hx, -hy, hz),
                Vec3::new(hx, -hy, -hz),
                Vec3::new(hx, hy, -hz),
                Vec3::new(hx, hy, hz),
            ],
        ),
        (
            Vec3::NEG_X,
            [
                Vec3::new(-hx, -hy, -hz),
                Vec3::new(-hx, -hy, hz),
                Vec3::new(-hx, hy, hz),
                Vec3::new(-hx, hy, -hz),
            ],
        ),
        (
            Vec3::Y,
            [
                Vec3::new(-hx, hy, hz),
                Vec3::new(hx, hy, hz),
                Vec3::new(hx, hy, -hz),
                Vec3::new(-hx, hy, -hz),
            ],
        ),
        (
            Vec3::NEG_Y,
            [
                Vec3::new(-hx, -hy, -hz),
                Vec3::new(hx, -hy, -hz),
                Vec3::new(hx, -hy, hz),
                Vec3::new(-hx, -hy, hz),
            ],
        ),
    ];

    let mut mesh = MeshData::default();
    for (normal, corners) in faces {
        let base = mesh.positions.len() as u32;
        for (i, c) in corners.iter().enumerate() {
            mesh.positions.push(c.to_array());
            mesh.normals.push(normal.to_array());
            let uv = match i {
                0 => [0.0, 1.0],
                1 => [1.0, 1.0],
                2 => [1.0, 0.0],
                _ => [0.0, 0.0],
            };
            mesh.uvs.push(uv);
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh
}

/// UV sphere centered on the pivot.
pub fn uv_sphere(radius: f32, longitudes: u32, latitudes: u32) -> MeshData {
    let longitudes = longitudes.max(3);
    let latitudes = latitudes.max(2);
    let mut mesh = MeshData::default();

    for lat in 0..=latitudes {
        let v = lat as f32 / latitudes as f32;
        let theta = v * PI; // 0 at north pole
        let (sin_t, cos_t) = theta.sin_cos();
        for lon in 0..=longitudes {
            let u = lon as f32 / longitudes as f32;
            let phi = u * TAU;
            let (sin_p, cos_p) = phi.sin_cos();
            let n = Vec3::new(sin_t * cos_p, cos_t, sin_t * sin_p);
            mesh.positions.push((n * radius).to_array());
            mesh.normals.push(n.to_array());
            mesh.uvs.push([u, v]);
        }
    }

    let stride = longitudes + 1;
    for lat in 0..latitudes {
        for lon in 0..longitudes {
            let a = lat * stride + lon;
            let b = a + stride;
            mesh.indices
                .extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    mesh
}

/// Cylinder along +Y, centered on pivot, with caps.
pub fn cylinder(radius: f32, height: f32, segments: u32) -> MeshData {
    lathe(
        &[
            (Vec2::new(0.0, -height * 0.5), Vec2::new(0.0, -1.0)),
            (Vec2::new(radius, -height * 0.5), Vec2::new(0.0, -1.0)),
            (Vec2::new(radius, -height * 0.5), Vec2::new(1.0, 0.0)),
            (Vec2::new(radius, height * 0.5), Vec2::new(1.0, 0.0)),
            (Vec2::new(radius, height * 0.5), Vec2::new(0.0, 1.0)),
            (Vec2::new(0.0, height * 0.5), Vec2::new(0.0, 1.0)),
        ],
        segments,
    )
}

/// Cone along +Y with base at -h/2 and apex at +h/2.
pub fn cone(radius: f32, height: f32, segments: u32) -> MeshData {
    let slope = radius / height;
    let n = Vec2::new(1.0, slope).normalize();
    lathe(
        &[
            (Vec2::new(0.0, -height * 0.5), Vec2::new(0.0, -1.0)),
            (Vec2::new(radius, -height * 0.5), Vec2::new(0.0, -1.0)),
            (Vec2::new(radius, -height * 0.5), n),
            (Vec2::new(0.0, height * 0.5), n),
        ],
        segments,
    )
}

/// Torus in the XZ plane, centered on pivot.
pub fn torus(
    major_radius: f32,
    minor_radius: f32,
    major_segments: u32,
    minor_segments: u32,
) -> MeshData {
    let major_segments = major_segments.max(3);
    let minor_segments = minor_segments.max(3);
    let mut mesh = MeshData::default();

    for i in 0..=major_segments {
        let u = i as f32 / major_segments as f32;
        let phi = u * TAU;
        let (sin_p, cos_p) = phi.sin_cos();
        let ring_center = Vec3::new(cos_p * major_radius, 0.0, sin_p * major_radius);
        let ring_dir = Vec3::new(cos_p, 0.0, sin_p);
        for j in 0..=minor_segments {
            let v = j as f32 / minor_segments as f32;
            let theta = v * TAU;
            let (sin_t, cos_t) = theta.sin_cos();
            let normal = ring_dir * cos_t + Vec3::Y * sin_t;
            mesh.positions
                .push((ring_center + normal * minor_radius).to_array());
            mesh.normals.push(normal.to_array());
            mesh.uvs.push([u, v]);
        }
    }

    let stride = minor_segments + 1;
    for i in 0..major_segments {
        for j in 0..minor_segments {
            let a = i * stride + j;
            let b = a + stride;
            mesh.indices
                .extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    mesh
}

/// Flat ring (annulus) in the XZ plane facing +Y.
///
/// UV.x runs RADIALLY: 0 at the inner edge, 1 at the outer. UV.y runs around
/// the circumference. That orientation exists for one consumer pattern:
/// planetary rings sample a 1-D radial strip texture, so a ring texture is
/// just an Nx1 image and the mesh brings the mapping.
pub fn annulus(inner_radius: f32, outer_radius: f32, segments: u32) -> MeshData {
    let segments = segments.max(8);
    let mut mesh = MeshData::default();
    for s in 0..=segments {
        let v = s as f32 / segments as f32;
        let phi = v * TAU;
        let (sin_p, cos_p) = phi.sin_cos();
        let dir = Vec3::new(cos_p, 0.0, sin_p);
        mesh.positions.push((dir * inner_radius).to_array());
        mesh.normals.push([0.0, 1.0, 0.0]);
        mesh.uvs.push([0.0, v]);
        mesh.positions.push((dir * outer_radius).to_array());
        mesh.normals.push([0.0, 1.0, 0.0]);
        mesh.uvs.push([1.0, v]);
    }
    for s in 0..segments {
        let a = s * 2; // inner edge, this segment
        let b = a + 1; // outer edge, this segment
        let c = a + 2; // inner edge, next segment
        let d = a + 3; // outer edge, next segment
        mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
    }
    mesh
}

/// Flat quad in the XZ plane facing +Y (`size` on each side).
pub fn quad_xz(size_x: f32, size_z: f32) -> MeshData {
    let (hx, hz) = (size_x * 0.5, size_z * 0.5);
    MeshData {
        positions: vec![
            [-hx, 0.0, -hz],
            [hx, 0.0, -hz],
            [hx, 0.0, hz],
            [-hx, 0.0, hz],
        ],
        normals: vec![[0.0, 1.0, 0.0]; 4],
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        indices: vec![0, 2, 1, 0, 3, 2],
    }
}

/// Surface of revolution around +Y. `profile` is (point, normal) pairs in the
/// XY half-plane (x = radius, y = height); consecutive pairs form bands.
fn lathe(profile: &[(Vec2, Vec2)], segments: u32) -> MeshData {
    let segments = segments.max(3);
    let mut mesh = MeshData::default();
    let rows = profile.len() as u32;

    for (p, n2) in profile {
        for s in 0..=segments {
            let u = s as f32 / segments as f32;
            let phi = u * TAU;
            let (sin_p, cos_p) = phi.sin_cos();
            let pos = Vec3::new(p.x * cos_p, p.y, p.x * sin_p);
            let normal = Vec3::new(n2.x * cos_p, n2.y, n2.x * sin_p).normalize_or_zero();
            mesh.positions.push(pos.to_array());
            mesh.normals.push(normal.to_array());
            mesh.uvs.push([u, p.y]);
        }
    }

    let stride = segments + 1;
    for row in 0..rows - 1 {
        // Only stitch bands that share a normal "family" (avoid welding the
        // cap ring to the side ring — duplicated profile rows handle creases).
        let (pa, na) = profile[row as usize];
        let (pb, nb) = profile[row as usize + 1];
        // Skip degenerate bands where both rows are the same point.
        if (pa - pb).length_squared() < 1e-12 && (na - nb).length_squared() > 1e-6 {
            continue;
        }
        for s in 0..segments {
            let a = row * stride + s;
            let b = a + stride;
            mesh.indices
                .extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    mesh
}

/// Apply a transform to a mesh (positions by TRS, normals by rotation and
/// inverse-scale, renormalized).
pub fn transform_mesh(mesh: &MeshData, transform: &TransformDesc) -> MeshData {
    let rot = Mat3::from_quat(transform.rotation);
    let scale = transform.scale;
    let inv_scale = Vec3::new(
        if scale.x != 0.0 { 1.0 / scale.x } else { 0.0 },
        if scale.y != 0.0 { 1.0 / scale.y } else { 0.0 },
        if scale.z != 0.0 { 1.0 / scale.z } else { 0.0 },
    );
    let mut out = mesh.clone();
    for p in out.positions.iter_mut() {
        let v = transform.translation + rot * (scale * Vec3::from_array(*p));
        *p = v.to_array();
    }
    for n in out.normals.iter_mut() {
        let v = (rot * (inv_scale * Vec3::from_array(*n))).normalize_or_zero();
        *n = v.to_array();
    }
    out
}

/// Concatenate meshes (already in a common space) into one.
pub fn merge(parts: &[MeshData]) -> MeshData {
    let mut out = MeshData::default();
    for part in parts {
        let base = out.positions.len() as u32;
        out.positions.extend_from_slice(&part.positions);
        out.normals.extend_from_slice(&part.normals);
        out.uvs.extend_from_slice(&part.uvs);
        out.indices.extend(part.indices.iter().map(|i| i + base));
    }
    out
}

/// Merge with per-part placement — the workhorse for composing multi-part
/// ships and stations out of primitives.
pub fn merge_placed(parts: &[(MeshData, TransformDesc)]) -> MeshData {
    let transformed: Vec<MeshData> = parts.iter().map(|(m, t)| transform_mesh(m, t)).collect();
    merge(&transformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuboid_is_valid_and_bounded() {
        let m = cuboid(2.0, 4.0, 6.0);
        m.validate().unwrap();
        let b = m.bounds().unwrap();
        assert_eq!(b.extents(), Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(m.triangle_count(), 12);
    }

    #[test]
    fn sphere_vertices_lie_on_radius() {
        let m = uv_sphere(3.0, 16, 12);
        m.validate().unwrap();
        for p in &m.positions {
            let r = Vec3::from_array(*p).length();
            assert!((r - 3.0).abs() < 1e-4);
        }
    }

    #[test]
    fn cylinder_and_cone_are_valid() {
        cylinder(1.0, 2.0, 12).validate().unwrap();
        cone(1.0, 2.0, 12).validate().unwrap();
        torus(3.0, 0.4, 24, 8).validate().unwrap();
    }

    #[test]
    fn transform_moves_bounds() {
        let m = cuboid(2.0, 2.0, 2.0);
        let moved = transform_mesh(&m, &TransformDesc::from_translation(Vec3::X * 10.0));
        let b = moved.bounds().unwrap();
        assert!((b.center().x - 10.0).abs() < 1e-5);
    }

    #[test]
    fn merge_preserves_triangles_and_validity() {
        let a = cuboid(1.0, 1.0, 1.0);
        let b = uv_sphere(0.5, 8, 6);
        let m = merge_placed(&[
            (a.clone(), TransformDesc::from_translation(Vec3::X * -2.0)),
            (b.clone(), TransformDesc::from_translation(Vec3::X * 2.0)),
        ]);
        m.validate().unwrap();
        assert_eq!(m.triangle_count(), a.triangle_count() + b.triangle_count());
    }
}
