# Aether Engine

Private, reusable, AI-first Rust game engine. See `docs/CHARTER.md` for the
boundary contract and `docs/adr/` for architecture decisions.

## Crates

| Crate | Purpose | Bevy dep |
|-------|---------|----------|
| `aether_core` | App lifecycle, plugins, fixed tick, time, ids, events, seeded RNG | no |
| `aether_scene` | Renderer-neutral scene description (meshes, materials, lights, cameras) | no |
| `aether_physics` | Rapier 3D adapter: rigid bodies, colliders, forces, queries | no |
| `aether_input` | Input state + action mapping abstraction | no |
| `aether_assets` | Procedural mesh builders, asset manifest + validation | no |
| `aether_render` | Bevy adapter: runs the app, syncs scene, input, cameras; sanctioned Bevy extension surface | yes |
| `aether_testkit` | Scenario runner, deterministic assertions, replay-ready logs (full replay lands at M4) | no |
| `samples/minimal` | Engine generality proof — builds with public APIs only | yes |

## Quick start

```powershell
./scripts/check.ps1          # fmt + clippy + tests + sample build
cargo run -p minimal         # windowed sample
cargo run -p minimal -- --headless   # headless sample (no GPU required)
```

Both this workspace and the TWFI workspace share the target directory
`../.cargo-target` (see `.cargo/config.toml`).
