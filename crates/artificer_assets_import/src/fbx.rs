//! FBX and OBJ front-end, via ufbx.
//!
//! ufbx reads binary FBX (7.x), ASCII FBX, and OBJ through one API, so all
//! three arrive here. It also performs the axis and unit normalisation, which
//! is deliberate: doing it inside the reader means it happens once, on the
//! file's own declared metadata, instead of being reconstructed per asset
//! from a manifest guess.

use crate::error::ImportError;
use crate::source::{SourceMesh, SourcePart, SourceScene};
use artificer_assets::AxisConvention;

/// Read a file into the neutral [`SourceScene`].
///
/// `frame` is the manifest's declared convention. Under
/// [`AxisConvention::FromSource`] the file's own metadata is trusted, which
/// is right for FBX and glTF because both carry signed axis information. An
/// explicit frame means the caller is OVERRIDING that metadata (absent or
/// wrong), so the reader is told not to convert and the post-processor
/// applies the caller's basis instead — one conversion, not two.
pub fn read(path: &str, frame: AxisConvention) -> Result<SourceScene, ImportError> {
    let convert_in_reader = matches!(frame, AxisConvention::FromSource);

    let opts = ufbx::LoadOpts {
        // Target the engine's frame directly. Verified against real art:
        // SM_Ship_Fighter_01 comes out 13.33 x 3.70 x 12.91 m.
        target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
        target_unit_meters: 1.0,
        // LOAD-BEARING. ufbx's default (`TransformRoot`) leaves the
        // conversion on the root node transform, so `mesh.vertices` come back
        // in the file's original units -- a Synty hull reads as 1332 "metres"
        // wide. `ModifyGeometry` bakes it into the vertex data, which is what
        // every consumer here assumes.
        space_conversion: if convert_in_reader {
            ufbx::SpaceConversion::ModifyGeometry
        } else {
            // The caller is overriding the file's frame, so let vertices come
            // through untouched and correct them once, downstream.
            ufbx::SpaceConversion::TransformRoot
        },
        generate_missing_normals: false, // the post-processor owns this
        ..Default::default()
    };

    let scene = ufbx::load_file(path, opts)
        .map_err(|e| ImportError::Read(path.to_string(), format!("{e:?}")))?;

    let mut out = SourceScene {
        declared_units_per_metre: Some(scene.settings.unit_meters as f32),
        declared_frame: Some(format!("{:?}", scene.settings.axes)),
        ..Default::default()
    };

    for (mesh_index, mesh) in scene.meshes.iter().enumerate() {
        out.meshes.push(read_mesh(mesh, mesh_index)?);
    }
    Ok(out)
}

fn read_mesh(mesh: &ufbx::Mesh, mesh_index: usize) -> Result<SourceMesh, ImportError> {
    // ufbx meshes carry no name of their own; the name lives on the node that
    // instances them. Fall back to a positional name so `MeshSelect::Named`
    // still has something to match and diagnostics stay readable.
    let name = mesh
        .element
        .instances
        .iter()
        .next()
        .map(|node| node.element.name.to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("mesh_{mesh_index}"));

    let positions = &mesh.vertex_position;
    if !positions.exists {
        return Err(ImportError::Geometry(
            name.clone(),
            "mesh has no vertex positions".into(),
        ));
    }

    // Vertex attributes are indexed independently in FBX. Flattening to one
    // vertex per index is what makes the result usable as a GPU buffer, and
    // it is why vertex counts differ from a format like OBJ that dedupes.
    let index_count = mesh.num_indices;
    let mut out = SourceMesh {
        name: name.clone(),
        positions: Vec::with_capacity(index_count),
        normals: Vec::new(),
        uvs: Vec::new(),
        parts: Vec::new(),
    };
    let has_normals = mesh.vertex_normal.exists;
    let has_uvs = mesh.vertex_uv.exists;

    for i in 0..index_count {
        let p = positions[i];
        out.positions.push([p.x as f32, p.y as f32, p.z as f32]);
        if has_normals {
            let n = mesh.vertex_normal[i];
            out.normals.push([n.x as f32, n.y as f32, n.z as f32]);
        }
        if has_uvs {
            let uv = mesh.vertex_uv[i];
            out.uvs.push([uv.x as f32, uv.y as f32]);
        }
    }

    // Triangulate per material part, so the grouping survives into submeshes
    // instead of being flattened away.
    let mut scratch = vec![0u32; mesh.max_face_triangles * 3];
    for part in mesh.material_parts.iter() {
        let material = mesh
            .materials
            .get(part.index as usize)
            .map(|m| m.element.name.to_string())
            .filter(|n| !n.is_empty());

        let mut indices = Vec::with_capacity(part.num_triangles * 3);
        for &face_index in part.face_indices.iter() {
            let face = mesh.faces[face_index as usize];
            // Points and lines carry no area; skipping them keeps a stray
            // helper object from producing degenerate triangles.
            if face.num_indices < 3 {
                continue;
            }
            let tris = mesh.triangulate_face(&mut scratch, face);
            indices.extend_from_slice(&scratch[..(tris as usize) * 3]);
        }
        if indices.is_empty() {
            continue;
        }
        out.parts.push(SourcePart {
            material,
            material_index: part.index,
            indices,
        });
    }

    if out.parts.is_empty() {
        return Err(ImportError::Geometry(
            name,
            "mesh has no triangles (only points or lines?)".into(),
        ));
    }
    Ok(out)
}
