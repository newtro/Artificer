//! Ready-made planet archetypes.
//!
//! Each archetype is a recipe: palette + terrain/gas parameters + default
//! atmosphere, jittered per seed so two EarthLikes are siblings, not twins.
//! Games that want full control build [`PlanetSpec`]s by hand; these exist
//! so a game's planet table can be `(archetype, seed, radius)` per row.

use crate::{
    AtmosphereSpec, ColorRamp, GasSpec, PlanetSpec, RingSpec, Surface, TerrainSpec,
};
use artificer_core::SeededRng;

/// The stock planet families. `ALL` exists for demos and tests that want to
/// walk every one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Archetype {
    /// Blue-green continents, oceans, ice caps, thin blue atmosphere.
    EarthLike,
    /// Water world: islands in a deep sea, hazy blue air.
    Ocean,
    /// Dry rust-and-sand world, thin dusty atmosphere.
    Desert,
    /// Frozen surface, pale sky.
    Ice,
    /// Dark volcanic crust, glowing crack network, smoggy red air.
    Lava,
    /// Sickly green-yellow ball under a thick murky atmosphere.
    Toxic,
    /// Airless cratered grey rock (moons, dead worlds).
    Barren,
    /// Warm-banded giant, Jupiter family.
    GasGiant,
    /// Cold blue giant, Neptune family.
    IceGiant,
}

impl Archetype {
    pub const ALL: [Archetype; 9] = [
        Archetype::EarthLike,
        Archetype::Ocean,
        Archetype::Desert,
        Archetype::Ice,
        Archetype::Lava,
        Archetype::Toxic,
        Archetype::Barren,
        Archetype::GasGiant,
        Archetype::IceGiant,
    ];

    /// Giants are rendered as smooth balls; solid worlds get displacement.
    pub fn is_gas(self) -> bool {
        matches!(self, Archetype::GasGiant | Archetype::IceGiant)
    }
}

fn ramp(stops: &[(f32, [f32; 3])]) -> ColorRamp {
    ColorRamp {
        stops: stops.to_vec(),
    }
}

/// Build the spec for an archetype. Deterministic in `(archetype, seed)`;
/// `radius` is world units and does not affect the surface pattern.
pub fn planet_spec(archetype: Archetype, seed: u64, radius: f32) -> PlanetSpec {
    let mut rng = SeededRng::new(seed ^ 0x9_1A4E7_5EED);
    // Jitter shared by all solid worlds: where the continents sit and how
    // rugged they are is per-planet, not per-family.
    let continent_scale = 1.0 + rng.next_f32() * 1.2;
    let mountain = 0.35 + rng.next_f32() * 0.4;

    let surface = match archetype {
        Archetype::EarthLike => Surface::Terrain(TerrainSpec {
            palette: ramp(&[
                (-1.0, [0.01, 0.05, 0.18]),
                (-0.25, [0.02, 0.12, 0.34]),
                (-0.02, [0.10, 0.35, 0.48]),
                (0.0, [0.76, 0.70, 0.50]),
                (0.06, [0.22, 0.44, 0.18]),
                (0.35, [0.14, 0.30, 0.13]),
                (0.6, [0.42, 0.38, 0.32]),
                (0.85, [0.95, 0.95, 0.97]),
            ]),
            ocean_level: -0.02 + rng.next_f32() * 0.08,
            continent_scale,
            mountain,
            detail: 0.06,
            ice_caps: 0.78 - rng.next_f32() * 0.08,
            displacement: 0.035,
            glow_cracks: None,
            water_roughness: 0.15,
            land_roughness: 0.85,
        }),
        Archetype::Ocean => Surface::Terrain(TerrainSpec {
            palette: ramp(&[
                (-1.0, [0.01, 0.04, 0.16]),
                (-0.2, [0.02, 0.10, 0.30]),
                (0.08, [0.05, 0.22, 0.42]),
                (0.12, [0.72, 0.68, 0.50]),
                (0.2, [0.20, 0.40, 0.20]),
                (0.6, [0.30, 0.34, 0.28]),
            ]),
            ocean_level: 0.1,
            continent_scale: continent_scale * 1.3,
            mountain: mountain * 0.6,
            detail: 0.05,
            ice_caps: 0.85,
            displacement: 0.02,
            glow_cracks: None,
            water_roughness: 0.12,
            land_roughness: 0.8,
        }),
        Archetype::Desert => Surface::Terrain(TerrainSpec {
            palette: ramp(&[
                (-1.0, [0.30, 0.16, 0.08]),
                (-0.3, [0.48, 0.26, 0.12]),
                (0.0, [0.66, 0.42, 0.20]),
                (0.3, [0.78, 0.58, 0.32]),
                (0.6, [0.55, 0.36, 0.22]),
                (0.9, [0.42, 0.28, 0.20]),
            ]),
            ocean_level: -1.0,
            continent_scale,
            mountain: mountain * 1.2,
            detail: 0.08,
            ice_caps: 0.92,
            displacement: 0.045,
            glow_cracks: None,
            water_roughness: 0.6,
            land_roughness: 0.9,
        }),
        Archetype::Ice => Surface::Terrain(TerrainSpec {
            palette: ramp(&[
                (-1.0, [0.35, 0.48, 0.62]),
                (-0.2, [0.55, 0.68, 0.80]),
                (0.1, [0.78, 0.86, 0.93]),
                (0.5, [0.90, 0.94, 0.98]),
                (0.9, [0.70, 0.78, 0.88]),
            ]),
            ocean_level: -0.35,
            continent_scale,
            mountain: mountain * 0.9,
            detail: 0.07,
            ice_caps: 1.0, // the whole thing is a cap already
            displacement: 0.03,
            glow_cracks: None,
            water_roughness: 0.2,
            land_roughness: 0.45,
        }),
        Archetype::Lava => Surface::Terrain(TerrainSpec {
            palette: ramp(&[
                (-1.0, [0.06, 0.03, 0.03]),
                (-0.2, [0.12, 0.06, 0.05]),
                (0.2, [0.16, 0.09, 0.07]),
                (0.6, [0.22, 0.13, 0.10]),
                (0.9, [0.13, 0.08, 0.07]),
            ]),
            ocean_level: -1.0,
            continent_scale: continent_scale * 1.1,
            mountain: mountain * 1.3,
            detail: 0.09,
            ice_caps: 1.0,
            displacement: 0.04,
            glow_cracks: Some([1.0, 0.28, 0.05]),
            water_roughness: 0.5,
            land_roughness: 0.92,
        }),
        Archetype::Toxic => Surface::Terrain(TerrainSpec {
            palette: ramp(&[
                (-1.0, [0.10, 0.14, 0.05]),
                (-0.2, [0.22, 0.28, 0.08]),
                (0.1, [0.38, 0.42, 0.12]),
                (0.5, [0.52, 0.50, 0.18]),
                (0.9, [0.35, 0.38, 0.14]),
            ]),
            ocean_level: -0.3,
            continent_scale,
            mountain: mountain * 0.8,
            detail: 0.06,
            ice_caps: 1.0,
            displacement: 0.03,
            glow_cracks: None,
            water_roughness: 0.3,
            land_roughness: 0.75,
        }),
        Archetype::Barren => Surface::Terrain(TerrainSpec {
            palette: ramp(&[
                (-1.0, [0.18, 0.17, 0.16]),
                (-0.3, [0.30, 0.29, 0.28]),
                (0.1, [0.42, 0.41, 0.39]),
                (0.5, [0.52, 0.50, 0.48]),
                (0.9, [0.36, 0.35, 0.34]),
            ]),
            ocean_level: -1.0,
            continent_scale: continent_scale * 1.4,
            mountain: mountain * 1.1,
            detail: 0.12,
            ice_caps: 1.0,
            displacement: 0.05,
            glow_cracks: None,
            water_roughness: 0.8,
            land_roughness: 0.95,
        }),
        Archetype::GasGiant => Surface::GasGiant(GasSpec {
            palette: vec![
                [0.76, 0.62, 0.44],
                [0.88, 0.78, 0.60],
                [0.62, 0.44, 0.30],
                [0.82, 0.68, 0.50],
                [0.55, 0.40, 0.32],
                [0.90, 0.84, 0.70],
            ],
            bands: 5.0 + rng.next_f32() * 4.0,
            turbulence: 0.5 + rng.next_f32() * 0.5,
            storms: 1 + (rng.next_u64() % 3) as u32,
        }),
        Archetype::IceGiant => Surface::GasGiant(GasSpec {
            palette: vec![
                [0.24, 0.42, 0.75],
                [0.34, 0.56, 0.85],
                [0.20, 0.34, 0.66],
                [0.44, 0.66, 0.90],
            ],
            bands: 3.0 + rng.next_f32() * 3.0,
            turbulence: 0.3 + rng.next_f32() * 0.35,
            storms: (rng.next_u64() % 2) as u32,
        }),
    };

    let atmosphere = match archetype {
        Archetype::EarthLike | Archetype::Ocean => Some(AtmosphereSpec {
            tint: [0.25, 0.5, 1.0],
            thickness: 0.09,
            density: 1.0,
        }),
        Archetype::Desert => Some(AtmosphereSpec {
            tint: [0.9, 0.55, 0.25],
            thickness: 0.06,
            density: 0.6,
        }),
        Archetype::Ice => Some(AtmosphereSpec {
            tint: [0.55, 0.7, 0.95],
            thickness: 0.05,
            density: 0.45,
        }),
        Archetype::Lava => Some(AtmosphereSpec {
            tint: [1.0, 0.32, 0.10],
            thickness: 0.07,
            density: 0.7,
        }),
        Archetype::Toxic => Some(AtmosphereSpec {
            tint: [0.55, 0.85, 0.25],
            thickness: 0.14,
            // 1.0, not more: density is calibrated so 1.0 puts the brightest
            // band on the limb; pushing past it detaches the glow into a
            // floating ring (see atmosphere_desc). Toxic reads "soupy" from
            // its thickness, not from extra density.
            density: 1.0,
        }),
        Archetype::Barren => None,
        Archetype::GasGiant => Some(AtmosphereSpec {
            tint: [0.85, 0.65, 0.40],
            thickness: 0.05,
            density: 0.5,
        }),
        Archetype::IceGiant => Some(AtmosphereSpec {
            tint: [0.35, 0.55, 1.0],
            thickness: 0.06,
            density: 0.6,
        }),
    };

    PlanetSpec {
        seed,
        radius,
        surface,
        atmosphere,
        ring: None, // opt in via `ring_spec`; rings are rarer than air
        texture_width: 512,
        mesh_resolution: if archetype.is_gas() {
            (64, 48)
        } else {
            (96, 64)
        },
    }
}

/// A ring system jittered per seed: span, tint, and translucency vary; the
/// gap layout comes from the seed inside the baker.
pub fn ring_spec(seed: u64) -> RingSpec {
    let mut rng = SeededRng::new(seed ^ 0x21C6_0002);
    let inner = 1.45 + rng.next_f32() * 0.35;
    RingSpec {
        inner,
        outer: inner + 0.7 + rng.next_f32() * 0.8,
        tint: [
            0.65 + rng.next_f32() * 0.25,
            0.58 + rng.next_f32() * 0.22,
            0.48 + rng.next_f32() * 0.22,
        ],
        opacity: 0.75 + rng.next_f32() * 0.25,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_are_deterministic_per_seed() {
        let a = planet_spec(Archetype::GasGiant, 5, 100.0);
        let b = planet_spec(Archetype::GasGiant, 5, 100.0);
        match (&a.surface, &b.surface) {
            (Surface::GasGiant(x), Surface::GasGiant(y)) => {
                assert_eq!(x.bands, y.bands);
                assert_eq!(x.storms, y.storms);
            }
            _ => panic!("gas giant spec expected"),
        }
    }

    #[test]
    fn barren_worlds_have_no_air() {
        assert!(planet_spec(Archetype::Barren, 1, 50.0).atmosphere.is_none());
        assert!(planet_spec(Archetype::EarthLike, 1, 50.0)
            .atmosphere
            .is_some());
    }

    #[test]
    fn ring_specs_stay_ordered() {
        for seed in 0..32 {
            let r = ring_spec(seed);
            assert!(r.inner > 1.0);
            assert!(r.outer > r.inner);
        }
    }
}
