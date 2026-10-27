# blockengine

Rust voxel engine experiment with Java-edition save compatibility.

- `nbt`, `anvil`, `chunk`: read/write Java NBT and `.mca` regions, decode 1.18+ chunk sections (exact).
- `rng`, `noise`: java.util.Random, Xoroshiro128++, improved Perlin.
- `worldgen`: original terrain generator. NOT seed-identical to Java edition.

`cargo run --release --bin mapdump [seed]` writes a top-down `map.ppm`.
- `world`: Java world folder load/save (level.dat, regions; preserves untouched chunks).
- `light`: chunk-local sky/block light. `mesh`: face-culled meshing.
- `cargo run --release --bin render [seed]` software-renders terrain to `render.ppm`.
- `game`: world state with raycasting, block break/place (relights + remeshes), 20 TPS fixed-step fluids/entities,
  player controller (walk/sprint/sneak/jump/fly), and saving edits into a Java-format world folder.
- `cargo run --release --features gpu --bin viewer [seed|java-world-dir] [--sdr] [--paper-white=NITS] [--write]`:
  playable wgpu viewer. Click to grab the mouse; WASD, Space jump, Shift sneak, Ctrl sprint, F fly,
  left click break, right click place, 1-9 / wheel select (stone, dirt, grass, log, leaves, sand, glowstone, water, lava),
  F5 save, Esc release/quit. Seed worlds autosave to `saves/seed_N`; an existing Java world is read-only unless `--write`
  (our encoder drops data we don't model, such as block entities and structures).
  Native HDR: scRGB (Rgba16Float) when available, else HDR10/PQ, else SDR. Emissive blocks exceed 1.0.

## Resource packs

`viewer ... --pack PATH` (repeatable; later packs override earlier ones, like the pack screen's bottom-to-top order).
`PATH` can be a folder, a `.zip`, or a client `.jar`, so a vanilla jar plus a third-party pack works:
`viewer 123 --pack ~/.minecraft/versions/1.21/1.21.jar --pack MyPack.zip`.
You supply the assets; nothing from Mojang is bundled here.

- `pack`: folder/zip/jar reader with priority stacking. `atlas`: block texture atlas; animated textures
  (`.mcmeta` frame lists, per-frame times, custom frame size, `interpolate`) are re-uploaded tile by tile at 20 Hz.
- `model`: blockstate variants/multipart, model parent inheritance, `#texture` refs, element and blockstate rotation,
  UV rotation, cullfaces, tint indices, biome colormaps (`colormap/grass.png`, `foliage.png`).
- Not yet: item models, entity textures, CTM/OptiFine formats, overlays, language files, sounds, `uvlock`, `ambientocclusion`.
- `cargo run --release --bin mkpack OUT_DIR` writes a small synthetic pack (procedural textures, animated water/lava).
