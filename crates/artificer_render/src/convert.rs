//! artificer_scene -> Bevy type conversions.

use artificer_scene::{
    AlphaModeDesc, MaterialDesc, MeshData, TextureColorSpace, TextureSampling, ToneMapDesc,
    TransformDesc,
};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::mesh::Indices;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Face, PrimitiveTopology};

pub(crate) fn to_bevy_transform(t: &TransformDesc) -> Transform {
    Transform {
        translation: t.translation,
        rotation: t.rotation,
        scale: t.scale,
    }
}

pub(crate) fn to_bevy_mesh(data: &MeshData) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, data.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, data.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, data.uvs.clone());
    mesh.insert_indices(Indices::U32(data.indices.clone()));

    // TANGENTS, or normal maps do nothing.
    //
    // A tangent-space normal map is expressed relative to a per-vertex
    // tangent frame. With no `ATTRIBUTE_TANGENT` Bevy's PBR shader compiles
    // without its normal-mapping branch and simply IGNORES the bound map --
    // no warning, no error, and the surface still renders. That is exactly
    // how a 4K normal map gets baked into a pack, bound to a material, and
    // never once affects a pixel while everything appears to work.
    //
    // Generated here rather than at bake time because mikktspace needs the
    // final positions, normals, UVs and indices together, which is what this
    // function has. Meshes without UVs (untextured primitives) are skipped:
    // they cannot carry a normal map, and generation would fail on them.
    if !data.uvs.is_empty() && !data.normals.is_empty() {
        if let Err(e) = mesh.generate_tangents() {
            log::warn!("could not generate tangents ({e:?}); normal maps will be ignored");
        }
    }
    mesh
}

/// Decode a PNG into a Bevy image with the sampler the material asked for.
///
/// Atlas pages need NEAREST: neighbouring swatches sit pixels apart on one
/// page, and bilinear filtering bleeds one swatch into the next along every
/// UV seam — the classic "why does this hull have a stripe of the wrong
/// colour along its edge" artifact.
pub(crate) fn decode_texture(
    png: &[u8],
    sampling: TextureSampling,
    color_space: TextureColorSpace,
) -> Result<Image, bevy::image::TextureError> {
    let mut image = Image::from_buffer(
        png,
        bevy::image::ImageType::Extension("png"),
        bevy::image::CompressedImageFormats::NONE,
        // Colour is gamma-encoded; a normal, metallic-roughness or occlusion
        // map is NOT, and decoding one as sRGB bends its values through a
        // gamma curve. The result still renders -- just lit as though the
        // surface faced somewhere it does not.
        matches!(color_space, TextureColorSpace::Srgb),
        bevy::image::ImageSampler::Default,
        RenderAssetUsages::default(),
    )?;
    let descriptor = match sampling {
        TextureSampling::Nearest => bevy::image::ImageSamplerDescriptor::nearest(),
        TextureSampling::Linear => bevy::image::ImageSamplerDescriptor::linear(),
    };
    image.sampler = bevy::image::ImageSampler::Descriptor(descriptor);
    Ok(image)
}

/// Build a renderable material from a scene description, resolving every
/// texture slot through the adapter maps.
///
/// Exposed because anything that draws pack geometry OUTSIDE the scene graph
/// — icon captures, an asset browser's turntable, an editor gizmo — needs the
/// same conversion the adapter does, and hand-rolling it in a game crate has
/// already gone wrong once: a copy that bound `base_color_texture` alone
/// rendered hard-surface art as a smooth shape with lines painted on it,
/// because the relief lives in the normal map. Callers get all four slots or
/// none.
pub fn material_from_desc(desc: &MaterialDesc, maps: &crate::AdapterMaps) -> StandardMaterial {
    let mut material = to_std_material(desc);
    let one = |slot: Option<artificer_scene::TextureId>| {
        slot.and_then(|id| maps.textures.get(&id).cloned())
    };
    material.base_color_texture = one(desc.base_color_texture);
    material.normal_map_texture = one(desc.normal_texture);
    material.metallic_roughness_texture = one(desc.metallic_roughness_texture);
    material.occlusion_texture = one(desc.occlusion_texture);
    material
}

pub(crate) fn to_std_material(desc: &MaterialDesc) -> StandardMaterial {
    let [r, g, b, a] = desc.base_color;
    let [er, eg, eb] = desc.emissive;
    StandardMaterial {
        base_color: Color::srgba(r, g, b, a),
        metallic: desc.metallic,
        perceptual_roughness: desc.roughness.clamp(0.045, 1.0),
        emissive: LinearRgba::rgb(er, eg, eb),
        unlit: desc.unlit,
        alpha_mode: match desc.alpha {
            AlphaModeDesc::Opaque => AlphaMode::Opaque,
            AlphaModeDesc::Blend => AlphaMode::Blend,
            AlphaModeDesc::Add => AlphaMode::Add,
        },
        double_sided: desc.double_sided,
        cull_mode: if desc.double_sided {
            None
        } else {
            Some(Face::Back)
        },
        ..Default::default()
    }
}

pub(crate) fn to_tonemapping(desc: ToneMapDesc) -> Tonemapping {
    match desc {
        ToneMapDesc::None => Tonemapping::None,
        ToneMapDesc::Reinhard => Tonemapping::ReinhardLuminance,
        ToneMapDesc::Filmic => Tonemapping::TonyMcMapface,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use artificer_scene::TextureSampling;

    /// A real 2x2 PNG, encoded by hand so the test needs no encoder and no
    /// fixture file.
    fn tiny_png() -> Vec<u8> {
        // Signature + IHDR + IDAT + IEND, CRCs included: a decoder rejects it
        // otherwise, which is the whole point of decoding it here.
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00,
            0x00, 0xFD, 0xD4, 0x9A, 0x73, 0x00, 0x00, 0x00, 0x13, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xC0, 0xF0, 0x1F, 0x8C, 0x18, 0xFE, 0x33, 0x00, 0x00,
            0x1D, 0xF0, 0x03, 0xFD, 0xA6, 0x89, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
            0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        PNG.to_vec()
    }

    #[test]
    fn a_baked_png_decodes_into_an_image() {
        // The pack carries encoded PNG rather than raw pixels, so this is the
        // step that turns a texture blob into something drawable. It had no
        // coverage at all until an adversarial review pointed that out.
        let image = decode_texture(
            &tiny_png(),
            TextureSampling::Nearest,
            TextureColorSpace::Srgb,
        )
        .expect("a valid PNG should decode");
        assert_eq!(image.width(), 2);
        assert_eq!(image.height(), 2);
    }

    #[test]
    fn atlas_pages_decode_with_nearest_sampling() {
        // Bilinear filtering bleeds neighbouring atlas swatches into each
        // other along every UV seam, so the sampler the material asked for
        // has to survive decoding.
        let nearest = decode_texture(
            &tiny_png(),
            TextureSampling::Nearest,
            TextureColorSpace::Srgb,
        )
        .unwrap();
        let linear = decode_texture(
            &tiny_png(),
            TextureSampling::Linear,
            TextureColorSpace::Srgb,
        )
        .unwrap();
        let is_nearest = |image: &Image| match &image.sampler {
            bevy::image::ImageSampler::Descriptor(d) => {
                matches!(d.mag_filter, bevy::image::ImageFilterMode::Nearest)
            }
            _ => false,
        };
        assert!(is_nearest(&nearest), "atlas pages must sample nearest");
        assert!(!is_nearest(&linear), "Linear must not be overridden");
    }

    #[test]
    fn a_blob_that_is_not_a_png_fails_rather_than_panicking() {
        assert!(decode_texture(
            b"definitely not a png",
            TextureSampling::Nearest,
            TextureColorSpace::Srgb
        )
        .is_err());
        assert!(decode_texture(&[], TextureSampling::Nearest, TextureColorSpace::Srgb).is_err());
    }

    #[test]
    fn a_uv_mesh_gets_tangents_so_normal_maps_actually_apply() {
        // Bevy's PBR shader drops its normal-mapping branch entirely when a
        // mesh has no ATTRIBUTE_TANGENT. A bound normal map is then ignored
        // silently -- the surface still renders, just without any of the
        // relief the map exists to provide. An adversarial review caught this
        // after the maps had been plumbed end to end and declared working.
        let quad = MeshData {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
        };
        let mesh = to_bevy_mesh(&quad);
        assert!(
            mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_some(),
            "a mesh with UVs must carry tangents, or every normal map it is \
             given is silently discarded by the shader"
        );
    }

    #[test]
    fn a_mesh_without_uvs_still_converts() {
        // Tangent generation needs UVs. A procedural primitive has none, and
        // must not be turned into a hard failure by the tangent step.
        let tri = MeshData {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uvs: vec![],
            indices: vec![0, 1, 2],
        };
        let mesh = to_bevy_mesh(&tri);
        assert!(mesh.attribute(Mesh::ATTRIBUTE_POSITION).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_none());
    }

    #[test]
    fn data_maps_decode_linear_and_colour_decodes_srgb() {
        // A normal map decoded as sRGB has every component bent through a
        // gamma curve, so the surface lights as though it faced somewhere
        // else. The failure is subtle on screen -- "the model looks a bit
        // off" -- so the distinction is pinned here rather than by eye.
        let colour = decode_texture(
            &tiny_png(),
            TextureSampling::Linear,
            TextureColorSpace::Srgb,
        )
        .unwrap();
        let data = decode_texture(
            &tiny_png(),
            TextureSampling::Linear,
            TextureColorSpace::Linear,
        )
        .unwrap();
        assert!(
            format!("{:?}", colour.texture_descriptor.format).contains("Srgb"),
            "base colour must decode as sRGB, got {:?}",
            colour.texture_descriptor.format
        );
        assert!(
            !format!("{:?}", data.texture_descriptor.format).contains("Srgb"),
            "a normal or metallic-roughness map must NOT decode as sRGB, got {:?}",
            data.texture_descriptor.format
        );
    }

    #[test]
    fn an_untextured_material_stays_untextured() {
        // The texture field defaults to None, so every existing material is
        // unaffected by textures having been added at all.
        let std = to_std_material(&MaterialDesc::color(0.5, 0.25, 0.125));
        assert!(std.base_color_texture.is_none());
    }
}
