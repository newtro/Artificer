//! Noise fields sampled on the unit sphere.
//!
//! Everything samples by DIRECTION (a point on the unit sphere), never by
//! UV: that way the displaced mesh and the baked equirect textures are
//! guaranteed to agree — both ask the same field the same question — and
//! there are no seams at the texture wrap column, because a direction has
//! no seam.

use crate::{GasSpec, TerrainSpec};
use artificer_core::SeededRng;
use glam::Vec3;
use noise::{Fbm, MultiFractal, NoiseFn, Perlin, RidgedMulti};

fn at(noise: &impl NoiseFn<f64, 3>, dir: Vec3, frequency: f32) -> f32 {
    let p = dir * frequency;
    noise.get([p.x as f64, p.y as f64, p.z as f64]) as f32
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The solid-surface heightfield: continents + masked ridged mountains +
/// fine detail. Built once per body, sampled by both mesh displacement and
/// texture bake.
pub(crate) struct TerrainField {
    continents: Fbm<Perlin>,
    mountains: RidgedMulti<Perlin>,
    detail: Fbm<Perlin>,
    cracks: RidgedMulti<Perlin>,
    continent_scale: f32,
    mountain: f32,
    detail_amp: f32,
    ocean_level: f32,
}

impl TerrainField {
    pub fn new(seed: u64, spec: &TerrainSpec) -> Self {
        let mut rng = SeededRng::new(seed ^ 0x7E44_A1_5EED);
        let s = |rng: &mut SeededRng| (rng.next_u64() & 0xFFFF_FFFF) as u32;
        Self {
            continents: Fbm::<Perlin>::new(s(&mut rng)).set_octaves(5),
            mountains: RidgedMulti::<Perlin>::new(s(&mut rng)).set_octaves(4),
            detail: Fbm::<Perlin>::new(s(&mut rng)).set_octaves(3),
            cracks: RidgedMulti::<Perlin>::new(s(&mut rng)).set_octaves(3),
            continent_scale: spec.continent_scale,
            mountain: spec.mountain,
            detail_amp: spec.detail,
            ocean_level: spec.ocean_level,
        }
    }

    /// Terrain height in roughly [-1, 1].
    pub fn height(&self, dir: Vec3) -> f32 {
        let base = at(&self.continents, dir, self.continent_scale);
        // Mountains only rise on land, and fade in past the coast so beaches
        // stay flat: a ridge erupting from the sea floor to sea level reads
        // as a wall, not a coastline.
        let land = smoothstep(self.ocean_level, self.ocean_level + 0.25, base);
        let ridged = at(&self.mountains, dir, self.continent_scale * 2.3);
        let fine = at(&self.detail, dir, self.continent_scale * 7.0);
        base + ridged.max(0.0) * self.mountain * land + fine * self.detail_amp
    }

    /// Lava-crack mask in [0, 1]: the sharp valley lines of ridged noise.
    pub fn crack_mask(&self, dir: Vec3) -> f32 {
        let r = at(&self.cracks, dir, self.continent_scale * 3.0);
        smoothstep(0.55, 0.95, r)
    }

    /// Water depth shading key: how far below sea level, 0 on land.
    pub fn depth(&self, height: f32) -> f32 {
        (self.ocean_level - height).max(0.0)
    }
}

/// The gas-giant colour field: latitude bands, domain-warped, with oval
/// storms stamped on. Returns sRGB.
pub(crate) struct GasField {
    warp: Fbm<Perlin>,
    swirl: Fbm<Perlin>,
    /// (unit direction, angular radius, colour shift)
    storms: Vec<(Vec3, f32, f32)>,
    palette: Vec<[f32; 3]>,
    bands: f32,
    turbulence: f32,
}

impl GasField {
    pub fn new(seed: u64, spec: &GasSpec) -> Self {
        let mut rng = SeededRng::new(seed ^ 0x6A5_61A47);
        let warp = Fbm::<Perlin>::new((rng.next_u64() & 0xFFFF_FFFF) as u32).set_octaves(4);
        let swirl = Fbm::<Perlin>::new((rng.next_u64() & 0xFFFF_FFFF) as u32).set_octaves(2);
        // An empty palette would divide by zero in `color`; a grey ball is
        // an obviously-wrong-but-running answer to an authoring mistake.
        let palette = if spec.palette.is_empty() {
            vec![[0.5, 0.5, 0.5]]
        } else {
            spec.palette.clone()
        };
        let mut storms = Vec::new();
        for _ in 0..spec.storms {
            // Storms live in the temperate bands, like the real ones.
            let lat = (rng.next_f32() * 2.0 - 1.0) * 0.7;
            let lon = rng.next_f32() * std::f32::consts::TAU;
            let r = (1.0f32 - lat * lat).max(0.0).sqrt();
            let dir = Vec3::new(r * lon.cos(), lat, r * lon.sin());
            let radius = 0.06 + rng.next_f32() * 0.10;
            let shift = if rng.next_f32() < 0.5 { -0.5 } else { 0.5 };
            storms.push((dir, radius, shift));
        }
        Self {
            warp,
            swirl,
            storms,
            palette,
            bands: spec.bands,
            turbulence: spec.turbulence,
        }
    }

    pub fn color(&self, dir: Vec3) -> [f32; 3] {
        // Bands are a function of latitude; the warp displaces WHERE a band
        // is sampled, which is what turns straight stripes into weather.
        // Stretching the warp lookup 3x along latitude makes the billows
        // wide and flat, the signature gas-giant look.
        let stretched = Vec3::new(dir.x, dir.y * 3.0, dir.z);
        let w = at(&self.warp, stretched, 1.6);
        let s = at(&self.swirl, dir, 4.0);
        let mut band_pos = dir.y * self.bands + w * self.turbulence + s * 0.15;

        // Storms locally bend the bands into an oval eye.
        for (storm_dir, radius, shift) in &self.storms {
            let d = dir.angle_between(*storm_dir);
            if d < *radius {
                let falloff = 1.0 - (d / radius);
                band_pos += shift * falloff * falloff * 2.0;
            }
        }

        // Cycle the palette smoothly, pole to pole and wrapping.
        let n = self.palette.len().max(1) as f32;
        let t = (band_pos * 0.5 + 100.0).fract() * n; // +100 keeps it positive
        let i0 = (t as usize) % self.palette.len();
        let i1 = (i0 + 1) % self.palette.len();
        let f = t.fract();
        // Sharpen the blend so bands have bodies with soft edges rather
        // than being one long gradient.
        let f = smoothstep(0.25, 0.75, f);
        let a = self.palette[i0];
        let b = self.palette[i1];
        [
            a[0] + (b[0] - a[0]) * f,
            a[1] + (b[1] - a[1]) * f,
            a[2] + (b[2] - a[2]) * f,
        ]
    }
}

/// Ring density along the radial axis, in [0, 1].
pub(crate) struct RingField {
    noise: Fbm<Perlin>,
    /// (centre t, half-width) of the Cassini-style gaps.
    gaps: Vec<(f32, f32)>,
}

impl RingField {
    pub fn new(seed: u64) -> Self {
        let mut rng = SeededRng::new(seed ^ 0x21C6_F1E1D);
        let noise = Fbm::<Perlin>::new((rng.next_u64() & 0xFFFF_FFFF) as u32).set_octaves(4);
        let gap_count = 1 + (rng.next_u64() % 3) as usize;
        let gaps = (0..gap_count)
            .map(|_| {
                (
                    0.2 + rng.next_f32() * 0.6,
                    0.01 + rng.next_f32() * 0.04,
                )
            })
            .collect();
        Self { noise, gaps }
    }

    pub fn density(&self, t: f32) -> f32 {
        // Banded clumps: 1-D noise sampled along the radius.
        let n = self.noise.get([t as f64 * 9.0, 0.37, 0.71]) as f32;
        let mut d = 0.45 + 0.55 * n.abs();
        // Fade both edges so the ring never ends on a hard line.
        d *= smoothstep(0.0, 0.12, t) * (1.0 - smoothstep(0.85, 1.0, t));
        for (centre, half_width) in &self.gaps {
            let dist = (t - centre).abs();
            d *= smoothstep(*half_width * 0.4, *half_width, dist);
        }
        d.clamp(0.0, 1.0)
    }
}
