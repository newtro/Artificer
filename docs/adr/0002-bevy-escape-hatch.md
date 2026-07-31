# ADR-0002: Sanctioned Bevy extension surface for game clients

Date: 2026-07-25
Status: Accepted

## Context

The engine exposes a renderer-neutral scene API (`artificer_scene`) that covers
common presentation: meshes, PBR materials, lights, cameras, transforms.
AAA-grade game-specific rendering (volumetric nebula shaders, bloom-driven
star fields, animated route lines on the star map) requires custom WGSL
materials and render-graph features that a generic scene API cannot expose
without re-inventing Bevy's material system one-to-one.

The plan's boundary rule is precise: *game domain data* must not store
Bevy-specific objects, and engine public APIs must stay game-agnostic. The
plan explicitly assigns "cockpit, map, market, and game-specific UI" to the
game repository — that code is presentation-layer, not domain data.

## Decision

`artificer_render` re-exports Bevy under `artificer_render::bevy` as a sanctioned,
documented extension surface. Game *client* crates may register Bevy plugins,
custom `Material` implementations, and UI through it.

Constraints that keep the boundary honest:

1. Only client (presentation) crates may import `artificer_render`. Domain,
   protocol, simulation, server, and AI crates must not (enforced by
   dependency direction; checked in review).
2. Everything the server or headless simulation needs must flow through
   Bevy-free crates (`artificer_core`, `artificer_physics`, `artificer_scene` types).
3. If a second game needs the same extension pattern, it uses the same
   surface — nothing TWFI-specific enters the engine.

## Amendment (M1): direct `bevy` dependency for derive macros

Bevy's derive macros (`#[derive(Component)]` etc.) resolve crate paths from
the *calling* crate's manifest, so game client crates additionally declare a
direct `bevy` dependency pinned to the exact engine version with
`default-features = false` and no extra features (cargo unifies it with the
engine's build — zero additional compilation). The sanctioned surface for
everything else remains `artificer_render::bevy`; the direct dependency exists
only so derives compile, and must never widen the feature set.

## Consequences

- The star map and cockpit can use full-fidelity custom shaders without
  the engine growing game-specific API.
- Swapping the render adapter later means porting game presentation code,
  which is the industry-normal cost; authoritative gameplay is unaffected.
