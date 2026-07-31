//! Assembling a modular vehicle from an import manifest.
//!
//! Runs the whole pipeline — describe an import in data, read real model
//! files, bake a pack, load it into a scene — using only the engine's public
//! API and no game code. That is the point: it is the generality test for the
//! asset pipeline (plan §9.4), and it doubles as the worked example.
//!
//! The content is Kenney's CC0 rocket kit: a base, fins, a fuel section and a
//! nose, authored as separate models and assembled by placing each one. This
//! is the same shape as a ship built from hull plus fitted parts, which is
//! what the pipeline exists to serve.
//!
//! Headless on purpose. Importing and baking need no GPU, so this runs in CI
//! and actually exercises the code rather than merely compiling against it.

use artificer_assets::{
    load_pack, AssetCategory, CollisionProxy, ImportManifest, ImportSource, LodPolicy,
    MaterialMapping, MeshImport, MeshSelect, PerfBudget, PivotPolicy, SamplerMode, TextureImport,
};
use artificer_assets_import::import_manifest;
use artificer_scene::SceneGraph;
use std::path::{Path, PathBuf};

/// One piece of the kit: which file, and where it sits on the assembled
/// vehicle. Placement is the GAME's business — the pipeline's job is to
/// deliver each part correctly, in metres and facing the right way.
struct Part {
    id: &'static str,
    file: &'static str,
    /// Metres along Y, stacking the rocket.
    height: f32,
}

const KIT: &[Part] = &[
    Part {
        id: "rocket.base",
        file: "rocket_baseA.fbx",
        height: 0.0,
    },
    Part {
        id: "rocket.fins",
        file: "rocket_finsA.fbx",
        height: 0.0,
    },
];

fn mesh_import(part: &Part, textured: bool) -> MeshImport {
    MeshImport {
        id: part.id.to_string(),
        source: part.id.to_string(),
        select: MeshSelect::All,
        unit_scale: 1.0,
        // The files declare their own axes and units, which is the normal
        // case for FBX and glTF; an explicit frame is for overriding a file
        // whose metadata is absent or wrong.
        axis: Default::default(),
        rotation_deg: [0.0; 3],
        mirror_x: false,
        flip_winding: false,
        // Kit parts are authored around a shared origin so they stack, so the
        // authored pivot is exactly what we want to keep.
        pivot: PivotPolicy::AsAuthored,
        material: if textured {
            MaterialMapping::Atlas {
                texture: "kit_atlas".to_string(),
            }
        } else {
            MaterialMapping::Default
        },
        material_bindings: vec![],
        category: AssetCategory::ShipModule,
        collision: CollisionProxy::None,
        sockets: vec![],
        lod: LodPolicy::Single,
        budget: PerfBudget {
            max_triangles: 5_000,
            max_vertices: 10_000,
        },
        declared_bounds: None,
        provenance: "kenney:space-kit".to_string(),
        license: "CC0".to_string(),
        gameplay_ref: part.id.to_string(),
    }
}

fn build_manifest(content: &Path, textured: bool) -> ImportManifest {
    ImportManifest {
        sources: KIT
            .iter()
            .map(|part| ImportSource {
                id: part.id.to_string(),
                path: content.join(part.file).display().to_string(),
                format: Default::default(),
            })
            .collect(),
        textures: if textured {
            vec![TextureImport {
                id: "kit_atlas".to_string(),
                path: content.join("colormap.png").display().to_string(),
                // One page for the whole kit, shrunk once at bake.
                max_size: Some(512),
                sampler: SamplerMode::Nearest,
            }]
        } else {
            vec![]
        },
        meshes: KIT.iter().map(|p| mesh_import(p, textured)).collect(),
    }
}

fn main() {
    // Default to the importer's CC0 fixtures so the sample runs with no
    // arguments; a real game points this at its own content directory.
    let content: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../crates/artificer_assets_import/tests/fixtures")
        });

    let textured = content.join("colormap.png").exists();
    let manifest = build_manifest(&content, textured);

    let pack = match import_manifest(&content, &manifest) {
        Ok(pack) => pack,
        Err(e) => {
            eprintln!("import failed: {e}");
            std::process::exit(1);
        }
    };

    let report = pack.size_report().expect("a valid pack reports its size");
    println!("imported {report}");
    for asset in pack.assets.iter() {
        let bounds = asset.mesh.bounds().expect("geometry");
        let size = bounds.extents();
        println!(
            "  {:<14} {:>5} tris  {:.2} x {:.2} x {:.2} m",
            asset.record.id,
            asset.mesh.triangle_count(),
            size.x,
            size.y,
            size.z
        );
    }

    // Round-trip through the on-disk format, because that is what a shipped
    // game loads — not the in-memory pack the importer just produced.
    let bytes = pack.to_postcard_current().expect("bake");
    let loaded_pack = artificer_assets::AssetPack::from_postcard(&bytes).expect("load");

    // And into a scene, which is the last step before something is drawn.
    let mut scene = SceneGraph::new();
    let loaded = load_pack(&mut scene, &loaded_pack);
    let mut y = 0.0;
    for part in KIT {
        let Some(mesh) = loaded.mesh(part.id) else {
            eprintln!("{} did not make it into the pack", part.id);
            std::process::exit(1);
        };
        let material = loaded
            .primary_material(&loaded_pack, part.id)
            .unwrap_or_default();
        scene.spawn_mesh(
            mesh,
            material,
            artificer_scene::TransformDesc::from_translation(artificer_scene::glam::Vec3::new(
                0.0,
                y + part.height,
                0.0,
            )),
        );
        y += 1.0;
    }

    let commands = scene.drain_commands().len();
    println!(
        "assembled {} parts into a scene ({commands} scene commands, {} bytes baked)",
        KIT.len(),
        bytes.len()
    );
}
