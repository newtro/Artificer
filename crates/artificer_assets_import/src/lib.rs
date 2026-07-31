//! Turning source art into an [`AssetPack`].
//!
//! This crate is the ONLY place in the engine that parses a model file, and
//! it is native-only by construction: nothing that ships to a browser depends
//! on it. `artificer_assets` — which the WASM client does use — must never
//! gain a dependency here. That direction is what keeps an FBX reader out of
//! the bundle; a feature flag would leave it one `default-features` away.
//!
//! ```text
//!   FBX / OBJ  --ufbx--\
//!                       >--> SourceScene --> convert --> PackedAsset --> AssetPack
//!   glTF / GLB --gltf--/
//! ```
//!
//! The split matters: front-ends disagree about everything, so they are kept
//! small and dumb, and every correction (axes, mirroring, pivots, winding,
//! budgets, submeshes) happens once in [`convert`], in the order
//! [`artificer_assets::MeshImport`] documents.

pub mod convert;
pub mod error;
pub mod fbx;
pub mod gltf;
pub mod source;
pub mod texture;

pub use error::ImportError;
pub use source::{SourceMesh, SourcePart, SourceScene};

use artificer_assets::{AssetPack, ImportManifest, MeshImport, SourceFormat};
use std::path::{Path, PathBuf};

/// Read one source file, choosing a front-end by declared format or by
/// extension.
pub fn read_source(
    path: &Path,
    format: SourceFormat,
    frame: artificer_assets::AxisConvention,
) -> Result<SourceScene, ImportError> {
    let text = path.to_string_lossy().to_string();
    let resolved = match format {
        SourceFormat::Auto => match extension_of(path).as_deref() {
            Some("gltf") | Some("glb") => SourceFormat::Gltf,
            Some("obj") => SourceFormat::Obj,
            Some("fbx") => SourceFormat::Fbx,
            other => {
                return Err(ImportError::Read(
                    text,
                    format!(
                        "unknown extension {:?} — declare `format` in the manifest",
                        other.unwrap_or("<none>")
                    ),
                ))
            }
        },
        explicit => explicit,
    };

    match resolved {
        // ufbx reads FBX (binary and ASCII) and OBJ through one API.
        SourceFormat::Fbx | SourceFormat::Obj => fbx::read(&text, frame),
        SourceFormat::Gltf => gltf::read(&text),
        SourceFormat::Auto => unreachable!("resolved above"),
    }
}

fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// Import one mesh described by a manifest entry, into an existing pack.
pub fn import_mesh(
    root: &Path,
    manifest: &ImportManifest,
    import: &MeshImport,
    pack: &mut AssetPack,
) -> Result<(), ImportError> {
    let source = manifest.source(&import.source).ok_or_else(|| {
        ImportError::Manifest(vec![artificer_assets::ValidationIssue {
            asset_id: import.id.clone(),
            message: format!("references unknown import source '{}'", import.source),
        }])
    })?;
    let path = resolve(root, &source.path);
    let scene = read_source(&path, source.format, import.axis)?;
    let asset = convert::convert(&scene, import, pack)?;
    pack.assets.push(asset);
    Ok(())
}

/// Import an entire manifest.
///
/// Source files are read ONCE each and reused across every asset that draws
/// from them — a modular kit is dozens of assets out of a handful of files,
/// and re-parsing a 4 MB FBX per asset turns a fast bake into a slow one.
pub fn import_manifest(root: &Path, manifest: &ImportManifest) -> Result<AssetPack, ImportError> {
    let issues = manifest.validate();
    if !issues.is_empty() {
        // Fail before opening anything: a typo should cost a second, not a
        // full parse of every file in the library.
        return Err(ImportError::Manifest(issues));
    }

    let mut pack = AssetPack::new();

    // Textures first: an asset's material references one by id, and baking
    // them up front means a dangling reference is caught by pack validation
    // rather than by a blank surface at runtime.
    for texture in &manifest.textures {
        texture::bake_texture(&mut pack, root, texture)?;
    }

    // Keyed by (source id, axis frame) because the frame decides whether the
    // reader converts, so the same file under two frames is two scenes.
    let mut cache: Vec<(String, artificer_assets::AxisConvention, SourceScene)> = Vec::new();

    for import in &manifest.meshes {
        let source = manifest
            .source(&import.source)
            .expect("validated above that every source resolves");
        let cached = cache
            .iter()
            .find(|(id, frame, _)| id == &source.id && *frame == import.axis)
            .map(|(_, _, scene)| scene);
        let scene = match cached {
            Some(scene) => scene,
            None => {
                let path = resolve(root, &source.path);
                let scene = read_source(&path, source.format, import.axis)?;
                log::debug!(
                    "read {} ({} meshes, units/m {:?})",
                    source.path,
                    scene.meshes.len(),
                    scene.declared_units_per_metre
                );
                cache.push((source.id.clone(), import.axis, scene));
                &cache.last().expect("just pushed").2
            }
        };
        let asset = convert::convert(scene, import, &mut pack)?;
        pack.assets.push(asset);
    }

    pack.canonicalize();
    let issues = pack.validate();
    if !issues.is_empty() {
        return Err(ImportError::Invalid(issues));
    }
    Ok(pack)
}

/// Manifest paths are relative to the manifest, so a content directory can be
/// moved or checked out anywhere without editing every entry.
fn resolve(root: &Path, path: &str) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    }
}
