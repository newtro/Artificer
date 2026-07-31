//! artificer_scene -> Bevy type conversions.

use artificer_scene::{
    AlphaModeDesc, MaterialDesc, MeshData, TextureSampling, ToneMapDesc, TransformDesc,
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
) -> Result<Image, bevy::image::TextureError> {
    let mut image = Image::from_buffer(
        png,
        bevy::image::ImageType::Extension("png"),
        bevy::image::CompressedImageFormats::NONE,
        // Base-colour textures are authored in sRGB.
        true,
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
        let image = decode_texture(&tiny_png(), TextureSampling::Nearest)
            .expect("a valid PNG should decode");
        assert_eq!(image.width(), 2);
        assert_eq!(image.height(), 2);
    }

    #[test]
    fn atlas_pages_decode_with_nearest_sampling() {
        // Bilinear filtering bleeds neighbouring atlas swatches into each
        // other along every UV seam, so the sampler the material asked for
        // has to survive decoding.
        let nearest = decode_texture(&tiny_png(), TextureSampling::Nearest).unwrap();
        let linear = decode_texture(&tiny_png(), TextureSampling::Linear).unwrap();
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
        assert!(decode_texture(b"definitely not a png", TextureSampling::Nearest).is_err());
        assert!(decode_texture(&[], TextureSampling::Nearest).is_err());
    }

    #[test]
    fn an_untextured_material_stays_untextured() {
        // The texture field defaults to None, so every existing material is
        // unaffected by textures having been added at all.
        let std = to_std_material(&MaterialDesc::color(0.5, 0.25, 0.125));
        assert!(std.base_color_texture.is_none());
    }
}
