//! `artificer_bake` — turn an import manifest into a baked asset pack.
//!
//! Deliberately a CLI and not a build script. A `build.rs` would re-parse
//! every FBX on every build, and it would have to be disabled for WASM
//! anyway, since the parsers cannot go there. A bake is a content step: run
//! it when the art changes, commit the pack.
//!
//! ```text
//! artificer_bake --manifest content/ships.json --out assets/ships.apack
//! artificer_bake --manifest content/ships.json --check   # validate only
//! ```

use artificer_assets::ImportManifest;
use artificer_assets_import::import_manifest;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

struct Args {
    manifest: PathBuf,
    out: Option<PathBuf>,
    check_only: bool,
    verbose: bool,
}

fn usage() -> &'static str {
    "artificer_bake — bake an import manifest into an asset pack

USAGE:
    artificer_bake --manifest <FILE> [--out <FILE>] [--check] [--verbose]

OPTIONS:
    --manifest <FILE>   Import manifest (JSON). Source paths are resolved
                        relative to the manifest's own directory.
    --out <FILE>        Where to write the pack. Required unless --check.
    --check             Import and validate, but write nothing. This is what
                        a CI job runs to prove the content still bakes.
    --verbose           Log every asset as it is imported.
"
}

fn parse() -> Result<Args, String> {
    let mut manifest = None;
    let mut out = None;
    let mut check_only = false;
    let mut verbose = false;

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--manifest" => {
                manifest = Some(PathBuf::from(argv.next().ok_or("--manifest needs a path")?))
            }
            "--out" => out = Some(PathBuf::from(argv.next().ok_or("--out needs a path")?)),
            "--check" => check_only = true,
            "--verbose" | "-v" => verbose = true,
            "--help" | "-h" => return Err(usage().to_string()),
            other => return Err(format!("unknown argument '{other}'\n\n{}", usage())),
        }
    }

    let manifest = manifest.ok_or_else(|| format!("--manifest is required\n\n{}", usage()))?;
    if out.is_none() && !check_only {
        return Err(format!(
            "--out is required unless --check is given\n\n{}",
            usage()
        ));
    }
    Ok(Args {
        manifest,
        out,
        check_only,
        verbose,
    })
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(if args.verbose { "debug" } else { "info" }),
    )
    .format_timestamp(None)
    .init();

    match run(&args) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            // One error, on stderr, naming the asset and the fix. A bake runs
            // unattended over a content library; a bare "failed" costs
            // whoever reads the log a bisect.
            eprintln!("bake failed: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<String, String> {
    let text = std::fs::read_to_string(&args.manifest)
        .map_err(|e| format!("could not read {}: {e}", args.manifest.display()))?;
    let manifest = ImportManifest::from_json(&text)
        .map_err(|e| format!("{} is not a valid manifest: {e}", args.manifest.display()))?;

    // Source paths are relative to the MANIFEST, not the working directory,
    // so a content folder can be checked out anywhere and a bake can be run
    // from anywhere.
    let root = args
        .manifest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));

    let pack = import_manifest(root, &manifest).map_err(|e| e.to_string())?;
    let report = pack.size_report().map_err(|e| e.to_string())?;

    match (&args.out, args.check_only) {
        (_, true) => Ok(format!("{report}\nchecked, nothing written")),
        (Some(out), false) => {
            let bytes = pack.to_postcard_current().map_err(|e| e.to_string())?;
            if let Some(dir) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
            }
            std::fs::write(out, &bytes)
                .map_err(|e| format!("could not write {}: {e}", out.display()))?;
            Ok(format!("{report}\nwrote {}", out.display()))
        }
        (None, false) => unreachable!("parse() requires --out unless --check"),
    }
}
