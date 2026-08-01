//! The panel material: one pipeline, skin selected by uniform.

use bevy::asset::weak_handle;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
};

use crate::skin::{Skin, SkinParams, TexturedSkin, SHADER_MODE_TEXTURED};

pub const PANEL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("a5f1c0de-1a2b-4a11-9c3e-9d1b7c0e0001");

/// Material for a world-space UI panel.
///
/// `content` is the texture a panel's UI camera renders into; everything else
/// is skin styling. Packed into vec4s because uniform layout rules make a
/// struct of loose scalars a padding minefield across backends.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PanelMaterial {
    #[uniform(0)]
    pub accent: Vec4,
    #[uniform(0)]
    pub text_tint: Vec4,
    #[uniform(0)]
    pub backdrop: Vec4,
    /// x emissive, y backdrop_opacity, z scanline_strength, w edge_glow
    #[uniform(0)]
    pub a: Vec4,
    /// x flicker, y bezel, z corner_radius, w aspect
    #[uniform(0)]
    pub b: Vec4,
    /// x skin mode, y selected, z opacity, w unused
    #[uniform(0)]
    pub c: Vec4,
    /// x,y source border fraction; z,w panel border fraction (nine-slice)
    #[uniform(0)]
    pub d: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub content: Handle<Image>,
    /// Nine-sliced frame overlay. A 1x1 transparent image for procedural
    /// skins -- the binding must exist even when the branch ignores it.
    #[texture(3)]
    #[sampler(4)]
    pub frame: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub backdrop_tex: Handle<Image>,
}

impl PanelMaterial {
    pub fn new(skin: Skin, content: Handle<Image>, blank: Handle<Image>, aspect: f32) -> Self {
        let p = skin.params();
        let mut m = Self {
            accent: Vec4::ZERO,
            text_tint: Vec4::ZERO,
            backdrop: Vec4::ZERO,
            a: Vec4::ZERO,
            b: Vec4::ZERO,
            c: Vec4::ZERO,
            d: Vec4::ZERO,
            content,
            frame: blank.clone(),
            backdrop_tex: blank,
        };
        m.apply(skin, &p, aspect);
        m
    }

    /// Restyle to an art-driven skin: point the frame and body bindings at
    /// its textures and switch the shader branch.
    pub fn apply_textured(&mut self, skin: &TexturedSkin, aspect: f32, selected: bool) {
        self.apply_params(&skin.params, aspect);
        self.c.x = SHADER_MODE_TEXTURED as f32;
        self.d = Vec4::new(
            skin.source_border.x,
            skin.source_border.y,
            skin.panel_border.x,
            skin.panel_border.y,
        );
        let frame = match (selected, &skin.frame_selected) {
            (true, Some(sel)) => sel.clone(),
            _ => skin.frame.clone(),
        };
        self.frame = frame;
        if let Some(bg) = &skin.background {
            self.backdrop_tex = bg.clone();
        }
    }

    /// The palette half of restyling, shared by both skin kinds.
    fn apply_params(&mut self, p: &SkinParams, aspect: f32) {
        let rgba = |c: Color| {
            let s = c.to_srgba();
            Vec4::new(s.red, s.green, s.blue, s.alpha)
        };
        self.accent = rgba(p.accent);
        self.text_tint = rgba(p.text);
        self.backdrop = rgba(p.backdrop);
        self.a = Vec4::new(
            p.emissive,
            p.backdrop_opacity,
            p.scanline_strength,
            p.edge_glow,
        );
        self.b = Vec4::new(p.flicker, p.bezel, p.corner_radius, aspect);
    }

    /// Restyle in place. Swapping skins must not rebuild the panel, its
    /// render target, or its UI tree — only these numbers change.
    pub fn apply(&mut self, skin: Skin, p: &SkinParams, aspect: f32) {
        self.apply_params(p, aspect);
        self.c = Vec4::new(skin.shader_mode() as f32, self.c.y, self.c.z.max(1.0), 0.0);
    }

    pub fn set_selected(&mut self, selected: bool) {
        self.c.y = if selected { 1.0 } else { 0.0 };
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.c.z = opacity.clamp(0.0, 1.0);
    }
}

impl Material for PanelMaterial {
    fn fragment_shader() -> ShaderRef {
        PANEL_SHADER_HANDLE.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        // Blended, not additive: the Industrial skin is opaque and must
        // occlude, while the other two need real transparency.
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Panels are read from both sides -- a hologram you walk around
        // should not vanish when you pass behind it.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}
