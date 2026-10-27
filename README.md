# blockengine

Rust voxel engine experiment with Java-edition save compatibility.

- `nbt`, `anvil`, `chunk`: read/write Java NBT and `.mca` regions, decode 1.18+ chunk sections (exact).
- `rng`, `noise`: java.util.Random, Xoroshiro128++, improved Perlin.
- `worldgen`: original terrain generator. NOT seed-identical to Java edition.

`cargo run --release --bin mapdump [seed]` writes a top-down `map.ppm`.
