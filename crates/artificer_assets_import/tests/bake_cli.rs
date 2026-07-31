//! End-to-end test of the `artificer_bake` CLI.
//!
//! Runs the real binary against the real fixtures and reads back the real
//! file, because the thing a game actually depends on is the artifact on
//! disk, not the library call that produced it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn bake_bin() -> PathBuf {
    // The integration test binary sits next to the CLI in the same profile
    // directory (target/<profile>/deps/<test>), so walk up one level.
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join(if cfg!(windows) {
        "artificer_bake.exe"
    } else {
        "artificer_bake"
    })
}

fn write_manifest(dir: &Path, name: &str, json: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, json).expect("write manifest");
    path
}

/// A manifest that imports one CC0 model onto one atlas page.
fn manifest_json() -> String {
    // Paths are relative to the MANIFEST, which is the property being tested:
    // the manifest lives in a temp dir and points back at the fixtures.
    let fixtures = fixtures().display().to_string().replace('\\', "/");
    format!(
        r#"{{
  "sources": [
    {{ "id": "racer", "path": "{fixtures}/craft_racer.fbx" }}
  ],
  "textures": [
    {{ "id": "page", "path": "{fixtures}/colormap.png", "max_size": 256 }}
  ],
  "meshes": [
    {{
      "id": "ship.racer",
      "source": "racer",
      "material": {{ "Atlas": {{ "texture": "page" }} }},
      "category": "ShipHull",
      "budget": {{ "max_triangles": 5000, "max_vertices": 10000 }},
      "provenance": "kenney:space-kit",
      "license": "CC0",
      "gameplay_ref": "ship.racer"
    }}
  ]
}}"#
    )
}

fn run_bake(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(bake_bin())
        .args(args)
        .output()
        .expect("run artificer_bake");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn the_cli_bakes_a_loadable_pack() {
    let dir = std::env::temp_dir().join("artificer_bake_cli");
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = write_manifest(&dir, "content.json", &manifest_json());
    let out = dir.join("nested/dir/ships.apack");
    let _ = std::fs::remove_file(&out);

    let (ok, stdout, stderr) = run_bake(&[
        "--manifest",
        &manifest.display().to_string(),
        "--out",
        &out.display().to_string(),
    ]);
    assert!(ok, "bake failed:\n{stderr}");
    assert!(stdout.contains("wrote"), "stdout was: {stdout}");
    // Output directories are created rather than requiring a prior mkdir.
    assert!(out.exists(), "pack not written");

    // The artifact must be readable by the RUNTIME half, which is the only
    // thing a shipped game runs.
    let bytes = std::fs::read(&out).unwrap();
    let pack = artificer_assets::AssetPack::from_postcard(&bytes).expect("pack should load");
    assert_eq!(pack.len(), 1);
    let asset = pack.find("ship.racer").expect("asset by id");
    assert_eq!(asset.mesh.triangle_count(), 280);
    assert_eq!(pack.validate(), vec![]);

    // The texture was downscaled at bake, per the manifest.
    let texture = pack.texture("page").expect("atlas page");
    assert_eq!((texture.width, texture.height), (256, 256));
}

#[test]
fn baking_twice_writes_identical_files() {
    // The determinism guarantee, at the level that matters: the file on disk.
    let dir = std::env::temp_dir().join("artificer_bake_cli_determinism");
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = write_manifest(&dir, "content.json", &manifest_json());

    let mut written = Vec::new();
    for name in ["first.apack", "second.apack"] {
        let out = dir.join(name);
        let (ok, _, stderr) = run_bake(&[
            "--manifest",
            &manifest.display().to_string(),
            "--out",
            &out.display().to_string(),
        ]);
        assert!(ok, "bake failed:\n{stderr}");
        written.push(std::fs::read(&out).unwrap());
    }
    assert_eq!(written[0], written[1], "two bakes differ");
}

#[test]
fn check_mode_validates_without_writing() {
    // What a CI job runs to prove the content still bakes.
    let dir = std::env::temp_dir().join("artificer_bake_cli_check");
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = write_manifest(&dir, "content.json", &manifest_json());

    let (ok, stdout, stderr) =
        run_bake(&["--manifest", &manifest.display().to_string(), "--check"]);
    assert!(ok, "check failed:\n{stderr}");
    assert!(stdout.contains("nothing written"), "stdout: {stdout}");
    assert!(
        stdout.contains("1 assets"),
        "should report contents: {stdout}"
    );
}

#[test]
fn a_bad_manifest_fails_with_a_message_that_names_the_problem() {
    let dir = std::env::temp_dir().join("artificer_bake_cli_bad");
    std::fs::create_dir_all(&dir).unwrap();

    // A misspelled key, which is the commonest real authoring mistake and the
    // reason the manifest types deny unknown fields.
    let manifest = write_manifest(
        &dir,
        "typo.json",
        &manifest_json().replace("\"max_size\"", "\"maxsize\""),
    );
    let (ok, _, stderr) = run_bake(&["--manifest", &manifest.display().to_string(), "--check"]);
    assert!(!ok, "a typo should fail the bake");
    assert!(
        stderr.contains("maxsize"),
        "error should name the key: {stderr}"
    );
}

#[test]
fn missing_arguments_explain_themselves() {
    let (ok, _, stderr) = run_bake(&["--manifest", "x.json"]);
    assert!(!ok);
    assert!(stderr.contains("--out is required"), "stderr: {stderr}");

    let (ok, _, stderr) = run_bake(&["--nonsense"]);
    assert!(!ok);
    assert!(stderr.contains("unknown argument"), "stderr: {stderr}");
}
