//! Getting a baked [`AssetPack`] into a live [`SceneGraph`].
//!
//! This is the runtime half of the pipeline and the only part a shipped game
//! actually runs. It does no parsing: meshes arrive ready, textures arrive as
//! encoded PNG bytes the renderer decodes, and the loader's whole job is to
//! register them and hand back handles.
//!
//! Texture ids are allocated by the scene at load, not stored in the pack.
//! That keeps the pack free of handle numbers, which would otherwise be a
//! second thing to keep deterministic for no benefit.

use crate::pack::AssetPack;
use artificer_scene::{
    MaterialDesc, MeshId, SceneGraph, TextureColorSpace, TextureId, TextureSampling,
};
use std::collections::HashMap;

use crate::manifest::SamplerMode;

/// One drawable piece of an asset: a mesh handle and the material it draws
/// with.
///
/// Most assets are a single part -- that is the point of an atlas. Assets that
/// carry several materials (a hull plus its glass canopy) become several
/// parts, because a renderer draws one material per mesh and flattening them
/// would silently paint the canopy in hull paint.
#[derive(Debug, Clone, Copy)]
pub struct LoadedPart {
    pub mesh: MeshId,
    pub material: MaterialDesc,
}

/// Everything a loaded pack registered, so a game can spawn from it.
#[derive(Debug, Default)]
pub struct LoadedPack {
    parts: HashMap<String, Vec<LoadedPart>>,
    /// Asset id -> (min, max) in metres, straight from the record.
    ///
    /// A game placing anything against imported art needs its extent: an
    /// engine plume has to clear the nacelle, a socket has to sit on a
    /// surface. Without this the caller guesses, and a guess that is wrong by
    /// a third of a metre puts the flame inside the engine.
    bounds: HashMap<String, ([f32; 3], [f32; 3])>,
    materials: HashMap<String, MaterialDesc>,
    textures: HashMap<String, TextureId>,
}

impl LoadedPack {
    /// Every drawable part of an asset, in submesh order.
    pub fn parts(&self, asset_id: &str) -> &[LoadedPart] {
        self.parts.get(asset_id).map(|v| &v[..]).unwrap_or(&[])
    }

    /// Axis-aligned bounds of a baked asset, in metres.
    pub fn bounds(&self, asset_id: &str) -> Option<([f32; 3], [f32; 3])> {
        self.bounds.get(asset_id).copied()
    }

    /// Mesh handle for a single-part asset.
    ///
    /// Returns `None` for a multi-material asset rather than silently handing
    /// back the first of several — losing the rest is exactly the failure
    /// this API exists to prevent. Use [`LoadedPack::parts`] for those.
    pub fn mesh(&self, asset_id: &str) -> Option<MeshId> {
        match self.parts.get(asset_id).map(|v| &v[..]) {
            Some([single]) => Some(single.mesh),
            _ => None,
        }
    }

    /// Material for a packed material id, with its texture already bound.
    pub fn material(&self, material_id: &str) -> Option<MaterialDesc> {
        self.materials.get(material_id).copied()
    }

    pub fn texture(&self, texture_id: &str) -> Option<TextureId> {
        self.textures.get(texture_id).copied()
    }

    /// The material of a single-part asset. `None` when the asset has several
    /// parts, for the same reason as [`LoadedPack::mesh`].
    pub fn single_material(&self, asset_id: &str) -> Option<MaterialDesc> {
        match self.parts.get(asset_id).map(|v| &v[..]) {
            Some([single]) => Some(single.material),
            _ => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Fold another loaded pack into this one.
    ///
    /// A game rarely ships one pack: art arrives per-theme, per-DLC, or as a
    /// separate evaluation set, and every screen that resolves an asset id
    /// wants ONE place to ask. Without this each caller carries a list of
    /// packs and a lookup loop, and the day someone forgets the loop an
    /// asset silently renders as nothing.
    ///
    /// Ids are global: a later pack that repeats an id WINS, which makes an
    /// override pack work by simply loading last. Both packs must already
    /// have been loaded into the same [`SceneGraph`], since the handles this
    /// carries are that scene's -- the handles are meaningless in any other.
    ///
    /// Returns every asset id that was overwritten. Callers are expected to
    /// SAY SO: a silent collision means an artist renames a mesh, two packs
    /// quietly claim one id, and the wrong model ships. The caller knows
    /// which files it read and can name them; this type does not.
    ///
    /// Note the overwritten pack's meshes and textures stay registered with
    /// the scene. Nothing draws them, but they are not reclaimed either, so
    /// merging is for load-time composition rather than hot-swapping.
    #[must_use = "collisions mean two packs claim one id; report them"]
    pub fn merge(&mut self, other: LoadedPack) -> Vec<String> {
        let mut clashed: Vec<String> = other
            .parts
            .keys()
            .filter(|id| self.parts.contains_key(*id))
            .cloned()
            .collect();
        clashed.sort_unstable();
        self.parts.extend(other.parts);
        self.bounds.extend(other.bounds);
        self.materials.extend(other.materials);
        self.textures.extend(other.textures);
        clashed
    }
}

fn sampling_of(mode: SamplerMode) -> TextureSampling {
    match mode {
        SamplerMode::Nearest => TextureSampling::Nearest,
        SamplerMode::Linear => TextureSampling::Linear,
    }
}

/// Register every mesh, texture and material in a pack with the scene.
///
/// Textures are registered BEFORE materials so a material can be handed its
/// texture id immediately, rather than the caller having to remember to bind
/// it later — the kind of two-step that renders untextured the one time
/// somebody forgets.
pub fn load_pack(scene: &mut SceneGraph, pack: &AssetPack) -> LoadedPack {
    let mut loaded = LoadedPack::default();

    // Which textures are DATA rather than colour has to be decided before any
    // is registered, because the colour space is fixed at decode. A material
    // naming a texture as its normal map is the only evidence available.
    let mut linear: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for material in &pack.materials {
        for slot in [
            &material.normal_texture,
            &material.metallic_roughness_texture,
            &material.occlusion_texture,
        ]
        .into_iter()
        .flatten()
        {
            linear.insert(slot.as_str());
        }
    }

    for texture in &pack.textures {
        let space = if linear.contains(texture.id.as_str()) {
            TextureColorSpace::Linear
        } else {
            TextureColorSpace::Srgb
        };
        let id = scene.add_texture_in(texture.png.clone(), sampling_of(texture.sampler), space);
        loaded.textures.insert(texture.id.clone(), id);
    }

    for material in &pack.materials {
        let mut desc = material.desc;
        // Every map resolves the same way: a string id in the pack becomes a
        // handle here. Resolving them through one closure means a new map can
        // never be added and then silently left unresolved, which is how a
        // 4K normal map ends up shipped in the pack and never drawn.
        let resolve = |slot: &Option<String>, what: &str| -> Option<TextureId> {
            let id = slot.as_ref()?;
            let handle = loaded.textures.get(id).copied();
            if handle.is_none() {
                log::warn!(
                    "material '{}' wants {what} texture '{id}', which the pack does not carry",
                    material.id
                );
            }
            handle
        };
        desc.base_color_texture = resolve(&material.base_color_texture, "base colour");
        desc.normal_texture = resolve(&material.normal_texture, "normal");
        desc.metallic_roughness_texture =
            resolve(&material.metallic_roughness_texture, "metallic-roughness");
        desc.occlusion_texture = resolve(&material.occlusion_texture, "occlusion");

        // Sampling follows the BASE COLOUR blob. The maps of one material are
        // authored together and sampled together; letting a normal map's own
        // setting win would mean one surface filtered two ways.
        if let Some(blob) = material
            .base_color_texture
            .as_ref()
            .and_then(|id| pack.texture(id))
        {
            desc.sampling = sampling_of(blob.sampler);
        }
        loaded.materials.insert(material.id.clone(), desc);
    }

    for asset in &pack.assets {
        // One registered mesh PER SUBMESH. A renderer draws a mesh with a
        // single material, so an asset carrying several has to become several
        // meshes here -- otherwise the split that the pack format defines,
        // the importer produces and validation checks exhaustively would be
        // discarded at the last step, and a glass canopy would render in hull
        // paint with nothing to say so.
        let mut parts = Vec::with_capacity(asset.submeshes.len());
        for sub in &asset.submeshes {
            let material = match &sub.material {
                Some(id) => loaded.materials.get(id).copied().unwrap_or_else(|| {
                    log::warn!(
                        "asset '{}' draws with material '{id}', which the pack does not carry",
                        asset.record.id
                    );
                    MaterialDesc::default()
                }),
                None => MaterialDesc::default(),
            };
            let mesh = if asset.submeshes.len() == 1 {
                // The common case: no slicing, no copy.
                scene.add_mesh(asset.mesh.clone())
            } else {
                scene.add_mesh(submesh_data(&asset.mesh, sub))
            };
            parts.push(LoadedPart { mesh, material });
        }
        loaded.bounds.insert(
            asset.record.id.clone(),
            (asset.record.bounds_min, asset.record.bounds_max),
        );
        loaded.parts.insert(asset.record.id.clone(), parts);
    }

    loaded
}

/// Extract one submesh as a standalone mesh, re-indexed so it carries only
/// the vertices it uses.
fn submesh_data(
    mesh: &artificer_scene::MeshData,
    sub: &crate::pack::SubMesh,
) -> artificer_scene::MeshData {
    let start = sub.index_start as usize;
    let end = start + sub.index_count as usize;
    let mut out = artificer_scene::MeshData::default();
    // Keeping every vertex and only slicing indices would leave each part
    // carrying the whole asset's vertex buffer -- N parts, N copies.
    let mut remap: HashMap<u32, u32> = HashMap::new();
    for &index in &mesh.indices[start..end] {
        let next = *remap.entry(index).or_insert_with(|| {
            out.positions.push(mesh.positions[index as usize]);
            out.normals.push(mesh.normals[index as usize]);
            out.uvs.push(mesh.uvs[index as usize]);
            (out.positions.len() - 1) as u32
        });
        out.indices.push(next);
    }
    out
}

/// Assets in a pack that a game asked for but that are not there.
///
/// Worth calling at startup: a typo'd asset id otherwise shows up as an
/// invisible ship, which reads as a rendering bug rather than a content one.
pub fn missing<'a>(pack: &AssetPack, wanted: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    wanted
        .into_iter()
        .filter(|id| pack.find(id).is_none())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{PackedMaterial, TextureBlob};
    use crate::procmesh;
    use crate::{AssetCategory, AssetRecord, CollisionProxy, LodPolicy, PerfBudget};
    use artificer_scene::{MeshData, SceneCommand};

    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&w.to_be_bytes());
        png.extend_from_slice(&h.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        png
    }

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
            gameplay_ref: "test".into(),
        }
    }

    fn atlas_pack() -> AssetPack {
        let mut pack = AssetPack::new();
        pack.textures.push(TextureBlob {
            id: "page_a".into(),
            png: png_header(4, 4),
            sampler: SamplerMode::Nearest,
            width: 4,
            height: 4,
        });
        pack.materials.push(PackedMaterial {
            id: "atlas.page_a".into(),
            desc: MaterialDesc::default(),
            base_color_texture: Some("page_a".into()),
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
        });
        for id in ["ship.one", "ship.two"] {
            let mesh = procmesh::cuboid(1.0, 1.0, 1.0);
            pack.assets.push(
                crate::pack::PackedAsset::single(
                    record(id, &mesh),
                    mesh,
                    Some("atlas.page_a".into()),
                )
                .unwrap(),
            );
        }
        pack.canonicalize();
        pack
    }

    #[test]
    fn loading_registers_meshes_textures_and_materials() {
        let mut scene = SceneGraph::new();
        let pack = atlas_pack();
        let loaded = load_pack(&mut scene, &pack);

        assert!(loaded.mesh("ship.one").is_some());
        assert!(loaded.mesh("ship.two").is_some());
        assert!(loaded.texture("page_a").is_some());
        assert_eq!(loaded.parts("ship.one").len(), 1);

        let commands = scene.drain_commands();
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, SceneCommand::AddTexture { .. }))
                .count(),
            1
        );
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, SceneCommand::AddMesh { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn a_material_comes_back_with_its_texture_already_bound() {
        // The caller must not have to remember a second binding step; that is
        // the one somebody forgets and everything renders untextured.
        let mut scene = SceneGraph::new();
        let pack = atlas_pack();
        let loaded = load_pack(&mut scene, &pack);
        let material = loaded.material("atlas.page_a").expect("material");
        assert_eq!(material.base_color_texture, loaded.texture("page_a"));
        assert!(material.base_color_texture.is_some());
    }

    #[test]
    fn many_assets_share_one_atlas_texture_and_one_material() {
        // The draw-call property, end to end: two assets, one AddTexture.
        let mut scene = SceneGraph::new();
        let pack = atlas_pack();
        let loaded = load_pack(&mut scene, &pack);
        let a = loaded.single_material("ship.one").unwrap();
        let b = loaded.single_material("ship.two").unwrap();
        assert_eq!(a.base_color_texture, b.base_color_texture);
        assert_eq!(loaded.textures.len(), 1);
    }

    #[test]
    fn an_atlas_texture_loads_with_nearest_sampling() {
        // Bilinear filtering bleeds neighbouring atlas swatches into each
        // other along every UV seam.
        let mut scene = SceneGraph::new();
        let pack = atlas_pack();
        let loaded = load_pack(&mut scene, &pack);
        assert_eq!(
            loaded.material("atlas.page_a").unwrap().sampling,
            TextureSampling::Nearest
        );
        let commands = scene.drain_commands();
        assert!(commands.iter().any(|c| matches!(
            c,
            SceneCommand::AddTexture {
                sampling: TextureSampling::Nearest,
                ..
            }
        )));
    }

    #[test]
    fn a_multi_material_asset_becomes_one_drawable_part_per_material() {
        // Regression: load_pack used to register ONE mesh per asset and hand
        // back only the first submesh's material, so a hull-plus-canopy asset
        // rendered entirely as hull with nothing to say so -- discarding a
        // split that the pack format defines, the importer produces and
        // validation checks exhaustively.
        let mut pack = AssetPack::new();
        let mesh = procmesh::cuboid(1.0, 1.0, 1.0);
        let total = mesh.indices.len() as u32;
        let half = total / 2 / 3 * 3;
        let mut rec = record("ship.split", &mesh);
        rec.material_slots = vec!["mat.glass".into(), "mat.hull".into()];
        pack.assets.push(crate::pack::PackedAsset {
            record: rec,
            mesh,
            submeshes: vec![
                crate::pack::SubMesh {
                    material: Some("mat.hull".into()),
                    index_start: 0,
                    index_count: half,
                },
                crate::pack::SubMesh {
                    material: Some("mat.glass".into()),
                    index_start: half,
                    index_count: total - half,
                },
            ],
        });
        for (id, colour) in [("mat.hull", 0.5), ("mat.glass", 0.1)] {
            pack.materials.push(PackedMaterial {
                id: id.into(),
                desc: MaterialDesc::color(colour, colour, colour),
                base_color_texture: None,
                normal_texture: None,
                metallic_roughness_texture: None,
                occlusion_texture: None,
            });
        }
        pack.canonicalize();
        assert_eq!(pack.validate(), vec![]);

        let mut scene = SceneGraph::new();
        let loaded = load_pack(&mut scene, &pack);
        let parts = loaded.parts("ship.split");
        assert_eq!(parts.len(), 2, "one part per material");
        assert_ne!(
            parts[0].material.base_color, parts[1].material.base_color,
            "each part keeps its own material"
        );
        assert_ne!(parts[0].mesh, parts[1].mesh, "and its own mesh");

        // The single-part accessors refuse rather than silently returning the
        // first of several.
        assert!(loaded.mesh("ship.split").is_none());
        assert!(loaded.single_material("ship.split").is_none());
    }

    #[test]
    fn a_split_part_carries_only_the_vertices_it_uses() {
        // Slicing indices while keeping the whole vertex buffer would give
        // every part a copy of the entire asset.
        let mut pack = AssetPack::new();
        let mesh = procmesh::cuboid(1.0, 1.0, 1.0);
        let total = mesh.indices.len() as u32;
        let half = total / 2 / 3 * 3;
        let full_vertices = mesh.vertex_count();
        pack.assets.push(crate::pack::PackedAsset {
            record: record("ship.split", &mesh),
            mesh,
            submeshes: vec![
                crate::pack::SubMesh {
                    material: None,
                    index_start: 0,
                    index_count: half,
                },
                crate::pack::SubMesh {
                    material: None,
                    index_start: half,
                    index_count: total - half,
                },
            ],
        });
        let mut scene = SceneGraph::new();
        let loaded = load_pack(&mut scene, &pack);
        assert_eq!(loaded.parts("ship.split").len(), 2);

        let mut part_vertices = 0;
        for command in scene.drain_commands() {
            if let SceneCommand::AddMesh { data, .. } = command {
                assert!(
                    data.validate().is_ok(),
                    "a sliced part must still be a valid mesh"
                );
                part_vertices += data.vertex_count();
            }
        }
        assert!(
            part_vertices <= full_vertices + 8,
            "parts carry {part_vertices} vertices against {full_vertices} in the whole asset"
        );
    }

    #[test]
    fn a_missing_asset_id_is_reported_rather_than_rendering_nothing() {
        let pack = atlas_pack();
        assert_eq!(
            missing(&pack, ["ship.one", "ship.nope"]),
            vec!["ship.nope".to_string()]
        );
    }

    #[test]
    fn an_untextured_pack_still_loads() {
        let mut pack = AssetPack::new();
        let mesh = procmesh::cuboid(1.0, 1.0, 1.0);
        pack.assets
            .push(crate::pack::PackedAsset::single(record("plain", &mesh), mesh, None).unwrap());
        let mut scene = SceneGraph::new();
        let loaded = load_pack(&mut scene, &pack);
        assert!(loaded.mesh("plain").is_some());
        assert!(loaded
            .single_material("plain")
            .unwrap()
            .base_color_texture
            .is_none());
    }
}
