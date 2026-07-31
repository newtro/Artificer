# Artificer

Headless-first Rust game engine built for autonomous agents to build games
on — deterministic, replayable, no editor in the loop.

Everything an agent needs to own a game end to end is reachable from code:
reproducible builds, declarative content, structured inspection, headless
simulation, event replay, and scripted verification. There is no GUI step
in the pipeline, and nothing requires a human hand on a mouse.

Three targets from one codebase: native window, WebAssembly, and headless
(no window, no GPU) for tests and accelerated simulation.

See `docs/CHARTER.md` for the engine/game boundary contract and `docs/adr/`
for architecture decisions.

## Crates

| Crate | Purpose | Bevy dep |
|-------|---------|----------|
| `artificer_core` | App lifecycle, fixed tick, time, ids, events, seeded RNG | no |
| `artificer_scene` | Renderer-neutral scene description (meshes, materials, lights, cameras) | no |
| `artificer_physics` | Rapier 3D adapter: rigid bodies, colliders, forces, spatial queries | no |
| `artificer_input` | Input state + action mapping abstraction | no |
| `artificer_assets` | Procedural mesh builders, asset manifest + validation | no |
| `artificer_net` | Versioned wire codec, client transports (native/wasm/loopback/latency-lab), WebSocket server, prediction primitives | no |
| `artificer_agent` | Headless agent SDK: byte-oriented `HeadlessClient` + fixed-rate `AgentLoop` — AI actors, load bots, soak harnesses | no |
| `artificer_render` | Bevy adapter: runs the app, syncs scene, input, cameras, cursor grab (mouse-look); sanctioned Bevy extension surface | yes |
| `artificer_testkit` | Scenario runner, deterministic assertions, event replay (`replay::EventLog` + fold — games prove their ledgers reproduce end state) | no |
| `samples/minimal` | Engine generality proof — builds with public APIs only | yes |

## Quick start

```powershell
./scripts/check.ps1          # fmt + clippy + tests + sample build
cargo run -p minimal         # windowed sample
cargo run -p minimal -- --headless   # headless sample (no GPU required)
```

Both this workspace and the TWFI workspace share the target directory
`../.cargo-target` (see `.cargo/config.toml`).
