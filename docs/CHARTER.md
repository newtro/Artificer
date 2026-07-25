# Aether Engine Charter

Aether is a private, reusable, AI-first Rust game engine. It exists to let an
AI development agent own architecture, code, builds, tests, and verification
end to end: reproducible builds, declarative content, structured inspection,
headless operation, replay, and automated testing are first-class concerns.

Aether is developed in its own repository and consumed by games (first
consumer: TWFI) as a pinned dependency. During MVP development, games may use
a sibling-path override; semantic versioning begins after MVP stabilization.

## Responsibilities (engine owns)

- App lifecycle and plugin boundary (`aether_core`)
- Fixed ticks, scheduling, deterministic time, seeded RNG (`aether_core`)
- Entity id allocation and reusable simulation primitives (`aether_core`)
- Commands, events, snapshots, and replay primitives (`aether_core::events`, `aether_testkit`)
- Renderer-neutral scene API (`aether_scene`)
- Bevy/wgpu rendering adapter (`aether_render`)
- Rapier physics adapter (`aether_physics`)
- Input and camera foundations (`aether_input`, `aether_render`)
- Network transport, prediction, interpolation, reconciliation (`aether_net`, added at M2)
- Asset manifests, procedural mesh builders, validation (`aether_assets`)
- Browser, Windows, and headless platform adapters (`aether_render` / headless runners)
- Agent client SDK (`aether_agent`, added at M4)
- Scenario compiler, inspection, testkit (`aether_testkit`)

## Non-responsibilities (games own)

Sectors, jump rules, flight tuning, ships/blueprints, scanning, markets,
production, trading, combat rules, factions, security, captain identity,
game UI, world content, balance, branding, deployment configuration.

## Boundary rules

1. Engine tests and samples must never require a game repository.
2. Engine public APIs must remain free of game-specific concepts
   (no "sector", "captain", "port", "trade" in engine API names).
3. Rendering, physics, and transport remain replaceable adapters; game
   *domain data* must never store Bevy-, Rapier-, or transport-specific types.
4. The generality test: `samples/minimal` must build and run using only
   public engine APIs. If it needs game imports, the boundary has failed.
5. Game clients MAY use the sanctioned Bevy extension surface re-exported by
   `aether_render` for game-specific rendering (custom shaders, UI). See
   ADR-0002. Domain and protocol crates may not.

## Platform targets

- Browser (wasm32-unknown-unknown, WebGL2 via Bevy/wgpu, bundled by Trunk)
- Native Windows (msvc)
- Headless Linux/Windows (no render/window dependencies compiled at all)
