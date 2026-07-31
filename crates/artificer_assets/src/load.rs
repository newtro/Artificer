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

use crate::pack::{AssetPack, PackedAsset};
use artificer_scene::{MaterialDesc, MeshId, SceneGraph, TextureId, TextureSampling};
use std::collections::HashMap;

use crate::manifest::SamplerMode;

/// Everything a loaded pack registered, so a game can spawn from it.
#[derive(Debug, Default)]
pub struct LoadedPack {
    meshes: HashMap<String, MeshId>,
    materials: HashMap<String, MaterialDesc>,
    textures: HashMap<String, TextureId>,
}

impl LoadedPack {
    /// Mesh handle for an asset id.
    pub fn mesh(&self, asset_id: &str) -> Option<MeshId> {
        self.meshes.get(asset_id).copied()
    }

    /// Material for a packed material id, with its texture already bound.
    pub fn material(&self, material_id: &str) -> Option<MaterialDesc> {
        self.materials.get(material_id).copied()
    }

    pub fn texture(&self, texture_id: &str) -> Option<TextureId> {
        self.textures.get(texture_id).copied()
    }

    /// The material an asset's FIRST submesh draws with, which is the whole
    /// story for the single-material assets an atlas pack is made of.
    ///
    /// Multi-material assets need [`LoadedPack::material`] per submesh; this
    /// is the convenience for the common case, not a substitute.
    pub fn primary_material(&self, pack: &AssetPack, asset_id: &str) -> Option<MaterialDesc> {
        let asset = pack.find(asset_id)?;
        match asset.submeshes.first().and_then(|s| s.material.as_deref()) {
            Some(id) => self.material(id),
            None => Some(MaterialDesc::default()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
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

    for texture in &pack.textures {
        let id = scene.add_texture(texture.png.clone(), sampling_of(texture.sampler));
        loaded.textures.insert(texture.id.clone(), id);
    }

    for material in &pack.materials {
        let mut desc = material.desc;
        if let Some(texture_id) = &material.base_color_texture {
            desc.base_color_texture = loaded.textures.get(texture_id).copied();
            if desc.base_color_texture.is_none() {
                log::warn!(
                    "material '{}' wants texture '{texture_id}', which the pack does not carry",
                    material.id
                );
            }
            if let Some(blob) = pack.texture(texture_id) {
                desc.sampling = sampling_of(blob.sampler);
            }
        }
        loaded.materials.insert(material.id.clone(), desc);
    }

    for asset in &pack.assets {
        let id = scene.add_mesh(asset.mesh.clone());
        loaded.meshes.insert(asset.record.id.clone(), id);
    }

    loaded
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

/// Every asset in the pack, for tooling that wants to enumerate content.
pub fn assets(pack: &AssetPack) -> impl Iterator<Item = &PackedAsset> {
    pack.assets.iter()
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
        });
        for id in ["ship.one", "ship.two"] {
            let mesh = procmesh::cuboid(1.0, 1.0, 1.0);
            pack.assets.push(
                PackedAsset::single(record(id, &mesh), mesh, Some("atlas.page_a".into())).unwrap(),
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
        let a = loaded.primary_material(&pack, "ship.one").unwrap();
        let b = loaded.primary_material(&pack, "ship.two").unwrap();
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
            .push(PackedAsset::single(record("plain", &mesh), mesh, None).unwrap());
        let mut scene = SceneGraph::new();
        let loaded = load_pack(&mut scene, &pack);
        assert!(loaded.mesh("plain").is_some());
        assert!(loaded
            .primary_material(&pack, "plain")
            .unwrap()
            .base_color_texture
            .is_none());
    }
}
