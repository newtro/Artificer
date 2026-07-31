# ADR-0001: Engine boundary and technology stack

Date: 2026-07-25
Status: Accepted

## Context

The TWFI project plan (§8, §9) mandates a private reusable Rust engine in its
own repository, using selected Bevy crates and wgpu as internal substrate,
with Rapier physics and renderer/physics/transport kept behind replaceable
adapter boundaries. Game domain data must not depend on Bevy representations.

## Decision

- Rust toolchain pinned at 1.92.0 via `rust-toolchain.toml` in both repos.
- Bevy pinned at 0.16 (wgpu-based HDR pipeline, WGSL custom materials,
  required-components ECS). Chosen over 0.19 deliberately: 0.16 is a stable,
  well-understood API surface, reducing integration risk for an AI-led build.
- rapier3d 0.34 used directly (not `bevy_rapier`), wrapped by
  `artificer_physics`. This keeps the physics adapter renderer-agnostic and
  usable from the headless server without any Bevy dependency, and decouples
  Bevy and Rapier upgrade cadences.
- glam 0.29 (matching Bevy 0.16) is the shared math vocabulary of engine
  public APIs. `artificer_physics` converts glam <-> nalgebra internally by
  component, avoiding fragile version-interop features.
- Crate layout: `artificer_core` (lifecycle, ticks, ids, events, RNG),
  `artificer_scene` (renderer-neutral scene description), `artificer_render`
  (Bevy adapter), `artificer_physics` (Rapier adapter), `artificer_input`,
  `artificer_assets` (manifests + procedural meshes + validation),
  `artificer_testkit` (scenario running, assertions, replay). `artificer_net`
  and `artificer_agent` are added at M2/M4 per the roadmap.
- Headless builds must not compile Bevy at all: only `artificer_render`
  depends on Bevy, and only clients depend on `artificer_render`.
- Both workspaces share one cargo target directory
  (`../.cargo-target`) so heavy dependencies compile once per feature set.

## Consequences

- Server and simulation crates iterate at plain-Rust compile speeds.
- A second renderer or physics backend can be added by implementing the
  `artificer_scene` / `artificer_physics` contracts without touching game code.
- Engine version pinning happens through the game's Cargo.toml path/git pin.
