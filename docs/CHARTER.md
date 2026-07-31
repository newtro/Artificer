# Artificer Engine Charter

Artificer is a public, reusable, AI-first Rust game engine. It exists to let an
AI development agent own architecture, code, builds, tests, and verification
end to end: reproducible builds, declarative content, structured inspection,
headless operation, replay, and automated testing are first-class concerns.

Artificer is developed in its own repository and consumed by games (first
consumer: TWFI) as a pinned dependency. During MVP development, games may use
a sibling-path override; semantic versioning begins after MVP stabilization.

## Responsibilities (engine owns)

- App lifecycle and plugin boundary (`artificer_core`)
- Fixed ticks, scheduling, deterministic time, seeded RNG (`artificer_core`)
- Entity id allocation and reusable simulation primitives (`artificer_core`)
- Commands, events, snapshots, and replay (`artificer_core::events` for tick-scoped queues; `artificer_testkit::replay` for the EventLog + fold discipline games use to prove persistent ledgers reproduce end state)
- Renderer-neutral scene API (`artificer_scene`)
- Bevy/wgpu rendering adapter (`artificer_render`)
- Rapier physics adapter (`artificer_physics`)
- Input and camera foundations (`artificer_input`, `artificer_render`)
- Network transport, prediction, interpolation, reconciliation (`artificer_net`, added at M2)
- Asset manifests, procedural mesh builders, validation, the baked pack
  format and its runtime loader (`artificer_assets`)
- Importing source art -- FBX, OBJ, glTF -- into that pack
  (`artificer_assets_import`). NATIVE ONLY: `artificer_assets` compiles into
  the WASM client and must never depend on this crate, which is what keeps a
  model parser out of a browser bundle. Both check scripts assert the edge
  does not exist. See ADR-0003.

**The engine ships the pipeline; games ship the art.** No game art lives in
this repository. Test fixtures are CC0 (Kenney) precisely so a public repo can
carry them; licensed art belongs in the consuming game's own repo.
- Browser, Windows, and headless platform adapters (`artificer_render` / headless runners)
- Agent client SDK (`artificer_agent`, added at M4)
- Scenario compiler, inspection, testkit (`artificer_testkit`)

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
   `artificer_render` for game-specific rendering (custom shaders, UI). See
   ADR-0002. Domain and protocol crates may not.

## Platform targets

- Browser (wasm32-unknown-unknown, WebGL2 via Bevy/wgpu, bundled by Trunk)
- Native Windows (msvc)
- Headless Linux/Windows (no render/window dependencies compiled at all)
