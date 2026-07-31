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
| `craft_racer.glb` | Space Kit | GLB | Same model again — the FBX/glTF cross-check |
| `corridor.glb` | Modular Space Kit | GLB | Second cross-check pair |
| `rocket_baseA.fbx`, `rocket_finsA.fbx` | Space Kit | ASCII FBX | Pieces of a modular kit, for the assembly sample |
| `colormap.png` | Modular Space Kit | PNG, 512² | The atlas page, for texture baking and downscaling |

Both FBX flavours are represented deliberately. Real production art for this
project is binary FBX 7400, so a suite that only ever parsed hand-authored
ASCII would prove very little about the reader that matters.

Having the SAME model in two formats is the point of the `.glb` pair, not
redundancy: two independent readers producing the same geometry is what caught
the FBX reader dropping node transforms, once the comparison was strengthened
from bounding-box SIZE (translation-invariant, so it saw nothing) to position.

What these fixtures do NOT cover on their own: they are authored in metres, so
correction maths is also checked against synthetic geometry in the unit tests,
where the input frame can be stated exactly and the expected output computed
rather than eyeballed. Both are needed — testing corrections only on synthetic
scenes, and file reading only on real files, is precisely the gap two blockers
fell through.
