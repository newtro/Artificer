//! FBX and OBJ front-end, via ufbx.
//!
//! ufbx reads binary FBX (7.x), ASCII FBX, and OBJ through one API, so all
//! three arrive here.
//!
//! Two things this reader gets right that are easy to get wrong silently, and
//! that a bounding-box assertion cannot see:
//!
//! * It targets a frame chosen BY MEASUREMENT against the glTF export of the
//!   same models — see [`engine_axes`]. ufbx's `right_handed_y_up` helper
//!   leaves hulls 180 degrees out, and the obvious-looking correction is a
//!   mirror rather than a yaw.
//! * It applies NODE TRANSFORMS. `mesh.vertex_position` is mesh-LOCAL, and a
//!   mesh's placement lives on the node that instances it. Three of this
//!   repo's own fixtures carry a node translation of (-2, 0, -1.5); reading
//!   mesh-local vertices drops it and lands the model 2.5 m from where the
//!   glTF reader puts the same model.

use crate::error::ImportError;
use crate::source::{SourceMesh, SourcePart, SourceScene};
use artificer_assets::AxisConvention;
use glam::{Mat3, Mat4, Vec3};

/// The engine's frame, DERIVED BY MEASUREMENT rather than by reasoning about
/// what ufbx's axis vocabulary ought to mean.
///
/// `ufbx::CoordinateAxes::right_handed_y_up()` is not it. Compared against the
/// glTF export of the SAME Kenney models — vertex cloud to vertex cloud, since
/// both fixtures are symmetric in X and Z and their bounding boxes cannot tell
/// a 180-degree yaw from nothing — only this combination agrees with glTF on
/// both:
///
/// ```text
///   target axes        craft_racer   corridor
///   +X +Y +Z (ufbx)      4.4112       0.0000   <- passes only because
///   +X +Y -Z             4.4375       1.9697      corridor is 180-symmetric
///   -X +Y -Z             0.0000       0.0000   <- agrees on both
///   -X +Y +Z             0.3384       1.9697
/// ```
///
/// It is a 180-degree yaw relative to ufbx's helper, so it is a ROTATION:
/// winding and handedness are untouched. `+X +Y -Z` looks like the obvious
/// "-Z forward" spelling and is a MIRROR — it would have turned every
/// imported hull inside out while the bounding box still looked right.
///
/// It also lands Synty hulls the correct way round: `SM_Ship_Bomber_01`'s
/// narrow nose sits at +Z under ufbx's helper and at -Z under this frame,
/// which is where the engine expects forward to be.
fn engine_axes() -> ufbx::CoordinateAxes {
    ufbx::CoordinateAxes {
        right: ufbx::CoordinateAxis::NegativeX,
        up: ufbx::CoordinateAxis::PositiveY,
        front: ufbx::CoordinateAxis::NegativeZ,
    }
}

fn load_opts(target: ufbx::CoordinateAxes) -> ufbx::LoadOpts<'static> {
    ufbx::LoadOpts {
        target_axes: target,
        target_unit_meters: 1.0,
        // LOAD-BEARING. ufbx's default (`TransformRoot`) leaves the conversion
        // on the root node transform, so vertices come back in the file's
        // original units -- a Synty hull reads as 1332 "metres" wide.
        // `ModifyGeometry` bakes it into the vertex data.
        space_conversion: ufbx::SpaceConversion::ModifyGeometry,
        generate_missing_normals: false, // the post-processor owns this
        // OBJ carries no grouping metadata worth splitting on, and leaving
        // these to their defaults made an OBJ report twice the triangles of
        // the FBX of the same model.
        obj_merge_objects: true,
        obj_merge_groups: true,
        obj_split_groups: false,
        ..Default::default()
    }
}

/// Textures stored inside the FBX itself.
///
/// FBX keeps embedded media on `Video` elements: a filename and the encoded
/// bytes. A file that references textures on disk instead simply has empty
/// content, which is why zero-length blobs are dropped rather than reported
/// as textures the caller can bind.
fn embedded_textures(scene: &ufbx::Scene) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for video in scene.videos.iter() {
        if video.content.is_empty() {
            continue;
        }
        // Prefer the file's own name for the map, so `..._Normal.jpg` binds as
        // "normal" rather than as an opaque index. Deriving the role from
        // ORDER is what makes a hand-rolled extractor swap the colour and
        // roughness maps on the next model.
        let stem = std::path::Path::new(&*video.relative_filename)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let role = stem
            .rsplit(['_', '-'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("texture")
            .to_lowercase();
        out.push((role, video.content.to_vec()));
    }
    out
}

/// Read a file into the neutral [`SourceScene`].
///
/// Under [`AxisConvention::FromSource`] the file's own axis metadata is
/// trusted and ufbx converts straight to the engine frame.
///
/// An explicit frame means the caller is OVERRIDING metadata that is absent
/// or wrong, so the axes must NOT be converted here — but the units still
/// must be, because `MeshImport` documents that an explicit frame "overrides
/// axes and handedness ONLY". That is done by targeting the file's own
/// declared axes (a no-op rotation) while still normalising to metres, which
/// costs a first pass to discover what those axes are. Only the override path
/// pays it, and skipping it is how a declared-no-op frame silently multiplied
/// every coordinate by 100.
pub fn read(path: &str, frame: AxisConvention) -> Result<SourceScene, ImportError> {
    let target = match frame {
        AxisConvention::FromSource => engine_axes(),
        AxisConvention::Explicit { .. } => {
            let probe = ufbx::load_file(path, load_opts(engine_axes()))
                .map_err(|e| ImportError::Read(path.to_string(), format!("{e:?}")))?;
            probe.settings.axes
        }
    };

    let scene = ufbx::load_file(path, load_opts(target))
        .map_err(|e| ImportError::Read(path.to_string(), format!("{e:?}")))?;

    let mut out = SourceScene {
        declared_units_per_metre: Some(scene.settings.unit_meters as f32),
        declared_frame: Some(format!("{:?}", scene.settings.axes)),
        embedded_textures: embedded_textures(&scene),
        ..Default::default()
    };

    // Walk NODES, not meshes: a mesh's placement lives on the node that
    // instances it, and the same mesh may be instanced more than once.
    for node in scene.nodes.iter() {
        let Some(mesh) = &node.mesh else { continue };
        let name = if node.element.name.is_empty() {
            format!("mesh_{}", out.meshes.len())
        } else {
            node.element.name.to_string()
        };
        out.meshes
            .push(read_mesh(mesh, &name, to_mat4(&node.geometry_to_world))?);
    }

    if out.meshes.is_empty() {
        return Err(ImportError::Read(
            path.to_string(),
            "file contains no meshes".into(),
        ));
    }
    Ok(out)
}

fn to_mat4(m: &ufbx::Matrix) -> Mat4 {
    // ufbx matrices are 3x4, column-major, with an implicit (0,0,0,1) row.
    Mat4::from_cols(
        glam::Vec4::new(m.m00 as f32, m.m10 as f32, m.m20 as f32, 0.0),
        glam::Vec4::new(m.m01 as f32, m.m11 as f32, m.m21 as f32, 0.0),
        glam::Vec4::new(m.m02 as f32, m.m12 as f32, m.m22 as f32, 0.0),
        glam::Vec4::new(m.m03 as f32, m.m13 as f32, m.m23 as f32, 1.0),
    )
}

fn read_mesh(mesh: &ufbx::Mesh, name: &str, world: Mat4) -> Result<SourceMesh, ImportError> {
    let positions = &mesh.vertex_position;
    if !positions.exists {
        return Err(ImportError::Geometry(
            name.to_string(),
            "mesh has no vertex positions".into(),
        ));
    }

    // Node transforms can carry non-uniform scale, so normals take the
    // inverse-transpose or a squashed asset ends up mis-lit.
    let normal_matrix = Mat3::from_mat4(world).inverse().transpose();

    // Vertex attributes are indexed independently in FBX. Flattening to one
    // vertex per index is the only way to get a GPU-shaped buffer out; the
    // post-processor welds the duplicates back down afterwards.
    let index_count = mesh.num_indices;
    let mut out = SourceMesh {
        name: name.to_string(),
        positions: Vec::with_capacity(index_count),
        normals: Vec::new(),
        uvs: Vec::new(),
        parts: Vec::new(),
    };
    let has_normals = mesh.vertex_normal.exists;
    let has_uvs = mesh.vertex_uv.exists;

    for i in 0..index_count {
        let p = positions[i];
        let v = world.transform_point3(Vec3::new(p.x as f32, p.y as f32, p.z as f32));
        out.positions.push(v.to_array());
        if has_normals {
            let n = mesh.vertex_normal[i];
            let n =
                (normal_matrix * Vec3::new(n.x as f32, n.y as f32, n.z as f32)).normalize_or_zero();
            out.normals.push(n.to_array());
        }
        if has_uvs {
            let uv = mesh.vertex_uv[i];
            // FBX puts the V origin at the BOTTOM-left; glTF, and every
            // renderer that follows it, put it at the top-left. Passing V
            // through raw sends each UV island to the mirrored row of the
            // atlas, which does not look like an error -- it looks like a
            // ship painted in camouflage, because every island still lands on
            // *some* texture. Cost an afternoon to find. Flip it here, at the
            // one point where FBX's convention is known.
            out.uvs.push([uv.x as f32, 1.0 - uv.y as f32]);
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
            name.to_string(),
            "mesh has no triangles (only points or lines?)".into(),
        ));
    }
    Ok(out)
}
