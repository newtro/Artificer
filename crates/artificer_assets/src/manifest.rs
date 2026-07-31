//! The asset contract (plan §11.2): every runtime asset provides or passes
//! validation for scale, orientation, pivot, bounds, collision proxy,
//! sockets, category, material compatibility, LOD policy, performance
//! budget, provenance/license, and gameplay metadata reference.

use artificer_scene::{Aabb, MaterialDesc, MeshData};
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
#[serde(deny_unknown_fields)]
pub enum CollisionProxy {
    /// Half-extents of a box centered on the pivot.
    Cuboid {
        half_extents: [f32; 3],
    },
    Ball {
        radius: f32,
    },
    CapsuleZ {
        half_height: f32,
        radius: f32,
    },
    /// Deliberately no collision (pure visual effects).
    None,
}

/// Named attachment point (hardpoints, engines, docking ports).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Socket {
    pub name: String,
    pub position: [f32; 3],
    /// Unit direction the socket faces.
    pub direction: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum LodPolicy {
    /// Single mesh at all distances (small props, MVP default).
    Single,
    /// Swap to nothing beyond `hide_beyond` meters.
    Cull { hide_beyond_m: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerfBudget {
    pub max_triangles: u32,
    pub max_vertices: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRecord {
    /// Stable asset id referenced by gameplay definitions.
    pub id: String,
    pub category: AssetCategory,
    /// Meters per model unit; 1.0 for natively metric assets.
    ///
    /// IMPORTED assets always record 1.0: the importer bakes scale into the
    /// vertex data, so by the time geometry reaches a record it is already
    /// metric. This is NOT the same field as `MeshImport::unit_scale`, which
    /// is an authoring-time correction knob.
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
    /// PACKED MATERIAL IDS this asset draws with (`atlas.page_a`,
    /// `mat.ship.hull`), deduplicated and sorted — NOT source-file material
    /// names. Empty means "unspecified", not "none": the pack cross-checks
    /// this against the asset's submeshes only when it is populated, so a
    /// record may leave it empty and let the submeshes speak for themselves.
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

// ---------------------------------------------------------------------------
// Import manifest: how source art becomes engine assets, described in data.
//
// The engine ships this vocabulary; games supply the manifest AND the art.
// Correction (axes, units, winding, pivot) is declared ONCE here rather than
// fixed per-asset downstream, which is what keeps orientation bugs from
// multiplying across a content library.
// ---------------------------------------------------------------------------

/// A signed cardinal direction in the SOURCE file's space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Axis {
    pub fn to_vec3(self) -> Vec3 {
        match self {
            Axis::PosX => Vec3::X,
            Axis::NegX => Vec3::NEG_X,
            Axis::PosY => Vec3::Y,
            Axis::NegY => Vec3::NEG_Y,
            Axis::PosZ => Vec3::Z,
            Axis::NegZ => Vec3::NEG_Z,
        }
    }

    /// True when the two axes lie on the same line, which cannot describe a
    /// coordinate frame.
    pub fn is_parallel_to(self, other: Axis) -> bool {
        self.to_vec3().abs() == other.to_vec3().abs()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Handedness {
    Right,
    Left,
}

/// Source coordinate convention, corrected to the engine's
/// `y-up,-z-forward,right-handed` at import.
///
/// Handedness is explicit and NOT cosmetic: converting a left-handed source
/// mirrors an axis, which reverses triangle winding, and the importer must
/// flip winding back or every face renders inside-out. Up/forward alone
/// cannot distinguish (say) Maya from Unity, which share Y-up/+Z-forward but
/// differ in handedness.
///
/// The representation is a general frame rather than a fixed menu of named
/// tools, because a menu is always missing somebody's convention — Unreal is
/// left-handed Z-up with **+X** forward, which no up/forward-only naming
/// scheme covers. Constructors for the common tools are provided below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub enum AxisConvention {
    /// Trust the axis, handedness, and unit metadata the FILE declares. FBX
    /// and glTF both carry it, so this is correct for them and is the
    /// default. For formats that declare nothing (OBJ), this means "already
    /// engine-native".
    ///
    /// This is an ASSERTION-FREE mode: the importer reads whatever the file
    /// says. Use an `Explicit` frame to OVERRIDE a file whose metadata is
    /// absent or wrong. An explicit frame overrides axes and handedness ONLY;
    /// unit normalization to metres always follows the file (or, where the
    /// file declares no units, `MeshImport::unit_scale` is the sole scale).
    #[default]
    FromSource,
    Explicit {
        up: Axis,
        forward: Axis,
        handedness: Handedness,
    },
}

impl AxisConvention {
    /// The engine's own frame: right-handed, Y up, -Z forward.
    pub fn engine_native() -> Self {
        AxisConvention::Explicit {
            up: Axis::PosY,
            forward: Axis::NegZ,
            handedness: Handedness::Right,
        }
    }

    /// Maya: right-handed, Y up, +Z forward.
    pub fn maya() -> Self {
        AxisConvention::Explicit {
            up: Axis::PosY,
            forward: Axis::PosZ,
            handedness: Handedness::Right,
        }
    }

    /// Blender / 3ds Max: right-handed, Z up, -Y forward.
    pub fn blender() -> Self {
        AxisConvention::Explicit {
            up: Axis::PosZ,
            forward: Axis::NegY,
            handedness: Handedness::Right,
        }
    }

    /// Unity: left-handed, Y up, +Z forward.
    pub fn unity() -> Self {
        AxisConvention::Explicit {
            up: Axis::PosY,
            forward: Axis::PosZ,
            handedness: Handedness::Left,
        }
    }

    /// Unreal: left-handed, Z up, **+X** forward.
    pub fn unreal() -> Self {
        AxisConvention::Explicit {
            up: Axis::PosZ,
            forward: Axis::PosX,
            handedness: Handedness::Left,
        }
    }

    /// Whether converting from this frame mirrors an axis (and so reverses
    /// triangle winding, which the importer must flip back).
    ///
    /// `None` for [`AxisConvention::FromSource`] — and that is the whole
    /// point of the `Option`. Under `FromSource` the answer lives in the FILE
    /// (FBX carries signed `UpAxis`/`FrontAxis`/`CoordAxis`, and Unity- or
    /// Unreal-exported FBX is routinely left-handed), so a bare `false` here
    /// would confidently skip a winding flip that IS needed and hand back
    /// inside-out geometry. Callers must resolve `FromSource` against the
    /// file's own metadata before asking.
    pub fn changes_handedness(self) -> Option<bool> {
        match self {
            AxisConvention::FromSource => None,
            AxisConvention::Explicit { handedness, .. } => Some(handedness == Handedness::Left),
        }
    }

    /// Structural sanity: up and forward must describe a real frame.
    pub fn is_well_formed(self) -> bool {
        match self {
            AxisConvention::FromSource => true,
            AxisConvention::Explicit { up, forward, .. } => !up.is_parallel_to(forward),
        }
    }

    /// The matrix that takes a point from this frame into the engine's
    /// (`right-handed, Y up, -Z forward`).
    ///
    /// This lives here, computed once, because the third-axis sign is the
    /// easiest thing in an importer to get backwards and the failure is
    /// silent — mirrored geometry that looks almost right. The rule flips
    /// with handedness:
    ///
    /// * right-handed: `right = forward × up`
    /// * left-handed:  `right = up × forward`
    ///
    /// Check it against the engine's own frame: `up = +Y`, `forward = -Z`,
    /// right-handed gives `right = (-Z) × (+Y) = +X`. Correct. Using the
    /// left-handed rule there would yield `-X` and mirror every asset.
    ///
    /// Returns `None` for [`AxisConvention::FromSource`], where the frame is
    /// whatever the file declares and the importer must resolve it first, and
    /// for a malformed frame.
    pub fn to_engine_basis(self) -> Option<glam::Mat3> {
        let AxisConvention::Explicit {
            up,
            forward,
            handedness,
        } = self
        else {
            return None;
        };
        if up.is_parallel_to(forward) {
            return None;
        }
        let up_v = up.to_vec3();
        let fwd_v = forward.to_vec3();
        let right_v = match handedness {
            Handedness::Right => fwd_v.cross(up_v),
            Handedness::Left => up_v.cross(fwd_v),
        };
        // Rows map the source's (right, up, forward) onto the engine's
        // (+X, +Y, -Z), so a point expressed in the source frame comes out
        // expressed in the engine's.
        Some(glam::Mat3::from_cols(
            glam::Vec3::new(right_v.x, up_v.x, -fwd_v.x),
            glam::Vec3::new(right_v.y, up_v.y, -fwd_v.y),
            glam::Vec3::new(right_v.z, up_v.z, -fwd_v.z),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SourceFormat {
    /// Detect from the file extension.
    #[default]
    Auto,
    Fbx,
    Gltf,
    Obj,
}

/// Where the pivot ends up relative to the imported geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PivotPolicy {
    /// Trust the authored origin.
    #[default]
    AsAuthored,
    /// Recentre on the bounding-box centre.
    BoundsCenter,
    /// Centre in X/Z, drop the origin to the lowest Y (props that sit on decks).
    BaseY,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SamplerMode {
    /// Nearest keeps atlas swatches from bleeding into each other.
    #[default]
    Nearest,
    Linear,
}

/// Which meshes to take out of a source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub enum MeshSelect {
    /// Every mesh in the file, merged into one asset.
    #[default]
    All,
    /// Meshes whose source name matches exactly; merged in the listed order.
    Named(Vec<String>),
    /// Mesh at this index in file order.
    Index(u32),
}

/// How an imported asset gets its surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub enum MaterialMapping {
    /// Engine default PBR, no texture.
    #[default]
    Default,
    /// Share a texture atlas — the whole point of packs like Synty's, where
    /// one 2K page serves a library and every asset is one draw call.
    Atlas { texture: String },
    /// Explicit material, for sources whose own material cannot be honoured
    /// (e.g. meshes authored against a custom shader).
    Override(Box<MaterialDesc>),
}

/// Bind ONE material slot of the source file to an engine material.
///
/// Source material names are what the FBX/glTF actually carries, which is not
/// necessarily what a DCC tool's sidecar documentation calls it — the Synty
/// Sci-Fi Space meshes carry `SciFiSpace` / `SciFi11` / `SciFi` while the
/// pack's `MaterialList` text file names them `PolygonScifiSpace_Material_01_A`.
/// Bindings match the name IN THE FILE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum MaterialSelector {
    /// Material name as it appears in the source file.
    Name(String),
    /// Material slot index in the MERGED asset: source materials numbered in
    /// the order they are first encountered while merging the meshes selected
    /// by [`MeshSelect`], which fixes both the merge order and this numbering.
    /// For sources whose materials are unnamed or share a name — both of
    /// which real exporters produce, and neither of which a name can address.
    ///
    /// A slot addressed by BOTH a `Name` and an `Index` binding takes the
    /// `Name` one: names are what an author writes deliberately, indices are
    /// positional and shift when a source file is re-exported.
    Index(u32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialBinding {
    pub select: MaterialSelector,
    pub mapping: MaterialMapping,
}

impl MaterialBinding {
    /// A stable key for duplicate detection.
    fn key(&self) -> String {
        match &self.select {
            MaterialSelector::Name(n) => format!("name:{n}"),
            MaterialSelector::Index(i) => format!("index:{i}"),
        }
    }
}

/// One source file the bake reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSource {
    /// Referenced by `MeshImport::source`.
    pub id: String,
    /// Path relative to the manifest file.
    pub path: String,
    #[serde(default)]
    pub format: SourceFormat,
}

/// A texture to bake into the pack. Stored as encoded PNG bytes, so the
/// runtime needs no image decoder of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureImport {
    pub id: String,
    pub path: String,
    /// Downscale to at most this many pixels on the long edge. 4K atlases are
    /// common in source art and ruinous in a browser bundle.
    #[serde(default)]
    pub max_size: Option<u32>,
    #[serde(default)]
    pub sampler: SamplerMode,
}

/// One asset to produce: which geometry, corrected how, described by what
/// contract. Everything the [`AssetRecord`] needs is declared here so an
/// imported asset faces the same validation as a procedural one.
///
/// # Correction order
///
/// The knobs below compose in EXACTLY this order. Order matters — mirroring
/// before or after an arbitrary rotation gives different geometry, and a
/// bounds-centred pivot computed before rotation lands somewhere else — so it
/// is specified here once rather than left to each importer to guess:
///
/// 1. **Normalize to metres and to `y-up,-z-forward` right-handed**, using
///    [`AxisConvention`]. For FBX and glTF this is the file's own declared
///    axes and units unless [`AxisConvention::FromSource`] is overridden.
/// 2. **`rotation_deg`** — intrinsic Euler XYZ (glam's `EulerRot::XYZ`),
///    applied about the ORIGIN, not about the pivot.
/// 3. **`mirror_x`** — negate X.
/// 4. **`unit_scale`** — uniform scale about the origin.
/// 5. **`pivot`** — translate so the origin sits where [`PivotPolicy`] says,
///    measured on the geometry as it stands after steps 1-4.
/// 6. **Winding** — a face flip is applied when an odd number of mirroring
///    operations occurred. `mirror_x` and a handedness-changing
///    [`AxisConvention`] each count as one, and `flip_winding` XORs on top.
///    Setting `mirror_x` with a left-handed source therefore does NOT double-
///    flip back to inside-out geometry.
/// 7. **Normals** are TRANSFORMED by every step above, not merely
///    re-normalized: rotated with the geometry and negated in X by a mirror
///    (the inverse-transpose of a mirror is the mirror), then re-normalized.
///    Skipping the transform is what leaves mirrored parts lit inside-out.
///    Where the source carries no normals, they are generated with
///    [`artificer_scene::MeshData::recompute_normals`] — area-weighted smooth
///    — and NOT by any other scheme, because the baked bytes are supposed to
///    be reproducible and a different smoothing rule silently changes them.
/// 8. **UVs** — [`artificer_scene::MeshData`] requires one UV per vertex, so a
///    source with no UV set gets zeroes. That is correct for untextured
///    assets and visibly wrong for atlas-mapped ones, which is the intent: a
///    missing UV set should be obvious, not quietly plausible.
///
/// [`MeshSelect::All`] merges the file's meshes **in the order the file
/// declares them**, and [`MeshSelect::Named`] in the order listed. Merge order
/// determines the index buffer, and therefore the baked bytes, so it is fixed
/// rather than left to whatever order a reader happens to enumerate.
///
/// `declared_bounds` and `collision` are, like `sockets`, expressed in FINAL
/// engine space — after every step above.
///
/// `sockets` are declared in FINAL engine space — metres, y-up/-z-forward,
/// relative to the post-pivot origin — because that is the space
/// [`validate_asset`] bounds-checks them against. Socket coordinates copied
/// raw out of a DCC tool (source axes, centimetres) will be rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshImport {
    /// Asset id in the baked pack.
    pub id: String,
    /// [`ImportSource::id`] to read from.
    pub source: String,
    #[serde(default)]
    pub select: MeshSelect,
    /// Extra scale applied AFTER the importer normalises to metres. 1.0 keeps
    /// the source's real-world size.
    #[serde(default = "one")]
    pub unit_scale: f32,
    #[serde(default)]
    pub axis: AxisConvention,
    /// Euler XYZ degrees applied after axis correction — the per-asset escape
    /// hatch for art whose nose points the wrong way.
    #[serde(default)]
    pub rotation_deg: [f32; 3],
    /// Mirror across X, flipping winding to match. Kits often ship only the
    /// port-side part and expect the starboard copy to be mirrored.
    #[serde(default)]
    pub mirror_x: bool,
    /// Force a winding flip (last resort when a source is authored inside-out).
    #[serde(default)]
    pub flip_winding: bool,
    #[serde(default)]
    pub pivot: PivotPolicy,
    /// Fallback surface for source materials with no explicit binding.
    #[serde(default)]
    pub material: MaterialMapping,
    /// Per-source-material bindings, matched on the material name carried in
    /// the file. A source material with no binding falls back to `material`.
    /// Meshes that carry more than one material (a hull plus its glass
    /// canopy, say) become one asset with one submesh per material, so the
    /// split survives into the pack instead of being flattened away.
    #[serde(default)]
    pub material_bindings: Vec<MaterialBinding>,
    pub category: AssetCategory,
    #[serde(default = "no_collision")]
    pub collision: CollisionProxy,
    #[serde(default)]
    pub sockets: Vec<Socket>,
    #[serde(default = "default_lod")]
    pub lod: LodPolicy,
    pub budget: PerfBudget,
    /// Optional author-declared bounds. When present the importer's measured
    /// bounds are checked against them, which is what makes validation a real
    /// cross-check rather than a tautology.
    #[serde(default)]
    pub declared_bounds: Option<DeclaredBounds>,
    pub provenance: String,
    pub license: String,
    pub gameplay_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl DeclaredBounds {
    /// Bridge into the flat pair [`AssetRecord`] stores. Having one conversion
    /// keeps the declared-vs-measured cross-check from being re-derived (and
    /// re-got-wrong) by every importer.
    pub fn as_record_bounds(&self) -> ([f32; 3], [f32; 3]) {
        (self.min, self.max)
    }

    /// Measured bounds of real geometry, for the case where the author
    /// declared none and the importer records what it found.
    pub fn from_aabb(bounds: Aabb) -> Self {
        Self {
            min: bounds.min.to_array(),
            max: bounds.max.to_array(),
        }
    }

    pub fn is_well_formed(&self) -> bool {
        self.min
            .iter()
            .chain(self.max.iter())
            .all(|c| c.is_finite())
            && self
                .min
                .iter()
                .zip(self.max.iter())
                .all(|(lo, hi)| lo <= hi)
    }
}

fn one() -> f32 {
    1.0
}

fn no_collision() -> CollisionProxy {
    CollisionProxy::None
}

fn default_lod() -> LodPolicy {
    LodPolicy::Single
}

/// A complete, declarative description of an import. This is the file a game
/// authors; the engine ships no art of its own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportManifest {
    #[serde(default)]
    pub sources: Vec<ImportSource>,
    #[serde(default)]
    pub textures: Vec<TextureImport>,
    #[serde(default)]
    pub meshes: Vec<MeshImport>,
}

impl ImportManifest {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn source(&self, id: &str) -> Option<&ImportSource> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// Structural check, run before any file is opened so a typo fails fast
    /// with a useful message instead of a missing-mesh error deep in a bake.
    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        let mut seen_sources: Vec<&str> = Vec::new();
        for source in &self.sources {
            if source.id.trim().is_empty() {
                issues.push(issue("<manifest>", "import source has an empty id"));
            }
            if source.path.trim().is_empty() {
                issues.push(issue(&source.id, "import source has an empty path"));
            }
            if seen_sources.contains(&source.id.as_str()) {
                issues.push(issue(&source.id, "duplicate import source id"));
            }
            seen_sources.push(&source.id);
        }

        let mut seen_textures: Vec<&str> = Vec::new();
        for texture in &self.textures {
            if texture.id.trim().is_empty() {
                issues.push(issue("<manifest>", "texture has an empty id"));
            }
            if texture.path.trim().is_empty() {
                issues.push(issue(&texture.id, "texture has an empty path"));
            }
            if seen_textures.contains(&texture.id.as_str()) {
                issues.push(issue(&texture.id, "duplicate texture id"));
            }
            if matches!(texture.max_size, Some(0)) {
                issues.push(issue(&texture.id, "max_size 0 would erase the texture"));
            }
            seen_textures.push(&texture.id);
        }

        let mut seen_meshes: Vec<&str> = Vec::new();
        for mesh in &self.meshes {
            let id = mesh.id.as_str();
            if id.trim().is_empty() {
                issues.push(issue("<manifest>", "mesh import has an empty id"));
            }
            if seen_meshes.contains(&id) {
                issues.push(issue(id, "duplicate mesh import id"));
            }
            seen_meshes.push(id);

            if self.source(&mesh.source).is_none() {
                issues.push(issue(
                    id,
                    format!("references unknown import source '{}'", mesh.source),
                ));
            }
            if !mesh.unit_scale.is_finite() || mesh.unit_scale <= 0.0 {
                issues.push(issue(
                    id,
                    format!("unit_scale {} is not positive", mesh.unit_scale),
                ));
            }
            if !mesh.rotation_deg.iter().all(|c| c.is_finite()) {
                issues.push(issue(
                    id,
                    format!("rotation_deg {:?} is not finite", mesh.rotation_deg),
                ));
            }
            if let Some(bounds) = &mesh.declared_bounds {
                if !bounds.is_well_formed() {
                    issues.push(issue(
                        id,
                        format!(
                            "declared_bounds {:?}..{:?} are inverted or non-finite",
                            bounds.min, bounds.max
                        ),
                    ));
                }
            }
            for socket in &mesh.sockets {
                let d = socket.direction;
                if !d.iter().all(|c| c.is_finite())
                    || !socket.position.iter().all(|c| c.is_finite())
                {
                    issues.push(issue(
                        id,
                        format!("socket '{}' has non-finite coordinates", socket.name),
                    ));
                    continue;
                }
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if (len - 1.0).abs() > 1e-3 {
                    issues.push(issue(
                        id,
                        format!("socket '{}' direction is not unit length", socket.name),
                    ));
                }
            }
            if let Some(problem) = collision_proxy_problem(mesh.collision) {
                issues.push(issue(id, problem));
            }
            if !mesh.axis.is_well_formed() {
                issues.push(issue(
                    id,
                    "axis convention has parallel up and forward axes",
                ));
            }
            if mesh.budget.max_triangles == 0 || mesh.budget.max_vertices == 0 {
                issues.push(issue(id, "perf budget of zero can never be satisfied"));
            }
            if let LodPolicy::Cull { hide_beyond_m: 0 } = mesh.lod {
                issues.push(issue(id, "a cull distance of 0 hides the asset always"));
            }
            for socket in &mesh.sockets {
                if socket.name.trim().is_empty() {
                    issues.push(issue(id, "socket has an empty name"));
                }
            }
            {
                let mut names: Vec<&str> = Vec::new();
                for socket in &mesh.sockets {
                    if names.contains(&socket.name.as_str()) {
                        issues.push(issue(
                            id,
                            format!("duplicate socket name '{}'", socket.name),
                        ));
                    }
                    names.push(&socket.name);
                }
            }
            for mapping in std::iter::once(&mesh.material)
                .chain(mesh.material_bindings.iter().map(|b| &b.mapping))
            {
                match mapping {
                    MaterialMapping::Atlas { texture } => {
                        if !self.textures.iter().any(|t| &t.id == texture) {
                            issues.push(issue(
                                id,
                                format!("references unknown atlas texture '{texture}'"),
                            ));
                        }
                    }
                    MaterialMapping::Override(desc) => {
                        if let Some(problem) = material_problem(desc) {
                            issues.push(issue(id, problem));
                        }
                    }
                    MaterialMapping::Default => {}
                }
            }
            let mut bound_keys: Vec<String> = Vec::new();
            for binding in &mesh.material_bindings {
                if let MaterialSelector::Name(name) = &binding.select {
                    if name.trim().is_empty() {
                        issues.push(issue(id, "material binding has an empty source name"));
                    }
                }
                let key = binding.key();
                if bound_keys.contains(&key) {
                    issues.push(issue(id, format!("duplicate material binding for {key}")));
                }
                bound_keys.push(key);
            }
            if let MeshSelect::Named(names) = &mesh.select {
                if names.is_empty() {
                    issues.push(issue(id, "Named selection lists no meshes"));
                }
                let mut seen: Vec<&str> = Vec::new();
                for name in names {
                    if name.trim().is_empty() {
                        issues.push(issue(id, "Named selection contains an empty mesh name"));
                    }
                    if seen.contains(&name.as_str()) {
                        issues.push(issue(id, format!("Named selection lists '{name}' twice")));
                    }
                    seen.push(name);
                }
            }
            if mesh.provenance.trim().is_empty() {
                issues.push(issue(id, "provenance record is empty"));
            }
            if mesh.license.trim().is_empty() {
                issues.push(issue(id, "license record is empty"));
            }
            if mesh.gameplay_ref.trim().is_empty() {
                issues.push(issue(id, "gameplay metadata reference is empty"));
            }
        }
        issues
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
        issues.push(issue(
            id,
            format!("unit_scale {} not positive", record.unit_scale),
        ));
    }
    if record.orientation != "y-up,-z-forward" {
        issues.push(issue(
            id,
            format!(
                "orientation '{}' != engine convention 'y-up,-z-forward'",
                record.orientation
            ),
        ));
    }

    // Finiteness is checked HERE, on the record, rather than only on the
    // import manifest — a pack can be constructed or decoded directly, so a
    // check that lives only on the authoring side is a check a bad pack walks
    // straight past. NaN is especially dangerous because every comparison
    // against it is false, so an unchecked NaN reads as "in range".
    if !record.bounds_min.iter().all(|c| c.is_finite())
        || !record.bounds_max.iter().all(|c| c.is_finite())
    {
        issues.push(issue(id, "declared bounds are not finite"));
        return issues; // every geometric check below would silently pass
    }
    if !record.pivot.iter().all(|c| c.is_finite()) {
        issues.push(issue(id, "pivot is not finite"));
    }

    let actual = mesh.bounds().expect("validated mesh has bounds");
    let declared = Aabb {
        min: Vec3::from_array(record.bounds_min),
        max: Vec3::from_array(record.bounds_max),
    };
    if declared.min.cmpgt(declared.max).any() {
        issues.push(issue(
            id,
            format!(
                "declared bounds {:?}..{:?} are inverted",
                declared.min, declared.max
            ),
        ));
    }
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

    // ONE generous "plausible neighbourhood" shared by the pivot and every
    // socket. It is deliberately loose: attachment points and shared-origin
    // pivots legitimately sit well outside the geometry, and a tight bound
    // fails good content. What it still catches is the failure that actually
    // happens — coordinates left in the source file's units, which land
    // orders of magnitude out (a 10 m hull with a socket at 560).
    //
    // The margin uses the LARGEST extent rather than the diagonal so a
    // degenerate axis (a flat decal, a thin sail) does not shrink the
    // allowance to nothing in the very direction the socket sticks out.
    let margin = (actual.extents().max_element() * 2.0).max(2.0);
    let plausible = Aabb {
        min: actual.min - Vec3::splat(margin),
        max: actual.max + Vec3::splat(margin),
    };

    // The pivot is checked for finiteness ONLY — deliberately not for
    // containment.
    //
    // A pivot outside the geometry is normal, not suspicious: modular kit
    // parts are authored around a SHARED ship origin so they snap together,
    // which puts a wing's pivot several part-lengths from the wing. There is
    // no geometric test that separates that from a units mistake, and any
    // distance threshold either rejects real kit parts or is too loose to
    // catch anything. The check that DOES have ground truth is the
    // declared-vs-actual bounds cross-check above, which compares the record
    // against the geometry it describes; this one only produced false bake
    // failures on exactly the content the pipeline exists to import.

    // Sockets get a GENEROUS containment margin, not the tight bounds
    // tolerance. Attachment points legitimately sit proud of the geometry —
    // a gun muzzle ahead of the barrel, a thruster nozzle behind the hull, a
    // docking approach point off the collar — so a tight check produces false
    // bake failures on perfectly good content. The margin still catches the
    // failure that matters: coordinates copied out of a DCC tool in the source
    // file's units, which land orders of magnitude away, not centimetres.
    let mut socket_names: Vec<&str> = Vec::new();
    for socket in &record.sockets {
        if socket.name.trim().is_empty() {
            issues.push(issue(id, "socket has an empty name"));
        }
        if socket_names.contains(&socket.name.as_str()) {
            issues.push(issue(
                id,
                format!("duplicate socket name '{}'", socket.name),
            ));
        }
        socket_names.push(&socket.name);

        let p = Vec3::from_array(socket.position);
        let d = Vec3::from_array(socket.direction);
        if !p.is_finite() || !d.is_finite() {
            issues.push(issue(
                id,
                format!("socket '{}' has non-finite coordinates", socket.name),
            ));
            continue;
        }
        if !plausible.contains(p) {
            issues.push(issue(
                id,
                format!(
                    "socket '{}' at {p:?} is implausibly far from the geometry \
                     (>{margin:.2}m outside) — wrong units?",
                    socket.name
                ),
            ));
        }
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

    if let Some(problem) = collision_proxy_problem(record.collision) {
        issues.push(issue(id, problem));
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

/// Shared collision-proxy sanity, so the authoring side and the packed side
/// cannot drift apart on what counts as a valid proxy. Returns the problem,
/// or `None` when the proxy is usable.
///
/// Non-finite dimensions are reported as such rather than as "non-positive":
/// NaN fails every comparison, so lumping it in with a negative number sends
/// whoever is debugging the bake looking for a minus sign that isn't there.
pub fn collision_proxy_problem(proxy: CollisionProxy) -> Option<String> {
    let non_finite = |vals: &[f32]| vals.iter().any(|v| !v.is_finite());
    match proxy {
        CollisionProxy::Cuboid { half_extents } => {
            if non_finite(&half_extents) {
                Some("cuboid collision proxy has non-finite extents".into())
            } else if half_extents.iter().any(|&e| e <= 0.0) {
                Some("cuboid collision proxy has non-positive extents".into())
            } else {
                None
            }
        }
        CollisionProxy::Ball { radius } => {
            if !radius.is_finite() {
                Some("ball collision proxy has a non-finite radius".into())
            } else if radius <= 0.0 {
                Some("ball collision proxy has non-positive radius".into())
            } else {
                None
            }
        }
        CollisionProxy::CapsuleZ {
            half_height,
            radius,
        } => {
            if non_finite(&[half_height, radius]) {
                Some("capsule collision proxy has non-finite dimensions".into())
            } else if half_height <= 0.0 || radius <= 0.0 {
                Some("capsule collision proxy has non-positive dimensions".into())
            } else {
                None
            }
        }
        CollisionProxy::None => None,
    }
}

/// Shared material sanity. A NaN roughness or an infinite emissive reaches
/// the GPU as undefined behaviour rather than a visible mistake, so both the
/// manifest and the pack screen for it.
pub fn material_problem(desc: &MaterialDesc) -> Option<String> {
    let finite = |vals: &[f32]| vals.iter().all(|v| v.is_finite());
    if !finite(&desc.base_color) || !finite(&desc.emissive) {
        return Some("material colour or emissive is not finite".into());
    }
    if !desc.metallic.is_finite() || !desc.roughness.is_finite() {
        return Some("material metallic/roughness is not finite".into());
    }
    if !(0.0..=1.0).contains(&desc.metallic) {
        return Some(format!("material metallic {} outside 0..=1", desc.metallic));
    }
    if !(0.0..=1.0).contains(&desc.roughness) {
        return Some(format!(
            "material roughness {} outside 0..=1",
            desc.roughness
        ));
    }
    if desc.base_color.iter().any(|&c| c < 0.0) || desc.emissive.iter().any(|&c| c < 0.0) {
        return Some("material colour or emissive is negative".into());
    }
    None
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

    // ----- import manifest -----

    fn mesh_import(id: &str, source: &str) -> MeshImport {
        MeshImport {
            id: id.into(),
            source: source.into(),
            select: MeshSelect::All,
            unit_scale: 1.0,
            axis: AxisConvention::FromSource,
            rotation_deg: [0.0; 3],
            mirror_x: false,
            flip_winding: false,
            pivot: PivotPolicy::AsAuthored,
            material: MaterialMapping::Default,
            material_bindings: vec![],
            category: AssetCategory::Prop,
            collision: CollisionProxy::None,
            sockets: vec![],
            lod: LodPolicy::Single,
            budget: PerfBudget {
                max_triangles: 5_000,
                max_vertices: 10_000,
            },
            declared_bounds: None,
            provenance: "kenney:space-kit".into(),
            license: "CC0".into(),
            gameplay_ref: "test.part".into(),
        }
    }

    fn valid_import_manifest() -> ImportManifest {
        ImportManifest {
            sources: vec![ImportSource {
                id: "racer".into(),
                path: "models/craft_racer.fbx".into(),
                format: SourceFormat::Auto,
            }],
            textures: vec![],
            meshes: vec![mesh_import("asset.racer", "racer")],
        }
    }

    #[test]
    fn import_manifest_round_trips_json() {
        let manifest = valid_import_manifest();
        let json = manifest.to_json().unwrap();
        let parsed = ImportManifest::from_json(&json).unwrap();
        assert_eq!(parsed, manifest);
        assert!(parsed.validate().is_empty());
    }

    #[test]
    fn import_manifest_defaults_keep_authoring_terse() {
        // A game should not have to spell out every correction knob to
        // describe an asset that needs no correction.
        let json = r#"{
            "sources": [{ "id": "s", "path": "a.glb" }],
            "meshes": [{
                "id": "asset.a", "source": "s", "category": "Prop",
                "budget": { "max_triangles": 100, "max_vertices": 200 },
                "provenance": "kenney:space-kit", "license": "CC0",
                "gameplay_ref": "x"
            }]
        }"#;
        let parsed = ImportManifest::from_json(json).unwrap();
        let mesh = &parsed.meshes[0];
        assert_eq!(mesh.unit_scale, 1.0);
        assert_eq!(mesh.axis, AxisConvention::FromSource);
        assert_eq!(mesh.pivot, PivotPolicy::AsAuthored);
        assert!(!mesh.mirror_x);
        assert!(parsed.validate().is_empty());
    }

    #[test]
    fn unknown_source_reference_is_caught_before_any_file_is_opened() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].source = "does-not-exist".into();
        let issues = manifest.validate();
        assert!(issues
            .iter()
            .any(|i| i.message.contains("unknown import source")));
    }

    #[test]
    fn unknown_atlas_texture_is_caught() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].material = MaterialMapping::Atlas {
            texture: "page_missing".into(),
        };
        let issues = manifest.validate();
        assert!(issues
            .iter()
            .any(|i| i.message.contains("unknown atlas texture")));
    }

    #[test]
    fn duplicate_ids_are_caught() {
        let mut manifest = valid_import_manifest();
        manifest.meshes.push(mesh_import("asset.racer", "racer"));
        manifest.sources.push(ImportSource {
            id: "racer".into(),
            path: "other.fbx".into(),
            format: SourceFormat::Auto,
        });
        let issues = manifest.validate();
        assert!(issues.iter().any(|i| i.message.contains("duplicate mesh")));
        assert!(issues
            .iter()
            .any(|i| i.message.contains("duplicate import source")));
    }

    #[test]
    fn nonsense_correction_values_are_caught() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].unit_scale = 0.0;
        manifest.meshes[0].budget.max_triangles = 0;
        let issues = manifest.validate();
        assert!(issues.iter().any(|i| i.message.contains("not positive")));
        assert!(issues.iter().any(|i| i.message.contains("budget of zero")));
    }

    #[test]
    fn licence_and_provenance_are_mandatory_for_imports() {
        // Imported art carries someone else's licence terms; an unattributed
        // asset must not be able to reach a pack silently.
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].license = "  ".into();
        manifest.meshes[0].provenance = String::new();
        let issues = manifest.validate();
        assert!(issues.iter().any(|i| i.message.contains("license")));
        assert!(issues.iter().any(|i| i.message.contains("provenance")));
    }

    #[test]
    fn a_zero_max_size_texture_is_rejected() {
        let mut manifest = valid_import_manifest();
        manifest.textures.push(TextureImport {
            id: "page".into(),
            path: "atlas.png".into(),
            max_size: Some(0),
            sampler: SamplerMode::Nearest,
        });
        let issues = manifest.validate();
        assert!(issues
            .iter()
            .any(|i| i.message.contains("erase the texture")));
    }

    #[test]
    fn an_empty_texture_path_is_rejected() {
        let mut manifest = valid_import_manifest();
        manifest.textures.push(TextureImport {
            id: "page".into(),
            path: "  ".into(),
            max_size: None,
            sampler: SamplerMode::Nearest,
        });
        assert!(manifest
            .validate()
            .iter()
            .any(|i| i.message.contains("empty path")));
    }

    // ----- correction-knob validation (one rejection case per branch) -----

    fn issues_of(manifest: &ImportManifest) -> Vec<String> {
        manifest.validate().into_iter().map(|i| i.message).collect()
    }

    #[test]
    fn a_non_finite_rotation_is_caught_before_any_file_is_opened() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].rotation_deg = [f32::NAN, 0.0, 0.0];
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("rotation_deg")));
    }

    #[test]
    fn inverted_or_non_finite_declared_bounds_are_caught() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].declared_bounds = Some(DeclaredBounds {
            min: [10.0, 10.0, 10.0],
            max: [-10.0, -10.0, -10.0],
        });
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("inverted or non-finite")));

        manifest.meshes[0].declared_bounds = Some(DeclaredBounds {
            min: [f32::NAN; 3],
            max: [1.0; 3],
        });
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("inverted or non-finite")));
    }

    #[test]
    fn a_flat_plate_is_still_well_formed_bounds() {
        // Zero thickness in one axis is legitimate (decals, sails); only
        // inversion and non-finiteness are errors.
        let flat = DeclaredBounds {
            min: [-1.0, 0.0, -1.0],
            max: [1.0, 0.0, 1.0],
        };
        assert!(flat.is_well_formed());
        assert_eq!(flat.as_record_bounds(), (flat.min, flat.max));
    }

    #[test]
    fn declared_bounds_bridge_to_record_bounds() {
        let aabb = Aabb {
            min: Vec3::new(-1.0, -2.0, -3.0),
            max: Vec3::new(1.0, 2.0, 3.0),
        };
        let bounds = DeclaredBounds::from_aabb(aabb);
        assert_eq!(
            bounds.as_record_bounds(),
            ([-1.0, -2.0, -3.0], [1.0, 2.0, 3.0])
        );
    }

    #[test]
    fn bad_socket_directions_and_positions_are_caught() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].sockets = vec![Socket {
            name: "muzzle".into(),
            position: [0.0, 0.0, 0.0],
            direction: [0.0, 0.0, 0.0],
        }];
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("not unit length")));

        manifest.meshes[0].sockets = vec![Socket {
            name: "muzzle".into(),
            position: [f32::INFINITY, 0.0, 0.0],
            direction: [0.0, 0.0, -1.0],
        }];
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("non-finite")));
    }

    #[test]
    fn a_legitimate_normalized_socket_direction_passes() {
        // Guard against the validator degenerating into "reject everything".
        let mut manifest = valid_import_manifest();
        let d = 1.0f32 / 3.0f32.sqrt();
        manifest.meshes[0].sockets = vec![Socket {
            name: "diagonal".into(),
            position: [0.5, 0.5, 0.5],
            direction: [d, d, d],
        }];
        assert_eq!(issues_of(&manifest), Vec::<String>::new());
    }

    #[test]
    fn bad_collision_proxies_are_caught_with_an_accurate_message() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].collision = CollisionProxy::Ball { radius: -1.0 };
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("non-positive radius")));

        // NaN must not be described as "non-positive" — that sends whoever is
        // debugging looking for a minus sign that is not there.
        manifest.meshes[0].collision = CollisionProxy::Cuboid {
            half_extents: [f32::NAN, 1.0, 1.0],
        };
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("non-finite extents")));

        manifest.meshes[0].collision = CollisionProxy::CapsuleZ {
            half_height: 0.0,
            radius: 1.0,
        };
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("non-positive dimensions")));
    }

    #[test]
    fn a_thin_but_real_collision_proxy_passes() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].collision = CollisionProxy::Cuboid {
            half_extents: [0.001, 1.0, 1.0],
        };
        assert_eq!(issues_of(&manifest), Vec::<String>::new());
    }

    #[test]
    fn a_nonsense_override_material_is_caught() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].material = MaterialMapping::Override(Box::new(MaterialDesc {
            roughness: f32::NAN,
            ..Default::default()
        }));
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("metallic/roughness is not finite")));

        manifest.meshes[0].material = MaterialMapping::Override(Box::new(MaterialDesc {
            metallic: 5.0,
            ..Default::default()
        }));
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("metallic 5 outside")));
    }

    #[test]
    fn an_axis_frame_with_parallel_up_and_forward_is_rejected() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].axis = AxisConvention::Explicit {
            up: Axis::PosY,
            forward: Axis::NegY,
            handedness: Handedness::Right,
        };
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("parallel up and forward")));
    }

    #[test]
    fn handedness_is_unknown_until_the_source_is_resolved() {
        // The whole point of the Option: under FromSource the answer lives in
        // the file, and a confident `false` would skip a winding flip that IS
        // needed and hand back inside-out geometry.
        assert_eq!(AxisConvention::FromSource.changes_handedness(), None);
        assert_eq!(AxisConvention::maya().changes_handedness(), Some(false));
        assert_eq!(AxisConvention::blender().changes_handedness(), Some(false));
        assert_eq!(
            AxisConvention::engine_native().changes_handedness(),
            Some(false)
        );
        assert_eq!(AxisConvention::unity().changes_handedness(), Some(true));
        assert_eq!(AxisConvention::unreal().changes_handedness(), Some(true));
    }

    #[test]
    fn the_engine_frame_maps_to_itself() {
        // The identity case is the one that catches a wrong third-axis sign:
        // if the handedness rule were inverted, engine-native would come back
        // mirrored in X rather than as the identity.
        let m = AxisConvention::engine_native().to_engine_basis().unwrap();
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            assert!((m * axis - axis).length() < 1e-6, "{axis:?} moved: {m:?}");
        }
    }

    #[test]
    fn a_z_up_source_becomes_y_up() {
        // Blender: right-handed, Z up, -Y forward. Its up (+Z) must land on
        // the engine's up (+Y), and its forward (-Y) on the engine's -Z.
        let m = AxisConvention::blender().to_engine_basis().unwrap();
        let up = m * Vec3::Z;
        let fwd = m * Vec3::NEG_Y;
        assert!((up - Vec3::Y).length() < 1e-6, "up became {up:?}");
        assert!(
            (fwd - Vec3::NEG_Z).length() < 1e-6,
            "forward became {fwd:?}"
        );
        // Right-handed in, right-handed out: determinant is +1, no mirroring.
        assert!((m.determinant() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_left_handed_source_mirrors_and_says_so() {
        // Unity and Unreal are left-handed; converting mirrors an axis, which
        // is exactly why changes_handedness() drives a winding flip.
        for convention in [AxisConvention::unity(), AxisConvention::unreal()] {
            let m = convention.to_engine_basis().unwrap();
            assert!(
                m.determinant() < 0.0,
                "a left-handed frame must mirror: {convention:?} gave det {}",
                m.determinant()
            );
            assert_eq!(convention.changes_handedness(), Some(true));
        }
        // ...and the up/forward mapping still lands where it should.
        let m = AxisConvention::unreal().to_engine_basis().unwrap();
        assert!((m * Vec3::Z - Vec3::Y).length() < 1e-6);
        assert!((m * Vec3::X - Vec3::NEG_Z).length() < 1e-6);
    }

    #[test]
    fn an_unresolved_or_malformed_frame_has_no_basis() {
        assert!(AxisConvention::FromSource.to_engine_basis().is_none());
        assert!(AxisConvention::Explicit {
            up: Axis::PosY,
            forward: Axis::NegY,
            handedness: Handedness::Right,
        }
        .to_engine_basis()
        .is_none());
    }

    #[test]
    fn unreal_is_z_up_x_forward_not_y_forward() {
        // Named constructors exist because up/forward alone cannot describe
        // every tool: Unreal is the case a Y-forward-only vocabulary gets
        // silently wrong by 90 degrees.
        assert_eq!(
            AxisConvention::unreal(),
            AxisConvention::Explicit {
                up: Axis::PosZ,
                forward: Axis::PosX,
                handedness: Handedness::Left,
            }
        );
        assert!(AxisConvention::unreal().is_well_formed());
    }

    #[test]
    fn material_bindings_can_select_by_index_when_names_are_unusable() {
        // Exporters emit unnamed and duplicate-named materials; an
        // index selector is the only way to address those slots.
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].material_bindings = vec![
            MaterialBinding {
                select: MaterialSelector::Index(0),
                mapping: MaterialMapping::Default,
            },
            MaterialBinding {
                select: MaterialSelector::Index(1),
                mapping: MaterialMapping::Default,
            },
        ];
        assert_eq!(issues_of(&manifest), Vec::<String>::new());
    }

    #[test]
    fn duplicate_material_bindings_are_caught() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].material_bindings = vec![
            MaterialBinding {
                select: MaterialSelector::Name("Glass".into()),
                mapping: MaterialMapping::Default,
            },
            MaterialBinding {
                select: MaterialSelector::Name("Glass".into()),
                mapping: MaterialMapping::Default,
            },
        ];
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("duplicate material binding")));
    }

    #[test]
    fn an_empty_or_duplicated_named_selection_entry_is_caught() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].select =
            MeshSelect::Named(vec!["hull".into(), "hull".into(), "  ".into()]);
        let issues = issues_of(&manifest);
        assert!(issues.iter().any(|m| m.contains("lists 'hull' twice")));
        assert!(issues.iter().any(|m| m.contains("empty mesh name")));
    }

    #[test]
    fn an_empty_gameplay_ref_is_caught() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].gameplay_ref = "  ".into();
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("gameplay metadata reference")));
    }

    #[test]
    fn a_misspelled_manifest_key_is_a_hard_error_not_a_silent_default() {
        // The highest-frequency real failure for a data-driven pipeline: every
        // correction knob has a serde default, so without deny_unknown_fields
        // "mirrorx" would parse fine, mirror nothing, and say nothing.
        let json = r#"{
            "sources": [{ "id": "s", "path": "a.glb" }],
            "meshes": [{
                "id": "asset.a", "source": "s", "category": "Prop",
                "mirrorx": true,
                "budget": { "max_triangles": 100, "max_vertices": 200 },
                "provenance": "kenney:space-kit", "license": "CC0",
                "gameplay_ref": "x"
            }]
        }"#;
        let err = ImportManifest::from_json(json).unwrap_err().to_string();
        assert!(
            err.contains("mirrorx"),
            "error should name the offending key, got: {err}"
        );
    }

    // ----- the asset contract itself -----

    #[test]
    fn a_socket_proud_of_the_hull_is_accepted() {
        // Muzzles, nozzles and docking points legitimately sit outside the
        // mesh. A tight containment check would fail good content.
        let mesh = procmesh::cuboid(2.0, 2.0, 10.0);
        let mut record = record_for(&mesh, "test.hull");
        record.sockets = vec![Socket {
            name: "muzzle".into(),
            position: [0.0, 0.0, -5.6],
            direction: [0.0, 0.0, -1.0],
        }];
        assert_eq!(validate_asset(&record, &mesh), vec![]);
    }

    #[test]
    fn a_socket_in_source_units_is_still_caught() {
        // The failure that matters: coordinates copied straight out of a DCC
        // tool in centimetres land orders of magnitude away.
        let mesh = procmesh::cuboid(2.0, 2.0, 10.0);
        let mut record = record_for(&mesh, "test.hull");
        record.sockets = vec![Socket {
            name: "muzzle".into(),
            position: [0.0, 0.0, -560.0],
            direction: [0.0, 0.0, -1.0],
        }];
        assert!(validate_asset(&record, &mesh)
            .iter()
            .any(|i| i.message.contains("wrong units")));
    }

    #[test]
    fn duplicate_socket_names_are_caught() {
        let mesh = procmesh::cuboid(2.0, 2.0, 2.0);
        let mut record = record_for(&mesh, "test.box");
        record.sockets = vec![
            Socket {
                name: "mount".into(),
                position: [0.0, 0.0, 0.0],
                direction: [0.0, 0.0, -1.0],
            },
            Socket {
                name: "mount".into(),
                position: [0.1, 0.0, 0.0],
                direction: [0.0, 0.0, -1.0],
            },
        ];
        assert!(validate_asset(&record, &mesh)
            .iter()
            .any(|i| i.message.contains("duplicate socket name")));
    }

    #[test]
    fn an_off_origin_kit_piece_keeps_its_authored_pivot() {
        // Modular kit parts are authored around a SHARED ship origin so they
        // snap together: a nacelle at x=+3, a wing at x=-6. A strict pivot
        // containment check rejects every one of them and makes the default
        // AsAuthored policy unusable.
        for offset in [[3.0, 0.0, 0.0], [-6.0, 0.0, 0.0], [0.0, 4.0, 0.0]] {
            let mesh = procmesh::transform_mesh(
                &procmesh::cuboid(1.0, 1.0, 2.0),
                &artificer_scene::TransformDesc::from_translation(Vec3::from_array(offset)),
            );
            let record = record_for(&mesh, "kit.part");
            assert_eq!(
                validate_asset(&record, &mesh),
                vec![],
                "an off-origin kit piece at {offset:?} must validate"
            );
        }
    }

    #[test]
    fn a_non_finite_pivot_is_still_caught() {
        // Containment is deliberately not checked (shared-origin kit parts),
        // but a NaN pivot is unusable by anything downstream.
        let mesh = procmesh::cuboid(1.0, 1.0, 2.0);
        let mut record = record_for(&mesh, "kit.part");
        record.pivot = [f32::NAN, 0.0, 0.0];
        assert!(validate_asset(&record, &mesh)
            .iter()
            .any(|i| i.message.contains("pivot is not finite")));
    }

    #[test]
    fn a_socket_on_a_flat_decal_is_not_squeezed_out_by_the_margin() {
        // The margin uses the largest extent, not the diagonal, so a
        // degenerate axis does not shrink the allowance to nothing in the
        // very direction the socket sticks out.
        let mesh = procmesh::quad_xz(4.0, 4.0);
        let mut record = record_for(&mesh, "decal.flat");
        record.sockets = vec![Socket {
            name: "anchor".into(),
            position: [0.0, 2.0, 0.0],
            direction: [0.0, 1.0, 0.0],
        }];
        assert_eq!(validate_asset(&record, &mesh), vec![]);
    }

    #[test]
    fn a_zero_cull_distance_is_rejected() {
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].lod = LodPolicy::Cull { hide_beyond_m: 0 };
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("hides the asset always")));
    }

    #[test]
    fn socket_name_problems_are_caught_before_any_file_is_opened() {
        // validate_asset catches these too, but only after the FBX has been
        // read -- which defeats the "fails fast" contract.
        let mut manifest = valid_import_manifest();
        manifest.meshes[0].sockets = vec![
            Socket {
                name: "mount".into(),
                position: [0.0; 3],
                direction: [0.0, 0.0, -1.0],
            },
            Socket {
                name: "mount".into(),
                position: [0.0; 3],
                direction: [0.0, 0.0, -1.0],
            },
        ];
        assert!(issues_of(&manifest)
            .iter()
            .any(|m| m.contains("duplicate socket name")));
    }

    #[test]
    fn an_override_material_can_be_authored_by_naming_one_field() {
        // Every other MeshImport knob is optional; Override must be too, or
        // the documented custom-shader path costs seven fields of boilerplate.
        let json = r#"{
            "sources": [{ "id": "s", "path": "a.glb" }],
            "meshes": [{
                "id": "asset.a", "source": "s", "category": "Prop",
                "material": { "Override": { "base_color": [0.1, 0.2, 0.3, 1.0] } },
                "budget": { "max_triangles": 100, "max_vertices": 200 },
                "provenance": "kenney:space-kit", "license": "CC0",
                "gameplay_ref": "x"
            }]
        }"#;
        let parsed = ImportManifest::from_json(json).expect("Override should be terse");
        assert!(parsed.validate().is_empty());
    }

    #[test]
    fn a_misspelled_key_in_a_nested_type_is_also_a_hard_error() {
        // deny_unknown_fields must reach the nested vocabulary, not just the
        // top-level structs -- a typo inside "collision" is just as invisible.
        let json = r#"{
            "sources": [{ "id": "s", "path": "a.glb" }],
            "meshes": [{
                "id": "asset.a", "source": "s", "category": "Prop",
                "collision": { "CapsuleZ": { "half_height": 1.0, "radius": 1.0, "axis": "Y" } },
                "budget": { "max_triangles": 100, "max_vertices": 200 },
                "provenance": "kenney:space-kit", "license": "CC0",
                "gameplay_ref": "x"
            }]
        }"#;
        let err = ImportManifest::from_json(json).unwrap_err().to_string();
        assert!(
            err.contains("axis"),
            "error should name the key, got: {err}"
        );
    }

    #[test]
    fn non_finite_record_bounds_are_caught_by_the_contract() {
        let mesh = procmesh::cuboid(1.0, 1.0, 1.0);
        let mut record = record_for(&mesh, "test.box");
        record.bounds_max = [f32::NAN; 3];
        assert!(validate_asset(&record, &mesh)
            .iter()
            .any(|i| i.message.contains("not finite")));
    }
}
