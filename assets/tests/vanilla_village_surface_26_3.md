# Vanilla village terrain and biome fixtures

These fixtures were captured from the unmodified 26.3 server's generation logic with
seed `1790825110648364942`, default Overworld settings, and no datapacks. They are
reference data for the village-surface investigation. The Rust comparisons now
pass for all 24,064 columns (height and top two states) and 144,384 biome samples.

`vanilla_village_surface_1790825110648364942.json` contains 94 chunks. The original
five village samples are `(-20,15)`, `(-21,15)`, `(-19,15)`, `(-20,14)`, and `(-20,16)`.
They are retained unchanged. Village coverage now includes all 90 chunks at
X `-25..=-17`, Z `9..=18`, covering the complete piece bounding box and its
12-block terrain-adaptation margin. `village_bounds` gives its inclusive X/Z bounds.
Additional samples are the outpost `(22,-58)`, the land above an ancient city
`(62,97)`, trail ruins `(-88,6)`, and a control chunk `(0,0)`. The ancient city
sample is surface terrain, not a check of its underground terrain adaptation.

The capture server ran outside the checkout, on port 25599, with `online-mode=false`.
Console `locate structure` commands selected the structures; `forceload add`
generated their surroundings and the control chunk. A scratch Java instrumentation
hook at `MaterialSystem.buildSurface` return only read the chunk. It recorded:

- the floor of the column's `MaterialSystem.preliminarySurfaceFunction()` sample;
- the highest solid, non-fluid block's Y and the state IDs there and one block below;
- every section's biome at its 4x4x4 quart coordinates.

Columns are ordered by local X, then Z. Each row is
`[preliminary_surface, top_solid_y, top_state_id, state_id_below]`. This is after
surface generation and before carvers, structures and features, so template roofs,
paths, trees and later terrain changes cannot contaminate the surface comparison.
Biome samples were independently read from the saved vanilla Anvil sections. All
144,384 samples matched the hook's samples. Fixture biome IDs index `biome_names`,
which is sorted by name; they are not vanilla's numerical registry IDs.

The biome registry fixture now comes from a configuration-phase packet capture
through vanilla's `Biome.NETWORK_CODEC`, not raw biome JSON. See
[capture method and serialization corrections](vanilla_biome_registry_26_3.md).
The protocol test compares complete decoded compounds and entry order/ids,
including absent fields and NBT numeric types. In 26.3, sky/fog colours, particles
and sounds live in environment attributes; non-syncable gameplay attributes are
omitted from the network encoding.

The plains village start in vanilla has 96 pieces, with bounding box
`[-383,62,171]` through `[-270,95,287]`. Before decoration, its 13,338 surface
columns contain 6,713 grass blocks, 2,409 dirt, 3,242 sand, 963 gravel and 11
sandstone. The fixtures cover this entire bounding box and the surrounding chunks.

Vanilla's `RandomState` supplies `router.chunkSurfaceLevel()` to `MaterialSystem`.
The Overworld datapack defines that as 16-block interpolation of the preliminary
surface function. The captured values match interpolation from the four chunk
corners. A proposed change to sample the un-interpolated function at every block
was therefore discarded.

The comparison initially failed in 1,053 village columns; all other sampled
chunks matched. The village start's center is `(-324,63,244)`, outside its start
chunk `(-20,15)`. Vanilla's `Structure.GenerationContext.isValidBiome`
(`Structure.java:251`) resolves that absolute quart position through the biome
source. Pumpkin instead wrapped it into the start chunk's stored biome palette,
returning river instead of plains. It rejected and cached the invalid start,
omitting the village's beardifier inputs. Start validation now shares the same
absolute biome-source lookup as structure reference generation.

Expanding coverage found five additional columns just west of the piece bounds
whose surface heights were one block too high. `ChunkGenerator.createReferences`
(`ChunkGenerator.java:649`) tests `StructureStart.getBoundingBox`
(`StructureStart.java:72`), which invokes `Structure.adjustBoundingBox`
(`Structure.java:80-81`) and expands terrain-adapted bounds by 12 blocks. Pumpkin
tested the unadjusted piece bounds for normal references, omitting beardifier
inputs in those edge chunks. It now reuses the existing adjusted-bounds helper.

With both production fixes temporarily reverted, the expanded surface comparison
failed in 4,795 columns, and `village_start_outside_source_chunk_uses_biome_source`
failed with the plains/river diagnostic. With only the biome-source fix, the
surface comparison failed in the five edge columns. With both fixes restored,
all four world tests passed. The protocol registry comparison also passed.

The village counts below are for its complete 13,338-column piece footprint.
Other rows each cover one 256-column chunk, at the coordinates listed above.

| Sample | Top blocks before fix | Top blocks in vanilla and after fix |
| --- | --- | --- |
| Village footprint | grass 5,640; dirt 3,099; sand 2,884; gravel 1,715 | grass 6,713; dirt 2,409; sand 3,242; gravel 963; sandstone 11 |
| Outpost | grass 256 | grass 256 |
| Trail ruins | grass 256 | grass 256 |
| Control | grass 120; dirt 136 | grass 120; dirt 136 |
| Above ancient city | grass 138; coarse dirt 112; terracotta 1; yellow terracotta 4; orange terracotta 1 | same |

The biome test converts generated proto chunks into actual level chunk sections
and decodes their network palettes. All 2,160 village sections (138,240 cells),
24 outpost sections (1,536 cells), and 72 other sections (4,608 cells) match the
saved vanilla Anvil biome sections, with no mismatched biome pairs. The registry
packet comparison passes for all 67 entries after codegen removes generation
fields and non-syncable attributes and matches vanilla's float encoding. No
biome palette fix was needed. Underground ancient-city adaptation and fully
decorated terrain are outside this surface-stage regression's coverage.

Capture scripts, the three stopped server worlds, logs and raw samples remain in
`F:\minecraft-rust\scratch\lane-e-village`. No vanilla jar is needed at test time.
