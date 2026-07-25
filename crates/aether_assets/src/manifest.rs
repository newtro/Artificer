//! The asset contract (plan §11.2): every runtime asset provides or passes
//! validation for scale, orientation, pivot, bounds, collision proxy,
//! sockets, category, material compatibility, LOD policy, performance
//! budget, provenance/license, and gameplay metadata reference.

use aether_scene::{Aabb, MeshData};
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetCategory {
    ShipHull,
    ShipModule,
    Station,
    Prop,
    Effect,
    Environment,
    Wreckage,
}

/// Simplified collision geometry the physics adapter can build directly.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CollisionProxy {
    /// Half-extents of a box centered on the pivot.
    Cuboid { half_extents: [f32; 3] },
    Ball { radius: f32 },
    CapsuleZ { half_height: f32, radius: f32 },
    /// Deliberately no collision (pure visual effects).
    None,
}

/// Named attachment point (hardpoints, engines, docking ports).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Socket {
    pub name: String,
    pub position: [f32; 3],
    /// Unit direction the socket faces.
    pub direction: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LodPolicy {
    /// Single mesh at all distances (small props, MVP default).
    Single,
    /// Swap to nothing beyond `hide_beyond` meters.
    Cull { hide_beyond_m: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PerfBudget {
    pub max_triangles: u32,
    pub max_vertices: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetRecord {
    /// Stable asset id referenced by gameplay definitions.
    pub id: String,
    pub category: AssetCategory,
    /// Meters per model unit; 1.0 for natively metric assets.
    pub unit_scale: f32,
    /// Convention marker — engine expects "y-up,-z-forward".
    pub orientation: String,
    /// Declared pivot, expressed in the asset's own space.
    pub pivot: [f32; 3],
    /// Declared bounds; validated against actual geometry.
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub collision: CollisionProxy,
    pub sockets: Vec<Socket>,
    /// Material slots the asset expects (empty = engine default PBR).
    pub material_slots: Vec<String>,
    pub lod: LodPolicy,
    pub budget: PerfBudget,
    /// Where this asset came from: "procedural:<generator>", "synty:<pack>", "meshy:<job>".
    pub provenance: String,
    pub license: String,
    /// Gameplay definition that references this asset (e.g. a part id).
    pub gameplay_ref: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetManifest {
    pub assets: Vec<AssetRecord>,
}

impl AssetManifest {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn find(&self, id: &str) -> Option<&AssetRecord> {
        self.assets.iter().find(|a| a.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub asset_id: String,
    pub message: String,
}

fn issue(id: &str, msg: impl Into<String>) -> ValidationIssue {
    ValidationIssue {
        asset_id: id.to_string(),
        message: msg.into(),
    }
}

/// Validate a mesh against its contract record. Empty result = pass.
pub fn validate_asset(record: &AssetRecord, mesh: &MeshData) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let id = record.id.as_str();

    if record.id.trim().is_empty() {
        issues.push(issue(id, "asset id is empty"));
    }
    if let Err(e) = mesh.validate() {
        issues.push(issue(id, format!("mesh invalid: {e}")));
        return issues; // structural failure makes the rest meaningless
    }
    if record.unit_scale <= 0.0 || !record.unit_scale.is_finite() {
        issues.push(issue(id, format!("unit_scale {} not positive", record.unit_scale)));
    }
    if record.orientation != "y-up,-z-forward" {
        issues.push(issue(
            id,
            format!("orientation '{}' != engine convention 'y-up,-z-forward'", record.orientation),
        ));
    }

    let actual = mesh.bounds().expect("validated mesh has bounds");
    let declared = Aabb {
        min: Vec3::from_array(record.bounds_min),
        max: Vec3::from_array(record.bounds_max),
    };
    let tolerance = (declared.extents().length() * 0.05).max(0.01);
    if (actual.min - declared.min).length() > tolerance
        || (actual.max - declared.max).length() > tolerance
    {
        issues.push(issue(
            id,
            format!(
                "declared bounds {:?}..{:?} disagree with actual {:?}..{:?}",
                declared.min, declared.max, actual.min, actual.max
            ),
        ));
    }

    let pivot = Vec3::from_array(record.pivot);
    let grown = Aabb {
        min: actual.min - Vec3::splat(tolerance),
        max: actual.max + Vec3::splat(tolerance),
    };
    if !grown.contains(pivot) {
        issues.push(issue(id, format!("pivot {pivot:?} lies outside geometry bounds")));
    }

    for socket in &record.sockets {
        let p = Vec3::from_array(socket.position);
        if !grown.contains(p) {
            issues.push(issue(
                id,
                format!("socket '{}' at {p:?} lies outside bounds", socket.name),
            ));
        }
        let d = Vec3::from_array(socket.direction);
        if (d.length() - 1.0).abs() > 1e-3 {
            issues.push(issue(
                id,
                format!("socket '{}' direction is not unit length", socket.name),
            ));
        }
    }

    if mesh.triangle_count() as u32 > record.budget.max_triangles {
        issues.push(issue(
            id,
            format!(
                "triangle count {} exceeds budget {}",
                mesh.triangle_count(),
                record.budget.max_triangles
            ),
        ));
    }
    if mesh.vertex_count() as u32 > record.budget.max_vertices {
        issues.push(issue(
            id,
            format!(
                "vertex count {} exceeds budget {}",
                mesh.vertex_count(),
                record.budget.max_vertices
            ),
        ));
    }

    match record.collision {
        CollisionProxy::Cuboid { half_extents } => {
            let he = Vec3::from_array(half_extents);
            if he.cmple(Vec3::ZERO).any() {
                issues.push(issue(id, "cuboid collision proxy has non-positive extents"));
            }
        }
        CollisionProxy::Ball { radius } => {
            if radius <= 0.0 {
                issues.push(issue(id, "ball collision proxy has non-positive radius"));
            }
        }
        CollisionProxy::CapsuleZ { half_height, radius } => {
            if half_height <= 0.0 || radius <= 0.0 {
                issues.push(issue(id, "capsule collision proxy has non-positive dimensions"));
            }
        }
        CollisionProxy::None => {}
    }

    if record.provenance.trim().is_empty() {
        issues.push(issue(id, "provenance record is empty"));
    }
    if record.license.trim().is_empty() {
        issues.push(issue(id, "license record is empty"));
    }
    if record.gameplay_ref.trim().is_empty() {
        issues.push(issue(id, "gameplay metadata reference is empty"));
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procmesh;

    fn record_for(mesh: &MeshData, id: &str) -> AssetRecord {
        let b = mesh.bounds().unwrap();
        AssetRecord {
            id: id.into(),
            category: AssetCategory::Prop,
            unit_scale: 1.0,
            orientation: "y-up,-z-forward".into(),
            pivot: [0.0; 3],
            bounds_min: b.min.to_array(),
            bounds_max: b.max.to_array(),
            collision: CollisionProxy::Ball { radius: 1.0 },
            sockets: vec![],
            material_slots: vec![],
            lod: LodPolicy::Single,
            budget: PerfBudget {
                max_triangles: 10_000,
                max_vertices: 10_000,
            },
            provenance: "procedural:test".into(),
            license: "internal".into(),
            gameplay_ref: "test.part".into(),
        }
    }

    #[test]
    fn valid_asset_passes() {
        let mesh = procmesh::uv_sphere(1.0, 12, 8);
        let record = record_for(&mesh, "test.sphere");
        assert!(validate_asset(&record, &mesh).is_empty());
    }

    #[test]
    fn bounds_mismatch_detected() {
        let mesh = procmesh::uv_sphere(1.0, 12, 8);
        let mut record = record_for(&mesh, "test.sphere");
        record.bounds_max = [9.0, 9.0, 9.0];
        let issues = validate_asset(&record, &mesh);
        assert!(issues.iter().any(|i| i.message.contains("bounds")));
    }

    #[test]
    fn budget_violation_detected() {
        let mesh = procmesh::uv_sphere(1.0, 32, 24);
        let mut record = record_for(&mesh, "test.sphere");
        record.budget = PerfBudget {
            max_triangles: 10,
            max_vertices: 10,
        };
        let issues = validate_asset(&record, &mesh);
        assert!(issues.iter().any(|i| i.message.contains("exceeds budget")));
    }

    #[test]
    fn missing_provenance_detected() {
        let mesh = procmesh::cuboid(1.0, 1.0, 1.0);
        let mut record = record_for(&mesh, "test.box");
        record.provenance = "".into();
        let issues = validate_asset(&record, &mesh);
        assert!(issues.iter().any(|i| i.message.contains("provenance")));
    }

    #[test]
    fn manifest_round_trips_json() {
        let mesh = procmesh::cuboid(1.0, 1.0, 1.0);
        let manifest = AssetManifest {
            assets: vec![record_for(&mesh, "test.box")],
        };
        let json = manifest.to_json().unwrap();
        let parsed = AssetManifest::from_json(&json).unwrap();
        assert!(parsed.find("test.box").is_some());
    }
}
