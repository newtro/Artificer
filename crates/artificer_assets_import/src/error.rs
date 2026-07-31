//! Import failures.
//!
//! Every variant names the asset that failed and what would fix it. A bake
//! runs unattended over a content library, so "import failed" without the id
//! and the numbers costs whoever reads the log a bisect.

use artificer_assets::{PackError, ValidationIssue};

#[derive(Debug, Clone, PartialEq)]
pub enum ImportError {
    /// The source file could not be opened or parsed.
    Read(String, String),
    /// The manifest is structurally wrong; nothing was opened.
    Manifest(Vec<ValidationIssue>),
    /// A mesh selection matched nothing. Carries what WAS available, because
    /// the usual cause is a name that differs from the one in the file.
    NoSuchMesh {
        asset: String,
        wanted: String,
        available: Vec<String>,
    },
    Geometry(String, String),
    BadCorrection(String, String),
    OverBudget {
        asset: String,
        triangles: u64,
        vertices: u64,
        max_triangles: u32,
        max_vertices: u32,
    },
    TooLarge {
        asset: String,
        indices: usize,
    },
    /// The finished pack failed the asset contract.
    Invalid(Vec<ValidationIssue>),
    Pack(PackError),
    Io(String),
}

impl From<PackError> for ImportError {
    fn from(e: PackError) -> Self {
        ImportError::Pack(e)
    }
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Read(path, why) => write!(f, "could not read '{path}': {why}"),
            ImportError::Manifest(issues) => {
                write!(f, "import manifest is invalid ({} issues):", issues.len())?;
                for i in issues {
                    write!(f, "\n  {}: {}", i.asset_id, i.message)?;
                }
                Ok(())
            }
            ImportError::NoSuchMesh {
                asset,
                wanted,
                available,
            } => {
                write!(f, "asset '{asset}': no mesh matching {wanted}")?;
                if available.is_empty() {
                    write!(f, " (the file contains no meshes)")
                } else {
                    write!(f, ". The file contains: {}", available.join(", "))
                }
            }
            ImportError::Geometry(asset, why) => write!(f, "asset '{asset}': {why}"),
            ImportError::BadCorrection(asset, why) => {
                write!(f, "asset '{asset}': correction cannot be applied: {why}")
            }
            ImportError::OverBudget {
                asset,
                triangles,
                vertices,
                max_triangles,
                max_vertices,
            } => write!(
                f,
                "asset '{asset}' is over budget: {triangles} triangles (max {max_triangles}), \
                 {vertices} vertices (max {max_vertices})"
            ),
            ImportError::TooLarge { asset, indices } => write!(
                f,
                "asset '{asset}' has {indices} indices, beyond what a u32 submesh can address"
            ),
            ImportError::Invalid(issues) => {
                write!(
                    f,
                    "imported pack failed validation ({} issues):",
                    issues.len()
                )?;
                for i in issues {
                    write!(f, "\n  {}: {}", i.asset_id, i.message)?;
                }
                Ok(())
            }
            ImportError::Pack(e) => write!(f, "{e}"),
            ImportError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ImportError {}
