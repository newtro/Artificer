# ADR-0003: The asset pipeline reads source art directly, and bakes

Status: accepted (2026-07-31)

## Context

The engine could only make geometry procedurally. Any game wanting bought or
authored art had to convert it by hand, and hand conversion is where scale and
orientation mistakes come from — each asset corrected individually, each one a
chance to get it wrong differently.

The consuming game is browser-first, so bundle size and load time are real
constraints rather than aspirations.

## Decision

**Read FBX, OBJ and glTF directly. No Blender, no external tools, no manual
conversion step.** That is the whole point of the feature; a pipeline that
still needs a person to open a DCC tool has not removed the failure mode.

**The parsers live in a separate native-only crate, `artificer_assets_import`.**
`artificer_assets` — which compiles into the WASM client — must never depend
on it. A cargo feature was considered and rejected: features unify across a
workspace, so an FBX parser would be one accidental `default-features` away
from the browser bundle. A crate boundary cannot be enabled by accident, and
both check scripts assert the edge does not exist.

**Correction is declared once, in data.** An `ImportManifest` states the axis
frame, units, rotation, mirroring, pivot policy and material bindings per
asset, and `MeshImport`'s doc block fixes the ORDER those compose in. Order is
part of the contract because mirroring before or after a rotation gives
different geometry, and "the importer will pick something" is how a library
ends up with assets corrected inconsistently.

**Games ship art; the engine ships the pipeline.** No game art lives here. The
test fixtures are CC0 (Kenney) precisely so a public repo can hold them.

**Bake to a postcard pack, load it at runtime.** The runtime parses nothing:
meshes arrive ready and textures arrive as encoded PNG the renderer already
knows how to decode. Baking is a CLI, not a `build.rs` — a build script would
re-parse every model on every build and would have to be disabled for WASM
anyway.

**Packs are byte-deterministic.** Every collection is a sorted `Vec`, no
`HashMap`; duplicate ids are refused at encode, because a stable sort would
otherwise let insertion order leak into supposedly canonical bytes. This makes
a pack diffable and cacheable, and lets CI assert that content did not change.

## Consequences

- Adding a format is a reader, not a pipeline: front-ends produce a neutral
  `SourceScene` and every correction happens once in `convert`.
- The bake is the quality gate. It enforces the same `validate_asset` contract
  procedural assets face, so importing cannot smuggle in geometry a generated
  mesh would have been rejected for, and `to_postcard_current` refuses to
  write a pack that fails its own validation.
- Vertex welding is part of import. Front-ends hand back one vertex per index
  because FBX indexes each attribute separately; welding on exact bit patterns
  recovered 840 → 410 vertices on a test model. Exact bits, not a tolerance: a
  tolerance would silently smooth every hard edge.
- The engine now has an `image` dependency, but only on the native import
  side, and only to honour `max_size` downscaling.

## Alternatives considered

- **A `build.rs` that imports at compile time.** Rejected: slow, and it cannot
  run for the WASM target that most needs the output.
- **Storing decoded pixels in the pack.** Rejected: a 2048² RGBA page is 16 MB
  raw against under 1 MB encoded, and the renderer has a decoder already.
- **Trusting the source file's material names to match its documentation.**
  Rejected after measuring: the Synty Sci-Fi Space meshes carry `SciFiSpace` /
  `SciFi11` internally while the pack's own MaterialList text file names them
  `PolygonScifiSpace_Material_01_A`. Bindings match what is IN THE FILE.
