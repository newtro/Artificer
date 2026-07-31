//! Asset foundations: procedural mesh builders, the asset contract, the
//! declarative import manifest, and the baked pack format.
//!
//! Every runtime asset — generated or imported — carries an [`AssetRecord`]
//! and must pass [`validate_asset`]. Games compose ship/station geometry from
//! the builders in [`procmesh`], or describe an import in an
//! [`ImportManifest`] and ship the resulting [`AssetPack`].
//!
//! This crate stays deliberately light: no file parsers, no image decoders,
//! no platform dependencies. Reading FBX and glTF lives in the native-only
//! `artificer_assets_import` crate, so a WASM build physically cannot pull a
//! model parser into the bundle.

pub mod manifest;
pub mod pack;
pub mod procmesh;

pub use manifest::{collision_proxy_problem, material_problem, validate_asset};
pub use manifest::{
    AssetCategory, AssetManifest, AssetRecord, Axis, AxisConvention, CollisionProxy,
    DeclaredBounds, Handedness, ImportManifest, ImportSource, LodPolicy, MaterialBinding,
    MaterialMapping, MaterialSelector, MeshImport, MeshSelect, PerfBudget, PivotPolicy,
    SamplerMode, Socket, SourceFormat, TextureImport, ValidationIssue,
};
pub use pack::{
    material_for, AssetPack, PackError, PackSizeReport, PackedAsset, PackedMaterial, SubMesh,
    TextureBlob, PACK_FORMAT_VERSION, PACK_MAGIC,
};
