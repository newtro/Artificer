//! Equirectangular texture baking: fields → PNG bytes.
//!
//! The scene API transports textures as encoded PNG (`AddTexture`), so the
//! bakers hand back exactly that. Directions are reconstructed per pixel the
//! same way `procmesh::uv_sphere` builds its vertices, so a baked map lands
//! on the mesh with no rotation or mirror surprises.

use crate::field::{GasField, RingField, TerrainField};
use crate::{GasSpec, RingSpec, TerrainSpec};
use glam::Vec3;
use std::f32::consts::{PI, TAU};

fn srgb8(c: [f32; 3]) -> [u8; 3] {
    [
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8,
    ]
}

fn encode_rgb(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    encode(width, height, pixels, png::ColorType::Rgb)
}

fn encode_rgba(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    encode(width, height, pixels, png::ColorType::Rgba)
}

fn encode(width: u32, height: u32, pixels: &[u8], color: png::ColorType) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .expect("in-memory PNG header cannot fail");
        writer
            .write_image_data(pixels)
            .expect("in-memory PNG data cannot fail");
    }
    out
}

/// Pixel centre (x, y) in a width×height equirect map → unit direction.
/// Matches `uv_sphere`: v=0 is the north pole, u wraps eastward.
fn equirect_dir(x: u32, y: u32, width: u32, height: u32) -> Vec3 {
    let u = (x as f32 + 0.5) / width as f32;
    let v = (y as f32 + 0.5) / height as f32;
    let theta = v * PI;
    let phi = u * TAU;
    let (sin_t, cos_t) = theta.sin_cos();
    let (sin_p, cos_p) = phi.sin_cos();
    Vec3::new(sin_t * cos_p, cos_t, sin_t * sin_p)
}

pub(crate) struct TerrainMaps {
    pub albedo: Vec<u8>,
    /// glTF layout: G = roughness, B = metallic (R unused).
    pub metallic_roughness: Vec<u8>,
    pub emissive: Option<Vec<u8>>,
}

pub(crate) fn bake_terrain(
    field: &TerrainField,
    spec: &TerrainSpec,
    width: u32,
) -> TerrainMaps {
    // A zero/tiny width would panic the PNG encoder; 8 is the smallest map
    // that still resembles a planet rather than an authoring accident.
    let width = width.max(8);
    let height = (width / 2).max(1);
    let mut albedo = Vec::with_capacity((width * height * 3) as usize);
    let mut mr = Vec::with_capacity((width * height * 3) as usize);
    let mut emissive = spec
        .glow_cracks
        .map(|_| Vec::with_capacity((width * height * 3) as usize));

    for y in 0..height {
        for x in 0..width {
            let dir = equirect_dir(x, y, width, height);
            let h = field.height(dir);
            let underwater = h < spec.ocean_level;

            let mut color = field_color(field, spec, dir, h);
            let mut rough = if underwater {
                spec.water_roughness
            } else {
                spec.land_roughness
            };

            // Ice caps: poleward latitudes whiten, biased so highlands
            // freeze before lowlands and the edge is noise-ragged (the
            // heightfield supplies the raggedness for free).
            let lat = dir.y.abs();
            if spec.ice_caps < 1.0 {
                let cap = smoothstep(spec.ice_caps, spec.ice_caps + 0.12, lat + h * 0.08);
                if cap > 0.0 {
                    color = mix3(color, [0.93, 0.95, 0.98], cap);
                    rough = rough + (0.35 - rough) * cap;
                }
            }

            if let (Some(glow), Some(out)) = (spec.glow_cracks, emissive.as_mut()) {
                let crack = field.crack_mask(dir) * if underwater { 0.0 } else { 1.0 };
                // Cracks darken the rock they cut through, then glow.
                color = mix3(color, [0.05, 0.03, 0.03], crack * 0.7);
                out.extend_from_slice(&srgb8([
                    glow[0] * crack,
                    glow[1] * crack,
                    glow[2] * crack,
                ]));
            }

            albedo.extend_from_slice(&srgb8(color));
            mr.extend_from_slice(&[0, (rough.clamp(0.0, 1.0) * 255.0) as u8, 0]);
        }
    }

    TerrainMaps {
        albedo: encode_rgb(width, height, &albedo),
        metallic_roughness: encode_rgb(width, height, &mr),
        emissive: emissive.map(|px| encode_rgb(width, height, &px)),
    }
}

fn field_color(field: &TerrainField, spec: &TerrainSpec, _dir: Vec3, h: f32) -> [f32; 3] {
    let depth = field.depth(h);
    if depth > 0.0 {
        // Under water the palette's sub-sea stops apply, keyed by depth so
        // shelves read lighter than trenches.
        spec.palette.sample(spec.ocean_level - depth)
    } else {
        spec.palette.sample(h)
    }
}

pub(crate) fn bake_gas(seed: u64, spec: &GasSpec, width: u32) -> Vec<u8> {
    let width = width.max(8);
    let height = (width / 2).max(1);
    let field = GasField::new(seed, spec);
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let dir = equirect_dir(x, y, width, height);
            let mut c = field.color(dir);
            // Limb-to-pole shading is the lighting's job; what the texture
            // adds is a slight polar dimming so caps read as caps.
            let polar = smoothstep(0.75, 1.0, dir.y.abs());
            c = mix3(c, [c[0] * 0.75, c[1] * 0.75, c[2] * 0.8], polar);
            pixels.extend_from_slice(&srgb8(c));
        }
    }
    encode_rgb(width, height, &pixels)
}

/// Radial strip, 256×4 RGBA: UV.x is the radius (annulus mapping), the four
/// rows are identical (bilinear safety margin).
pub(crate) fn bake_ring(seed: u64, spec: &RingSpec) -> Vec<u8> {
    const W: u32 = 256;
    const H: u32 = 4;
    let field = RingField::new(seed);
    let mut row = Vec::with_capacity((W * 4) as usize);
    for x in 0..W {
        let t = (x as f32 + 0.5) / W as f32;
        let d = field.density(t);
        // Denser regions are brighter AND more opaque; a slight tint drift
        // across the radius keeps wide rings from looking like one band.
        let drift = 0.85 + 0.15 * (t * 9.0).sin();
        let c = [
            spec.tint[0] * (0.6 + 0.4 * d) * drift,
            spec.tint[1] * (0.6 + 0.4 * d) * drift,
            spec.tint[2] * (0.6 + 0.4 * d),
        ];
        let [r, g, b] = srgb8(c);
        row.extend_from_slice(&[r, g, b, (d * spec.opacity * 255.0) as u8]);
    }
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for _ in 0..H {
        pixels.extend_from_slice(&row);
    }
    encode_rgba(W, H, &pixels)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
