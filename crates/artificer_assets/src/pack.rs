//! The baked asset pack: what a game ships instead of source art.
//!
//! A pack is the output of the import pipeline and the ONLY thing the runtime
//! reads. It carries a small header and a postcard body, so loading is a
//! deserialize rather than a parse, and a browser build never carries an FBX
//! or glTF reader.
//!
//! Three properties are deliberate and all three are tested:
//!
//! * **Deterministic bytes.** Every collection is a sorted `Vec` — no
//!   `HashMap` anywhere — so the same content bakes to the same bytes on any
//!   machine, in any insertion order. Duplicate ids are refused at encode
//!   time, because a stable sort would otherwise let insertion order leak
//!   into the output through equal keys.
//! * **Same contract as procedural assets.** Every packed asset carries an
//!   [`AssetRecord`] and faces [`validate_asset`], so importing cannot smuggle
//!   in geometry that a generated mesh would have been rejected for.
//! * **Order-independent reads.** Lookups and validation never assume the
//!   caller sorted anything. A half-built pack mid-bake answers questions
//!   correctly, so validation failures mean real problems rather than
//!   "you have not called `canonicalize` yet".

use crate::manifest::{validate_asset, AssetRecord, MaterialMapping, SamplerMode, ValidationIssue};
use artificer_scene::{MaterialDesc, MeshData};
use serde::{Deserialize, Serialize};

/// Identifies a file as an Artificer pack before anything is decoded, so a
/// truncated download or an unrelated file fails with a clear message instead
/// of a postcard error from deep inside deserialization.
pub const PACK_MAGIC: [u8; 4] = *b"ARTP";

/// Bumped whenever the pack layout changes in a way older readers cannot
/// handle. A pack that disagrees is rejected rather than misread.
/// Bumped whenever the serialized layout changes.
///
/// v2 added `MaterialDesc::casts_shadows`. Postcard is positional, so a v1
/// reader handed v2 bytes does not fail cleanly -- it misreads every field
/// after that point. This gate is the only thing between a stale pack and
/// nonsense geometry, which is exactly what happened during development: the
/// game silently fell back to primitives until the pack was rebaked.
///
/// v3 added the normal, metallic-roughness and occlusion maps, to
/// `MaterialDesc` and to `PackedMaterial`. Both are positional, so the same
/// rule applies: every pack must be rebaked.
pub const PACK_FORMAT_VERSION: u16 = 3;

/// The eight bytes every PNG starts with.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Read a PNG's real width and height out of its IHDR chunk.
///
/// Checking the signature alone lets signature-prefixed garbage through, and
/// the blob is handed straight to an image decoder at load — so a mislabelled
/// texture would surface as a broken page in a browser rather than as a bake
/// failure. IHDR is always the first chunk and always at a fixed offset, so
/// this needs no image decoder of its own.
fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if !bytes.starts_with(&PNG_SIGNATURE) {
        return Err("texture blob is not a PNG (bad signature)".into());
    }
    if bytes.len() < 24 {
        return Err("texture blob is too short to hold a PNG header".into());
    }
    if &bytes[12..16] != b"IHDR" {
        return Err("texture blob has no IHDR chunk where a PNG must have one".into());
    }
    let read = |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let (w, h) = (read(16), read(20));
    if w == 0 || h == 0 {
        return Err(format!("PNG header declares a {w}x{h} image"));
    }
    Ok((w, h))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureBlob {
    pub id: String,
    /// Encoded PNG. Kept encoded so the pack stays small and the renderer can
    /// hand the bytes straight to its own image decoder.
    pub png: Vec<u8>,
    pub sampler: SamplerMode,
    pub width: u32,
    pub height: u32,
}

/// A material as baked: engine-neutral description plus an optional reference
/// to a [`TextureBlob`] by id.
///
/// The texture reference lives HERE and only here. [`MaterialDesc`] describes
/// surface response (colour, metallic, roughness, emissive); binding it to an
/// actual image is the runtime's job at load, when texture ids have become
/// real handles. Keeping one owner avoids the two-sources-of-truth problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackedMaterial {
    pub id: String,
    pub desc: MaterialDesc,
    pub base_color_texture: Option<String>,
    /// Tangent-space normal map, by [`TextureBlob::id`].
    ///
    /// Held beside the base colour rather than inside `desc` for the same
    /// reason that one is: the pack names textures with STRINGS and the
    /// loader turns them into handles, so a bake never has to invent id
    /// numbers that would differ between runs.
    #[serde(default)]
    pub normal_texture: Option<String>,
    /// Combined metallic-roughness map (glTF packing: roughness G, metallic B).
    #[serde(default)]
    pub metallic_roughness_texture: Option<String>,
    /// Baked ambient occlusion (R).
    #[serde(default)]
    pub occlusion_texture: Option<String>,
}

/// One material's slice of an asset's index buffer.
///
/// Most assets are a single submesh — the whole point of an atlas is that a
/// model needs only one material. Multi-material sources (a hull plus its
/// glass canopy) keep their split instead of being flattened, so the runtime
/// can still draw them correctly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubMesh {
    /// [`PackedMaterial::id`], or `None` for the engine default material.
    pub material: Option<String>,
    /// Offset into [`PackedAsset::mesh`]'s index buffer.
    pub index_start: u32,
    pub index_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackedAsset {
    pub record: AssetRecord,
    pub mesh: MeshData,
    /// Index ranges by material. Always at least one entry covering the whole
    /// mesh; [`PackedAsset::single`] builds that common case.
    pub submeshes: Vec<SubMesh>,
}

impl PackedAsset {
    /// The common case: one mesh, one material, one draw.
    ///
    /// Fallible because submesh ranges are `u32`: a mesh with more indices
    /// than `u32::MAX` cannot be described, and truncating with `as u32`
    /// would silently produce an asset whose submesh covers a fraction of its
    /// geometry.
    pub fn single(
        record: AssetRecord,
        mesh: MeshData,
        material: Option<String>,
    ) -> Result<Self, PackError> {
        let index_count = u32::try_from(mesh.indices.len())
            .map_err(|_| PackError::MeshTooLarge(mesh.indices.len()))?;
        Ok(Self {
            record,
            mesh,
            submeshes: vec![SubMesh {
                material,
                index_start: 0,
                index_count,
            }],
        })
    }

    /// Material ids in submesh order. May repeat when several submeshes share
    /// a material, and omits submeshes drawn with the engine default.
    pub fn material_ids(&self) -> impl Iterator<Item = &str> {
        self.submeshes.iter().filter_map(|s| s.material.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetPack {
    /// Carried in the FILE HEADER, not in the serialized body. One authority
    /// means a header and body can never disagree about what a pack is.
    #[serde(skip)]
    pub format_version: u16,
    pub assets: Vec<PackedAsset>,
    pub materials: Vec<PackedMaterial>,
    pub textures: Vec<TextureBlob>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    Encode(String),
    Decode(String),
    NotAPack,
    TrailingBytes(usize),
    DuplicateId(String),
    MeshTooLarge(usize),
    MaterialConflict { id: String },
    Invalid(Vec<ValidationIssue>),
    UnsupportedVersion { found: u16, expected: u16 },
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::Encode(e) => write!(f, "pack encode failed: {e}"),
            PackError::Decode(e) => write!(f, "pack decode failed: {e}"),
            PackError::NotAPack => write!(f, "not an Artificer asset pack (bad magic)"),
            PackError::TrailingBytes(n) => {
                write!(f, "{n} unexpected bytes after the pack body")
            }
            PackError::DuplicateId(id) => {
                write!(f, "duplicate id '{id}' makes the bake non-deterministic")
            }
            PackError::MeshTooLarge(n) => {
                write!(
                    f,
                    "mesh has {n} indices, more than a u32 submesh range can address"
                )
            }
            PackError::MaterialConflict { id } => write!(
                f,
                "material id '{id}' is claimed by two different descriptions"
            ),
            PackError::Invalid(issues) => {
                write!(f, "pack failed validation ({} issues):", issues.len())?;
                for i in issues {
                    write!(
                        f,
                        "
  {}: {}",
                        i.asset_id, i.message
                    )?;
                }
                Ok(())
            }
            PackError::UnsupportedVersion { found, expected } => write!(
                f,
                "pack format version {found} is not readable by this build (expects {expected})"
            ),
        }
    }
}

impl std::error::Error for PackError {}

impl Default for AssetPack {
    /// Derived `Default` would give `format_version: 0`, which encodes into a
    /// pack this very build then refuses to load. `Default` and `new` must
    /// agree.
    fn default() -> Self {
        Self {
            format_version: PACK_FORMAT_VERSION,
            assets: Vec::new(),
            materials: Vec::new(),
            textures: Vec::new(),
        }
    }
}

impl AssetPack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lookups scan rather than binary-search. Packs hold tens to hundreds of
    /// assets, so the cost is irrelevant, and it removes a whole class of bug
    /// where an unsorted (mid-bake) pack answers "missing" for things it holds.
    pub fn find(&self, id: &str) -> Option<&PackedAsset> {
        self.assets.iter().find(|a| a.record.id == id)
    }

    pub fn material(&self, id: &str) -> Option<&PackedMaterial> {
        self.materials.iter().find(|m| m.id == id)
    }

    pub fn texture(&self, id: &str) -> Option<&TextureBlob> {
        self.textures.iter().find(|t| t.id == id)
    }

    /// True when the pack holds no assets. Materials and textures without an
    /// asset to draw are meaningless, so assets are the measure.
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }

    /// Number of ASSETS (not materials or textures).
    pub fn len(&self) -> usize {
        self.assets.len()
    }

    /// Sort every collection by id, so the encoded bytes depend on content
    /// and not on the order things were added.
    pub fn canonicalize(&mut self) {
        self.assets.sort_by(|a, b| a.record.id.cmp(&b.record.id));
        self.materials.sort_by(|a, b| a.id.cmp(&b.id));
        self.textures.sort_by(|a, b| a.id.cmp(&b.id));
        // Submeshes too. Validation deliberately accepts them in any order
        // (an importer emits them in encounter order), so without this the
        // same content encodes to different bytes depending on that order —
        // which is exactly the determinism guarantee this method exists for.
        for asset in &mut self.assets {
            asset.submeshes.sort_by_key(|s| s.index_start);
            // Semantically unordered record vectors, for the same reason:
            // reversing either must not change the file. material_slots is
            // documented as sorted, so this is also what makes that true.
            asset.record.material_slots.sort();
            asset.record.material_slots.dedup();
            asset.record.sockets.sort_by(|a, b| a.name.cmp(&b.name));
        }
    }

    /// First duplicate id across any collection, if there is one. Duplicates
    /// break determinism: `sort_by` is stable, so equal keys keep insertion
    /// order and two bakes of the same content can differ.
    fn first_duplicate(&self) -> Option<String> {
        fn dup(mut ids: Vec<&str>) -> Option<&str> {
            ids.sort_unstable();
            ids.windows(2).find(|w| w[0] == w[1]).map(|w| w[0])
        }
        dup(self.assets.iter().map(|a| a.record.id.as_str()).collect())
            .or_else(|| dup(self.materials.iter().map(|m| m.id.as_str()).collect()))
            .or_else(|| dup(self.textures.iter().map(|t| t.id.as_str()).collect()))
            .map(str::to_string)
    }

    /// Canonicalize and encode, with a magic + version header ahead of the
    /// postcard body. Encoding always canonicalizes, so callers cannot
    /// accidentally emit an order-dependent pack.
    pub fn to_postcard(&self) -> Result<Vec<u8>, PackError> {
        if let Some(id) = self.first_duplicate() {
            return Err(PackError::DuplicateId(id));
        }
        let mut canonical = self.clone();
        canonical.canonicalize();
        let body = postcard::to_stdvec(&canonical).map_err(|e| PackError::Encode(e.to_string()))?;

        let mut out = Vec::with_capacity(body.len() + 6);
        out.extend_from_slice(&PACK_MAGIC);
        out.extend_from_slice(&self.format_version.to_le_bytes());
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Encode at the CURRENT format version regardless of what the in-memory
    /// value says. This is what a bake calls; `to_postcard` honours the field
    /// so tests can construct a pack from another version honestly.
    pub fn to_postcard_current(&self) -> Result<Vec<u8>, PackError> {
        let issues = self.validate();
        if !issues.is_empty() {
            return Err(PackError::Invalid(issues));
        }
        let mut stamped = self.clone();
        stamped.format_version = PACK_FORMAT_VERSION;
        stamped.to_postcard()
    }

    pub fn from_postcard(bytes: &[u8]) -> Result<Self, PackError> {
        if bytes.len() < 6 || bytes[..4] != PACK_MAGIC {
            return Err(PackError::NotAPack);
        }
        // Version is checked BEFORE decoding, so a pack from a newer build
        // reports its version instead of surfacing as a confusing layout error.
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != PACK_FORMAT_VERSION {
            return Err(PackError::UnsupportedVersion {
                found: version,
                expected: PACK_FORMAT_VERSION,
            });
        }
        let (mut pack, rest): (AssetPack, &[u8]) =
            postcard::take_from_bytes(&bytes[6..]).map_err(|e| PackError::Decode(e.to_string()))?;
        if !rest.is_empty() {
            // postcard stops at the end of the value and ignores anything
            // after it; a truncated-then-concatenated file must not read as OK.
            return Err(PackError::TrailingBytes(rest.len()));
        }
        // The body carries no version (it is `#[serde(skip)]`), so the header
        // is what tells the in-memory pack which format it came from.
        pack.format_version = version;
        Ok(pack)
    }

    /// Submeshes must EXACTLY TILE the index buffer: sorted by start, each
    /// beginning where the previous ended, starting at 0 and ending at the
    /// last index, every boundary on a triangle.
    ///
    /// Checking only that the counts SUM to the index count is not enough,
    /// and that mistake is easy to make: `[{start:0,count:18},
    /// {start:0,count:18}]` sums correctly on a 36-index mesh while drawing
    /// the first half twice and the second half never. A misaligned start
    /// like `{start:1,count:33}` sums correctly too, and stitches every
    /// triangle from the wrong three indices.
    fn submesh_issues(&self, asset: &PackedAsset) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        let id = &asset.record.id;
        let mut push = |message: String| {
            issues.push(ValidationIssue {
                asset_id: id.clone(),
                message,
            })
        };

        let Ok(indices) = u32::try_from(asset.mesh.indices.len()) else {
            push(format!(
                "mesh has {} indices, beyond what a u32 submesh range can address",
                asset.mesh.indices.len()
            ));
            return issues;
        };

        if asset.submeshes.is_empty() {
            push("asset has no submesh covering its indices".into());
            return issues;
        }

        let mut ranges: Vec<(u32, u32, usize)> = asset
            .submeshes
            .iter()
            .enumerate()
            .map(|(i, s)| (s.index_start, s.index_count, i))
            .collect();
        ranges.sort_unstable();

        let mut cursor = 0u32;
        for (start, count, i) in ranges {
            if count == 0 {
                push(format!(
                    "submesh {i} is empty — a draw call that draws nothing"
                ));
                continue;
            }
            if !start.is_multiple_of(3) || !count.is_multiple_of(3) {
                push(format!(
                    "submesh {i} spans {start}..+{count}, which does not fall on triangle \
                     boundaries"
                ));
            }
            let Some(end) = start.checked_add(count) else {
                push(format!("submesh {i} range {start}..+{count} overflows"));
                continue;
            };
            if end > indices {
                push(format!(
                    "submesh {i} spans {start}..{end} beyond the {indices}-index buffer"
                ));
                continue;
            }
            if start != cursor {
                push(format!(
                    "submesh {i} starts at {start} but the previous one ended at {cursor} — \
                     submeshes must tile the index buffer with no gaps or overlaps"
                ));
            }
            cursor = cursor.max(end);
        }
        if cursor != indices {
            push(format!(
                "submeshes cover {cursor} of {indices} indices — geometry would be partly undrawn"
            ));
        }

        for sub in &asset.submeshes {
            if let Some(material) = &sub.material {
                if self.material(material).is_none() {
                    push(format!("references missing material '{material}'"));
                }
            }
        }

        // `material_slots` is the contract's view of "what surfaces does this
        // asset need"; `submeshes` is the runtime's. Two representations of
        // the same fact must not be allowed to drift.
        if !asset.record.material_slots.is_empty() {
            let mut declared = asset.record.material_slots.clone();
            declared.sort();
            declared.dedup();
            let mut actual: Vec<String> = asset.material_ids().map(str::to_string).collect();
            actual.sort();
            actual.dedup();
            if declared != actual {
                push(format!(
                    "record declares material slots {declared:?} but submeshes draw with \
                     {actual:?}"
                ));
            }
        }

        issues
    }

    /// Decode AND validate.
    ///
    /// [`AssetPack::from_postcard`] is deliberately decode-only: a pack is a
    /// build artifact produced by a bake that already validated it, not
    /// untrusted input, and re-validating every mesh on a browser cold start
    /// costs load time for a check the bake already made. Use this when the
    /// bytes came from somewhere less certain.
    pub fn from_postcard_validated(bytes: &[u8]) -> Result<Self, PackError> {
        let pack = Self::from_postcard(bytes)?;
        let issues = pack.validate();
        if !issues.is_empty() {
            return Err(PackError::Invalid(issues));
        }
        Ok(pack)
    }

    /// Run the §11.2 asset contract over every asset, plus pack-level
    /// referential integrity. Empty result = the whole pack passes.
    ///
    /// Order-independent: valid on a half-built pack mid-bake.
    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();

        if let Some(id) = self.first_duplicate() {
            issues.push(ValidationIssue {
                asset_id: id,
                message: "duplicate id in pack (breaks deterministic bakes)".into(),
            });
        }

        for asset in &self.assets {
            issues.extend(validate_asset(&asset.record, &asset.mesh));

            issues.extend(self.submesh_issues(asset));
        }

        for material in &self.materials {
            if material.id.trim().is_empty() {
                issues.push(ValidationIssue {
                    asset_id: "<pack>".into(),
                    message: "material has an empty id".into(),
                });
            }
            if let Some(problem) = crate::manifest::material_problem(&material.desc) {
                issues.push(ValidationIssue {
                    asset_id: material.id.clone(),
                    message: problem,
                });
            }
            // EVERY slot, not just base colour. A dangling normal map does
            // not fail loudly at runtime -- it warns once and the surface
            // renders flat, which is indistinguishable from never having
            // authored one. Catching it here makes a typo a bake failure.
            for (slot, what) in [
                (&material.base_color_texture, "texture"),
                (&material.normal_texture, "normal texture"),
                (
                    &material.metallic_roughness_texture,
                    "metallic-roughness texture",
                ),
                (&material.occlusion_texture, "occlusion texture"),
            ] {
                let Some(texture) = slot else { continue };
                if self.texture(texture).is_none() {
                    issues.push(ValidationIssue {
                        asset_id: material.id.clone(),
                        message: format!("references missing {what} '{texture}'"),
                    });
                }
            }
        }
        // A texture cannot be both colour and data. Colour space is fixed
        // when the image is decoded, and one blob yields one decode, so a
        // page used as base colour by one material and as a normal map by
        // another would silently corrupt whichever role lost. Refuse it here
        // instead: the fix is to bake the same bytes under two ids.
        {
            let mut as_color: Vec<&str> = Vec::new();
            let mut as_data: Vec<&str> = Vec::new();
            for material in &self.materials {
                if let Some(id) = &material.base_color_texture {
                    as_color.push(id);
                }
                for slot in [
                    &material.normal_texture,
                    &material.metallic_roughness_texture,
                    &material.occlusion_texture,
                ]
                .into_iter()
                .flatten()
                {
                    as_data.push(slot);
                }
            }
            as_data.sort_unstable();
            as_data.dedup();
            for id in as_data {
                if as_color.contains(&id) {
                    issues.push(ValidationIssue {
                        asset_id: "<pack>".into(),
                        message: format!(
                            "texture '{id}' is used as BOTH base colour and a data map;                              colour space is fixed per texture, so bake it under two ids"
                        ),
                    });
                }
            }
        }

        for texture in &self.textures {
            if texture.id.trim().is_empty() {
                issues.push(ValidationIssue {
                    asset_id: "<pack>".into(),
                    message: "texture has an empty id".into(),
                });
            }
            if texture.png.is_empty() {
                issues.push(ValidationIssue {
                    asset_id: texture.id.clone(),
                    message: "texture blob carries no bytes".into(),
                });
            } else {
                match png_dimensions(&texture.png) {
                    Err(problem) => issues.push(ValidationIssue {
                        asset_id: texture.id.clone(),
                        message: problem,
                    }),
                    Ok((w, h)) if (w, h) != (texture.width, texture.height) => {
                        issues.push(ValidationIssue {
                            asset_id: texture.id.clone(),
                            message: format!(
                                "texture declares {}x{} but the PNG is {w}x{h}",
                                texture.width, texture.height
                            ),
                        })
                    }
                    Ok(_) => {}
                }
            }
            if texture.width == 0 || texture.height == 0 {
                issues.push(ValidationIssue {
                    asset_id: texture.id.clone(),
                    message: format!(
                        "texture declares a {}x{} size",
                        texture.width, texture.height
                    ),
                });
            }
        }
        issues
    }

    /// Measured baked size. `encoded_bytes` is the REAL encoded length — the
    /// number a browser-first project has to keep honest — obtained by
    /// actually encoding. The per-kind figures are in-memory estimates for
    /// attribution only and do not sum to the total (postcard varint-encodes
    /// indices, so small meshes cost far less on disk than in RAM).
    pub fn size_report(&self) -> Result<PackSizeReport, PackError> {
        let mesh_bytes: usize = self
            .assets
            .iter()
            .map(|a| {
                a.mesh.positions.len() * 12
                    + a.mesh.normals.len() * 12
                    + a.mesh.uvs.len() * 8
                    + a.mesh.indices.len() * 4
            })
            .sum();
        let texture_bytes: usize = self.textures.iter().map(|t| t.png.len()).sum();
        // Encoding can fail (duplicate ids), and reporting "0 bytes baked" for
        // a pack that cannot bake is worse than saying so.
        let encoded_bytes = self.to_postcard_current()?.len();
        Ok(PackSizeReport {
            assets: self.assets.len(),
            materials: self.materials.len(),
            textures: self.textures.len(),
            encoded_bytes,
            mesh_bytes_in_memory: mesh_bytes,
            texture_bytes,
            triangles: self.assets.iter().map(|a| a.mesh.triangle_count()).sum(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackSizeReport {
    pub assets: usize,
    pub materials: usize,
    pub textures: usize,
    /// True encoded length of the pack file.
    pub encoded_bytes: usize,
    /// In-memory geometry estimate (attribution only; not the on-disk cost).
    pub mesh_bytes_in_memory: usize,
    /// PNG bytes, which ARE carried verbatim into the file.
    pub texture_bytes: usize,
    pub triangles: usize,
}

impl std::fmt::Display for PackSizeReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} assets ({} tris), {} materials, {} textures ({:.1} KB png) \
             -> {:.1} KB baked",
            self.assets,
            self.triangles,
            self.materials,
            self.textures,
            self.texture_bytes as f32 / 1024.0,
            self.encoded_bytes as f32 / 1024.0,
        )
    }
}

/// Resolve a material mapping to the material id it should reference in the
/// pack, registering the material if it is new. Shared by the importer and by
/// tests so both agree on atlas material naming.
///
/// `slot` distinguishes the materials of a multi-material asset; pass the
/// source material name, or an empty string for a single-material asset.
pub fn material_for(
    pack: &mut AssetPack,
    asset_id: &str,
    slot: &str,
    mapping: &MaterialMapping,
) -> Result<Option<String>, PackError> {
    match mapping {
        MaterialMapping::Default => Ok(None),
        MaterialMapping::Atlas { texture } => {
            // One material per atlas page, shared by every asset that uses it
            // — this is what collapses a whole content library into a handful
            // of draw calls.
            let id = format!("atlas.{texture}");
            let desc = MaterialDesc {
                base_color: [1.0, 1.0, 1.0, 1.0],
                metallic: 0.0,
                roughness: 0.75,
                ..Default::default()
            };
            register(
                pack,
                id,
                desc,
                MaterialTextures::base(Some(texture.clone())),
            )
        }
        MaterialMapping::Pbr {
            base_color,
            normal,
            metallic_roughness,
            occlusion,
        } => {
            // Keyed on the WHOLE set, not just base colour: two assets that
            // share a colour page but carry different normal maps are
            // different materials, and collapsing them would paint one hull
            // with another's relief.
            let key = [
                base_color.as_str(),
                normal.as_deref().unwrap_or("-"),
                metallic_roughness.as_deref().unwrap_or("-"),
                occlusion.as_deref().unwrap_or("-"),
            ]
            .join("+");
            let id = format!("pbr.{key}");
            let desc = MaterialDesc {
                base_color: [1.0, 1.0, 1.0, 1.0],
                // Unit scalars: the maps carry the variation, and a non-unit
                // multiplier here would quietly darken every generated asset.
                metallic: 1.0,
                roughness: 1.0,
                ..Default::default()
            };
            register(
                pack,
                id,
                desc,
                MaterialTextures {
                    base_color: Some(base_color.clone()),
                    normal: normal.clone(),
                    metallic_roughness: metallic_roughness.clone(),
                    occlusion: occlusion.clone(),
                },
            )
        }
        MaterialMapping::Override(desc) => {
            let id = if slot.is_empty() {
                format!("mat.{asset_id}")
            } else {
                format!("mat.{asset_id}.{slot}")
            };
            register(pack, id, **desc, MaterialTextures::default())
        }
    }
}

/// Register a material, or reuse an identical existing one.
///
/// Readable ids like `mat.<asset>.<slot>` can collide — `("ship", "a.glass")`
/// and `("ship.a", "glass")` both render as `mat.ship.a.glass`. Rather than
/// mangle ids into something collision-proof but unreadable in logs, a
/// collision is DETECTED: reusing an id with a different description is an
/// error instead of silently discarding one of them (which would also make
/// the baked bytes depend on encounter order).
/// Every texture slot of one material, by [`TextureBlob::id`].
///
/// Passed as a group so a new map cannot be added to the pack and then
/// forgotten in the conflict check below -- two materials differing only in
/// their normal map would otherwise be treated as identical and silently
/// share one entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterialTextures {
    pub base_color: Option<String>,
    pub normal: Option<String>,
    pub metallic_roughness: Option<String>,
    pub occlusion: Option<String>,
}

impl MaterialTextures {
    pub fn base(texture: Option<String>) -> Self {
        Self {
            base_color: texture,
            ..Default::default()
        }
    }
}

fn register(
    pack: &mut AssetPack,
    id: String,
    desc: MaterialDesc,
    textures: MaterialTextures,
) -> Result<Option<String>, PackError> {
    if let Some(existing) = pack.materials.iter().find(|m| m.id == id) {
        let same = existing.desc == desc
            && existing.base_color_texture == textures.base_color
            && existing.normal_texture == textures.normal
            && existing.metallic_roughness_texture == textures.metallic_roughness
            && existing.occlusion_texture == textures.occlusion;
        if !same {
            return Err(PackError::MaterialConflict { id });
        }
        return Ok(Some(id));
    }
    pack.materials.push(PackedMaterial {
        id: id.clone(),
        desc,
        base_color_texture: textures.base_color,
        normal_texture: textures.normal,
        metallic_roughness_texture: textures.metallic_roughness,
        occlusion_texture: textures.occlusion,
    });
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{AssetCategory, CollisionProxy, LodPolicy, PerfBudget, Socket};
    use crate::procmesh;

    fn record(id: &str, mesh: &MeshData) -> AssetRecord {
        let b = mesh.bounds().unwrap();
        AssetRecord {
            id: id.into(),
            category: AssetCategory::Prop,
            unit_scale: 1.0,
            orientation: "y-up,-z-forward".into(),
            pivot: [0.0; 3],
            bounds_min: b.min.to_array(),
            bounds_max: b.max.to_array(),
            collision: CollisionProxy::None,
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

    fn asset(id: &str) -> PackedAsset {
        let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
        PackedAsset::single(record(id, &mesh), mesh, None).unwrap()
    }

    fn atlas(texture: &str) -> MaterialMapping {
        MaterialMapping::Atlas {
            texture: texture.into(),
        }
    }

    /// A real PNG header (signature + IHDR) declaring `w` x `h`. Pixel data
    /// is not needed: what the pack checks is that the header is genuine and
    /// agrees with the declared size.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend_from_slice(&13u32.to_be_bytes()); // IHDR length
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&w.to_be_bytes());
        png.extend_from_slice(&h.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]); // depth, colour type, etc.
        png.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
        png
    }

    fn texture_blob(id: &str) -> TextureBlob {
        TextureBlob {
            id: id.into(),
            png: png_header(2, 2),
            sampler: SamplerMode::Nearest,
            width: 2,
            height: 2,
        }
    }

    fn cuboid_indices() -> u32 {
        procmesh::cuboid(1.0, 2.0, 3.0).indices.len() as u32
    }

    // ----- round-trip and framing -----

    #[test]
    fn pack_round_trips_postcard() {
        let mut pack = AssetPack::new();
        pack.assets.push(asset("a.one"));
        pack.assets.push(asset("a.two"));
        let bytes = pack.to_postcard().unwrap();
        let back = AssetPack::from_postcard(&bytes).unwrap();
        assert_eq!(back.len(), 2);
        assert!(back.find("a.one").is_some());
        assert!(back.find("a.two").is_some());
        assert_eq!(back.format_version, PACK_FORMAT_VERSION);
    }

    #[test]
    fn an_empty_pack_round_trips() {
        let bytes = AssetPack::new().to_postcard().unwrap();
        assert!(AssetPack::from_postcard(&bytes).unwrap().is_empty());
    }

    #[test]
    fn a_future_pack_version_is_rejected_not_misread() {
        let mut pack = AssetPack::new();
        pack.assets.push(asset("a.one"));
        // The version is a real header field, so a future pack can be built
        // honestly rather than by poking bytes.
        pack.format_version = PACK_FORMAT_VERSION + 7;
        let bytes = pack.to_postcard().unwrap();
        assert!(matches!(
            AssetPack::from_postcard(&bytes),
            Err(PackError::UnsupportedVersion { found, expected })
                if found == PACK_FORMAT_VERSION + 7 && expected == PACK_FORMAT_VERSION
        ));
    }

    #[test]
    fn default_and_new_agree_so_a_default_pack_can_be_read_back() {
        // Regression: a derived Default left format_version at 0, which encoded
        // into a pack this same build then refused to load.
        assert_eq!(AssetPack::default(), AssetPack::new());
        let mut pack = AssetPack::default();
        pack.assets.push(asset("a.one"));
        let bytes = pack.to_postcard().unwrap();
        assert!(AssetPack::from_postcard(&bytes).is_ok());
    }

    #[test]
    fn the_header_is_the_only_version_authority() {
        // The body carries no version of its own, so a header and body can
        // never disagree about what the file is.
        let mut pack = AssetPack::new();
        pack.assets.push(asset("a.one"));
        let bytes = pack.to_postcard().unwrap();
        let body_only: AssetPack = postcard::from_bytes(&bytes[6..]).unwrap();
        assert_eq!(body_only.format_version, 0, "body carries no version");
        assert_eq!(
            AssetPack::from_postcard(&bytes).unwrap().format_version,
            PACK_FORMAT_VERSION,
            "the header supplies it"
        );
    }

    #[test]
    fn to_postcard_current_stamps_the_build_version() {
        let mut pack = AssetPack::new();
        pack.format_version = 999;
        let bytes = pack.to_postcard_current().unwrap();
        assert_eq!(
            AssetPack::from_postcard(&bytes).unwrap().format_version,
            PACK_FORMAT_VERSION
        );
    }

    #[test]
    fn a_non_pack_file_is_rejected_by_magic() {
        assert_eq!(
            AssetPack::from_postcard(b"not a pack at all"),
            Err(PackError::NotAPack)
        );
        assert_eq!(AssetPack::from_postcard(&[]), Err(PackError::NotAPack));
        assert_eq!(AssetPack::from_postcard(b"ARTP"), Err(PackError::NotAPack));
    }

    #[test]
    fn trailing_bytes_are_rejected_rather_than_ignored() {
        // postcard stops at the end of a value; a concatenated or corrupted
        // file must not read as a valid pack.
        let mut pack = AssetPack::new();
        pack.assets.push(asset("a.one"));
        let mut bytes = pack.to_postcard().unwrap();
        bytes.extend_from_slice(b"garbage");
        assert!(matches!(
            AssetPack::from_postcard(&bytes),
            Err(PackError::TrailingBytes(7))
        ));
    }

    // ----- determinism -----

    #[test]
    fn insertion_order_does_not_change_the_bytes() {
        // Reversed across ALL THREE collections, not just assets.
        fn build(reverse: bool) -> AssetPack {
            let mut pack = AssetPack::new();
            let mut asset_ids = ["a.one", "a.two", "a.three"];
            let mut pages = ["page_a", "page_b", "page_c"];
            if reverse {
                asset_ids.reverse();
                pages.reverse();
            }
            for page in &pages {
                pack.textures.push(texture_blob(page));
            }
            for (i, id) in asset_ids.iter().enumerate() {
                let material = material_for(&mut pack, id, "", &atlas(pages[i])).unwrap();
                let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
                pack.assets
                    .push(PackedAsset::single(record(id, &mesh), mesh, material).unwrap());
            }
            pack
        }
        assert_eq!(
            build(false).to_postcard().unwrap(),
            build(true).to_postcard().unwrap()
        );
    }

    #[test]
    fn two_independently_built_packs_are_byte_identical() {
        fn build() -> AssetPack {
            let mut pack = AssetPack::new();
            pack.textures.push(texture_blob("page_a"));
            for id in ["a.two", "a.one"] {
                let material = material_for(&mut pack, id, "", &atlas("page_a")).unwrap();
                let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
                pack.assets
                    .push(PackedAsset::single(record(id, &mesh), mesh, material).unwrap());
            }
            pack
        }
        assert_eq!(
            build().to_postcard().unwrap(),
            build().to_postcard().unwrap()
        );
    }

    #[test]
    fn duplicate_ids_are_refused_rather_than_silently_reordered() {
        // A stable sort keeps equal keys in insertion order, so duplicates
        // would let insertion order leak into supposedly canonical bytes.
        let mut pack = AssetPack::new();
        pack.assets.push(asset("a.same"));
        pack.assets.push(asset("a.same"));
        assert!(matches!(
            pack.to_postcard(),
            Err(PackError::DuplicateId(id)) if id == "a.same"
        ));
        assert!(pack
            .validate()
            .iter()
            .any(|i| i.message.contains("duplicate id")));
    }

    // ----- order independence (the A1 blocker) -----

    #[test]
    fn lookups_and_validation_do_not_require_a_sorted_pack() {
        // Regression: validate() used to binary-search unsorted vectors and
        // report present materials as missing. A bake builds packs in
        // encounter order, so this is the normal case, not an exotic one.
        let mut pack = AssetPack::new();
        for page in ["page_z", "page_a"] {
            pack.textures.push(texture_blob(page));
        }
        let mz = material_for(&mut pack, "z.last", "", &atlas("page_z")).unwrap();
        let ma = material_for(&mut pack, "a.first", "", &atlas("page_a")).unwrap();
        for (id, material) in [("z.last", mz), ("a.first", ma)] {
            let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
            pack.assets
                .push(PackedAsset::single(record(id, &mesh), mesh, material).unwrap());
        }
        assert_eq!(pack.materials[0].id, "atlas.page_z", "unsorted on purpose");
        assert!(pack.find("z.last").is_some());
        assert!(pack.find("a.first").is_some());
        assert_eq!(
            pack.validate(),
            vec![],
            "an unsorted but complete pack must validate clean"
        );
    }

    // ----- submesh tiling -----

    fn with_submeshes(id: &str, subs: Vec<(u32, u32)>) -> PackedAsset {
        let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
        PackedAsset {
            record: record(id, &mesh),
            mesh,
            submeshes: subs
                .into_iter()
                .map(|(index_start, index_count)| SubMesh {
                    material: None,
                    index_start,
                    index_count,
                })
                .collect(),
        }
    }

    fn issues_for(asset: PackedAsset) -> Vec<String> {
        let mut pack = AssetPack::new();
        pack.assets.push(asset);
        pack.validate().into_iter().map(|i| i.message).collect()
    }

    #[test]
    fn submeshes_must_tile_the_index_buffer_exactly() {
        let total = cuboid_indices();
        let none: Vec<String> = Vec::new();
        assert_eq!(issues_for(with_submeshes("a.ok", vec![(0, total)])), none);
        let half = total / 2 / 3 * 3;
        assert_eq!(
            issues_for(with_submeshes(
                "a.ok2",
                vec![(0, half), (half, total - half)]
            )),
            none
        );
    }

    #[test]
    fn overlapping_submeshes_that_sum_correctly_are_still_rejected() {
        // The exact counterexample a sum-only check misses: the first half is
        // drawn twice and the second half never, yet the counts add up.
        let total = cuboid_indices();
        let half = total / 2 / 3 * 3;
        let issues = issues_for(with_submeshes("a.overlap", vec![(0, half), (0, half)]));
        assert!(
            issues.iter().any(|m| m.contains("tile the index buffer")),
            "expected an overlap complaint, got {issues:?}"
        );
    }

    #[test]
    fn a_gap_between_submeshes_is_rejected() {
        let total = cuboid_indices();
        let issues = issues_for(with_submeshes("a.gap", vec![(0, 3), (6, total - 6)]));
        assert!(
            issues.iter().any(|m| m.contains("tile the index buffer")),
            "expected a gap complaint, got {issues:?}"
        );
    }

    #[test]
    fn a_misaligned_submesh_start_is_rejected() {
        // Starting mid-triangle stitches every triangle from the wrong three
        // indices while the totals still add up.
        let total = cuboid_indices();
        let issues = issues_for(with_submeshes("a.misaligned", vec![(1, total - 1), (0, 1)]));
        assert!(
            issues.iter().any(|m| m.contains("triangle boundaries")),
            "expected a triangle-boundary complaint, got {issues:?}"
        );
    }

    #[test]
    fn a_zero_length_submesh_is_rejected() {
        let total = cuboid_indices();
        let issues = issues_for(with_submeshes("a.empty", vec![(0, total), (total, 0)]));
        assert!(
            issues.iter().any(|m| m.contains("draws nothing")),
            "expected an empty-submesh complaint, got {issues:?}"
        );
    }

    #[test]
    fn submeshes_must_cover_the_whole_index_buffer() {
        let total = cuboid_indices();
        let issues = issues_for(with_submeshes("a.partial", vec![(0, total - 3)]));
        assert!(issues.iter().any(|m| m.contains("partly undrawn")));
    }

    #[test]
    fn a_submesh_may_not_run_off_the_end_of_the_indices() {
        let total = cuboid_indices();
        let issues = issues_for(with_submeshes("a.overrun", vec![(0, total + 30)]));
        assert!(issues.iter().any(|m| m.contains("beyond the")));
    }

    #[test]
    fn a_submesh_range_that_overflows_is_caught() {
        let issues = issues_for(with_submeshes("a.overflow", vec![(u32::MAX, 3)]));
        assert!(issues
            .iter()
            .any(|m| m.contains("beyond the") || m.contains("overflows")));
    }

    #[test]
    fn an_asset_with_no_submesh_is_rejected() {
        let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
        let issues = issues_for(PackedAsset {
            record: record("a.none", &mesh),
            mesh,
            submeshes: vec![],
        });
        assert!(issues.iter().any(|m| m.contains("no submesh")));
    }

    #[test]
    fn declared_material_slots_must_match_what_the_submeshes_draw() {
        // Two representations of "which surfaces does this asset need" must
        // not drift apart.
        let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
        let mut rec = record("a.slots", &mesh);
        rec.material_slots = vec!["mat.declared".into()];
        let mut pack = AssetPack::new();
        pack.assets
            .push(PackedAsset::single(rec, mesh, None).unwrap());
        let issues: Vec<String> = pack.validate().into_iter().map(|i| i.message).collect();
        assert!(
            issues.iter().any(|m| m.contains("material slots")),
            "expected a slot mismatch, got {issues:?}"
        );
    }

    // ----- contract + references -----

    #[test]
    fn validation_catches_a_budget_violation_in_a_packed_asset() {
        // Guard against the pack becoming a way to smuggle past the contract
        // that procedural assets have to satisfy.
        let mut pack = AssetPack::new();
        let mut bad = asset("a.big");
        bad.record.budget = PerfBudget {
            max_triangles: 1,
            max_vertices: 1,
        };
        pack.assets.push(bad);
        let issues = pack.validate();
        assert!(issues.iter().any(|i| i.message.contains("exceeds budget")));
    }

    #[test]
    fn a_directly_built_pack_cannot_smuggle_nan_past_the_contract() {
        // Finiteness lives on validate_asset, not only on the import manifest,
        // because a pack can be constructed or decoded without a manifest.
        let mut pack = AssetPack::new();
        let mut bad = asset("a.nan");
        bad.record.bounds_max = [f32::NAN; 3];
        pack.assets.push(bad);
        assert!(pack
            .validate()
            .iter()
            .any(|i| i.message.contains("not finite")));

        let mut pack = AssetPack::new();
        let mut bad = asset("a.nancollide");
        bad.record.collision = CollisionProxy::Ball { radius: f32::NAN };
        pack.assets.push(bad);
        assert!(pack
            .validate()
            .iter()
            .any(|i| i.message.contains("non-finite radius")));
    }

    #[test]
    fn validation_catches_dangling_material_and_texture_references() {
        let mut pack = AssetPack::new();
        let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
        pack.assets.push(
            PackedAsset::single(record("a.one", &mesh), mesh, Some("atlas.missing".into()))
                .unwrap(),
        );
        pack.materials.push(PackedMaterial {
            id: "atlas.present".into(),
            desc: MaterialDesc::default(),
            base_color_texture: Some("tex.missing".into()),
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
        });
        let issues = pack.validate();
        assert!(issues
            .iter()
            .any(|i| i.message.contains("missing material 'atlas.missing'")));
        assert!(issues
            .iter()
            .any(|i| i.message.contains("missing texture 'tex.missing'")));
    }

    #[test]
    fn a_two_material_asset_keeps_its_split() {
        // The hull-plus-glass-canopy case: one mesh, two materials, and the
        // split survives baking instead of being flattened.
        let mut pack = AssetPack::new();
        let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
        let total = mesh.indices.len() as u32;
        let half = total / 2 / 3 * 3;
        let mut rec = record("a.split", &mesh);
        rec.material_slots = vec!["mat.glass".into(), "mat.hull".into()];
        pack.assets.push(PackedAsset {
            record: rec,
            mesh,
            submeshes: vec![
                SubMesh {
                    material: Some("mat.hull".into()),
                    index_start: 0,
                    index_count: half,
                },
                SubMesh {
                    material: Some("mat.glass".into()),
                    index_start: half,
                    index_count: total - half,
                },
            ],
        });
        for id in ["mat.hull", "mat.glass"] {
            pack.materials.push(PackedMaterial {
                id: id.into(),
                desc: MaterialDesc::default(),
                base_color_texture: None,
                normal_texture: None,
                metallic_roughness_texture: None,
                occlusion_texture: None,
            });
        }
        assert_eq!(pack.validate(), vec![]);
        let bytes = pack.to_postcard().unwrap();
        let back = AssetPack::from_postcard(&bytes).unwrap();
        let split = back.find("a.split").unwrap();
        assert_eq!(split.submeshes.len(), 2);
        assert_eq!(
            split.material_ids().collect::<Vec<_>>(),
            vec!["mat.hull", "mat.glass"]
        );
    }

    // ----- material registration -----

    #[test]
    fn atlas_mapping_shares_one_material_across_assets() {
        // The draw-call property: many assets, one atlas page, one material.
        let mut pack = AssetPack::new();
        let first = material_for(&mut pack, "a.one", "", &atlas("page_a")).unwrap();
        let second = material_for(&mut pack, "a.two", "", &atlas("page_a")).unwrap();
        assert_eq!(first, second);
        assert_eq!(pack.materials.len(), 1);
        assert_eq!(
            pack.materials[0].base_color_texture.as_deref(),
            Some("page_a")
        );
    }

    #[test]
    fn override_materials_are_per_asset_and_per_slot() {
        let mut pack = AssetPack::new();
        let mapping = MaterialMapping::Override(Box::new(MaterialDesc::color(1.0, 0.0, 0.0)));
        let hull = material_for(&mut pack, "a.one", "hull", &mapping).unwrap();
        let glass = material_for(&mut pack, "a.one", "glass", &mapping).unwrap();
        assert_ne!(hull, glass);
        assert_eq!(pack.materials.len(), 2);
    }

    #[test]
    fn a_material_id_collision_is_an_error_not_a_silent_overwrite() {
        // Readable ids can collide: ("ship", "a.glass") and ("ship.a", "glass")
        // both render as mat.ship.a.glass. Detect it rather than discarding one
        // description and letting encounter order decide what renders.
        let mut pack = AssetPack::new();
        let red = MaterialMapping::Override(Box::new(MaterialDesc::color(1.0, 0.0, 0.0)));
        let blue = MaterialMapping::Override(Box::new(MaterialDesc::color(0.0, 0.0, 1.0)));
        material_for(&mut pack, "ship", "a.glass", &red).unwrap();
        assert!(matches!(
            material_for(&mut pack, "ship.a", "glass", &blue),
            Err(PackError::MaterialConflict { .. })
        ));
    }

    #[test]
    fn reusing_a_material_id_with_an_identical_description_is_fine() {
        let mut pack = AssetPack::new();
        let red = MaterialMapping::Override(Box::new(MaterialDesc::color(1.0, 0.0, 0.0)));
        let a = material_for(&mut pack, "ship", "hull", &red).unwrap();
        let b = material_for(&mut pack, "ship", "hull", &red).unwrap();
        assert_eq!(a, b);
        assert_eq!(pack.materials.len(), 1);
    }

    // ----- size reporting -----

    #[test]
    fn size_report_states_the_real_encoded_length() {
        let mut pack = AssetPack::new();
        pack.assets.push(asset("a.one"));
        let mut png = png_header(32, 32);
        png.resize(2048, 0);
        pack.textures.push(TextureBlob {
            id: "page_a".into(),
            png,
            sampler: SamplerMode::Nearest,
            width: 32,
            height: 32,
        });
        let report = pack.size_report().unwrap();
        assert_eq!(report.assets, 1);
        assert_eq!(report.texture_bytes, 2048);
        assert_eq!(
            report.encoded_bytes,
            pack.to_postcard_current().unwrap().len()
        );
        assert_eq!(report.triangles, pack.assets[0].mesh.triangle_count());
    }

    #[test]
    fn submesh_order_does_not_change_the_bytes() {
        // Determinism hole: validation accepts submeshes in any order (an
        // importer emits them in encounter order), so canonicalize must sort
        // them or the same content encodes differently.
        fn build(reverse: bool) -> AssetPack {
            let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
            let total = mesh.indices.len() as u32;
            let third = total / 3 / 3 * 3;
            let mut subs = vec![
                SubMesh {
                    material: None,
                    index_start: 0,
                    index_count: third,
                },
                SubMesh {
                    material: None,
                    index_start: third,
                    index_count: third,
                },
                SubMesh {
                    material: None,
                    index_start: third * 2,
                    index_count: total - third * 2,
                },
            ];
            if reverse {
                subs.reverse();
            }
            let mut pack = AssetPack::new();
            pack.assets.push(PackedAsset {
                record: record("a.one", &mesh),
                mesh,
                submeshes: subs,
            });
            pack
        }
        assert_eq!(build(false).validate(), vec![], "both orders are valid");
        assert_eq!(build(true).validate(), vec![]);
        assert_eq!(
            build(false).to_postcard().unwrap(),
            build(true).to_postcard().unwrap()
        );
    }

    #[test]
    fn a_nonsense_material_cannot_reach_a_pack() {
        // Same argument that moved finiteness into validate_asset: a pack can
        // be built or decoded without ever seeing an import manifest.
        let mut pack = AssetPack::new();
        pack.assets.push(asset("a.one"));
        pack.materials.push(PackedMaterial {
            id: "mat.bad".into(),
            desc: MaterialDesc {
                roughness: f32::NAN,
                metallic: 99.0,
                ..Default::default()
            },
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
        });
        assert!(pack
            .validate()
            .iter()
            .any(|i| i.message.contains("not finite")));
    }

    #[test]
    fn a_texture_blob_that_is_not_a_png_is_rejected() {
        let mut pack = AssetPack::new();
        pack.textures.push(TextureBlob {
            id: "page".into(),
            png: vec![1, 2, 3, 4],
            sampler: SamplerMode::Nearest,
            width: 2,
            height: 2,
        });
        assert!(pack
            .validate()
            .iter()
            .any(|i| i.message.contains("not a PNG")));
    }

    #[test]
    fn a_texture_whose_header_disagrees_with_its_declared_size_is_rejected() {
        // Signature-only checking would let this through, and the mismatch
        // would surface as a broken page in a browser instead of at bake.
        let mut pack = AssetPack::new();
        let mut blob = texture_blob("page");
        blob.png = png_header(64, 64);
        pack.textures.push(blob);
        assert!(pack
            .validate()
            .iter()
            .any(|i| i.message.contains("but the PNG is 64x64")));
    }

    #[test]
    fn signature_prefixed_garbage_is_not_accepted_as_a_png() {
        let mut pack = AssetPack::new();
        let mut blob = texture_blob("page");
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend_from_slice(b"IHDR-ish payload but not really");
        blob.png = png;
        pack.textures.push(blob);
        assert!(pack
            .validate()
            .iter()
            .any(|i| i.message.contains("no IHDR chunk")));
    }

    #[test]
    fn record_vectors_are_canonicalized_too() {
        // material_slots and sockets are semantically unordered, so reversing
        // either must not change the file.
        fn build(reverse: bool) -> AssetPack {
            let mesh = procmesh::cuboid(1.0, 2.0, 3.0);
            let mut rec = record("a.one", &mesh);
            let mut sockets = vec![
                Socket {
                    name: "alpha".into(),
                    position: [0.0; 3],
                    direction: [0.0, 0.0, -1.0],
                },
                Socket {
                    name: "beta".into(),
                    position: [0.0; 3],
                    direction: [0.0, 0.0, -1.0],
                },
            ];
            let mut slots = vec!["mat.a".to_string(), "mat.b".to_string()];
            if reverse {
                sockets.reverse();
                slots.reverse();
            }
            rec.sockets = sockets;
            rec.material_slots = slots;
            let total = mesh.indices.len() as u32;
            let half = total / 2 / 3 * 3;
            let mut pack = AssetPack::new();
            pack.assets.push(PackedAsset {
                record: rec,
                mesh,
                submeshes: vec![
                    SubMesh {
                        material: Some("mat.a".into()),
                        index_start: 0,
                        index_count: half,
                    },
                    SubMesh {
                        material: Some("mat.b".into()),
                        index_start: half,
                        index_count: total - half,
                    },
                ],
            });
            for id in ["mat.a", "mat.b"] {
                pack.materials.push(PackedMaterial {
                    id: id.into(),
                    desc: MaterialDesc::default(),
                    base_color_texture: None,
                    normal_texture: None,
                    metallic_roughness_texture: None,
                    occlusion_texture: None,
                });
            }
            pack
        }
        assert_eq!(build(false).validate(), vec![]);
        assert_eq!(build(true).validate(), vec![]);
        assert_eq!(
            build(false).to_postcard().unwrap(),
            build(true).to_postcard().unwrap()
        );
    }

    #[test]
    fn the_validated_decoder_rejects_a_bad_pack() {
        let mut pack = AssetPack::new();
        let mut bad = asset("a.big");
        bad.record.budget = PerfBudget {
            max_triangles: 1,
            max_vertices: 1,
        };
        pack.assets.push(bad);
        let bytes = pack.to_postcard().unwrap();
        assert!(
            AssetPack::from_postcard(&bytes).is_ok(),
            "decode-only is lenient"
        );
        assert!(matches!(
            AssetPack::from_postcard_validated(&bytes),
            Err(PackError::Invalid(_))
        ));
    }

    #[test]
    fn a_texture_with_no_declared_size_is_rejected() {
        let mut pack = AssetPack::new();
        let mut blob = texture_blob("page");
        blob.width = 0;
        pack.textures.push(blob);
        assert!(pack
            .validate()
            .iter()
            .any(|i| i.message.contains("0x2 size")));
        assert!(pack.to_postcard_current().is_err());
    }

    #[test]
    fn the_bake_encoder_refuses_to_emit_an_invalid_pack() {
        // to_postcard_current is what a bake calls; it must not be able to
        // write a file that fails the pack's own validation.
        let mut pack = AssetPack::new();
        let mut bad = asset("a.big");
        bad.record.budget = PerfBudget {
            max_triangles: 1,
            max_vertices: 1,
        };
        pack.assets.push(bad);
        assert!(matches!(
            pack.to_postcard_current(),
            Err(PackError::Invalid(_))
        ));
        // The unchecked encoder still works, for tests that need to build
        // deliberately-broken input.
        assert!(pack.to_postcard().is_ok());
    }

    #[test]
    fn size_report_reports_failure_rather_than_zero_bytes() {
        // "0 KB baked" for a pack that cannot bake is worse than an error.
        let mut pack = AssetPack::new();
        pack.assets.push(asset("a.same"));
        pack.assets.push(asset("a.same"));
        assert!(pack.size_report().is_err());
    }
}
