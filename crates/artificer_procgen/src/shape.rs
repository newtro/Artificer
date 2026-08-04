//! Sphere-derived geometry: displaced planets and lumpy asteroids.
//!
//! Both start from the same lat/lon grid as `procmesh::uv_sphere` (so the
//! equirect textures map identically), displace radially by a field, and
//! then REBUILD normals from the displaced faces — the analytic sphere
//! normal is wrong the moment a vertex moves, and lighting is what sells
//! the relief.

use artificer_core::SeededRng;
use artificer_scene::MeshData;
use glam::Vec3;
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use std::f32::consts::{PI, TAU};

/// UV sphere displaced radially by `offset(dir)` (a fraction: the vertex
/// lands at `radius * (1 + offset)`).
pub(crate) fn displaced_sphere(
    radius: f32,
    longitudes: u32,
    latitudes: u32,
    offset: impl Fn(Vec3) -> f32,
) -> MeshData {
    let longitudes = longitudes.max(8);
    let latitudes = latitudes.max(6);
    let mut mesh = MeshData::default();

    for lat in 0..=latitudes {
        let v = lat as f32 / latitudes as f32;
        let theta = v * PI;
        let (sin_t, cos_t) = theta.sin_cos();
        for lon in 0..=longitudes {
            let u = lon as f32 / longitudes as f32;
            let phi = u * TAU;
            let (sin_p, cos_p) = phi.sin_cos();
            let n = Vec3::new(sin_t * cos_p, cos_t, sin_t * sin_p);
            let r = radius * (1.0 + offset(n));
            mesh.positions.push((n * r).to_array());
            mesh.normals.push(n.to_array()); // replaced below
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

    rebuild_sphere_normals(&mut mesh, longitudes, latitudes);
    mesh
}

/// Recompute smooth normals with the sphere's duplicate vertices WELDED.
///
/// The grid duplicates the seam column (lon 0 == lon N) and fans every pole
/// row out of coincident vertices. Naive per-index accumulation gives those
/// duplicates different neighbour sets, so the seam and poles light with a
/// visible crease. Accumulating into a canonical slot per POSITION — seam
/// wrapped, each pole collapsed to one slot — makes the duplicates share
/// one normal and the crease disappear.
fn rebuild_sphere_normals(mesh: &mut MeshData, longitudes: u32, latitudes: u32) {
    let stride = longitudes + 1;
    let canonical = |index: u32| -> u32 {
        let lat = index / stride;
        let lon = index % stride;
        if lat == 0 {
            0
        } else if lat == latitudes {
            lat * stride
        } else {
            lat * stride + (lon % longitudes)
        }
    };

    let mut accum = vec![Vec3::ZERO; mesh.positions.len()];
    for tri in mesh.indices.chunks_exact(3) {
        let p0 = Vec3::from_array(mesh.positions[tri[0] as usize]);
        let p1 = Vec3::from_array(mesh.positions[tri[1] as usize]);
        let p2 = Vec3::from_array(mesh.positions[tri[2] as usize]);
        // Area-weighted (un-normalized cross): big faces dominate, tiny
        // pole slivers do not swing the pole normal around.
        //
        // Operand order matters and was WRONG once: with this grid's index
        // winding, (p1-p0)×(p2-p0) points INTO the sphere, and every
        // displaced planet lit from the wrong side while ambient light hid
        // it. `rebuilt_normals_point_outward` pins the correct orientation.
        let face = (p2 - p0).cross(p1 - p0);
        for &i in tri {
            accum[canonical(i) as usize] += face;
        }
    }
    for (i, normal) in mesh.normals.iter_mut().enumerate() {
        let n = accum[canonical(i as u32) as usize].normalize_or_zero();
        if n != Vec3::ZERO {
            *normal = n.to_array();
        }
    }
}

/// Asteroid/rock parameters.
#[derive(Debug, Clone)]
pub struct AsteroidSpec {
    pub seed: u64,
    /// BOUNDING radius: the finished rock is normalized so no vertex lies
    /// beyond it. Callers matching a physics collider can trust that a
    /// sphere of this radius strictly contains the visual.
    pub radius: f32,
    /// Large-scale lumpiness as a fraction of radius (~0.25 = potato).
    pub irregularity: f32,
    /// Crater depth as a fraction of radius (0 = none).
    pub cratering: f32,
    /// Grid resolution (longitudes; latitudes is 2/3 of it).
    pub resolution: u32,
}

impl Default for AsteroidSpec {
    fn default() -> Self {
        Self {
            seed: 0,
            radius: 1.0,
            irregularity: 0.25,
            cratering: 0.12,
            resolution: 20,
        }
    }
}

/// A lumpy, cratered, axis-squashed rock. Same seed, same rock.
pub fn asteroid_mesh(spec: &AsteroidSpec) -> MeshData {
    let mut rng = SeededRng::new(spec.seed ^ 0xA57E_201D);
    let lumps = Fbm::<Perlin>::new((rng.next_u64() & 0xFFFF_FFFF) as u32).set_octaves(4);

    // A few spherical dents. Each is (unit direction, angular radius, depth).
    let crater_count = if spec.cratering > 0.0 {
        3 + (rng.next_u64() % 4) as usize
    } else {
        0
    };
    let craters: Vec<(Vec3, f32, f32)> = (0..crater_count)
        .map(|_| {
            let y = rng.next_f32() * 2.0 - 1.0;
            let phi = rng.next_f32() * TAU;
            let r = (1.0f32 - y * y).max(0.0).sqrt();
            (
                Vec3::new(r * phi.cos(), y, r * phi.sin()),
                0.25 + rng.next_f32() * 0.5,
                spec.cratering * (0.5 + rng.next_f32()),
            )
        })
        .collect();

    // Per-axis squash makes rocks read as tumbling bodies, not marbles.
    let squash = Vec3::new(
        0.75 + rng.next_f32() * 0.35,
        0.6 + rng.next_f32() * 0.4,
        0.75 + rng.next_f32() * 0.35,
    );

    let lon = spec.resolution.max(12);
    let lat = (spec.resolution * 2 / 3).max(8);
    let mut mesh = displaced_sphere(spec.radius, lon, lat, |dir| {
        let p = dir * 1.9;
        let lump = lumps.get([p.x as f64, p.y as f64, p.z as f64]) as f32;
        let mut h = lump * spec.irregularity;
        for (c_dir, c_radius, c_depth) in &craters {
            let d = dir.angle_between(*c_dir);
            if d < *c_radius {
                // Bowl profile: deepest in the middle, rim flush with the
                // surface (a raised rim needs more resolution than a
                // background rock gets).
                let t = d / c_radius;
                h -= c_depth * (1.0 - t * t);
            }
        }
        h
    });

    for p in mesh.positions.iter_mut() {
        let v = Vec3::from_array(*p) * squash;
        *p = v.to_array();
    }

    // Normalize to the BOUNDING radius. Lumps and squash both push vertices
    // past the nominal sphere, and a visual that pokes outside its physics
    // collider is a rock you clip through — so the promise is inverted:
    // `radius` bounds the rock, the lumps live inside it.
    let max_len = mesh
        .positions
        .iter()
        .map(|p| Vec3::from_array(*p).length())
        .fold(0.0f32, f32::max);
    if max_len > 0.0 {
        // 0.99: STRICTLY inside, not touching. A vertex fitted to exactly
        // the bounding radius can land epsilon outside it after the
        // world-transform rotation the caller applies.
        let fit = spec.radius * 0.99 / max_len;
        for p in mesh.positions.iter_mut() {
            let v = Vec3::from_array(*p) * fit;
            *p = v.to_array();
        }
    }

    // Squashing and fitting moved the surface; the lighting must follow it.
    rebuild_sphere_normals(&mut mesh, lon, lat);
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displaced_sphere_is_valid_and_displaced() {
        let m = displaced_sphere(10.0, 24, 16, |d| 0.1 * d.y);
        m.validate().unwrap();
        let radii: Vec<f32> = m
            .positions
            .iter()
            .map(|p| Vec3::from_array(*p).length())
            .collect();
        let min = radii.iter().cloned().fold(f32::MAX, f32::min);
        let max = radii.iter().cloned().fold(f32::MIN, f32::max);
        assert!(max > 10.5, "north pole should push out");
        assert!(min < 9.5, "south pole should pull in");
    }

    #[test]
    fn rebuilt_normals_point_outward() {
        // The whole point of rebuilding: lighting. If the accumulated face
        // normals came out INWARD (winding misread), every planet would be
        // lit from the wrong side — subtly on textured surfaces, blatantly
        // on grey asteroids. Pin the direction, not just the welding.
        let planet = displaced_sphere(10.0, 24, 16, |d| 0.08 * (d.x * 3.0).sin());
        for (p, n) in planet.positions.iter().zip(planet.normals.iter()) {
            let outward = Vec3::from_array(*p).normalize();
            let normal = Vec3::from_array(*n);
            assert!(
                normal.dot(outward) > 0.2,
                "normal {normal} vs outward {outward}"
            );
        }
        let rock = asteroid_mesh(&AsteroidSpec {
            seed: 5,
            ..Default::default()
        });
        let mut outward_count = 0;
        for (p, n) in rock.positions.iter().zip(rock.normals.iter()) {
            let outward = Vec3::from_array(*p).normalize();
            if Vec3::from_array(*n).dot(outward) > 0.0 {
                outward_count += 1;
            }
        }
        // Craters legitimately tilt normals away from radial; the bulk
        // must still face out.
        assert!(
            outward_count * 10 > rock.positions.len() * 9,
            "{outward_count}/{} rock normals face outward",
            rock.positions.len()
        );
    }

    #[test]
    fn seam_and_pole_normals_are_welded() {
        let m = displaced_sphere(5.0, 16, 12, |d| {
            0.08 * (d.x * 3.0).sin() * (d.y * 2.0).cos()
        });
        let stride = 17u32;
        // Seam: lon 0 and lon 16 duplicate the same position on every row.
        for lat in 1..12 {
            let a = (lat * stride) as usize;
            let b = (lat * stride + 16) as usize;
            assert_eq!(m.normals[a], m.normals[b], "seam row {lat}");
        }
        // Pole: every vertex of the top row shares one normal.
        for lon in 1..17 {
            assert_eq!(m.normals[0], m.normals[lon as usize], "north pole");
        }
    }

    #[test]
    fn asteroids_are_valid_deterministic_and_lumpy() {
        let spec = AsteroidSpec {
            seed: 99,
            radius: 2.0,
            ..Default::default()
        };
        let a = asteroid_mesh(&spec);
        let b = asteroid_mesh(&spec);
        a.validate().unwrap();
        assert_eq!(a.positions, b.positions, "same seed, same rock");
        let radii: Vec<f32> = a
            .positions
            .iter()
            .map(|p| Vec3::from_array(*p).length())
            .collect();
        let min = radii.iter().cloned().fold(f32::MAX, f32::min);
        let max = radii.iter().cloned().fold(f32::MIN, f32::max);
        assert!(max / min > 1.15, "a rock should not be a sphere");
    }
}
