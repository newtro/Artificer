//! Scattering-atmosphere shell material (`NodeKind::Atmosphere`).
//!
//! One additive shell mesh per planet, running a single-scattering raymarch
//! (Rayleigh + Mie) in the fragment shader. The mesh only provides screen
//! coverage; all geometry the lighting needs is analytic, driven by the
//! uniforms below plus the mesh's own world matrix (which is where the
//! planet centre comes from — so a moving or reparented planet never leaves
//! a stale centre behind in a uniform).
//!
//! Engine-owned because atmospheres are as generic as lights (ADR-0004): a
//! game describes one with [`artificer_scene::AtmosphereDesc`] and never
//! touches Bevy.

// The ShaderType derive emits per-field layout-check functions nothing on
// the CPU side calls; neither struct- nor field-level allows reach that
// generated code, so the allow is scoped to this (single-purpose) module.
#![allow(dead_code)]

use artificer_scene::AtmosphereDesc;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderRef, ShaderType};

pub const ATMOSPHERE_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::weak_handle!("a2717f1c-e004-4a70-9d2e-a7a05ffe0001");

/// GPU layout of an atmosphere. Packed into four vec4s; see the WGSL twin in
/// `shaders/atmosphere.wgsl`, which must match field-for-field.
///
/// The field-level `allow(dead_code)` quiets the ShaderType derive, which
/// emits per-field layout-check functions nothing calls on the CPU side;
/// every field is genuinely written by `from_desc`.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct AtmosphereUniform {
    /// xyz = sun position (world), w = scattered-light intensity.
    #[allow(dead_code)]
    pub sun: Vec4,
    /// x = planet radius, y = atmosphere radius (world units).
    #[allow(dead_code)]
    pub radii: Vec4,
    /// xyz = Rayleigh scattering coefficients, w = Rayleigh scale height.
    #[allow(dead_code)]
    pub rayleigh: Vec4,
    /// x = Mie coefficient, y = Mie scale height, z = Mie g.
    #[allow(dead_code)]
    pub mie: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct AtmosphereMaterial {
    #[uniform(0)]
    pub data: AtmosphereUniform,
}

impl AtmosphereMaterial {
    pub fn from_desc(desc: &AtmosphereDesc) -> Self {
        let s = desc.sun_position;
        Self {
            data: AtmosphereUniform {
                sun: Vec4::new(s[0], s[1], s[2], desc.sun_intensity),
                radii: Vec4::new(desc.planet_radius, desc.atmosphere_radius, 0.0, 0.0),
                rayleigh: Vec4::new(
                    desc.rayleigh[0],
                    desc.rayleigh[1],
                    desc.rayleigh[2],
                    desc.rayleigh_scale_height,
                ),
                mie: Vec4::new(desc.mie, desc.mie_scale_height, desc.mie_g, 0.0),
            },
        }
    }
}

impl Material for AtmosphereMaterial {
    fn fragment_shader() -> ShaderRef {
        ATMOSPHERE_SHADER_HANDLE.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        // Additive: scattered light adds to whatever is behind the shell
        // (the planet's own lit surface, or space). Renders in the
        // transparent pass — depth-tested against opaques, no depth write —
        // and additive blending is order-independent, so overlapping shells
        // (a moon inside a giant's atmosphere) need no sorting care.
        AlphaMode::Add
    }
}

/// Registers the shader + material pipeline. Added by `run_app` for every
/// game, so `NodeKind::Atmosphere` is always renderable.
pub(crate) struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::load_internal_asset!(
            app,
            ATMOSPHERE_SHADER_HANDLE,
            "shaders/atmosphere.wgsl",
            Shader::from_wgsl
        );
        app.add_plugins(MaterialPlugin::<AtmosphereMaterial> {
            // A shell of pure added light: it has no depth to prepass and
            // must never darken a shadow map.
            prepass_enabled: false,
            shadows_enabled: false,
            ..Default::default()
        });
    }
}
