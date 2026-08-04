//! Seed-deterministic stellar-object generation (ADR-0004).
//!
//! Turns a small spec into everything a scene needs for a celestial body:
//! a displaced sphere mesh, baked equirectangular PNG maps (albedo,
//! roughness, emissive), an optional scattering-atmosphere description, and
//! an optional ring (annulus mesh + radial strip texture). Everything is a
//! pure function of the spec — the same seed always produces the same body,
//! on any machine, which is what lets a game store a `u64` in its sector
//! data instead of megabytes of texture.
//!
//! The division of labour (ADR-0004): this crate owns HOW a spec becomes
//! meshes and pixels; games own WHICH bodies exist where. [`presets`] holds
//! ready-made archetypes (earthlike, lava, gas giant, …) so a game's planet
//! table can be one enum + seed per body, but every knob a preset sets is
//! public — a game with opinions can build [`PlanetSpec`]s directly.
//!
//! ```no_run
//! use artificer_procgen::{presets, generate_planet, spawn_planet};
//! use artificer_scene::{SceneGraph, TransformDesc};
//! use glam::Vec3;
//!
//! let mut scene = SceneGraph::new();
//! let spec = presets::planet_spec(presets::Archetype::EarthLike, 42, 900.0);
//! let planet = generate_planet(&spec);
//! spawn_planet(
//!     &mut scene,
//!     &planet,
//!     TransformDesc::from_translation(Vec3::new(4_000.0, 0.0, -8_000.0)),
//!     Vec3::new(5_600.0, 900.0, 0.0), // the sector's sun
//! );
//! ```

mod bake;
mod field;
pub mod presets;
mod shape;

pub use shape::{asteroid_mesh, AsteroidSpec};

use artificer_scene::{
    AtmosphereDesc, MaterialDesc, MeshData, NodeId, SceneGraph, TextureColorSpace,
    TextureSampling, TransformDesc,
};
use glam::Vec3;

/// What kind of surface a body wears.
#[derive(Debug, Clone)]
pub enum Surface {
    /// Solid ground: heightfield-displaced mesh + height/latitude colouring.
    Terrain(TerrainSpec),
    /// Banded fluid ball: flat sphere, all the character is in the albedo.
    GasGiant(GasSpec),
}

/// A colour ramp keyed by terrain height in [-1, 1].
///
/// Stops must be sorted by key; lookups clamp at the ends. Colours are sRGB
/// like every other authored colour in the scene API.
#[derive(Debug, Clone)]
pub struct ColorRamp {
    pub stops: Vec<(f32, [f32; 3])>,
}

impl ColorRamp {
    pub fn sample(&self, t: f32) -> [f32; 3] {
        match self.stops.iter().position(|(k, _)| *k > t) {
            Some(0) => self.stops[0].1,
            None => self.stops.last().map(|(_, c)| *c).unwrap_or([1.0, 0.0, 1.0]),
            Some(i) => {
                let (k0, c0) = self.stops[i - 1];
                let (k1, c1) = self.stops[i];
                let f = ((t - k0) / (k1 - k0)).clamp(0.0, 1.0);
                [
                    c0[0] + (c1[0] - c0[0]) * f,
                    c0[1] + (c1[1] - c0[1]) * f,
                    c0[2] + (c1[2] - c0[2]) * f,
                ]
            }
        }
    }
}

/// Solid-surface parameters. Heights are in a normalized [-1, 1] space;
/// `ocean_level` splits water from land inside it.
#[derive(Debug, Clone)]
pub struct TerrainSpec {
    /// Colour by height. Water stops below `ocean_level`, land above.
    pub palette: ColorRamp,
    /// Height where land begins. `-1.0` = no ocean at all.
    pub ocean_level: f32,
    /// Continent noise frequency; ~0.8 = few huge landmasses, ~2.5 = many.
    pub continent_scale: f32,
    /// Ridged-mountain strength added on land.
    pub mountain: f32,
    /// High-frequency detail amplitude.
    pub detail: f32,
    /// |latitude| (0..1, poleward) where ice caps begin; 1.0 = never.
    pub ice_caps: f32,
    /// Radial displacement amplitude as a fraction of radius. This is the
    /// silhouette knob: 0 = billiard ball, ~0.05 = visible mountains.
    pub displacement: f32,
    /// Emissive crack colour (lava worlds). The cracks follow ridged-noise
    /// valleys; colour is multiplied by [`GeneratedPlanet::emissive`].
    pub glow_cracks: Option<[f32; 3]>,
    /// Water surface roughness (oceans read as water because they are
    /// glossier than land; see the baked metallic-roughness map).
    pub water_roughness: f32,
    /// Land roughness.
    pub land_roughness: f32,
}

/// Gas-giant parameters: latitude bands warped by turbulence.
#[derive(Debug, Clone)]
pub struct GasSpec {
    /// Band colours, cycled pole to pole. Adjacent bands blend.
    pub palette: Vec<[f32; 3]>,
    /// How many band cycles from pole to pole.
    pub bands: f32,
    /// Domain-warp strength — how much the bands billow and swirl.
    pub turbulence: f32,
    /// Count of large oval storms stamped into the bands.
    pub storms: u32,
}

/// Atmosphere in artist units; converted to [`AtmosphereDesc`] scattering
/// coefficients against the planet's radius at generation time.
#[derive(Debug, Clone, Copy)]
pub struct AtmosphereSpec {
    /// Scatter colour (what the halo looks like). The terminator glows in
    /// roughly the complement, exactly like real sunsets.
    pub tint: [f32; 3],
    /// Shell height as a fraction of planet radius (~0.06 thin, ~0.2 soupy).
    pub thickness: f32,
    /// Overall scatter strength; 1.0 is a reasonable earthlike default.
    pub density: f32,
}

/// Ring in planet-radius multiples, so one spec fits any size of planet.
#[derive(Debug, Clone, Copy)]
pub struct RingSpec {
    pub inner: f32,
    pub outer: f32,
    pub tint: [f32; 3],
    pub opacity: f32,
}

#[derive(Debug, Clone)]
pub struct PlanetSpec {
    pub seed: u64,
    pub radius: f32,
    pub surface: Surface,
    pub atmosphere: Option<AtmosphereSpec>,
    pub ring: Option<RingSpec>,
    /// Baked equirect texture width (height is width/2).
    pub texture_width: u32,
    /// Sphere tessellation (longitudes, latitudes).
    pub mesh_resolution: (u32, u32),
}

/// Everything [`generate_planet`] bakes. Feed it to [`spawn_planet`], or
/// pick it apart if a game composes scenes its own way.
#[derive(Debug, Clone)]
pub struct GeneratedPlanet {
    pub radius: f32,
    pub mesh: MeshData,
    pub albedo_png: Vec<u8>,
    /// glTF-convention metallic-roughness map (G = roughness, B = metallic);
    /// present for terrain worlds so oceans gloss and land does not.
    pub metallic_roughness_png: Option<Vec<u8>>,
    pub emissive_png: Option<Vec<u8>>,
    /// HDR multiplier for the emissive map (drives bloom on lava cracks).
    pub emissive: [f32; 3],
    pub roughness: f32,
    /// Ready except for the sun's position, which only the spawn site knows.
    pub atmosphere: Option<AtmosphereDesc>,
    pub ring: Option<GeneratedRing>,
}

#[derive(Debug, Clone)]
pub struct GeneratedRing {
    pub mesh: MeshData,
    pub albedo_png: Vec<u8>,
}

/// Scene nodes created by [`spawn_planet`], for later transforms/teardown.
#[derive(Debug, Clone, Copy)]
pub struct PlanetNodes {
    /// Group at the planet's position — despawn this to remove everything.
    pub root: NodeId,
    pub surface: NodeId,
    pub atmosphere: Option<NodeId>,
    pub ring: Option<NodeId>,
}

/// Generate a full body from a spec. Pure CPU; cost scales with
/// `texture_width`² (a 512-wide bake is tens of milliseconds).
pub fn generate_planet(spec: &PlanetSpec) -> GeneratedPlanet {
    let (lon, lat) = spec.mesh_resolution;
    match &spec.surface {
        Surface::Terrain(terrain) => {
            let field = field::TerrainField::new(spec.seed, terrain);
            let mesh = shape::displaced_sphere(spec.radius, lon, lat, |dir| {
                terrain.displacement * field.height(dir)
            });
            let maps = bake::bake_terrain(&field, terrain, spec.texture_width);
            GeneratedPlanet {
                radius: spec.radius,
                mesh,
                albedo_png: maps.albedo,
                metallic_roughness_png: Some(maps.metallic_roughness),
                emissive_png: maps.emissive,
                emissive: if terrain.glow_cracks.is_some() {
                    // >1 so cracks bloom on the HDR camera.
                    [2.5, 2.5, 2.5]
                } else {
                    [0.0, 0.0, 0.0]
                },
                roughness: 1.0,
                atmosphere: spec
                    .atmosphere
                    .map(|a| atmosphere_desc(&a, spec.radius)),
                ring: generate_ring(spec),
            }
        }
        Surface::GasGiant(gas) => {
            let mesh = shape::displaced_sphere(spec.radius, lon, lat, |_| 0.0);
            let albedo = bake::bake_gas(spec.seed, gas, spec.texture_width);
            GeneratedPlanet {
                radius: spec.radius,
                mesh,
                albedo_png: albedo,
                metallic_roughness_png: None,
                emissive_png: None,
                emissive: [0.0, 0.0, 0.0],
                roughness: 0.95,
                atmosphere: spec
                    .atmosphere
                    .map(|a| atmosphere_desc(&a, spec.radius)),
                ring: generate_ring(spec),
            }
        }
    }
}

fn generate_ring(spec: &PlanetSpec) -> Option<GeneratedRing> {
    let ring = spec.ring.as_ref()?;
    let inner = ring.inner * spec.radius;
    let outer = ring.outer * spec.radius;
    Some(GeneratedRing {
        mesh: artificer_assets::procmesh::annulus(inner, outer, 96),
        albedo_png: bake::bake_ring(spec.seed, ring),
    })
}

/// Convert artist units to shader scattering coefficients.
///
/// Coefficients scale with 1/radius so the OPTICAL depth through the shell
/// is radius-independent: a moon and a giant with the same spec look equally
/// dense, they just are their own sizes.
fn atmosphere_desc(spec: &AtmosphereSpec, radius: f32) -> AtmosphereDesc {
    let thickness = spec.thickness.max(0.01) * radius;
    let [r, g, b] = spec.tint;
    let peak = r.max(g).max(b).max(1e-3);
    let h_r = thickness * 0.25;
    let h_m = thickness * 0.12;
    // Normalize against the TANGENT path — the limb graze, the longest ray
    // through an exponential shell, whose dense length is √(2πRH). Setting
    // the peak channel's tangent optical depth to 1.5·density puts the
    // brightest tau≈1 band ON the limb for every radius and thickness.
    // (The first cut scaled by 1/radius alone; the optical-depth heat map
    // showed tau>>1 at the limb, which pushes the bright band outward and
    // renders as a glow ring floating DETACHED from the planet.)
    let tangent_r = (2.0 * std::f32::consts::PI * radius * h_r).sqrt();
    let tangent_m = (2.0 * std::f32::consts::PI * radius * h_m).sqrt();
    let k = 1.5 * spec.density / (peak * tangent_r);
    AtmosphereDesc {
        planet_radius: radius,
        atmosphere_radius: radius + thickness,
        rayleigh: [r * k, g * k, b * k],
        rayleigh_scale_height: h_r,
        mie: 0.5 * spec.density / tangent_m,
        mie_scale_height: h_m,
        mie_g: 0.76,
        sun_position: [0.0, 0.0, 0.0], // filled by the spawn site
        sun_intensity: 22.0,
    }
}

/// Put a generated planet into a scene: root group at `transform`, surface
/// mesh, atmosphere shell, ring — textures registered, materials wired.
pub fn spawn_planet(
    scene: &mut SceneGraph,
    planet: &GeneratedPlanet,
    transform: TransformDesc,
    sun_position: Vec3,
) -> PlanetNodes {
    let root = scene.spawn_group(transform);

    let albedo = scene.add_texture(planet.albedo_png.clone(), TextureSampling::Linear);
    let metallic_roughness = planet.metallic_roughness_png.as_ref().map(|png| {
        scene.add_texture_in(png.clone(), TextureSampling::Linear, TextureColorSpace::Linear)
    });
    let emissive_texture = planet
        .emissive_png
        .as_ref()
        .map(|png| scene.add_texture(png.clone(), TextureSampling::Linear));

    let mesh = scene.add_mesh(planet.mesh.clone());
    let surface = scene.spawn_mesh_child(
        root,
        mesh,
        MaterialDesc {
            base_color: [1.0, 1.0, 1.0, 1.0],
            base_color_texture: Some(albedo),
            metallic_roughness_texture: metallic_roughness,
            emissive_texture,
            emissive: planet.emissive,
            // Factors multiply the maps (glTF convention), so with a map
            // bound they must be 1.0 or the map is scaled down twice.
            metallic: if planet.metallic_roughness_png.is_some() {
                1.0
            } else {
                0.0
            },
            roughness: if planet.metallic_roughness_png.is_some() {
                1.0
            } else {
                planet.roughness
            },
            sampling: TextureSampling::Linear,
            ..Default::default()
        },
        TransformDesc::IDENTITY,
    );

    let atmosphere = planet.atmosphere.map(|mut desc| {
        desc.sun_position = sun_position.to_array();
        let shell = scene.add_mesh(artificer_assets::procmesh::uv_sphere(
            desc.atmosphere_radius,
            48,
            32,
        ));
        scene.spawn_atmosphere_child(root, shell, desc, TransformDesc::IDENTITY)
    });

    let ring = planet.ring.as_ref().map(|ring| {
        let tex = scene.add_texture(ring.albedo_png.clone(), TextureSampling::Linear);
        let mesh = scene.add_mesh(ring.mesh.clone());
        scene.spawn_mesh_child(
            root,
            mesh,
            MaterialDesc {
                base_color: [1.0, 1.0, 1.0, 1.0],
                base_color_texture: Some(tex),
                roughness: 0.9,
                alpha: artificer_scene::AlphaModeDesc::Blend,
                double_sided: true,
                // Ring shadows on the planet would need either shadow-map
                // wiring or the analytic trick from the research doc; both
                // are follow-ups. Casting today produces peter-panned bands.
                casts_shadows: false,
                sampling: TextureSampling::Linear,
                ..Default::default()
            },
            TransformDesc::IDENTITY,
        )
    });

    PlanetNodes {
        root,
        surface,
        atmosphere,
        ring,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quick_spec(archetype: presets::Archetype) -> PlanetSpec {
        let mut spec = presets::planet_spec(archetype, 7, 100.0);
        // Keep tests fast: tiny textures and meshes exercise every code path.
        spec.texture_width = 32;
        spec.mesh_resolution = (12, 8);
        spec
    }

    #[test]
    fn every_archetype_generates_a_valid_body() {
        for archetype in presets::Archetype::ALL {
            let spec = quick_spec(archetype);
            let planet = generate_planet(&spec);
            planet.mesh.validate().expect("mesh must be valid");
            assert!(!planet.albedo_png.is_empty());
        }
    }

    #[test]
    fn generation_is_deterministic() {
        let a = generate_planet(&quick_spec(presets::Archetype::EarthLike));
        let b = generate_planet(&quick_spec(presets::Archetype::EarthLike));
        assert_eq!(a.albedo_png, b.albedo_png);
        assert_eq!(a.mesh.positions, b.mesh.positions);
    }

    #[test]
    fn different_seeds_differ() {
        let mut s1 = quick_spec(presets::Archetype::EarthLike);
        let mut s2 = quick_spec(presets::Archetype::EarthLike);
        s1.seed = 1;
        s2.seed = 2;
        assert_ne!(
            generate_planet(&s1).albedo_png,
            generate_planet(&s2).albedo_png
        );
    }

    #[test]
    fn lava_worlds_glow_and_earthlikes_do_not() {
        let lava = generate_planet(&quick_spec(presets::Archetype::Lava));
        assert!(lava.emissive_png.is_some());
        assert!(lava.emissive[0] > 1.0);
        let earth = generate_planet(&quick_spec(presets::Archetype::EarthLike));
        assert!(earth.emissive_png.is_none());
    }

    #[test]
    fn spawn_wires_atmosphere_and_ring_nodes() {
        let mut spec = quick_spec(presets::Archetype::EarthLike);
        spec.ring = Some(presets::ring_spec(3));
        let planet = generate_planet(&spec);
        let mut scene = SceneGraph::new();
        let nodes = spawn_planet(
            &mut scene,
            &planet,
            TransformDesc::IDENTITY,
            Vec3::new(0.0, 0.0, 1000.0),
        );
        assert!(nodes.atmosphere.is_some(), "earthlike has an atmosphere");
        assert!(nodes.ring.is_some());
        assert!(scene.contains(nodes.root));
        assert!(scene.contains(nodes.surface));
    }

    #[test]
    fn atmosphere_tangent_depth_is_size_and_thickness_independent() {
        // The look-critical invariant: the limb-graze optical depth (peak
        // channel) is 1.5 × density regardless of planet size or shell
        // thickness, so every world's halo hugs its limb the same way.
        let tangent_tau = |desc: &AtmosphereDesc| {
            let h = desc.rayleigh_scale_height;
            let path = (2.0 * std::f32::consts::PI * desc.planet_radius * h).sqrt();
            desc.rayleigh[2] * path
        };
        let spec = AtmosphereSpec {
            tint: [0.3, 0.5, 1.0],
            thickness: 0.1,
            density: 1.0,
        };
        let thick = AtmosphereSpec {
            thickness: 0.2,
            ..spec
        };
        let a = tangent_tau(&atmosphere_desc(&spec, 100.0));
        let b = tangent_tau(&atmosphere_desc(&spec, 1000.0));
        let c = tangent_tau(&atmosphere_desc(&thick, 400.0));
        assert!((a - 1.5).abs() < 1e-3, "tau {a}");
        assert!((b - 1.5).abs() < 1e-3, "tau {b}");
        assert!((c - 1.5).abs() < 1e-3, "tau {c}");
    }
}
