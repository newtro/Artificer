# ADR-0005: Scene asset lifecycle (release by span)

Date: 2026-08-04
Status: Accepted

## Context

`SceneGraph` could register meshes and textures but never forget them. The
Bevy adapter holds a strong handle for every registered asset in
`AdapterMaps`, so nothing was ever freed: every sector jump in a game
re-registers its scenery — and since ADR-0004, freshly baked planet
textures — and the old set stayed resident for the life of the process.
Flagged as a MAJOR in the procgen review.

Auto-collection (free an asset when its last node despawns) was considered
and rejected: register-once/spawn-many is a load-bearing pattern — pack
hulls spawn and despawn with every docking, and collecting them on undock
would destroy assets the very next spawn needs.

## Decision

Ownership stays with the game; the engine gains the verbs:

1. `SceneCommand::RemoveMesh` / `RemoveTexture` — deregistration, not
   destruction. The adapter drops its handle; nodes already spawned hold
   their own, so the GPU asset lives exactly until the last user despawns.
   Removing an unknown id warns loudly (a double release is a lifecycle
   bug, not a condition to paper over).
2. The mark/span pattern for blocks: `asset_mark()` snapshots the
   monotonic id counters, `assets_since(mark)` closes a `AssetSpan`, and
   `release_assets(span)` deregisters exactly that range. A spawn block
   (sector, hangar) brackets itself with two calls and owns everything in
   between — no helper in the block has to hand ids back up the chain.
   Spans are CLOSED ranges: "everything since the mark" evaluated at
   teardown would sweep up assets other systems registered mid-lifetime.

## Consequences

- Teardown code pairs `despawn(roots)` with `release_assets(span)`; the
  command stream orders them so entities drop their handles first and
  assets free the same frame.
- A span release cannot touch assets registered before its mark or after
  its close — pinned by test. Marks and spans carry their graph's identity,
  so one taken from a different `SceneGraph` is refused with a logged
  error instead of releasing that graph's neighbours.
- Re-registration cost on revisit is unaddressed (regenerating a sector
  re-bakes its planets); a seed-keyed cache is a separate decision.
