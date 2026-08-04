//! Renderer-neutral scene description.
//!
//! Games mutate a [`SceneGraph`]; every mutation is captured as a
//! [`SceneCommand`]. A render adapter (e.g. `artificer_render`'s Bevy adapter)
//! drains the command stream each frame and mirrors it into its own world.
//! Because the command stream is serializable, scene evolution can be
//! recorded and replayed for visual regression testing.

pub mod framing;
mod mesh;
mod types;

/// The maths types this crate's API is expressed in, re-exported so
/// consumers use the SAME version rather than independently picking one that
/// has to happen to match. A mismatched glam is a wall of confusing type
/// errors at the boundary.
pub use glam;
pub use mesh::{Aabb, MeshData};
pub use types::*;

use std::collections::HashMap;

/// The game-facing scene handle. Owns id allocation, current node state
/// (so games can query transforms back), and the pending command stream.
#[derive(Debug)]
pub struct SceneGraph {
    /// Process-unique identity, stamped into every [`AssetMark`] this graph
    /// hands out. Asset ids are bare counters, so a mark taken from one
    /// graph describes a DIFFERENT set of assets on another — releasing
    /// through it would free innocent neighbours. The stamp turns that
    /// silent cross-graph corruption into a refused, logged no-op.
    graph_id: u64,
    next_node: u64,
    next_mesh: u64,
    next_texture: u64,
    nodes: HashMap<NodeId, NodeState>,
    commands: Vec<SceneCommand>,
}

impl Default for SceneGraph {
    fn default() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_GRAPH_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            graph_id: NEXT_GRAPH_ID.fetch_add(1, Ordering::Relaxed),
            next_node: 0,
            next_mesh: 0,
            next_texture: 0,
            nodes: HashMap::new(),
            commands: Vec::new(),
        }
    }
}

/// A snapshot of the asset id counters — the "before" half of a span.
///
/// Asset ids are allocated monotonically, so two marks bracket exactly the
/// meshes and textures registered between them. Take one before a spawn
/// block (a sector, a hangar scene), close it with
/// [`SceneGraph::assets_since`], and the resulting [`AssetSpan`] is
/// everything that block owns — without every helper in the block having to
/// hand its ids back up the call chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AssetMark {
    graph: u64,
    mesh: u64,
    texture: u64,
}

/// A CLOSED range of registered assets, bracketed by two marks.
///
/// Closed matters: releasing "everything since a mark" at teardown time
/// would also sweep up assets other systems registered in the meantime —
/// a ship that docked mid-sector would lose its hull art with the scenery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AssetSpan {
    graph: u64,
    /// Counter values: ids in (after ..= until] belong to the span.
    mesh_after: u64,
    mesh_until: u64,
    texture_after: u64,
    texture_until: u64,
}

impl AssetSpan {
    /// A span that owns nothing; releasing it is a no-op. The `Default`
    /// for state that has not spawned its block yet.
    pub const EMPTY: AssetSpan = AssetSpan {
        graph: 0,
        mesh_after: 0,
        mesh_until: 0,
        texture_after: 0,
        texture_until: 0,
    };

    pub fn is_empty(&self) -> bool {
        self.mesh_count() == 0 && self.texture_count() == 0
    }

    pub fn mesh_count(&self) -> u64 {
        self.mesh_until - self.mesh_after
    }

    pub fn texture_count(&self) -> u64 {
        self.texture_until - self.texture_after
    }
}

impl Default for AssetSpan {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Debug, Clone)]
struct NodeState {
    transform: TransformDesc,
    parent: Option<NodeId>,
    visible: bool,
    kind_tag: NodeKindTag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKindTag {
    Mesh,
    Atmosphere,
    Light,
    Camera,
    Group,
}

impl SceneGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register mesh geometry; the returned id can back many nodes.
    pub fn add_mesh(&mut self, data: MeshData) -> MeshId {
        self.next_mesh += 1;
        let id = MeshId(self.next_mesh);
        self.commands.push(SceneCommand::AddMesh { id, data });
        id
    }

    /// Register an encoded image (PNG) and get a handle for materials.
    ///
    /// Ids are allocated here rather than baked, so a pack never has to carry
    /// handle numbers that would differ between runs.
    pub fn add_texture(&mut self, png: Vec<u8>, sampling: TextureSampling) -> TextureId {
        self.add_texture_in(png, sampling, TextureColorSpace::Srgb)
    }

    /// Register an image whose bytes are DATA rather than colour.
    ///
    /// Separate from [`SceneGraph::add_texture`] so the sRGB default stays
    /// right for the common case while a normal map cannot be registered as
    /// colour by omission -- the caller has to say what it is holding.
    pub fn add_texture_in(
        &mut self,
        png: Vec<u8>,
        sampling: TextureSampling,
        color_space: TextureColorSpace,
    ) -> TextureId {
        self.next_texture += 1;
        let id = TextureId(self.next_texture);
        self.commands.push(SceneCommand::AddTexture {
            id,
            png,
            sampling,
            color_space,
        });
        id
    }

    /// Snapshot the asset counters. Pair with [`SceneGraph::assets_since`].
    pub fn asset_mark(&self) -> AssetMark {
        AssetMark {
            graph: self.graph_id,
            mesh: self.next_mesh,
            texture: self.next_texture,
        }
    }

    /// Close a span: every mesh and texture registered since `mark`.
    ///
    /// A mark from a different `SceneGraph` yields [`AssetSpan::EMPTY`]
    /// with a logged error — its counters describe that graph's assets,
    /// not this one's, and a span built from them would release strangers.
    pub fn assets_since(&self, mark: AssetMark) -> AssetSpan {
        if mark.graph != self.graph_id {
            log::error!(
                "asset mark from SceneGraph {} used on SceneGraph {}; returning an empty span",
                mark.graph,
                self.graph_id
            );
            return AssetSpan::EMPTY;
        }
        AssetSpan {
            graph: self.graph_id,
            mesh_after: mark.mesh,
            mesh_until: self.next_mesh,
            texture_after: mark.texture,
            texture_until: self.next_texture,
        }
    }

    /// Deregister one mesh. Nodes already using it keep rendering (they
    /// hold their own handles); the GPU asset is freed when the last one
    /// despawns. New spawns naming this id will warn.
    pub fn remove_mesh(&mut self, id: MeshId) {
        self.commands.push(SceneCommand::RemoveMesh { id });
    }

    /// Deregister one texture. Same semantics as [`SceneGraph::remove_mesh`].
    pub fn remove_texture(&mut self, id: TextureId) {
        self.commands.push(SceneCommand::RemoveTexture { id });
    }

    /// Deregister every asset in a span (see ADR-0005).
    ///
    /// The teardown half of the mark/span pattern: despawn the block's
    /// nodes, then release the span they were spawned from. Releasing the
    /// same span twice emits removes the adapter will warn about — that
    /// loudness is deliberate, a double release is a lifecycle bug. A span
    /// from a different graph is refused outright.
    pub fn release_assets(&mut self, span: AssetSpan) {
        if span.is_empty() {
            return;
        }
        if span.graph != self.graph_id {
            log::error!(
                "asset span from SceneGraph {} released on SceneGraph {}; refusing",
                span.graph,
                self.graph_id
            );
            return;
        }
        for mesh in span.mesh_after..span.mesh_until {
            self.remove_mesh(MeshId(mesh + 1));
        }
        for texture in span.texture_after..span.texture_until {
            self.remove_texture(TextureId(texture + 1));
        }
    }

    fn alloc_node(&mut self) -> NodeId {
        self.next_node += 1;
        NodeId(self.next_node)
    }

    fn spawn_inner(
        &mut self,
        kind: NodeKind,
        transform: TransformDesc,
        parent: Option<NodeId>,
    ) -> NodeId {
        let id = self.alloc_node();
        let tag = match &kind {
            NodeKind::Mesh { .. } => NodeKindTag::Mesh,
            NodeKind::Atmosphere { .. } => NodeKindTag::Atmosphere,
            NodeKind::Light(_) => NodeKindTag::Light,
            NodeKind::Camera(_) => NodeKindTag::Camera,
            NodeKind::Group => NodeKindTag::Group,
        };
        self.nodes.insert(
            id,
            NodeState {
                transform,
                parent,
                visible: true,
                kind_tag: tag,
            },
        );
        self.commands.push(SceneCommand::Spawn {
            id,
            parent,
            transform,
            kind,
        });
        id
    }

    pub fn spawn_mesh(
        &mut self,
        mesh: MeshId,
        material: MaterialDesc,
        transform: TransformDesc,
    ) -> NodeId {
        self.spawn_inner(NodeKind::Mesh { mesh, material }, transform, None)
    }

    pub fn spawn_mesh_child(
        &mut self,
        parent: NodeId,
        mesh: MeshId,
        material: MaterialDesc,
        transform: TransformDesc,
    ) -> NodeId {
        self.spawn_inner(NodeKind::Mesh { mesh, material }, transform, Some(parent))
    }

    /// Spawn a scattering-atmosphere shell (see [`AtmosphereDesc`]). `mesh`
    /// should be a sphere of the atmosphere radius; the transform is the
    /// planet's centre.
    pub fn spawn_atmosphere(
        &mut self,
        mesh: MeshId,
        atmosphere: AtmosphereDesc,
        transform: TransformDesc,
    ) -> NodeId {
        self.spawn_inner(NodeKind::Atmosphere { mesh, atmosphere }, transform, None)
    }

    pub fn spawn_atmosphere_child(
        &mut self,
        parent: NodeId,
        mesh: MeshId,
        atmosphere: AtmosphereDesc,
        transform: TransformDesc,
    ) -> NodeId {
        self.spawn_inner(
            NodeKind::Atmosphere { mesh, atmosphere },
            transform,
            Some(parent),
        )
    }

    pub fn spawn_light(&mut self, light: LightDesc, transform: TransformDesc) -> NodeId {
        self.spawn_inner(NodeKind::Light(light), transform, None)
    }

    pub fn spawn_camera(&mut self, camera: CameraDesc, transform: TransformDesc) -> NodeId {
        self.spawn_inner(NodeKind::Camera(camera), transform, None)
    }

    pub fn spawn_group(&mut self, transform: TransformDesc) -> NodeId {
        self.spawn_inner(NodeKind::Group, transform, None)
    }

    pub fn spawn_group_child(&mut self, parent: NodeId, transform: TransformDesc) -> NodeId {
        self.spawn_inner(NodeKind::Group, transform, Some(parent))
    }

    pub fn set_transform(&mut self, id: NodeId, transform: TransformDesc) {
        if let Some(state) = self.nodes.get_mut(&id) {
            state.transform = transform;
            self.commands
                .push(SceneCommand::SetTransform { id, transform });
        }
    }

    pub fn set_visible(&mut self, id: NodeId, visible: bool) {
        if let Some(state) = self.nodes.get_mut(&id) {
            if state.visible != visible {
                state.visible = visible;
                self.commands.push(SceneCommand::SetVisible { id, visible });
            }
        }
    }

    pub fn set_material(&mut self, id: NodeId, material: MaterialDesc) {
        if let Some(state) = self.nodes.get(&id) {
            if state.kind_tag == NodeKindTag::Mesh {
                self.commands
                    .push(SceneCommand::SetMaterial { id, material });
            }
        }
    }

    pub fn despawn(&mut self, id: NodeId) {
        if self.nodes.remove(&id).is_some() {
            // Children of a despawned node are the adapter's job to remove;
            // mirror that in local state too.
            let orphans: Vec<NodeId> = self
                .nodes
                .iter()
                .filter(|(_, s)| s.parent == Some(id))
                .map(|(k, _)| *k)
                .collect();
            for child in orphans {
                self.despawn(child);
            }
            self.commands.push(SceneCommand::Despawn { id });
        }
    }

    /// Select which camera node renders the view.
    pub fn set_active_camera(&mut self, id: NodeId) {
        self.commands.push(SceneCommand::SetActiveCamera { id });
    }

    pub fn set_environment(&mut self, env: EnvironmentDesc) {
        self.commands.push(SceneCommand::SetEnvironment { env });
    }

    /// Current local transform of a node, if it exists.
    pub fn transform(&self, id: NodeId) -> Option<TransformDesc> {
        self.nodes.get(&id).map(|s| s.transform)
    }

    /// World transform, resolving the parent chain.
    pub fn world_transform(&self, id: NodeId) -> Option<TransformDesc> {
        let state = self.nodes.get(&id)?;
        match state.parent {
            None => Some(state.transform),
            Some(parent) => {
                let pt = self.world_transform(parent)?;
                Some(pt.mul(&state.transform))
            }
        }
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Drain pending commands for the render adapter.
    pub fn drain_commands(&mut self) -> Vec<SceneCommand> {
        std::mem::take(&mut self.commands)
    }

    pub fn has_pending(&self) -> bool {
        !self.commands.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    #[test]
    fn spawn_records_command_and_state() {
        let mut sg = SceneGraph::new();
        let mesh = sg.add_mesh(MeshData::unit_test_triangle());
        let node = sg.spawn_mesh(
            mesh,
            MaterialDesc::default(),
            TransformDesc::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        );
        assert!(sg.contains(node));
        assert_eq!(
            sg.transform(node).unwrap().translation,
            Vec3::new(1.0, 2.0, 3.0)
        );
        let cmds = sg.drain_commands();
        assert_eq!(cmds.len(), 2); // AddMesh + Spawn
        assert!(!sg.has_pending());
    }

    #[test]
    fn despawn_cascades_to_children() {
        let mut sg = SceneGraph::new();
        let root = sg.spawn_group(TransformDesc::IDENTITY);
        let mesh = sg.add_mesh(MeshData::unit_test_triangle());
        let child =
            sg.spawn_mesh_child(root, mesh, MaterialDesc::default(), TransformDesc::IDENTITY);
        sg.drain_commands();
        sg.despawn(root);
        assert!(!sg.contains(root));
        assert!(!sg.contains(child));
        let cmds = sg.drain_commands();
        assert_eq!(cmds.len(), 2); // both despawns recorded
    }

    #[test]
    fn release_frees_exactly_the_span_and_nothing_before_it() {
        let mut sg = SceneGraph::new();
        // Persistent assets, registered BEFORE the block: a release of the
        // block must not touch them — this is the ship-hull-outliving-the-
        // sector case, and it is the whole reason spans are closed ranges.
        let keep_mesh = sg.add_mesh(MeshData::unit_test_triangle());
        let keep_tex = sg.add_texture(vec![1, 2, 3], TextureSampling::Nearest);

        let mark = sg.asset_mark();
        let sector_mesh_a = sg.add_mesh(MeshData::unit_test_triangle());
        let sector_mesh_b = sg.add_mesh(MeshData::unit_test_triangle());
        let sector_tex = sg.add_texture(vec![4, 5, 6], TextureSampling::Linear);
        let span = sg.assets_since(mark);

        // Registered AFTER the span closed: also untouchable.
        let later_mesh = sg.add_mesh(MeshData::unit_test_triangle());
        let later_tex = sg.add_texture(vec![7], TextureSampling::Nearest);

        assert_eq!(span.mesh_count(), 2);
        assert_eq!(span.texture_count(), 1);

        sg.drain_commands();
        sg.release_assets(span);
        let removed: Vec<_> = sg
            .drain_commands()
            .into_iter()
            .map(|c| match c {
                SceneCommand::RemoveMesh { id } => (Some(id), None),
                SceneCommand::RemoveTexture { id } => (None, Some(id)),
                other => panic!("unexpected command {other:?}"),
            })
            .collect();
        let meshes: Vec<_> = removed.iter().filter_map(|(m, _)| *m).collect();
        let textures: Vec<_> = removed.iter().filter_map(|(_, t)| *t).collect();
        assert_eq!(meshes, vec![sector_mesh_a, sector_mesh_b]);
        assert_eq!(textures, vec![sector_tex]);
        assert!(!meshes.contains(&keep_mesh));
        assert!(!meshes.contains(&later_mesh));
        assert!(!textures.contains(&keep_tex));
        assert!(!textures.contains(&later_tex));
    }

    #[test]
    fn a_mark_or_span_from_another_graph_is_refused() {
        // Counters are per-graph, so graph A's mark describes DIFFERENT
        // assets on graph B. Honouring it would release strangers —
        // typically persistent pack art, failing as missing hulls far from
        // the actual bug.
        let mut a = SceneGraph::new();
        let mut b = SceneGraph::new();
        b.add_mesh(MeshData::unit_test_triangle());
        b.add_mesh(MeshData::unit_test_triangle());

        let foreign = b.assets_since(a.asset_mark());
        assert_eq!(foreign, AssetSpan::EMPTY, "foreign mark must not close");

        let mark = b.asset_mark();
        b.add_mesh(MeshData::unit_test_triangle());
        let span = b.assets_since(mark);
        b.drain_commands();
        a.release_assets(span);
        assert!(!a.has_pending(), "foreign span must not release anything");
        b.release_assets(span);
        assert_eq!(b.drain_commands().len(), 1, "home graph still releases");
    }

    #[test]
    fn an_empty_span_releases_nothing() {
        let mut sg = SceneGraph::new();
        sg.add_mesh(MeshData::unit_test_triangle());
        let mark = sg.asset_mark();
        let span = sg.assets_since(mark);
        sg.drain_commands();
        sg.release_assets(span);
        assert!(!sg.has_pending(), "empty span must be a no-op");
        sg.release_assets(AssetSpan::EMPTY);
        assert!(!sg.has_pending(), "EMPTY must be a no-op");
    }

    #[test]
    fn world_transform_composes_parent_chain() {
        let mut sg = SceneGraph::new();
        let root = sg.spawn_group(TransformDesc::from_translation(Vec3::X * 10.0));
        let child = sg.spawn_group_child(
            root,
            TransformDesc {
                translation: Vec3::Y * 2.0,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        );
        let wt = sg.world_transform(child).unwrap();
        assert_eq!(wt.translation, Vec3::new(10.0, 2.0, 0.0));
    }
}
