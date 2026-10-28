# Experimental voxel engine

Rust engine foundation for a Minecraft-like game with **3D octree chunks, spherical planets, spaceflight, and native rigid-body physics**. The working crate name is `orbital-voxel`.

**This is a runnable engine prototype, not a port of Minecraft 26.3.** It does not contain Minecraft gameplay, assets, world loading, networking, or Java mod compatibility. The renderer uses engine-owned shaders; Iris shaderpack compatibility is outside the project requirements.

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

- [Architecture and octree decisions](docs/architecture.md)
- [Minecraft 26.3 parity and implementation roadmap](docs/roadmap.md)

The code and procedural visual assets in this repository are original project work. Third-party libraries are declared in Cargo manifests. Sable is a functional reference; no Sable source code is copied into this prototype.
