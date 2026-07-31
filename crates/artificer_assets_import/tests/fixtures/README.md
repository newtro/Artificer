# Test fixtures

These are **CC0** models from [Kenney](https://kenney.nl), included so the
importer's tests run against real DCC exports rather than hand-written toy
files. See `KENNEY-LICENSE.txt`.

CC0 is why they can live in this public repo. The engine ships the pipeline;
games ship their own art, and licensed art belongs in the private game repo —
so the importer's public test suite needs geometry that is free to
redistribute.

| File | Source pack | Format | Why it is here |
| --- | --- | --- | --- |
| `craft_racer.fbx` | Space Kit | **ASCII FBX 7.3.0** | The ASCII parser path |
| `craft_racer.obj` | Space Kit | OBJ | Same model, second format — cross-check |
| `corridor.fbx` | Modular Space Kit | **binary FBX 7700** | The binary parser path |
| `rocket_baseA.fbx`, `rocket_finsA.fbx` | Space Kit | ASCII FBX | Pieces of a modular kit, for the assembly sample |

Both FBX flavours are represented deliberately. Real production art for this
project is binary FBX 7400, so a suite that only ever parsed hand-authored
ASCII would prove very little about the reader that matters.

What these fixtures do NOT cover: they are authored Y-up in metres, so they
exercise the parser but not the axis-correction path. Conversion is covered by
synthetic geometry in the unit tests, where the input frame can be stated
exactly and the expected output computed rather than eyeballed.
