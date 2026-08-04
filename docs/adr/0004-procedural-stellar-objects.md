# ADR-0004: Procedural stellar objects (artificer_procgen)

Date: 2026-08-03
Status: Accepted

## Context

Games on this engine need whole families of celestial scenery — terrestrial
planets, gas giants, moons, asteroids, planetary rings, atmospheres — with
wide per-instance variety from a seed. Hand-modelling them does not scale and
the current answer (flat-coloured UV spheres) reads as a placeholder.

Research (`docs/research/procedural-planet-generation-open-source.md` in the
game workspace) established that no maintained permissive Bevy planet plugin
exists; the assembled techniques are: noise-graph heightfields (noise crate,
MIT/Apache), equirect texture baking onto spheres, banded-fBm gas giants,
annulus + 1D-strip rings, and a per-planet additive shell running a
single-scattering raymarch for atmospheres (public-domain reference:
wwwtyro/glsl-atmosphere).

Two boundary facts shape the design:

1. The scene API already carries runtime PNG textures
   (`SceneCommand::AddTexture`) and PBR texture slots on `MaterialDesc`, so
   almost everything bakes on CPU and rides the existing pipeline.
2. The one thing PBR cannot express is a scattering atmosphere — that needs a
   custom material, which per ADR-0002 belongs behind an engine-owned surface,
   not in per-game shader forks. Atmospheres are as generic as lights.

## Decision

1. **New engine crate `artificer_procgen`**: seed-deterministic generation of
   stellar objects. Input: a spec (`PlanetSpec`, `AsteroidSpec` + presets per
   archetype); output: `MeshData` + baked PNG maps + ready-to-spawn material
   descriptions. Depends only on `artificer_core`, `artificer_scene`,
   `artificer_assets` (procmesh), plus `noise` and `png`. No Bevy.
2. **Atmospheres become a first-class scene concept**:
   `NodeKind::Atmosphere { mesh, atmosphere: AtmosphereDesc }` in
   `artificer_scene`, rendered by the Bevy adapter with an engine-owned WGSL
   single-scattering shell shader (additive, non-shadowing, camera assumed
   outside the shell). Parameterised by planet/atmosphere radius, per-channel
   scattering coefficients (colour), scale heights, Mie term, and world-space
   sun position. The desc carries the planet centre implicitly via the node
   transform; the adapter refreshes the shader uniform on `SetTransform`.
3. **`MaterialDesc` gains `emissive_texture`** (lava glow, city lights,
   window masks) — a straight pass-through to the PBR emissive map slot.
4. Ring geometry (`procmesh::annulus`) joins the primitive library; ring
   strip textures come from the generator and render as ordinary
   alpha-blended, double-sided, non-shadowing PBR meshes.

Game repositories own *which* archetypes exist where (their sector data), and
call the generator with specs; the engine owns *how* a spec becomes pixels.

## Consequences

- Every stellar object stays renderer-neutral data until the adapter; server
  and headless sims can generate the same bodies (e.g. for colliders) without
  Bevy.
- The scene command stream grows one node kind and one material field; replay
  files from before this ADR remain readable (serde defaults).
- The atmosphere shader is deliberately the cheap tier (single scatter, no
  LUTs). If a game later needs ground-level skies, that is a separate
  decision (likely a Bevy upgrade, per the research doc).
- Texture bakes run at spawn time on CPU (~tens of ms per body at 512×256);
  callers that need hitch-free jumps should bake ahead of the transition.
