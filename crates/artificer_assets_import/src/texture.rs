//! Baking textures into a pack.
//!
//! Textures are stored as ENCODED PNG, not raw pixels: a 2048² RGBA page is
//! 16 MB raw and under 1 MB encoded, and the renderer has a PNG decoder
//! already. The only reason to decode here is to honour `max_size`.

use crate::error::ImportError;
use artificer_assets::{AssetPack, TextureBlob, TextureImport};
use std::path::Path;

/// Read, optionally downscale, and register one texture.
pub fn bake_texture(
    pack: &mut AssetPack,
    root: &Path,
    import: &TextureImport,
) -> Result<(), ImportError> {
    let path = if Path::new(&import.path).is_absolute() {
        Path::new(&import.path).to_path_buf()
    } else {
        root.join(&import.path)
    };
    let bytes = std::fs::read(&path)
        .map_err(|e| ImportError::Read(path.to_string_lossy().to_string(), e.to_string()))?;

    let image = image::load_from_memory(&bytes)
        .map_err(|e| ImportError::Read(path.to_string_lossy().to_string(), e.to_string()))?;
    let (width, height) = (image.width(), image.height());

    let (png, width, height) = match import.max_size {
        // Downscaling happens ONCE, here, rather than at load: source packs
        // ship 4096² pages at ~3.3 MB each, which would dominate a browser
        // bundle on their own, and paying to shrink them on every cold start
        // would be worse still.
        Some(max) if width.max(height) > max => {
            let scale = max as f32 / width.max(height) as f32;
            let (w, h) = (
                ((width as f32 * scale).round() as u32).max(1),
                ((height as f32 * scale).round() as u32).max(1),
            );
            // Lanczos3: atlas pages are downscaled once and looked at
            // forever, so quality is worth more than bake seconds.
            let resized = image.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
            (encode_png(&resized, &path)?, w, h)
        }
        // Otherwise keep the ORIGINAL bytes rather than re-encoding: a
        // re-encode is lossless but not byte-identical across image-crate
        // versions, which would make bakes differ for no gain.
        _ => (bytes, width, height),
    };

    pack.textures.push(TextureBlob {
        id: import.id.clone(),
        png,
        sampler: import.sampler,
        width,
        height,
    });
    Ok(())
}

fn encode_png(image: &image::DynamicImage, path: &Path) -> Result<Vec<u8>, ImportError> {
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| ImportError::Read(path.to_string_lossy().to_string(), e.to_string()))?;
    Ok(out.into_inner())
}
