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
