//! Asset foundations: procedural mesh builders and the asset contract.
//!
//! Every runtime asset — generated or imported — carries an [`AssetRecord`]
//! and must pass [`validate_asset`]. Games compose ship/station geometry from
//! the builders in [`procmesh`] and register the result with the scene.

pub mod manifest;
pub mod procmesh;

pub use manifest::validate_asset;
pub use manifest::{
    AssetCategory, AssetManifest, AssetRecord, CollisionProxy, LodPolicy, PerfBudget, Socket,
    ValidationIssue,
};
