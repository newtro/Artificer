//! glTF and GLB front-end.
//!
//! Small on purpose. Every correction lives in [`crate::convert`], so this
//! reader's only job is to produce a [`SourceScene`] — which is why adding a
//! format is a reader rather than a pipeline.
//!
//! glTF is already the engine's conventions: right-handed, Y up, -Z forward,
//! metres. So unlike the FBX path there is no unit or axis normalisation to
//! do here; a manifest that declares an explicit frame is overriding a file
//! that was probably already correct, and [`crate::convert`] applies that.

use crate::error::ImportError;
use crate::source::{SourceMesh, SourcePart, SourceScene};
use glam::{Mat4, Vec3};

pub fn read(path: &str) -> Result<SourceScene, ImportError> {
    // Buffers only — NOT images.
    //
    // `gltf::import` also resolves every referenced texture, so a model whose
    // images sit in a sibling folder (which is how both Kenney kits ship, and
    // most exporters) fails to import at all. Textures in this pipeline are
    // declared in the manifest, not inherited from the model file, so an
    // unreachable image is irrelevant to geometry and must not stop a bake.
    let gltf::Gltf { document, blob } =
        gltf::Gltf::open(path).map_err(|e| ImportError::Read(path.to_string(), e.to_string()))?;
    let base = std::path::Path::new(path).parent();
    let buffers = gltf::import_buffers(&document, base, blob)
        .map_err(|e| ImportError::Read(path.to_string(), e.to_string()))?;
    let doc = document;

    let mut out = SourceScene {
        // glTF fixes both by specification, so there is nothing to discover.
        declared_units_per_metre: Some(1.0),
        declared_frame: Some("gltf: right-handed, y-up, -z-forward, metres".into()),
        ..Default::default()
    };

    // Walk the node hierarchy rather than doc.meshes(), because a mesh's
    // placement lives on the nodes that instance it. Reading meshes directly
    // would drop every transform and pile every part at the origin — the
    // multi-part assets this pipeline exists for would collapse into a heap.
    for scene in doc.scenes() {
        for node in scene.nodes() {
            read_node(&node, Mat4::IDENTITY, &buffers, &mut out, path)?;
        }
    }

    if out.meshes.is_empty() {
        return Err(ImportError::Read(
            path.to_string(),
            "file contains no meshes".into(),
        ));
    }
    Ok(out)
}

fn read_node(
    node: &gltf::Node,
    parent: Mat4,
    buffers: &[gltf::buffer::Data],
    out: &mut SourceScene,
    path: &str,
) -> Result<(), ImportError> {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent * local;

    if let Some(mesh) = node.mesh() {
        let name = node
            .name()
            .or_else(|| mesh.name())
            .map(str::to_string)
            .unwrap_or_else(|| format!("mesh_{}", mesh.index()));
        out.meshes
            .push(read_mesh(&mesh, &name, world, buffers, path)?);
    }

    for child in node.children() {
        read_node(&child, world, buffers, out, path)?;
    }
    Ok(())
}

fn read_mesh(
    mesh: &gltf::Mesh,
    name: &str,
    world: Mat4,
    buffers: &[gltf::buffer::Data],
    path: &str,
) -> Result<SourceMesh, ImportError> {
    let mut out = SourceMesh {
        name: name.to_string(),
        ..Default::default()
    };
    // Node transforms can carry non-uniform scale, so normals need the
    // inverse-transpose rather than the matrix itself, or a squashed asset
    // ends up mis-lit.
    let normal_matrix = Mat4::from_mat3(glam::Mat3::from_mat4(world).inverse().transpose());

    for primitive in mesh.primitives() {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            // Lines and points carry no surface. Skipping is right, but say so
            // -- silently dropping geometry is how a missing wing becomes a
            // twenty-minute mystery.
            log::warn!(
                "{path}: mesh '{name}' primitive {} is {:?}, not triangles — skipped",
                primitive.index(),
                primitive.mode()
            );
            continue;
        }
        let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));

        let Some(positions) = reader.read_positions() else {
            return Err(ImportError::Geometry(
                name.to_string(),
                "primitive has no POSITION attribute".into(),
            ));
        };
        let base = out.positions.len() as u32;
        for p in positions {
            let v = world.transform_point3(Vec3::from_array(p));
            out.positions.push(v.to_array());
        }
        let added = out.positions.len() - base as usize;

        match reader.read_normals() {
            Some(normals) => {
                for n in normals {
                    let v = normal_matrix
                        .transform_vector3(Vec3::from_array(n))
                        .normalize_or_zero();
                    out.normals.push(v.to_array());
                }
            }
            // The post-processor generates them; leaving zeroes marks the gap
            // without inventing a different smoothing rule here.
            None => out.normals.extend(std::iter::repeat_n([0.0; 3], added)),
        }
        match reader.read_tex_coords(0) {
            Some(uvs) => out.uvs.extend(uvs.into_f32()),
            None => out.uvs.extend(std::iter::repeat_n([0.0; 2], added)),
        }

        let indices: Vec<u32> = match reader.read_indices() {
            Some(idx) => idx.into_u32().map(|i| i + base).collect(),
            // An unindexed primitive is a plain triangle list.
            None => (base..base + added as u32).collect(),
        };
        if !indices.len().is_multiple_of(3) {
            return Err(ImportError::Geometry(
                name.to_string(),
                format!("{} indices is not a triangle list", indices.len()),
            ));
        }
        if indices.is_empty() {
            continue;
        }

        let material = primitive.material();
        out.parts.push(SourcePart {
            material: material.name().map(str::to_string),
            // Unnamed glTF materials are addressed by index; the default
            // material has no index of its own, so it takes slot 0.
            material_index: material.index().unwrap_or(0) as u32,
            indices,
        });
    }

    if out.parts.is_empty() {
        return Err(ImportError::Geometry(
            name.to_string(),
            "mesh has no triangle primitives".into(),
        ));
    }
    Ok(out)
}
