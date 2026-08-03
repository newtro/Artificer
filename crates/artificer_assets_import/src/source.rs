//! The neutral shape every front-end produces.
//!
//! FBX, OBJ and glTF disagree about almost everything, but the pipeline needs
//! them to disagree in exactly ONE place: the reader. Everything downstream —
//! axis correction, mirroring, pivots, winding, budgets, submesh construction
//! — operates on [`SourceScene`], so a bug fixed for one format is fixed for
//! all of them, and a new format is a new reader rather than a new pipeline.

/// One material's worth of triangles inside a source mesh.
#[derive(Debug, Clone, Default)]
pub struct SourcePart {
    /// Material name as the FILE carries it. Not the name a DCC tool's
    /// sidecar documentation uses: Synty's Sci-Fi Space meshes carry
    /// `SciFiSpace` / `SciFi11` while its MaterialList text file calls them
    /// `PolygonScifiSpace_Material_01_A`. Bindings match this one.
    pub material: Option<String>,
    /// Slot index within the source mesh, for materials that are unnamed or
    /// share a name — both of which real exporters produce.
    pub material_index: u32,
    /// Triangle-list indices into the mesh's vertex arrays.
    pub indices: Vec<u32>,
}

/// One mesh as read, before any correction.
#[derive(Debug, Clone, Default)]
pub struct SourceMesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    /// Empty when the source carried none; the post-processor generates them.
    pub normals: Vec<[f32; 3]>,
    /// Empty when the source carried none.
    pub uvs: Vec<[f32; 2]>,
    /// Triangles grouped by material, in the order materials are first
    /// encountered. Always at least one part.
    pub parts: Vec<SourcePart>,
}

impl SourceMesh {
    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.indices.len()).sum::<usize>() / 3
    }
}

/// Everything a source file contained, in file order.
#[derive(Debug, Clone, Default)]
pub struct SourceScene {
    pub meshes: Vec<SourceMesh>,
    /// What the file said about its own axes and units, when it said
    /// anything. `None` for formats that carry no such metadata (OBJ).
    ///
    /// The importer normalises during reading, so this is informational —
    /// but it is what makes `AxisConvention::FromSource` honest rather than a
    /// guess, and it is what a diagnostic prints when an asset comes out
    /// facing the wrong way.
    pub declared_units_per_metre: Option<f32>,
    pub declared_frame: Option<String>,
    /// Textures carried INSIDE the file, as `(role, encoded bytes)`.
    ///
    /// Generators that emit a single self-contained file -- Tripo returns one
    /// FBX with its colour, normal, roughness and metallic maps embedded and
    /// no separate URLs -- would otherwise need the bytes fishing out by hand.
    /// Doing that by scanning for JPEG magic numbers works right up until it
    /// mislabels which map is which.
    ///
    /// `role` is the stem of the file's own name for the texture, lowercased
    /// (`color`, `normal`, `roughness`, `metallic`), so a manifest can bind by
    /// meaning rather than by index.
    pub embedded_textures: Vec<(String, Vec<u8>)>,
}

impl SourceScene {
    pub fn mesh_named(&self, name: &str) -> Option<&SourceMesh> {
        self.meshes.iter().find(|m| m.name == name)
    }

    pub fn mesh_names(&self) -> Vec<&str> {
        self.meshes.iter().map(|m| m.name.as_str()).collect()
    }
}
