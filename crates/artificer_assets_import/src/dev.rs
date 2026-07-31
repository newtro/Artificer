//! Importing at runtime, for iteration.
//!
//! The shipping path is: bake once, load the pack. That is fast and needs no
//! parsers in the binary. But while art is being authored, re-running a bake
//! by hand after every export is exactly the friction this pipeline exists to
//! remove, so a native build can be told to import from source instead.
//!
//! Deliberately opt-in and native-only. A game calls [`pack_or_import`] with
//! the pack it would normally ship; if the environment names a manifest, that
//! manifest is imported fresh instead.

use crate::{import_manifest, ImportError};
use artificer_assets::{AssetPack, ImportManifest};
use std::path::Path;

/// Environment variable naming an import manifest to use instead of a baked
/// pack.
pub const DEV_MANIFEST_ENV: &str = "ARTIFICER_ASSET_MANIFEST";

/// Import the manifest named by [`DEV_MANIFEST_ENV`], if it is set.
///
/// `Ok(None)` means "not requested" — the caller should use its baked pack.
pub fn import_from_env() -> Result<Option<AssetPack>, ImportError> {
    let Ok(path) = std::env::var(DEV_MANIFEST_ENV) else {
        return Ok(None);
    };
    if path.trim().is_empty() {
        return Ok(None);
    }
    import_from_file(Path::new(&path)).map(Some)
}

/// Read a manifest from disk and import it.
pub fn import_from_file(manifest_path: &Path) -> Result<AssetPack, ImportError> {
    let text = std::fs::read_to_string(manifest_path)
        .map_err(|e| ImportError::Io(format!("{}: {e}", manifest_path.display())))?;
    let manifest = ImportManifest::from_json(&text)
        .map_err(|e| ImportError::Io(format!("{}: {e}", manifest_path.display())))?;
    // Source paths are relative to the manifest, as everywhere else.
    let root = manifest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    import_manifest(root, &manifest)
}

/// The dev override if one is requested and works, otherwise the baked pack.
pub fn pack_or_import(baked: AssetPack) -> AssetPack {
    let requested = std::env::var(DEV_MANIFEST_ENV)
        .ok()
        .filter(|p| !p.trim().is_empty());
    pack_or_import_from(baked, requested.as_deref().map(Path::new))
}

/// The same decision, with the manifest passed in rather than read from the
/// environment.
///
/// Split out because environment variables are process-global: a test that
/// sets one races every other test in the same binary. The logic worth
/// testing is here, and [`pack_or_import`] is the thin env-reading shell.
///
/// A failed dev import is LOUD but not fatal: mid-iteration the art is often
/// half-exported, and taking the game down for it would be worse than falling
/// back to the last good bake.
pub fn pack_or_import_from(baked: AssetPack, manifest: Option<&Path>) -> AssetPack {
    let Some(path) = manifest else { return baked };
    match import_from_file(path) {
        Ok(fresh) => {
            log::info!(
                "dev import: using {} assets from {}",
                fresh.len(),
                path.display()
            );
            fresh
        }
        Err(e) => {
            log::error!("dev import failed, falling back to the baked pack: {e}");
            baked
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_manifest_means_use_the_baked_pack() {
        let baked = AssetPack::new();
        assert_eq!(pack_or_import_from(baked.clone(), None), baked);
    }

    #[test]
    fn a_missing_manifest_reports_the_path() {
        let err = import_from_file(Path::new("no_such_manifest.json")).unwrap_err();
        assert!(
            err.to_string().contains("no_such_manifest.json"),
            "got: {err}"
        );
    }

    #[test]
    fn a_failed_dev_import_falls_back_rather_than_taking_the_game_down() {
        // Mid-iteration the art is often half-exported; losing the last good
        // bake over it would be worse than the stale assets.
        let baked = AssetPack::new();
        let result = pack_or_import_from(
            baked.clone(),
            Some(Path::new("definitely_not_a_manifest.json")),
        );
        assert_eq!(result, baked, "a broken dev import must not lose the bake");
    }
}
