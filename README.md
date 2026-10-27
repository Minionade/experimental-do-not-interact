# blockengine

Rust voxel engine experiment with Java-edition save compatibility.

- `nbt`, `anvil`, `chunk`: read/write Java NBT and `.mca` regions, decode 1.18+ chunk sections (exact).
- `rng`, `noise`: java.util.Random, Xoroshiro128++, improved Perlin.
- `worldgen`: original terrain generator. NOT seed-identical to Java edition.

`cargo run --release --bin mapdump [seed]` writes a top-down `map.ppm`.
- `world`: Java world folder load/save (level.dat, regions; preserves untouched chunks).
- `light`: chunk-local sky/block light. `mesh`: face-culled meshing.
- `cargo run --release --bin render [seed]` software-renders terrain to `render.ppm`.
- `cargo run --release --features gpu --bin viewer [seed|java-world-dir] [--sdr] [--paper-white=NITS]`:
  wgpu viewer (WASD/Space/Shift, click to grab mouse, Ctrl sprint). Native HDR: uses scRGB
  (Rgba16Float) when available, else HDR10/PQ, else SDR. Emissive blocks (lava, glowstone) go above 1.0.
- `access`: `BlockAccess` trait over world coordinates. `light::stitch` carries light across chunk borders.
- `fluid`: tick-scheduled water/lava flow (7/3 block spread, falling, drain, lava+water contact).
- `entity`: AABB-vs-voxel physics (gravity, step/jump, walls, swimming) and a wandering pig.
