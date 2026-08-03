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
#[derive(Debug, Default)]
pub struct SceneGraph {
    next_node: u64,
    next_mesh: u64,
    next_texture: u64,
    nodes: HashMap<NodeId, NodeState>,
    commands: Vec<SceneCommand>,
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
