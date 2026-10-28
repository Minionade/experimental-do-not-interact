# Validation of the initial foundation

Validated on Linux x86-64 with Rust 1.99.1, using the dependencies in Cargo.lock.

## Completed locally

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo test --locked`: **23 tests passed**.
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
