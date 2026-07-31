//! Importer tests against real CC0 exports and against synthetic geometry.
//!
//! Two kinds, doing two different jobs:
//!
//! * **Fixtures** (Kenney, CC0) prove the READERS work on files a real DCC
//!   tool produced — both FBX flavours, plus OBJ. Hand-authored toy files
//!   would not: they omit exactly the quirks that break importers.
//! * **Synthetic geometry** proves the CORRECTIONS work, because the input
//!   frame can be stated exactly and the expected output computed rather than
//!   eyeballed. The fixtures are all Y-up metric, so they cannot test axis
//!   conversion at all.

use artificer_assets::{
    AssetCategory, AssetPack, Axis, AxisConvention, CollisionProxy, Handedness, ImportManifest,
    ImportSource, LodPolicy, MaterialBinding, MaterialMapping, MaterialSelector, MeshImport,
    MeshSelect, PerfBudget, PivotPolicy, SourceFormat,
};
use artificer_assets_import::{
    convert, import_manifest, read_source, ImportError, SourceMesh, SourcePart, SourceScene,
};
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn base_import(id: &str, source: &str) -> MeshImport {
    MeshImport {
        id: id.into(),
        source: source.into(),
        select: MeshSelect::All,
        unit_scale: 1.0,
        axis: AxisConvention::FromSource,
        rotation_deg: [0.0; 3],
        mirror_x: false,
        flip_winding: false,
        pivot: PivotPolicy::AsAuthored,
        material: MaterialMapping::Default,
        material_bindings: vec![],
        category: AssetCategory::Prop,
        collision: CollisionProxy::None,
        sockets: vec![],
        lod: LodPolicy::Single,
        budget: PerfBudget {
            max_triangles: 50_000,
            max_vertices: 100_000,
        },
        declared_bounds: None,
        provenance: "kenney:space-kit".into(),
        license: "CC0".into(),
        gameplay_ref: "test".into(),
    }
}

fn manifest_for(path: &str, format: SourceFormat, import: MeshImport) -> ImportManifest {
    ImportManifest {
        sources: vec![ImportSource {
            id: import.source.clone(),
            path: path.into(),
            format,
        }],
        textures: vec![],
        meshes: vec![import],
    }
}

fn extents(pack: &AssetPack, id: &str) -> [f32; 3] {
    let mesh = &pack.find(id).expect("asset in pack").mesh;
    mesh.bounds().expect("geometry").extents().to_array()
}

fn assert_close(actual: [f32; 3], expected: [f32; 3], what: &str) {
    for i in 0..3 {
        assert!(
            (actual[i] - expected[i]).abs() < 0.01,
            "{what}: axis {i} was {}, expected {}",
            actual[i],
            expected[i]
        );
    }
}

// ----- readers, against real exports -----

#[test]
fn reads_ascii_fbx_at_metre_scale() {
    // Kenney Space Kit is ASCII FBX 7.3.0. Measured with a standalone ufbx
    // probe before any of this code existed: 1.200 x 0.750 x 2.026 m, 280
    // triangles.
    let pack = import_manifest(
        &fixtures(),
        &manifest_for(
            "craft_racer.fbx",
            SourceFormat::Auto,
            base_import("racer", "src"),
        ),
    )
    .expect("ascii fbx should import");
    assert_close(
        extents(&pack, "racer"),
        [1.200, 0.750, 2.026],
        "craft_racer",
    );
    assert_eq!(pack.find("racer").unwrap().mesh.triangle_count(), 280);
}

#[test]
fn reads_binary_fbx_at_metre_scale() {
    // Kenney Modular Space Kit is binary FBX 7700 — the same parser path the
    // project's real production art (binary 7400) takes.
    let pack = import_manifest(
        &fixtures(),
        &manifest_for(
            "corridor.fbx",
            SourceFormat::Auto,
            base_import("corridor", "src"),
        ),
    )
    .expect("binary fbx should import");
    assert_close(
        extents(&pack, "corridor"),
        [4.000, 4.250, 4.000],
        "corridor",
    );
    assert_eq!(pack.find("corridor").unwrap().mesh.triangle_count(), 860);
}

#[test]
fn reads_obj_through_the_same_reader() {
    // ufbx reads OBJ too, so the OBJ front-end costs nothing extra.
    let pack = import_manifest(
        &fixtures(),
        &manifest_for(
            "craft_racer.obj",
            SourceFormat::Auto,
            base_import("racer_obj", "src"),
        ),
    )
    .expect("obj should import");
    assert_close(
        extents(&pack, "racer_obj"),
        [1.200, 0.750, 2.026],
        "craft_racer.obj",
    );
}

#[test]
fn the_same_model_agrees_across_formats() {
    // Bounds and triangle count, never vertex count: FBX splits vertices at
    // normal/UV seams while OBJ dedupes, so vertex counts legitimately differ
    // (410 vs 142 for this model).
    let fbx = import_manifest(
        &fixtures(),
        &manifest_for(
            "craft_racer.fbx",
            SourceFormat::Auto,
            base_import("a", "src"),
        ),
    )
    .unwrap();
    let obj = import_manifest(
        &fixtures(),
        &manifest_for(
            "craft_racer.obj",
            SourceFormat::Auto,
            base_import("a", "src"),
        ),
    )
    .unwrap();

    let fbx_mesh = &fbx.find("a").unwrap().mesh;
    let obj_mesh = &obj.find("a").unwrap().mesh;
    assert_eq!(fbx_mesh.triangle_count(), obj_mesh.triangle_count());
    assert_close(
        fbx_mesh.bounds().unwrap().extents().to_array(),
        obj_mesh.bounds().unwrap().extents().to_array(),
        "fbx vs obj",
    );
    // Vertex counts agree AFTER welding, and that is the stronger statement:
    // the two formats describe the same geometry with different amounts of
    // sharing (FBX 410, OBJ 142 as authored), and both reduce to the same
    // buffer once identical vertices are merged.
    assert_eq!(fbx_mesh.vertex_count(), obj_mesh.vertex_count());
}

#[test]
fn identical_vertices_are_welded_rather_than_left_duplicated() {
    // Front-ends hand back one vertex per INDEX, because FBX indexes each
    // attribute separately and flattening is the only way to get a GPU-shaped
    // buffer out. Without welding a 280-triangle model ships 840 vertices --
    // doubled geometry in a browser bundle, for nothing. Welding recovers
    // exactly the 410 the exporter itself declared.
    let pack = import_manifest(
        &fixtures(),
        &manifest_for(
            "craft_racer.fbx",
            SourceFormat::Auto,
            base_import("a", "src"),
        ),
    )
    .unwrap();
    let mesh = &pack.find("a").unwrap().mesh;
    assert_eq!(mesh.indices.len(), 840, "280 triangles, unshared indices");
    assert_eq!(mesh.vertex_count(), 410, "welded down from 840");
    assert_eq!(
        mesh.triangle_count(),
        280,
        "welding must not change what is drawn"
    );
    assert_eq!(pack.validate(), vec![]);
}

#[test]
fn welding_keeps_hard_edges_hard() {
    // Exact-bit welding, not tolerance welding: two vertices at the same
    // position with DIFFERENT normals are a crease and must stay separate.
    // A tolerance would silently smooth every hard edge in a library.
    let mut scene = wedge();
    {
        let mesh = &mut scene.meshes[0];
        mesh.normals = vec![[0.0, 1.0, 0.0]; 4];
        mesh.uvs = vec![[0.0, 0.0]; 4];
        // Same position as vertex 0, different normal, referenced by a face.
        mesh.positions.push(mesh.positions[0]);
        mesh.normals.push([1.0, 0.0, 0.0]);
        mesh.uvs.push([0.0, 0.0]);
        mesh.parts[0].indices.extend_from_slice(&[4, 1, 2]);
    }

    let mut import = base_import("crease", "src");
    import.budget = PerfBudget {
        max_triangles: 100,
        max_vertices: 100,
    };
    let mut pack = AssetPack::new();
    let asset = convert::convert(&scene, &import, &mut pack).unwrap();
    assert_eq!(
        asset.mesh.vertex_count(),
        5,
        "a shared position carrying two normals is two vertices"
    );
}

#[test]
fn an_imported_pack_passes_the_asset_contract() {
    let pack = import_manifest(
        &fixtures(),
        &manifest_for(
            "craft_racer.fbx",
            SourceFormat::Auto,
            base_import("a", "src"),
        ),
    )
    .unwrap();
    assert_eq!(pack.validate(), vec![]);
    // And it is shippable: the bake encoder validates before writing.
    assert!(pack.to_postcard_current().is_ok());
}

#[test]
fn importing_twice_produces_identical_bytes() {
    // The determinism bar the whole pack format exists for, end to end.
    let build = || {
        import_manifest(
            &fixtures(),
            &manifest_for(
                "craft_racer.fbx",
                SourceFormat::Auto,
                base_import("a", "src"),
            ),
        )
        .unwrap()
        .to_postcard_current()
        .unwrap()
    };
    assert_eq!(build(), build());
}

#[test]
fn several_assets_share_one_read_of_each_source() {
    // A modular kit is dozens of assets out of a handful of files; re-parsing
    // per asset turns a fast bake slow.
    let mut manifest = manifest_for(
        "craft_racer.fbx",
        SourceFormat::Auto,
        base_import("a", "src"),
    );
    manifest.meshes.push(base_import("b", "src"));
    manifest.meshes.push(base_import("c", "src"));
    let pack = import_manifest(&fixtures(), &manifest).unwrap();
    assert_eq!(pack.len(), 3);
}

// ----- selection and diagnostics -----

#[test]
fn a_missing_mesh_name_says_what_the_file_actually_contains() {
    // The usual cause is a name that differs from the one in the file, so the
    // error has to carry the real names or it costs a manual inspection.
    let mut import = base_import("a", "src");
    import.select = MeshSelect::Named(vec!["not_in_the_file".into()]);
    let err = import_manifest(
        &fixtures(),
        &manifest_for("craft_racer.fbx", SourceFormat::Auto, import),
    )
    .unwrap_err();
    match &err {
        ImportError::NoSuchMesh { available, .. } => {
            assert!(!available.is_empty(), "should list what the file has");
        }
        other => panic!("expected NoSuchMesh, got {other}"),
    }
    assert!(err.to_string().contains("The file contains"));
}

#[test]
fn a_manifest_typo_fails_before_any_file_is_opened() {
    let mut manifest = manifest_for(
        "does_not_exist.fbx",
        SourceFormat::Auto,
        base_import("a", "src"),
    );
    manifest.meshes[0].source = "wrong-source-id".into();
    // The file is missing too, but the manifest error must win — proving
    // nothing was opened.
    assert!(matches!(
        import_manifest(&fixtures(), &manifest),
        Err(ImportError::Manifest(_))
    ));
}

#[test]
fn over_budget_says_by_how_much() {
    let mut import = base_import("a", "src");
    import.budget = PerfBudget {
        max_triangles: 10,
        max_vertices: 10,
    };
    let err = import_manifest(
        &fixtures(),
        &manifest_for("craft_racer.fbx", SourceFormat::Auto, import),
    )
    .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("280 triangles"), "got: {text}");
    assert!(text.contains("max 10"), "got: {text}");
}

#[test]
fn an_unknown_extension_asks_for_an_explicit_format() {
    let err = read_source(
        &fixtures().join("KENNEY-LICENSE.txt"),
        SourceFormat::Auto,
        AxisConvention::FromSource,
    )
    .unwrap_err();
    assert!(err.to_string().contains("declare `format`"), "got: {err}");
}

// ----- corrections, against synthetic geometry -----

/// A deliberately ASYMMETRIC wedge, so every correction is observable: it
/// occupies +X only, spans 0..2 in Z, and sits at y=0..1. A symmetric box
/// would hide a mirror.
fn wedge() -> SourceScene {
    let positions = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 2.0],
    ];
    // Outward-ish winding for a tetrahedron.
    let indices = vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3];
    SourceScene {
        meshes: vec![SourceMesh {
            name: "wedge".into(),
            positions,
            normals: vec![],
            uvs: vec![],
            parts: vec![SourcePart {
                material: Some("Hull".into()),
                material_index: 0,
                indices,
            }],
        }],
        declared_units_per_metre: Some(1.0),
        declared_frame: None,
    }
}

/// Signed volume of a closed mesh: positive for outward winding, negative
/// when the winding has been reversed. This is what makes "mirroring flips
/// winding, and the importer flips it back" a measurable claim instead of an
/// assertion.
fn signed_volume(mesh: &artificer_scene::MeshData) -> f32 {
    let mut total = 0.0;
    for tri in mesh.indices.chunks_exact(3) {
        let a = glam::Vec3::from_array(mesh.positions[tri[0] as usize]);
        let b = glam::Vec3::from_array(mesh.positions[tri[1] as usize]);
        let c = glam::Vec3::from_array(mesh.positions[tri[2] as usize]);
        total += a.dot(b.cross(c)) / 6.0;
    }
    total
}

fn convert_wedge(mutate: impl FnOnce(&mut MeshImport)) -> artificer_assets::PackedAsset {
    let mut import = base_import("wedge", "src");
    mutate(&mut import);
    let mut pack = AssetPack::new();
    convert::convert(&wedge(), &import, &mut pack).expect("wedge should convert")
}

#[test]
fn mirroring_flips_geometry_and_keeps_faces_facing_out() {
    let plain = convert_wedge(|_| {});
    let mirrored = convert_wedge(|i| i.mirror_x = true);

    let plain_bounds = plain.mesh.bounds().unwrap();
    let mirrored_bounds = mirrored.mesh.bounds().unwrap();
    assert!(plain_bounds.max.x > 0.9 && plain_bounds.min.x.abs() < 1e-6);
    assert!(
        mirrored_bounds.min.x < -0.9 && mirrored_bounds.max.x.abs() < 1e-6,
        "X should be mirrored, got {mirrored_bounds:?}"
    );

    // The point of the winding rule: a mirror reverses face orientation, and
    // the importer undoes it, so the solid stays solid rather than inside-out.
    let before = signed_volume(&plain.mesh);
    let after = signed_volume(&mirrored.mesh);
    assert!(before.abs() > 1e-4, "degenerate test mesh");
    assert!(
        before.signum() == after.signum(),
        "mirroring inverted the winding: {before} -> {after}"
    );
}

#[test]
fn an_explicit_flip_on_top_of_a_mirror_cancels_it() {
    // The XOR rule, stated in the MeshImport doc: two flips are no flip.
    let mirrored = convert_wedge(|i| i.mirror_x = true);
    let both = convert_wedge(|i| {
        i.mirror_x = true;
        i.flip_winding = true;
    });
    assert!(
        signed_volume(&mirrored.mesh).signum() != signed_volume(&both.mesh).signum(),
        "flip_winding should XOR with the mirror-induced flip"
    );
}

#[test]
fn a_left_handed_source_comes_out_solid_not_inside_out() {
    // Unity-handed input: converting mirrors an axis, so winding must be
    // flipped back. Getting this wrong is the silent failure AxisConvention
    // exists to prevent.
    let plain = convert_wedge(|_| {});
    let unity = convert_wedge(|i| i.axis = AxisConvention::unity());
    assert!(
        signed_volume(&plain.mesh).signum() == signed_volume(&unity.mesh).signum(),
        "a left-handed source should not come out inside-out"
    );
}

#[test]
fn a_z_up_source_becomes_y_up() {
    // Blender: Z up, -Y forward. The wedge's 2-metre run along source Z must
    // end up along engine Y.
    let converted = convert_wedge(|i| i.axis = AxisConvention::blender());
    let e = converted.mesh.bounds().unwrap().extents();
    assert!(
        (e.y - 2.0).abs() < 1e-4,
        "the long axis should be vertical after Z-up conversion, got {e:?}"
    );
}

#[test]
fn rotation_is_applied_about_the_origin_after_the_axis_frame() {
    // 180 degrees about Y takes +X to -X and +Z to -Z.
    let rotated = convert_wedge(|i| i.rotation_deg = [0.0, 180.0, 0.0]);
    let b = rotated.mesh.bounds().unwrap();
    assert!(
        b.min.x < -0.9 && b.max.x.abs() < 1e-5,
        "x not rotated: {b:?}"
    );
    assert!(
        b.min.z < -1.9 && b.max.z.abs() < 1e-5,
        "z not rotated: {b:?}"
    );
}

#[test]
fn unit_scale_multiplies_after_the_frame() {
    let scaled = convert_wedge(|i| i.unit_scale = 10.0);
    let e = scaled.mesh.bounds().unwrap().extents();
    assert!((e.z - 20.0).abs() < 1e-4, "expected 20 m, got {e:?}");
}

#[test]
fn pivot_policies_move_the_origin_where_they_say() {
    let centred = convert_wedge(|i| i.pivot = PivotPolicy::BoundsCenter);
    let c = centred.mesh.bounds().unwrap().center();
    assert!(c.length() < 1e-5, "BoundsCenter should centre, got {c:?}");

    let based = convert_wedge(|i| i.pivot = PivotPolicy::BaseY);
    let b = based.mesh.bounds().unwrap();
    assert!(b.min.y.abs() < 1e-5, "BaseY should sit on y=0, got {b:?}");
    assert!(b.center().x.abs() < 1e-5 && b.center().z.abs() < 1e-5);
}

#[test]
fn the_pivot_is_measured_after_rotation_not_before() {
    // Order matters: centring before rotating would leave the mesh off-centre
    // afterwards.
    let both = convert_wedge(|i| {
        i.rotation_deg = [0.0, 90.0, 0.0];
        i.pivot = PivotPolicy::BoundsCenter;
    });
    assert!(both.mesh.bounds().unwrap().center().length() < 1e-5);
}

#[test]
fn a_source_without_normals_gets_generated_ones() {
    let converted = convert_wedge(|_| {});
    assert_eq!(converted.mesh.normals.len(), converted.mesh.positions.len());
    assert!(
        converted
            .mesh
            .normals
            .iter()
            .any(|n| glam::Vec3::from_array(*n).length() > 0.5),
        "normals should be real, not zeroes"
    );
    // And UVs exist, because MeshData requires one per vertex.
    assert_eq!(converted.mesh.uvs.len(), converted.mesh.positions.len());
}

#[test]
fn a_degenerate_axis_frame_is_refused() {
    let mut import = base_import("wedge", "src");
    import.axis = AxisConvention::Explicit {
        up: Axis::PosY,
        forward: Axis::NegY,
        handedness: Handedness::Right,
    };
    let mut pack = AssetPack::new();
    assert!(matches!(
        convert::convert(&wedge(), &import, &mut pack),
        Err(ImportError::BadCorrection(_, _))
    ));
}

// ----- materials and submeshes -----

/// Two materials on one mesh — the hull-plus-glass-canopy case. Synthetic
/// because the CC0 fixtures are all single-material.
fn two_material_scene() -> SourceScene {
    let mut scene = wedge();
    let mesh = &mut scene.meshes[0];
    let all = mesh.parts[0].indices.clone();
    let (first, second) = all.split_at(6);
    mesh.parts = vec![
        SourcePart {
            material: Some("Hull".into()),
            material_index: 0,
            indices: first.to_vec(),
        },
        SourcePart {
            material: Some("Glass".into()),
            material_index: 1,
            indices: second.to_vec(),
        },
    ];
    scene
}

#[test]
fn a_multi_material_mesh_keeps_its_split_and_tiles_the_buffer() {
    let mut import = base_import("ship", "src");
    import.material_bindings = vec![
        MaterialBinding {
            select: MaterialSelector::Name("Hull".into()),
            mapping: MaterialMapping::Override(Box::new(artificer_scene::MaterialDesc::color(
                0.5, 0.5, 0.5,
            ))),
        },
        MaterialBinding {
            select: MaterialSelector::Name("Glass".into()),
            mapping: MaterialMapping::Override(Box::new(artificer_scene::MaterialDesc::color(
                0.1, 0.3, 0.4,
            ))),
        },
    ];
    let mut pack = AssetPack::new();
    let asset = convert::convert(&two_material_scene(), &import, &mut pack).unwrap();
    assert_eq!(asset.submeshes.len(), 2);
    assert_eq!(asset.submeshes[0].index_start, 0);
    assert_eq!(
        asset.submeshes[0].index_count + asset.submeshes[1].index_count,
        asset.mesh.indices.len() as u32,
        "submeshes must tile the whole buffer"
    );
    assert_eq!(
        asset.submeshes[1].index_start,
        asset.submeshes[0].index_count
    );

    pack.assets.push(asset);
    assert_eq!(pack.validate(), vec![], "the split must satisfy the pack");
}

#[test]
fn an_atlas_binding_collapses_every_asset_onto_one_material() {
    // The draw-call property that makes a Synty-style atlas pack worth using.
    let mut pack = AssetPack::new();
    for id in ["one", "two", "three"] {
        let mut import = base_import(id, "src");
        import.material = MaterialMapping::Atlas {
            texture: "page_a".into(),
        };
        let asset = convert::convert(&wedge(), &import, &mut pack).unwrap();
        pack.assets.push(asset);
    }
    assert_eq!(pack.materials.len(), 1, "one atlas page, one material");
    assert_eq!(pack.materials[0].id, "atlas.page_a");
}

#[test]
fn a_name_binding_beats_an_index_binding_for_the_same_slot() {
    // Names are written deliberately; indices shift when a file is
    // re-exported, so the deliberate one wins.
    let mut import = base_import("ship", "src");
    import.material_bindings = vec![
        MaterialBinding {
            select: MaterialSelector::Index(0),
            mapping: MaterialMapping::Atlas {
                texture: "by_index".into(),
            },
        },
        MaterialBinding {
            select: MaterialSelector::Name("Hull".into()),
            mapping: MaterialMapping::Atlas {
                texture: "by_name".into(),
            },
        },
    ];
    let mut pack = AssetPack::new();
    let asset = convert::convert(&wedge(), &import, &mut pack).unwrap();
    assert_eq!(
        asset.submeshes[0].material.as_deref(),
        Some("atlas.by_name")
    );
}

#[test]
fn an_index_binding_addresses_a_slot_a_name_cannot() {
    // Unnamed materials are real; only an index can reach them.
    let mut scene = wedge();
    scene.meshes[0].parts[0].material = None;
    let mut import = base_import("ship", "src");
    import.material_bindings = vec![MaterialBinding {
        select: MaterialSelector::Index(0),
        mapping: MaterialMapping::Atlas {
            texture: "page_a".into(),
        },
    }];
    let mut pack = AssetPack::new();
    let asset = convert::convert(&scene, &import, &mut pack).unwrap();
    assert_eq!(asset.submeshes[0].material.as_deref(), Some("atlas.page_a"));
}

#[test]
fn declared_bounds_stay_declared_so_the_cross_check_means_something() {
    // Recording the measured bounds in both places would make validate_asset
    // compare a number with itself.
    let mut import = base_import("wedge", "src");
    import.declared_bounds = Some(artificer_assets::DeclaredBounds {
        min: [0.0, 0.0, 0.0],
        max: [9.0, 9.0, 9.0],
    });
    let mut pack = AssetPack::new();
    let asset = convert::convert(&wedge(), &import, &mut pack).unwrap();
    assert_eq!(asset.record.bounds_max, [9.0, 9.0, 9.0]);
    pack.assets.push(asset);
    assert!(
        pack.validate()
            .iter()
            .any(|i| i.message.contains("disagree with actual")),
        "a wrong declaration must be caught"
    );
}

#[test]
fn an_imported_record_is_metric_and_engine_oriented() {
    let asset = convert_wedge(|i| i.unit_scale = 100.0);
    assert_eq!(
        asset.record.unit_scale, 1.0,
        "scale is baked into the vertices, so the record is metric"
    );
    assert_eq!(asset.record.orientation, "y-up,-z-forward");
}

// ----- real production art, on demand -----

/// Import the project's actual source art when a path is provided.
///
/// Ignored by default and driven by an env var, because that art is licensed
/// and cannot live in this public repo — but the pipeline's whole claim is
/// that it reads real DCC output, so there has to be a repeatable way to
/// check it rather than a one-off done by hand and forgotten.
///
/// ```text
/// ARTIFICER_TEST_FBX_DIR=".../POLYGON_Scifi_Space_SourceFiles_v2/SourceFiles/FBX" \
///   cargo test -p artificer_assets_import -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs licensed art; set ARTIFICER_TEST_FBX_DIR"]
fn imports_real_production_art() {
    let Ok(dir) = std::env::var("ARTIFICER_TEST_FBX_DIR") else {
        panic!("set ARTIFICER_TEST_FBX_DIR to a directory of source FBX");
    };
    let root = PathBuf::from(&dir);

    // A whole hull and three modular kit parts: the two shapes of content
    // this pipeline exists to import.
    let expected: [(&str, [f32; 3]); 4] = [
        ("SM_Ship_Fighter_01.fbx", [13.33, 3.70, 12.91]),
        ("SM_Veh_Part_Body_01.fbx", [7.96, 2.72, 8.76]),
        ("SM_Veh_Part_Engine_01.fbx", [1.37, 1.50, 4.73]),
        ("SM_Veh_Part_Wing_01.fbx", [11.79, 1.12, 6.35]),
    ];

    for (file, want) in expected {
        let mut import = base_import("asset", "src");
        import.budget = PerfBudget {
            max_triangles: 100_000,
            max_vertices: 200_000,
        };
        import.provenance = format!("synty:scifi-space:{file}");
        import.license = "Synty licence (not redistributable)".into();
        let pack = import_manifest(&root, &manifest_for(file, SourceFormat::Auto, import))
            .unwrap_or_else(|e| panic!("{file}: {e}"));

        let mesh = &pack.find("asset").unwrap().mesh;
        let got = mesh.bounds().unwrap().extents().to_array();
        println!(
            "{file}: {:.2} x {:.2} x {:.2} m, {} tris, {} verts",
            got[0],
            got[1],
            got[2],
            mesh.triangle_count(),
            mesh.vertex_count()
        );
        for i in 0..3 {
            assert!(
                (got[i] - want[i]).abs() < 0.05,
                "{file}: axis {i} was {}, expected about {}",
                got[i],
                want[i]
            );
        }
        assert_eq!(pack.validate(), vec![], "{file} must satisfy the contract");
    }
}

/// Fraction of triangles whose winding agrees with the authored shading
/// normals.
///
/// This is the invariant that catches inside-out geometry on OPEN meshes,
/// where signed volume says nothing: the geometric normal implied by vertex
/// order should point the same way as the normals the artist exported. If a
/// conversion mirrored an axis without flipping winding, this collapses.
fn winding_agreement(mesh: &artificer_scene::MeshData) -> f32 {
    let mut agree = 0usize;
    let mut total = 0usize;
    for tri in mesh.indices.chunks_exact(3) {
        let [i, j, k] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let a = glam::Vec3::from_array(mesh.positions[i]);
        let b = glam::Vec3::from_array(mesh.positions[j]);
        let c = glam::Vec3::from_array(mesh.positions[k]);
        let geometric = (b - a).cross(c - a);
        if geometric.length_squared() < 1e-12 {
            continue; // degenerate sliver, carries no orientation
        }
        let shading = glam::Vec3::from_array(mesh.normals[i])
            + glam::Vec3::from_array(mesh.normals[j])
            + glam::Vec3::from_array(mesh.normals[k]);
        if shading.length_squared() < 1e-12 {
            continue;
        }
        total += 1;
        if geometric.dot(shading) > 0.0 {
            agree += 1;
        }
    }
    assert!(total > 0, "no orientable triangles to measure");
    agree as f32 / total as f32
}

#[test]
fn real_files_come_out_right_side_out() {
    // The phase's most dangerous silent failure: under FromSource the READER
    // performs the axis conversion, and if that conversion mirrors an axis
    // without reversing winding, every face is backwards. Nothing else in the
    // pipeline would notice -- the bounds are right, the triangle count is
    // right, and it only shows up as a ship rendered inside-out.
    //
    // Measured against the authored normals rather than assumed.
    for file in ["craft_racer.fbx", "corridor.fbx", "craft_racer.obj"] {
        let pack = import_manifest(
            &fixtures(),
            &manifest_for(file, SourceFormat::Auto, base_import("a", "src")),
        )
        .unwrap_or_else(|e| panic!("{file}: {e}"));
        let agreement = winding_agreement(&pack.find("a").unwrap().mesh);
        assert!(
            agreement > 0.95,
            "{file}: only {:.0}% of faces wind the way their normals point — \
             the axis conversion mirrored without flipping winding",
            agreement * 100.0
        );
    }
}

#[test]
fn an_explicit_left_handed_frame_also_comes_out_right_side_out() {
    // The other half: when the manifest overrides the frame, the winding flip
    // is OURS to apply, driven by changes_handedness(). Import the same file
    // twice -- once trusting it, once declaring a left-handed frame -- and
    // both must be right-side-out.
    let mut import = base_import("a", "src");
    import.axis = AxisConvention::unity();
    let pack = import_manifest(
        &fixtures(),
        &manifest_for("craft_racer.fbx", SourceFormat::Auto, import),
    )
    .unwrap();
    let agreement = winding_agreement(&pack.find("a").unwrap().mesh);
    assert!(
        agreement > 0.95,
        "a declared left-handed source came out {:.0}% consistent — the \
         mirror-induced winding flip was not applied",
        agreement * 100.0
    );
}

// ----- glTF front-end (A3) -----

#[test]
fn reads_glb_at_metre_scale() {
    // glTF fixes right-handed Y-up metres by specification, so this path has
    // no unit or axis normalisation to do — which is exactly why it must land
    // on the same numbers as the FBX of the same model.
    let pack = import_manifest(
        &fixtures(),
        &manifest_for(
            "craft_racer.glb",
            SourceFormat::Auto,
            base_import("a", "src"),
        ),
    )
    .expect("glb should import");
    assert_close(
        extents(&pack, "a"),
        [1.200, 0.750, 2.026],
        "craft_racer.glb",
    );
    assert_eq!(pack.find("a").unwrap().mesh.triangle_count(), 280);
}

#[test]
fn reads_binary_glb_from_the_other_kit() {
    let pack = import_manifest(
        &fixtures(),
        &manifest_for("corridor.glb", SourceFormat::Auto, base_import("a", "src")),
    )
    .expect("glb should import");
    assert_close(extents(&pack, "a"), [4.000, 4.250, 4.000], "corridor.glb");
    assert_eq!(pack.find("a").unwrap().mesh.triangle_count(), 860);
}

#[test]
fn the_two_front_ends_agree_on_the_same_model() {
    // The cross-check the multi-format fixtures exist for: two completely
    // separate readers, one shared post-processor, same geometry out. If a
    // reader mangles winding, scale or node transforms, this catches it.
    for (fbx_file, glb_file) in [
        ("craft_racer.fbx", "craft_racer.glb"),
        ("corridor.fbx", "corridor.glb"),
    ] {
        let via_fbx = import_manifest(
            &fixtures(),
            &manifest_for(fbx_file, SourceFormat::Auto, base_import("a", "src")),
        )
        .unwrap();
        let via_gltf = import_manifest(
            &fixtures(),
            &manifest_for(glb_file, SourceFormat::Auto, base_import("a", "src")),
        )
        .unwrap();

        let a = &via_fbx.find("a").unwrap().mesh;
        let b = &via_gltf.find("a").unwrap().mesh;
        assert_eq!(
            a.triangle_count(),
            b.triangle_count(),
            "{fbx_file} vs {glb_file}"
        );
        assert_close(
            a.bounds().unwrap().extents().to_array(),
            b.bounds().unwrap().extents().to_array(),
            &format!("{fbx_file} vs {glb_file}"),
        );
        // And both right-side-out, so neither reader is silently mirroring.
        assert!(winding_agreement(a) > 0.95, "{fbx_file}");
        assert!(winding_agreement(b) > 0.95, "{glb_file}");
    }
}

#[test]
fn glb_node_transforms_are_applied() {
    // Reading doc.meshes() directly would drop node placement and pile every
    // part at the origin. A model whose parts sit away from the origin proves
    // the hierarchy walk works: corridor is 4 m tall and does NOT straddle
    // y=0 the way an untransformed mesh would.
    let pack = import_manifest(
        &fixtures(),
        &manifest_for("corridor.glb", SourceFormat::Auto, base_import("a", "src")),
    )
    .unwrap();
    let fbx = import_manifest(
        &fixtures(),
        &manifest_for("corridor.fbx", SourceFormat::Auto, base_import("a", "src")),
    )
    .unwrap();
    let glb_bounds = pack.find("a").unwrap().mesh.bounds().unwrap();
    let fbx_bounds = fbx.find("a").unwrap().mesh.bounds().unwrap();
    // Same placement, not just the same size.
    assert!(
        (glb_bounds.min - fbx_bounds.min).length() < 0.01
            && (glb_bounds.max - fbx_bounds.max).length() < 0.01,
        "glb {glb_bounds:?} vs fbx {fbx_bounds:?} — node transforms lost?"
    );
}

#[test]
fn an_imported_glb_pack_passes_the_contract_and_bakes() {
    let pack = import_manifest(
        &fixtures(),
        &manifest_for(
            "craft_racer.glb",
            SourceFormat::Auto,
            base_import("a", "src"),
        ),
    )
    .unwrap();
    assert_eq!(pack.validate(), vec![]);
    assert!(pack.to_postcard_current().is_ok());
}

#[test]
fn a_corrupt_model_file_is_an_error_not_a_panic() {
    // A bake runs unattended over a library; a malformed file must report,
    // not abort the process.
    let dir = std::env::temp_dir().join("artificer_import_corrupt");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in [
        ("broken.glb", &b"glTF not really"[..]),
        ("broken.fbx", &b"Kaydara FBX Binary  \x00 truncated"[..]),
        ("broken.obj", &b"v not-a-number\nf 1 2 3\n"[..]),
    ] {
        std::fs::write(dir.join(name), bytes).unwrap();
        let result = import_manifest(
            &dir,
            &manifest_for(name, SourceFormat::Auto, base_import("a", "src")),
        );
        assert!(result.is_err(), "{name} should fail cleanly");
        assert!(
            !result.unwrap_err().to_string().is_empty(),
            "{name} error should say something"
        );
    }
}

// ----- textures and atlas materials (A4) -----

fn atlas_manifest(max_size: Option<u32>) -> ImportManifest {
    let mut import = base_import("ship", "src");
    import.material = MaterialMapping::Atlas {
        texture: "page".into(),
    };
    ImportManifest {
        sources: vec![ImportSource {
            id: "src".into(),
            path: "craft_racer.fbx".into(),
            format: SourceFormat::Auto,
        }],
        textures: vec![artificer_assets::TextureImport {
            id: "page".into(),
            path: "colormap.png".into(),
            max_size,
            sampler: artificer_assets::SamplerMode::Nearest,
        }],
        meshes: vec![import],
    }
}

#[test]
fn a_texture_is_baked_into_the_pack_and_bound_to_its_material() {
    let pack = import_manifest(&fixtures(), &atlas_manifest(None)).expect("should import");
    let texture = pack.texture("page").expect("texture in pack");
    assert_eq!((texture.width, texture.height), (512, 512));
    // Encoded PNG, not raw pixels: 512x512 RGBA raw would be a megabyte.
    assert!(texture.png.len() < 200_000, "{} bytes", texture.png.len());

    let material = pack.material("atlas.page").expect("atlas material");
    assert_eq!(material.base_color_texture.as_deref(), Some("page"));
    assert_eq!(pack.validate(), vec![]);
}

#[test]
fn max_size_downscales_at_bake_time() {
    // Source packs ship 4096-square pages at ~3.3 MB each, which would
    // dominate a browser bundle on their own. Shrinking happens once, here --
    // paying for it on every cold start would be worse.
    let pack = import_manifest(&fixtures(), &atlas_manifest(Some(128))).expect("should import");
    let texture = pack.texture("page").expect("texture in pack");
    assert_eq!((texture.width, texture.height), (128, 128));
    assert_eq!(pack.validate(), vec![], "declared size must match the PNG");

    let full = import_manifest(&fixtures(), &atlas_manifest(None)).unwrap();
    assert!(
        texture.png.len() < full.texture("page").unwrap().png.len(),
        "downscaling should shrink the bytes"
    );
}

#[test]
fn a_texture_already_within_max_size_keeps_its_original_bytes() {
    // Re-encoding is lossless but not byte-identical across image-crate
    // versions, which would make bakes differ for no gain.
    let untouched = import_manifest(&fixtures(), &atlas_manifest(None)).unwrap();
    let generous = import_manifest(&fixtures(), &atlas_manifest(Some(4096))).unwrap();
    assert_eq!(
        untouched.texture("page").unwrap().png,
        generous.texture("page").unwrap().png
    );
    let on_disk = std::fs::read(fixtures().join("colormap.png")).unwrap();
    assert_eq!(untouched.texture("page").unwrap().png, on_disk);
}

#[test]
fn baking_a_textured_pack_twice_is_byte_identical() {
    let build = || {
        import_manifest(&fixtures(), &atlas_manifest(Some(256)))
            .unwrap()
            .to_postcard_current()
            .unwrap()
    };
    assert_eq!(build(), build());
}

#[test]
fn a_missing_texture_file_names_the_path() {
    let mut manifest = atlas_manifest(None);
    manifest.textures[0].path = "no_such_atlas.png".into();
    let err = import_manifest(&fixtures(), &manifest)
        .unwrap_err()
        .to_string();
    assert!(err.contains("no_such_atlas.png"), "got: {err}");
}

#[test]
fn a_whole_kit_collapses_onto_one_atlas_material() {
    // The draw-call property this pipeline exists to deliver: many assets,
    // one page, one material.
    let mut manifest = atlas_manifest(None);
    manifest.sources.push(ImportSource {
        id: "rocket".into(),
        path: "rocket_baseA.fbx".into(),
        format: SourceFormat::Auto,
    });
    for (id, source) in [("fins", "rocket"), ("base", "rocket")] {
        let mut import = base_import(id, source);
        import.material = MaterialMapping::Atlas {
            texture: "page".into(),
        };
        manifest.meshes.push(import);
    }
    let pack = import_manifest(&fixtures(), &manifest).expect("should import");
    assert_eq!(pack.len(), 3);
    assert_eq!(pack.materials.len(), 1, "one atlas page, one material");
    assert_eq!(pack.textures.len(), 1);
    assert_eq!(pack.validate(), vec![]);
    println!("{}", pack.size_report().unwrap());
}

// ----- corrections THROUGH A REAL FILE -----
//
// Every other correction test builds a synthetic SourceScene and calls
// convert() directly, which is precise but bypasses the reader entirely.
// That is the exact shape of hole an earlier bug fell through: declaring an
// explicit frame silently multiplied every coordinate by the file's unit
// factor, because the reader stopped normalising units and nothing measured
// the result end to end.

#[test]
fn a_declared_no_op_frame_does_not_change_the_geometry() {
    // engine_native() says "this file is already in the engine's frame".
    // Declaring it must be a NO-OP. It previously turned a 1.2 m model into a
    // 12 m one, because the reader left the file's centimetres alone.
    let from_source = import_manifest(
        &fixtures(),
        &manifest_for(
            "craft_racer.fbx",
            SourceFormat::Auto,
            base_import("a", "src"),
        ),
    )
    .unwrap();

    let mut import = base_import("a", "src");
    import.axis = AxisConvention::engine_native();
    let explicit = import_manifest(
        &fixtures(),
        &manifest_for("craft_racer.fbx", SourceFormat::Auto, import),
    )
    .unwrap();

    let a = from_source.find("a").unwrap().mesh.bounds().unwrap();
    let b = explicit.find("a").unwrap().mesh.bounds().unwrap();
    assert_close(
        b.extents().to_array(),
        a.extents().to_array(),
        "a no-op frame must not resize anything",
    );
    assert_close(
        b.extents().to_array(),
        [1.200, 0.750, 2.026],
        "still metric",
    );
}

#[test]
fn an_explicit_frame_still_normalises_units() {
    // MeshImport documents that an explicit frame "overrides axes and
    // handedness ONLY; unit normalization to metres always follows the file".
    // Both fixtures declare non-metre units (0.1 and 0.01 metres per unit), so
    // a failure here shows up as a 10x or 100x model.
    for (file, expected) in [
        ("craft_racer.fbx", [1.200, 0.750, 2.026]),
        ("corridor.fbx", [4.000, 4.250, 4.000]),
    ] {
        let mut import = base_import("a", "src");
        import.axis = AxisConvention::engine_native();
        let pack =
            import_manifest(&fixtures(), &manifest_for(file, SourceFormat::Auto, import)).unwrap();
        assert_close(extents(&pack, "a"), expected, file);
    }
}

#[test]
fn node_transforms_reach_the_imported_geometry() {
    // craft_racer's mesh node carries a translation of (-2, 0, -1.5). Reading
    // mesh-local vertices drops it, which a bounding-box SIZE check cannot
    // see -- only the position can.
    let pack = import_manifest(
        &fixtures(),
        &manifest_for(
            "craft_racer.fbx",
            SourceFormat::Auto,
            base_import("a", "src"),
        ),
    )
    .unwrap();
    let bounds = pack.find("a").unwrap().mesh.bounds().unwrap();
    assert!(
        bounds.min.x > 0.5,
        "the node translation was dropped: min.x = {} (mesh-local would be \
         about -0.6, placed should be about +1.4)",
        bounds.min.x
    );
}

#[test]
fn a_z_up_file_is_corrected_by_a_declared_frame() {
    // The end-to-end version of the axis test: a real file on disk, read by a
    // real front-end, corrected by a frame declared in the manifest. Every
    // downloaded fixture is Y-up metric and cannot exercise this at all.
    //
    // The wedge is 1 x 2 x 4 m in SOURCE axes (X, Y, Z). Blender is Z-up with
    // -Y forward, so after correction the 4 m run must be vertical and the
    // 2 m run must lie along Z.
    let mut import = base_import("wedge", "src");
    import.axis = AxisConvention::blender();
    let pack = import_manifest(
        &fixtures(),
        &manifest_for("z_up_wedge.obj", SourceFormat::Auto, import),
    )
    .expect("z-up obj should import");
    let e = extents(&pack, "wedge");
    assert!(
        (e[0] - 1.0).abs() < 1e-4 && (e[1] - 4.0).abs() < 1e-4 && (e[2] - 2.0).abs() < 1e-4,
        "expected 1 x 4 x 2 m after Z-up correction, got {e:?}"
    );
}

#[test]
fn the_same_file_read_without_a_declared_frame_is_left_alone() {
    // OBJ carries no axis metadata, so FromSource means "already engine
    // native". The contrast with the test above is what proves the correction
    // came from the DECLARATION rather than from something the reader does
    // unconditionally.
    let pack = import_manifest(
        &fixtures(),
        &manifest_for(
            "z_up_wedge.obj",
            SourceFormat::Auto,
            base_import("wedge", "src"),
        ),
    )
    .expect("obj should import");
    let e = extents(&pack, "wedge");
    assert!(
        (e[0] - 1.0).abs() < 1e-4 && (e[1] - 2.0).abs() < 1e-4 && (e[2] - 4.0).abs() < 1e-4,
        "expected the authored 1 x 2 x 4 m, got {e:?}"
    );
}
