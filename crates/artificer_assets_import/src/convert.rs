//! The shared post-processor: [`SourceScene`] plus a [`MeshImport`] becomes a
//! [`PackedAsset`].
//!
//! Every front-end funnels through here, so correction happens in ONE place
//! and in the order [`MeshImport`] documents. Re-read that doc block before
//! changing anything in this file: the order is part of the contract, not an
//! implementation detail, because mirroring before or after a rotation gives
//! different geometry and a bounds-centred pivot computed before rotation
//! lands somewhere else.

use crate::error::ImportError;
use crate::source::{SourceMesh, SourceScene};
use artificer_assets::{
    material_for, AssetPack, AssetRecord, AxisConvention, MaterialMapping, MaterialSelector,
    MeshImport, MeshSelect, PackedAsset, PivotPolicy, SubMesh,
};
use artificer_scene::MeshData;
use glam::{EulerRot, Mat3, Quat, Vec3};

/// A merged, still-uncorrected mesh plus the material grouping it carries.
struct Merged {
    mesh: MeshData,
    /// (source material name, source slot index, index range) in merge order.
    parts: Vec<(Option<String>, u32, std::ops::Range<usize>)>,
}

/// Select the meshes this import wants and merge them into one.
///
/// Merge order is fixed by [`MeshSelect`] — file order for `All`, listed
/// order for `Named` — because it determines the index buffer and therefore
/// the baked bytes.
fn select_and_merge(scene: &SourceScene, import: &MeshImport) -> Result<Merged, ImportError> {
    let chosen: Vec<&SourceMesh> = match &import.select {
        MeshSelect::All => scene.meshes.iter().collect(),
        MeshSelect::Named(names) => {
            let mut out = Vec::with_capacity(names.len());
            for name in names {
                let mesh = scene
                    .mesh_named(name)
                    .ok_or_else(|| ImportError::NoSuchMesh {
                        asset: import.id.clone(),
                        wanted: name.clone(),
                        available: scene.mesh_names().iter().map(|s| s.to_string()).collect(),
                    })?;
                out.push(mesh);
            }
            out
        }
        MeshSelect::Index(i) => {
            let mesh = scene
                .meshes
                .get(*i as usize)
                .ok_or_else(|| ImportError::NoSuchMesh {
                    asset: import.id.clone(),
                    wanted: format!("index {i}"),
                    available: scene.mesh_names().iter().map(|s| s.to_string()).collect(),
                })?;
            vec![mesh]
        }
    };
    if chosen.is_empty() {
        return Err(ImportError::NoSuchMesh {
            asset: import.id.clone(),
            wanted: "any mesh".into(),
            available: vec![],
        });
    }

    let mut mesh = MeshData::default();
    let mut parts = Vec::new();
    for source in chosen {
        let base = mesh.positions.len() as u32;
        mesh.positions.extend_from_slice(&source.positions);

        // A source with no normals or UVs still has to produce one per
        // vertex. Normals are generated after correction; UVs are zeroed,
        // which is correct for untextured assets and deliberately obvious on
        // atlas-mapped ones.
        if source.normals.len() == source.positions.len() {
            mesh.normals.extend_from_slice(&source.normals);
        } else {
            mesh.normals
                .extend(std::iter::repeat_n([0.0, 0.0, 0.0], source.positions.len()));
        }
        if source.uvs.len() == source.positions.len() {
            mesh.uvs.extend_from_slice(&source.uvs);
        } else {
            mesh.uvs
                .extend(std::iter::repeat_n([0.0, 0.0], source.positions.len()));
        }

        for part in &source.parts {
            let start = mesh.indices.len();
            mesh.indices.extend(part.indices.iter().map(|i| i + base));
            parts.push((
                part.material.clone(),
                part.material_index,
                start..mesh.indices.len(),
            ));
        }
    }
    Ok(Merged { mesh, parts })
}

/// Steps 1-4 of the correction order, as one matrix.
fn correction_basis(import: &MeshImport) -> Result<Mat3, ImportError> {
    // 1. Axis frame. The reader always normalises UNITS; under `FromSource`
    //    it also converted axes using the file's own metadata, so there is
    //    nothing left to do. An explicit frame means the reader deliberately
    //    left the axes alone and the caller's basis applies here instead.
    let basis = match import.axis {
        AxisConvention::FromSource => Mat3::IDENTITY,
        explicit => explicit.to_engine_basis().ok_or_else(|| {
            ImportError::BadCorrection(import.id.clone(), "axis frame is degenerate".into())
        })?,
    };
    // 2. Euler XYZ, intrinsic, about the origin.
    let rotation = Mat3::from_quat(Quat::from_euler(
        EulerRot::XYZ,
        import.rotation_deg[0].to_radians(),
        import.rotation_deg[1].to_radians(),
        import.rotation_deg[2].to_radians(),
    ));
    // 3. Mirror across X. 4. Uniform scale.
    let mirror = if import.mirror_x {
        Mat3::from_diagonal(Vec3::new(-1.0, 1.0, 1.0))
    } else {
        Mat3::IDENTITY
    };
    let scale = Mat3::from_diagonal(Vec3::splat(import.unit_scale));
    Ok(scale * mirror * rotation * basis)
}

/// Whether an odd number of mirroring operations happened, which reverses
/// triangle winding and has to be undone.
///
/// `mirror_x` and a handedness-changing axis frame each count as one, and
/// `flip_winding` XORs on top — so mirroring a part from a left-handed source
/// does NOT double-flip back to inside-out geometry.
fn winding_flipped(import: &MeshImport) -> bool {
    let axis_mirrors = import.axis.changes_handedness().unwrap_or(false);
    axis_mirrors ^ import.mirror_x ^ import.flip_winding
}

pub fn convert(
    scene: &SourceScene,
    import: &MeshImport,
    pack: &mut AssetPack,
) -> Result<PackedAsset, ImportError> {
    let Merged { mut mesh, parts } = select_and_merge(scene, import)?;

    let basis = correction_basis(import)?;
    for p in mesh.positions.iter_mut() {
        *p = (basis * Vec3::from_array(*p)).to_array();
    }
    // Normals get the SAME basis, not just a re-normalise: rotated with the
    // geometry and negated in X by a mirror. (For a rotation the
    // inverse-transpose is the rotation; for a mirror it is the mirror; a
    // uniform scale washes out under normalisation.) Skipping this is what
    // leaves mirrored parts lit inside-out.
    // Which vertices arrived WITHOUT a normal, tracked per vertex rather than
    // per mesh. A merge can mix sources: one mesh carrying normals used to
    // suppress generation for every mesh that lacked them, shipping half an
    // asset to the GPU with (0,0,0) normals -- which renders unlit black and
    // passes validation, because zero is finite.
    let missing_normals: Vec<bool> = mesh.normals.iter().map(|n| *n == [0.0, 0.0, 0.0]).collect();
    for n in mesh.normals.iter_mut() {
        let v = (basis * Vec3::from_array(*n)).normalize_or_zero();
        *n = v.to_array();
    }

    // 5. Pivot, measured on the geometry as it now stands.
    let bounds = mesh
        .bounds()
        .ok_or_else(|| ImportError::Geometry(import.id.clone(), "no geometry".into()))?;
    let shift = match import.pivot {
        PivotPolicy::AsAuthored => Vec3::ZERO,
        PivotPolicy::BoundsCenter => -bounds.center(),
        PivotPolicy::BaseY => Vec3::new(-bounds.center().x, -bounds.min.y, -bounds.center().z),
    };
    if shift != Vec3::ZERO {
        for p in mesh.positions.iter_mut() {
            *p = (Vec3::from_array(*p) + shift).to_array();
        }
    }

    // 6. Winding.
    if winding_flipped(import) {
        for tri in mesh.indices.chunks_exact_mut(3) {
            tri.swap(1, 2);
        }
    }

    // 7. Generate normals for the vertices that arrived without them, and
    // ONLY those: overwriting authored normals with generated ones would
    // smooth every hard edge in flat-shaded art. Pinned to
    // recompute_normals (area-weighted smooth) because a different smoothing
    // rule silently changes the baked bytes.
    if missing_normals.iter().any(|m| *m) {
        let mut generated = mesh.clone();
        generated.recompute_normals();
        for (i, missing) in missing_normals.iter().enumerate() {
            if *missing {
                mesh.normals[i] = generated.normals[i];
            }
        }
    }

    // Weld identical vertices.
    //
    // Front-ends hand back one vertex per INDEX, because FBX indexes each
    // attribute separately and flattening is the only way to get a GPU-shaped
    // buffer out of it. That flattening is lossy in the wrong direction: a
    // 280-triangle model arrives with 840 vertices where 410 would do. For a
    // browser-first project that is doubled geometry in the bundle and in
    // vertex memory, for nothing.
    //
    // Welding is on exact bit patterns, not a tolerance: near-equal vertices
    // stay distinct (a tolerance would silently fuse a hard edge into a
    // smooth one), and first occurrence wins so the result is deterministic.
    weld(&mut mesh);

    if let Err(e) = mesh.validate() {
        return Err(ImportError::Geometry(import.id.clone(), e));
    }

    // Budget is enforced here rather than left to the contract check, so the
    // error names the import that blew it and the numbers that did.
    let triangles = mesh.triangle_count() as u64;
    let vertices = mesh.vertex_count() as u64;
    if triangles > import.budget.max_triangles as u64
        || vertices > import.budget.max_vertices as u64
    {
        return Err(ImportError::OverBudget {
            asset: import.id.clone(),
            triangles,
            vertices,
            max_triangles: import.budget.max_triangles,
            max_vertices: import.budget.max_vertices,
        });
    }

    // Submeshes must tile the index buffer contiguously, so parts are laid
    // out in merge order and each one takes the range it already occupies.
    // Winding reversal is in-place and per triangle, so ranges still hold.
    let mut submeshes = Vec::with_capacity(parts.len());
    let mut slots: Vec<String> = Vec::new();
    for (material_name, material_index, range) in parts {
        let mapping = binding_for(import, material_name.as_deref(), material_index);
        let slot = material_name
            .clone()
            .unwrap_or_else(|| format!("{material_index}"));
        let material = material_for(pack, &import.id, &slot, mapping)?;
        if let Some(id) = &material {
            slots.push(id.clone());
        }
        submeshes.push(SubMesh {
            material,
            index_start: u32::try_from(range.start).map_err(|_| ImportError::TooLarge {
                asset: import.id.clone(),
                indices: range.start,
            })?,
            index_count: u32::try_from(range.len()).map_err(|_| ImportError::TooLarge {
                asset: import.id.clone(),
                indices: range.len(),
            })?,
        });
    }
    slots.sort();
    slots.dedup();

    let record = build_record(import, &mesh, slots)?;
    Ok(PackedAsset {
        record,
        mesh,
        submeshes,
    })
}

/// Collapse duplicate vertices, rewriting indices to match.
///
/// The key is the raw bit pattern of position, normal and UV, so only
/// genuinely identical vertices merge. `-0.0` is normalised to `0.0` first,
/// because the two compare equal as floats but differ as bits, and leaving
/// that unhandled would make welding depend on which sign a exporter happened
/// to emit — a determinism hole hiding inside an optimisation.
fn weld(mesh: &mut MeshData) {
    fn bits(v: f32) -> u32 {
        (if v == 0.0 { 0.0 } else { v }).to_bits()
    }

    let mut seen: std::collections::HashMap<[u32; 8], u32> =
        std::collections::HashMap::with_capacity(mesh.positions.len());
    let mut positions = Vec::with_capacity(mesh.positions.len());
    let mut normals = Vec::with_capacity(mesh.normals.len());
    let mut uvs = Vec::with_capacity(mesh.uvs.len());
    let mut remap: Vec<u32> = Vec::with_capacity(mesh.positions.len());

    for ((p, n), uv) in mesh
        .positions
        .iter()
        .copied()
        .zip(mesh.normals.iter().copied())
        .zip(mesh.uvs.iter().copied())
    {
        let key = [
            bits(p[0]),
            bits(p[1]),
            bits(p[2]),
            bits(n[0]),
            bits(n[1]),
            bits(n[2]),
            bits(uv[0]),
            bits(uv[1]),
        ];
        // HashMap is used only as a lookup here; the OUTPUT order is the
        // order of first appearance, so nothing about the baked bytes depends
        // on hash iteration order.
        let next = *seen.entry(key).or_insert_with(|| {
            positions.push(p);
            normals.push(n);
            uvs.push(uv);
            (positions.len() - 1) as u32
        });
        remap.push(next);
    }

    if positions.len() == mesh.positions.len() {
        return; // nothing shared; leave the buffers alone
    }
    for index in mesh.indices.iter_mut() {
        *index = remap[*index as usize];
    }
    mesh.positions = positions;
    mesh.normals = normals;
    mesh.uvs = uvs;
}

/// Resolve which mapping applies to a source material slot.
///
/// A `Name` binding wins over an `Index` one: names are what an author writes
/// deliberately, indices are positional and shift when a file is re-exported.
fn binding_for<'a>(import: &'a MeshImport, name: Option<&str>, index: u32) -> &'a MaterialMapping {
    if let Some(name) = name {
        if let Some(b) = import
            .material_bindings
            .iter()
            .find(|b| matches!(&b.select, MaterialSelector::Name(n) if n == name))
        {
            return &b.mapping;
        }
    }
    if let Some(b) = import
        .material_bindings
        .iter()
        .find(|b| matches!(&b.select, MaterialSelector::Index(i) if *i == index))
    {
        return &b.mapping;
    }
    &import.material
}

fn build_record(
    import: &MeshImport,
    mesh: &MeshData,
    material_slots: Vec<String>,
) -> Result<AssetRecord, ImportError> {
    let measured = mesh
        .bounds()
        .ok_or_else(|| ImportError::Geometry(import.id.clone(), "no geometry".into()))?;

    // A declared bounds is a CROSS-CHECK, not a substitute: the record keeps
    // what the author declared so `validate_asset` compares it against the
    // geometry that was actually produced. Recording the measured value in
    // both places would make that check compare a number with itself.
    let (bounds_min, bounds_max) = match &import.declared_bounds {
        Some(declared) => declared.as_record_bounds(),
        None => (measured.min.to_array(), measured.max.to_array()),
    };

    Ok(AssetRecord {
        id: import.id.clone(),
        category: import.category,
        // Scale is baked into the vertices by now, so the record is metric.
        unit_scale: 1.0,
        orientation: "y-up,-z-forward".to_string(),
        pivot: [0.0; 3],
        bounds_min,
        bounds_max,
        collision: import.collision,
        sockets: import.sockets.clone(),
        material_slots,
        lod: import.lod,
        budget: import.budget,
        provenance: import.provenance.clone(),
        license: import.license.clone(),
        gameplay_ref: import.gameplay_ref.clone(),
    })
}
