# Validation of the initial foundation

Validated on Linux x86-64 with Rust 1.99.1, using the dependencies in Cargo.lock.

## Completed locally

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo test --locked`: **33 tests passed**.
- `cargo build --locked --release`: passed with dynamically linked SDL2.
- Desktop smoke test: three rendered frames, no OpenGL error, PNG output inspected. The renderer reported Mesa llvmpipe (LLVM 20.1.2), OpenGL 4.5 core. This is software rendering, not a dedicated-GPU performance benchmark.
- The three-frame scene submitted 65 draws and 76,800 triangles on the final frame.
- Full-circle walk diagnostic: return-position error approximately `7.242e-14` meters.
- Orbit diagnostic: radial error after one period rounded to zero at six decimal places; the test separately bounds the maximum radial deviation during the orbit.
- Octree structure JSON save/reload: succeeded, retaining 9 nodes and the expected hull material.
- Native mod example: after 120 fixed ticks of 5 m/s² acceleration in zero gravity, displacement was approximately 2.5 m and velocity 5 m/s.

## Test coverage

The tests cover all octants and negative coordinates, randomized octree edits against a dense reference, compression after undoing edits, serialization validation, Morton-style node ancestry, cubic bounds, pole crossing, surface circumnavigation, radial gravity continuity, large-coordinate subtraction, LOD hysteresis, surface winding, matching density samples at chunk boundaries, watertight mesh edges, hidden block-face removal, planetary contact, compound-body launch, frame-time partitioning, atomic mod registry loads, and native mod commands in fixed physics ticks.

The mesh-edge test verifies the uniform-depth sphere. It does not establish mixed-depth LOD correctness; mixed-depth rendering is not implemented.

## Vanilla resource validation

The exact official 26.3 client JAR was verified against its pinned SHA-1 and embedded version. The importer resolved all 4,192 original model inheritance chains; 1,269 terminate in a built-in special-renderer root that the desktop renderer does not implement yet. Original names for Diamond Pickaxe, Stone and Zombie were read, and original diamond-pickaxe/zombie PNG byte hashes were checked through the importer. The source assets are not committed.

Nine importer tests cover names, namespace defaults, model inheritance, unknown-field preservation, special-renderer roots, sprite-object metadata, alias and inheritance cycles, invalid resource IDs, unpinned input rejection, and separation of protocol/state IDs. One additional octree test verifies IDs above 65,535. The reports parser is tested with constructed fixtures; official generated-report provenance is not validated.

The first Windows CI run exposed missing `advapi32` linkage in static SDL. `build.rs` now supplies it for Windows bundled-SDL builds. Check the latest workflow result for verification.

## Measurements, not guarantees

One release-mode depth-3 terrain run generated 128 surface meshes containing 258,852 triangles. Generation took about 5 ms and parallel meshing about 13 ms on the host used for this run. These are single measurements of a small analytic planet. They are not average frame times, scalability claims, or comparisons with Minecraft.

## Not established locally

- Interactive Windows execution and hardware-GPU performance.
- Bundled/static SDL builds on Windows or Linux; CI is configured to check them.
- Multiplayer, Minecraft gameplay or file/protocol compatibility.
- Adaptive mixed-depth terrain, streaming, editable planet density, or far-scale physics-frame migration.
- General player/ship collision and moving-platform walking.
- Shaderpack compatibility, which is outside the current requirements.

GitHub Actions builds Linux and Windows desktop targets and runs a Linux render smoke test. Consult the actual workflow result; the presence of a workflow file alone is not a successful build.

## Vanilla element geometry and original-texture preview

The next stage adds three geometry tests (36 total tests): all six cube faces wind
outward, partial-element default UVs and quarter-turn UV rotation are preserved,
and special/rotated/malformed models fail explicitly. Sprite metadata survives baking.
Local default-feature Clippy passes with warnings denied. A three-frame OpenGL
preview of `minecraft:block/crafting_table` from the pinned official client renders
six quads with the original face PNGs; its screenshot was visually inspected.
This verifies a diagnostic asset path, not vanilla gameplay or final lighting parity.
The preceding foundation commit passed Windows and Linux CI, including Linux render
smoke testing; the new preview is included in both platform artifact manifests.

## Blockstate dispatch and element rotations

43 tests pass locally, along with formatting and default-feature Clippy. New coverage
includes weighted-choice boundaries, property matching, nested multipart logic,
ambiguous/missing states, element rotation origins/rescale, multi-axis rotation order,
and UV-lock plus transformed cull directions across all 64 X/Y/Z quarter-turn combinations.

Three-frame OpenGL previews using the verified original client were visually inspected:

- East-facing lit furnace: 6 quads, correctly oriented lit front.
- Oak fence connected north/east: 26 quads from the post and two side models.
- South-facing wall torch: 6 quads with element tilt.
- East-facing bottom outer-left oak stairs: 11 quads.

The rotation format/order follows the official [1.21.11 technical notes](https://feedback.minecraft.net/hc/en-us/articles/41809981427213-Minecraft-Java-Edition-1-21-11-Mounts-of-Mayhem)
and the actual pinned 26.3 client models, including arbitrary single-axis angles and
multi-axis hanging-sign elements. Legacy axis/angle fields take precedence. Element
rotation leaves nominal face/cull directions intact; blockstate rotation transforms them.
Rescale is implemented for legacy 0/22.5/45-degree single-axis cases; other rescale
combinations fail explicitly pending verification. These tests do not certify pixel parity,
lighting, tinted/animated sprites, or vanilla position-seeded weighted randomness.

## Authoritative registry and editable-world storage

The Java 25 data generator from the SHA-1-pinned official 26.3 server produced
1,286 registered blocks, 35,723 global block states, 1,658 items, and 161 entity
types. Independently repeated report generation produced identical blocks.json
and registries.json hashes, now pinned in config/minecraft-26.3.json. These counts
are registry counts, unlike the earlier resource-file counts.

`StateRegistry` validates property domains/combinations and resolves overrides
against authoritative defaults. `BlockWorld` stores those IDs in the cubic octree,
invalidates neighboring chunks at edited boundaries, supports voxel ray queries,
and rejects saves with mismatched version/registry fingerprints. Dense active-chunk
enumeration is budgeted. This is storage/edit infrastructure, not block behavior parity.
49 tests pass, including negative coordinates, ray edge ties, and save identity.

## Original textures in editable cubic chunks

53 tests pass, covering sprite timing/metadata and cross-chunk original-model face
culling in addition to the registry, storage, physics and geometry checks. Both
headless and default-feature Clippy pass with warnings denied. A real three-frame
Mesa OpenGL run of the asset world rendered 8 chunks, 3,166 triangles, 10 shared
original sprites and 22 draw calls; the frame was visually inspected. Loading the
saved world and meshing again is checked independently. The renderer uses chunk-local
coordinates, camera-relative transforms, conservative occlusion and dirty-chunk VBO
replacement. Translucency sorting is currently at batch granularity; intersecting
translucent faces and full vanilla shading remain parity gaps.

## Item component and inventory foundation

The 1,658 default item-component reports are verified with one ordered aggregate
SHA-1 (item ID, zero separator, little-endian byte length, exact report bytes).
Original components—including tool rules, damage, food and names—remain available
without renaming or replacing them. Inventory operations enforce effective stack
limits, merge only equal effective components, split without loss, and keep air as
an empty slot. Durability consumption reports breakage to the caller; enchantment
probabilities and survival integration remain unfinished. Added tests cover item
conservation, component-aware merging, stack limits and durability boundaries.

## Ordinary recipe execution

60 tests pass with formatting and Clippy clean. The exact client data loads 236 item
tags, 1,202 ordinary shaped/shapeless recipes and 116 cooking recipes. Another 724
recipes require specialized handlers and are retained without execution. Ingredient
matching uses complete assignment for overlapping shapeless alternatives. Tests cover
mirroring/offsets, surplus inputs, recursive tag cycles, full output inventories and
container remainders. End-to-end runs crafted an Iron Pickaxe and Cake from the original
recipes: the former consumed one item per occupied slot, and the latter returned all
three buckets. No cooking simulation, special crafting, recipe unlocks, or GUI parity
is claimed by these backend checks.

## Swept collision kernel

The collision module resolves f64 player-sized boxes against supplied unions of
axis-aligned boxes. Six regression tests cover high-speed wall impacts, sliding,
slab landing and stepping, low ceilings, negative coordinates, and rejected
penetrations/invalid inputs. This is infrastructure: the desktop camera remains
in flight mode until authoritative state shapes are connected. It does not claim
vanilla movement tuning, moving-collider response, or dynamic shape parity.


## Original shape integration

The reproducible Java 25 exporter was compiled and run against the pinned 26.3
server. Regenerating the complete reports reproduced the physics SHA-1 in the
version pin. The Rust importer validated all 35,723 state identities and all
1,658 item remainder records; 35,425 states have supported static shapes.
The original-data integration test passed for slab targeting, landing, stepping,
fence height, and explicit dynamic/position-offset rejection. The ordinary core
suite has 68 passing tests; the original-data test is separately opt-in.
Desktop walking uses these boxes at 120 Hz with stepping and sliding. It is
experimental movement, without vanilla acceleration/drag, fluid behavior, fall
damage, or dynamic collision semantics. Planetary character movement remains a
separate implementation.

## Stonecutting

All 351 pinned stonecutting recipes passed input consumption/output validation
and full-inventory rollback checks against original recipe JSON. Seventy core
tests pass; warning-free Clippy covers all targets. This is backend execution,
not stonecutter menu or progression parity. Specialized unsupported recipes are
now 373, down from 724; cooking recipes remain parsed but not simulated.

## Smithing transforms

All 12 original smithing-transform recipes passed original-data integration tests
for matching, component preservation, target prototype defaults, consumption and
blocked-output rollback. The public Java runtime probe confirmed damage 321 and
custom-name retention, explicit repair-cost removal, and recipe damage override
7 when upgrading a diamond pickaxe to netherite. Source values equal to their
prototype defaults are normalized away before transfer. The core suite contains
73 passing tests; two original-recipe integration tests are opt-in. Specialized
unsupported recipes now number 361. Smithing trims and menu/progression behavior
are still unimplemented.
