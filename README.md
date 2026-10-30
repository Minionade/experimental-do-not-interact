# Experimental voxel engine

Rust foundation targeting **1:1 Minecraft Java 26.3 content and gameplay**, with **3D octree chunks, spherical planets, spaceflight, and native rigid-body physics**. The working crate name is `orbital-voxel`.

**This is a runnable engine prototype, not a port of Minecraft 26.3.** Full Minecraft gameplay, vanilla world loading, networking, and Java mod compatibility are not implemented. The original-asset block world supports editing, original crafting recipes, and static-shape walking; the separate planetary scene uses diagnostic geometry and colors. The renderer uses engine-owned shaders; Iris shaderpack compatibility is outside the project requirements.

## Run

Install stable Rust. The desktop renderer requires OpenGL 3.3 or newer.

**Windows:** install the Visual Studio C++ Build Tools and CMake, then run:

```powershell
cargo run --locked --release --features bundled-sdl
```

`bundled-sdl` builds and statically links SDL, avoiding a separate SDL DLL installation. CI also builds a Windows executable; check the repository's **Actions** artifacts after a successful run.

**Linux (Debian/Ubuntu):**

```sh
sudo apt-get install libsdl2-dev
cargo run --locked --release
```

**Without a window or graphics libraries:**

```sh
cargo test --locked --no-default-features
cargo run --locked --release --no-default-features -- --headless --mesh-bench
cargo run --locked --no-default-features --example flight_mod
```

## Vanilla content parity

All base-game items, tools, blocks, entities, names, textures, models, recipes, and behavior must match Minecraft 26.3. The octree/planetary engine must not introduce renamed substitutes or a reduced replacement content roster.

The importer reads an official 26.3 client JAR, checks its exact size/SHA-1 and embedded version, and retains original names, PNG bytes, blockstates, item definitions and model fields. The report parser keeps registry IDs and global block-state IDs separate. Octree cells use 32-bit IDs.

```powershell
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar "$env:APPDATA\.minecraft\versions\26.3\26.3.jar" --audit-models --inspect-item minecraft:diamond_pickaxe --inspect-block minecraft:stone --inspect-entity minecraft:zombie
```

This currently audits and resolves resources. It does **not** yet render or implement the vanilla roster. Original game assets are read from your installation and are not included in the repository. See [the parity contract](docs/vanilla-parity.md).

## Controls

| Control | Action |
| --- | --- |
| Left click | Capture mouse for looking |
| WASD | Move; in ship mode, apply acceleration |
| Space / Left Ctrl | Up / down in free-flight and ship mode |
| Left Shift | Faster movement / stronger acceleration |
| F | Toggle surface walking on the main planet / free flight |
| G | Toggle third-person control of the first physics structure |
| E | Spawn another block-built structure near the camera; maximum 64 |
| Escape | Release mouse; press again to quit |

Surface walking follows a great circle with a continuously changing local up vector. Continue walking to return to the starting point. Free flight is a debug camera; ship mode controls a physical body. The prototype's walking controller is constrained to the analytic sphere and does not yet collide with ships or support jumping.

## Implemented

- **16×16×16 block bricks** in a sparse octree. Empty/uniform regions collapse, edits allocate only their path, and reverting edits compresses the tree again. Negative coordinates work on all three axes.
- **Spherical density terrain in an octree.** Surface leaves have 16³ cells and 17³ shared corner samples. Empty and solid nodes skip meshing. Each planet has its own bounded root; there are no column chunks or cube-face portals.
- Parallel marching-tetrahedra surface meshing, block-face culling, GPU-resident meshes, frustum culling, and camera-relative vertex transforms.
- Distance-based octree LOD with hysteresis. The renderer currently selects one uniform depth per planet, keeping neighboring surface chunks watertight. Local mixed-depth LOD, transition stitching, streaming, and asynchronous uploads are not implemented.
- Rapier `f64` rigid bodies, compound block colliders, contact response, CCD, 120 Hz fixed steps, radial gravity, and multiple gravity sources. Gravity continues smoothly into space. A split gravity integrator keeps the tested orbit bounded.
- Octree structure serialization and reload validation. This is not a complete game save format.
- Validated TOML block definitions and a compiled-in Rust mod event/command API. Native mods can issue acceleration commands during fixed ticks; [the working example](examples/flight_mod.rs) demonstrates this. The TOML registry is groundwork and is not yet wired to the demo's hard-coded block palette or density.
- Engine-owned GLSL terrain and post-processing shaders, compiled and validated by the OpenGL driver.

## Original model preview

The separate diagnostic viewer now renders vanilla element models with
original PNGs from your hash-verified 26.3 client JAR:

```sh
cargo run --locked --bin vanilla-preview -- --minecraft-jar /path/to/26.3.jar --block minecraft:furnace --state facing=east,lit=true
cargo run --locked --bin vanilla-preview -- --minecraft-jar /path/to/26.3.jar --model minecraft:block/stone --frames 3 --screenshot stone.png
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar /path/to/26.3.jar --bake-model minecraft:block/crafting_table
```

Escape closes the preview. The audit command prints baked geometry, normalized UVs,
and original texture references after checking every referenced PNG exists.
The viewer supports variant/property selection, weighted choices, multipart conditions,
X/Y/Z blockstate rotations, UV-lock, and single/multi-axis element rotations. For example:

```sh
cargo run --locked --bin vanilla-preview -- --minecraft-jar /path/to/26.3.jar --block minecraft:oak_fence --state north=true,east=true,south=false,west=false,waterlogged=false
cargo run --locked --bin vanilla-preview -- --minecraft-jar /path/to/26.3.jar --block minecraft:wall_torch --state facing=south
```

Supply complete registry-valid properties with `--state`; this command does not
infer defaults or validate the entire property domain. `--ticket N` chooses a weighted
model deterministically; vanilla's position-seeded random sequence is not implemented.
The viewer uses nearest-neighbor sampling and unlit colors. It remains separate from
the planet demo. Biome tint, animation, lighting, translucent sorting, generated items,
and entity renderers remain unfinished. Special renderers and unverified rescale
combinations produce explicit errors. Non-square textures are rejected rather than stretched.
Minecraft assets remain in your local client JAR and are not bundled with the build.

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo run --locked --release -- --frames 3 --screenshot smoke.png
cargo run --locked --release --no-default-features -- --save-demo demo-save.json
```

A graphics-free CI job runs the core tests. Desktop CI builds for Windows and Linux and renders on Linux. [Validation notes](docs/validation.md) distinguish checks actually performed from future targets.

## Direction

- [Vanilla parity contract and resource import](docs/vanilla-parity.md)
- [Architecture and octree decisions](docs/architecture.md)
- [Minecraft 26.3 parity and implementation roadmap](docs/roadmap.md)

The code and procedural visual assets in this repository are original project work. Third-party libraries are declared in Cargo manifests. Sable is a functional reference; no Sable source code is copied into this prototype.

## Generate exact registry reports

Use your official 26.3 server JAR and a Java 25 JDK (including `javac`) (Python 3 is needed only for this helper):

```sh
python tools/generate_reports.py --server-jar /path/to/server.jar --java /path/to/java --output /path/to/generated
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar /path/to/26.3.jar --pinned-reports /path/to/generated/reports
```

The helper verifies the server JAR before execution and verifies the generated
block/registry, item-component, and runtime physics report hashes afterward. The helper compiles the small `VanillaOracle.java` exporter against the extracted server libraries; `--javac` can select a compiler explicitly. `--pinned-reports` imports all 35,723 global
states from those exact reports. The existing `--reports` flag remains a schema-only
inspection mode. No game JARs or generated report payloads are distributed here.

## Editable textured octree world

```sh
cargo run --locked --release --features bundled-sdl -- --minecraft-jar /path/to/26.3.jar --reports /path/to/generated/reports --world-save my-world.json
cargo run --locked --release --features bundled-sdl -- --minecraft-jar /path/to/26.3.jar --reports /path/to/generated/reports --load-world my-world.json --world-save my-world.json
```

Click to capture the mouse; WASD flies, Space/Ctrl moves vertically, Shift moves
faster. F toggles walking with gravity, slab steps, and Space to jump. Walking uses a fixed 120 Hz step; its movement tuning is experimental. Keys 1–9 select original blocks, left click removes, right click places,
and middle click selects a matching palette state. F5 saves when `--world-save`
is supplied; it also saves on exit. Escape releases the mouse, then exits.

This mode is a small creative editing test world backed by the actual cubic octree
and global state registry. Chunk-local meshes share original sprites; edits rebuild
only affected chunks and boundary neighbors. Animated sprite metadata is decoded,
including frame ordering, durations and interpolation. Conservative full-face
occlusion avoids treating partial models or transparent textures as solid cubes.
The renderer uses diagnostic directional shading, not vanilla lighting/AO. It does
not yet implement survival, placement rules, automatic neighbor state changes,
item entities, biome tint, or vanilla world generation. Selection and walking use original static outline/collision boxes. Context-dependent shapes (including powder snow and position-offset plants) are rejected explicitly. These shapes, moving blocks, fall damage, fluids, and vanilla movement tuning remain unfinished.
The original spherical-planet mode remains available by omitting the asset arguments.

For repeatable asset-world checks, add `--frames 3 --screenshot world.png`, or
`--headless --world-save check.json` to validate CPU meshing and save round-tripping
(the asset-world path currently uses the default desktop build even when headless).

## Crafting backend validation

```sh
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar /path/to/26.3.jar --pinned-reports /path/to/generated/reports --audit-recipes
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar /path/to/26.3.jar --pinned-reports /path/to/generated/reports --craft-grid examples/crafting/iron_pickaxe.json --recipe minecraft:iron_pickaxe
```

The recipe backend loads original tags and shaped/shapeless recipes, including
mirroring, offsets and alternative ingredients. Crafting into an inventory is
transactional: output and container remainders must fit before inputs are consumed.
The audit prints the resulting grid and inventory. These are backend operations;
the desktop crafting interface and survival loop are not wired up yet. Cooking
recipes are parsed with original time/experience values but are not simulated yet.
Special recipe types are retained and counted explicitly rather than approximated.


## Original physics validation

Regenerate older report directories with the updated helper to add `physics.json`.
The pinned export includes 35,723 states' shapes, hardness, resistance, friction,
light emission and tool requirements, plus every item's crafting remainder.
35,425 states currently support context-free shape queries. Dynamic and
position-offset states require additional runtime behavior.

```sh
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar /path/to/26.3.jar --pinned-reports /path/to/generated/reports --audit-physics
MINECRAFT_REPORTS=/path/to/generated/reports cargo test --locked --no-default-features --test original_physics -- --ignored
```

The explicit integration test checks original slab selection, slab landing and
stepping, fence height, and rejection of contextual shapes. Original report data
is supplied locally and is not included in the repository.

## Stonecutting backend

All 351 original stonecutting recipes are executable through `RecipeBook`.
`find_stonecutting` lists valid recipe IDs; `stonecut_into` consumes one input only
when the entire selected output fits in the destination inventory. Menu ordering,
stonecutter UI, recipe unlocks, sounds and advancements are not implemented.

```sh
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar /path/to/26.3.jar --pinned-reports /path/to/generated/reports --stonecut-input stone --recipe stone_slab_from_stone_stonecutting
MINECRAFT_CLIENT_JAR=/path/to/26.3.jar MINECRAFT_REPORTS=/path/to/generated/reports cargo test --locked --no-default-features --test original_recipes -- --ignored
```

Omit `--recipe` to list available outputs. The original-data test executes every
stonecutting recipe and verifies source-defined output counts and blocked-output
rollback. The ordinary suite also checks partial-stack capacity and component
isolation between input and output.

## Smithing transforms

The 12 original netherite upgrade recipes now support matching, output previews,
and atomic inventory transactions. Explicit base-item component changes survive
upgrading; the output receives its new item's default components, and recipe
component overrides take precedence. Redundant source defaults are discarded so
an unchanged diamond tool's durability limit does not replace the netherite limit.
Armor trims, smithing UI, unlocks and advancements remain unfinished.

```sh
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar /path/to/26.3.jar --pinned-reports /path/to/generated/reports --smithing-input examples/smithing/netherite_pickaxe.json --recipe netherite_pickaxe_smithing
```

Omit `--recipe` to list matching transforms. The original recipe integration test
also checks all 12 transformations, retained data, output defaults and rollback.
`tools/SmithingProbe.java` provides a reproducible behavioral probe against the
pinned server: compile it with the extracted server/library JARs on the classpath,
then run it with the generated reports directory as its sole argument. It checks
original Java smithing/component-transfer behavior; it is not needed by the Rust
runtime. Use Java 25 and the hash-verified server used by `generate_reports.py`.

## Content-preserving crafting

All 33 original transmutation recipes execute through the crafting backend:
16 bundle colors, 16 shulker-box colors, and map cloning. Names and stored item
contents survive recoloring; dyeing an already matching color is rejected.
Map cloning counts occupied blank-map slots, consuming one from each and
preserving the source map ID. It does not render or generate map images yet.

```sh
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar /path/to/26.3.jar --pinned-reports /path/to/generated/reports --craft-grid examples/crafting/pink_bundle.json --recipe pink_bundle
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar /path/to/26.3.jar --pinned-reports /path/to/generated/reports --craft-grid examples/crafting/map_cloning.json --recipe map_cloning
```

Original-data tests cover all 33 recipes, stored contents, same-color rejection,
stacked materials, and rollback when the output cannot fit. Container interaction
screens and map drawing remain separate, unimplemented systems.
