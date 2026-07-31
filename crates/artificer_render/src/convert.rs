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
