# Architecture

## World space and octrees

Planet geometry is embedded in ordinary 3D Euclidean space. The terrain scalar field is currently `length(position - center) - radius`. A walk around the sphere returns to its start because of the geometry; no coordinate teleport or periodic flat-world wrapping occurs. Gravity follows the same center and is continuous at all octree boundaries.

Two leaf payloads serve different content:

1. `BlockOctree` stores editable block material IDs. A node is a uniform material, eight child nodes, or a dense 16³ brick. The root extent is a power of two, from 16 through 1,048,576 cells. Writes subdivide only the modified path and collapse homogeneous ancestors. Structure saves retain this hierarchy.
2. `TerrainOctree` stores a procedural signed-distance surface. Empty and solid nodes require no meshes. Surface leaves contain a 17³ sample lattice for 16³ cells. Nodes share `Bounds` and `NodeKey` addressing with the other spatial code. Whole terrain snapshots are currently regenerated from planet parameters rather than saved or edited.

This is a forest of planet/structure octrees, not yet an unbounded universe octree. There is no central spatial index over every planet or moving object. The current two-planet scene uses a short list of octree roots. A loose universe octree for astronomical body lookup is a later layer; moving structures retain their own local block coordinates when their transforms move.

Trees are used for world storage and terrain detail selection. Rapier retains its own specialized collision broadphase. Replacing a physics solver's broadphase solely to make every subsystem use octrees is not a prerequisite for octree terrain.

## LOD

A node identifies a cubic region. Its parent covers twice its side length and eight times its volume. Refining a node gives eight subregions. In the density terrain, coarse values are resampled from the same analytic field, rather than incorrectly averaging categorical block IDs.

The initial renderer uses a uniform-depth cut of each planet's tree. Distance selects depth 1–3 with hysteresis. This deliberately avoids mixed-depth boundaries. Surface chunks within a cut have matching boundary samples and a consistent tetrahedral decomposition; a test counts shared mesh edges across the entire sphere.

Local adaptive LOD needs all of the following before enabling it:

- A screen-space geometric error metric and explicit node summaries.
- A balanced cut (neighbor depth difference at most one).
- Transition meshing, such as Transvoxel or a compatible dual-contouring scheme. Changing meshing algorithm must preserve edit and collision semantics.
- Conservative aggregate bounds and dirty propagation after edits.
- Cancellation/versioning for mesh jobs, per-frame upload budgets, and mesh residency/eviction.
- A collision cut independent of the visual cut, with controlled activation around players and ships.

Octrees simplify selection, address calculation, culling, and residency. They do not remove the need to stitch different resolutions or define how small structures survive coarsening.

## Renderer

The implemented backend is OpenGL 3.3 core via SDL2 + glow. The built-in terrain and post-processing shaders are owned by this engine. Mesh generation uses Rayon. Each occupied surface chunk uploads a static vertex buffer. Mesh draws use conservative bounding-sphere frustum tests. Geometry has local `f32` vertices; world positions and camera subtraction stay `f64` until the relative transform is constructed.

This is an initial renderer, not a measured replacement for Sodium or Minecraft's renderer. It has no GPU-driven indirect draws, meshlet or hierarchical-Z culling, texture atlases, dynamic light propagation, asynchronous uploads, or local LOD stitching. Mesh buffers currently duplicate triangle vertices. Full planet LOD rebuilds are synchronous and may stall.

The initial projection uses a 0.05–20,000 m depth range. Floating-origin transforms reduce position jitter, but a single depth range is not sufficient for an astronomical renderer. A later renderer needs reversed-Z and/or partitioned depth ranges for terrain, ships, and celestial bodies. Vulkan or wgpu can become another backend when profiling and renderer scale justify the change; there is no Iris compatibility constraint.

## Physics and space

`Physics` owns Rapier double-precision bodies, colliders, joints, and collision-pipeline state. Planet colliders are analytic spheres. Ships assemble from occupied local octree blocks into compound cuboids. Contacts are genuine Rapier contacts. The demo has no joints, structural splitting, buoyancy, aerodynamics, or moving-platform character controller yet.

Fixed simulation steps are 1/120 second. Real-frame accumulation is capped at 0.1 second; excess time is recorded in `dropped_seconds`. Gravity is applied with half velocity updates before and after Rapier's drift/contact solve. This reduces the secular energy drift observed when holding one sampled gravity force over internal solver substeps.

Outside a planet, gravity is `-mu * delta / |delta|³`, with `mu = surface_gravity * radius²`. Inside, it uses a uniform-density model and approaches zero at the center. Contributions from all planets are summed. Planet centers are fixed in this prototype; it is not an N-body celestial simulator.

Physics currently uses one `f64` inertial frame. Large-world precision tests cover coordinate handling, not a production astronomical scale guarantee. Per-region physics islands with origin rebasing and explicit frame migration are needed at larger scales. Frame migration must retain linear velocity, angular velocity, contacts, and rider state; it must not reset a ship when it crosses a planet boundary.

## Movement

The initial walker follows exact great-circle steps and parallel-transports its forward vector. It has no pole singularity and no six-face gravity switches. It is a mathematical surface controller, separate from Rapier. It does not yet walk inside structures, jump, collide with block geometry, or move between planets automatically. Free flight and a physical ship provide the initial surface/space inspection modes.

## Vanilla resource boundary

`ClientAssets` reads the pinned official 26.3 JAR in place. Names come from the original English translation table, blockstate and item-definition JSON remain intact, model parents and texture aliases resolve from original data, and PNG byte sequences are returned unchanged. Sprite objects retain metadata such as `force_translucent`. Built-in generated/entity model roots remain explicit renderer requirements.

`RegistryReports` parses generated registry and block-state reports without deriving IDs from texture filenames. Protocol IDs, global block-state IDs and namespaced identities are distinct. Octree cells use 32-bit IDs. Schema validation does not establish report provenance or version; a verified report-generation harness is still needed.

The importer is separate from the diagnostic renderer. Vanilla atlases, model baking, entity animation, behavior and gameplay are not yet implemented. Additional languages and sounds in external asset-index objects are also pending. Exact content and gameplay parity is the acceptance target; see `vanilla-parity.md`.

## Modding

The TOML registry checks namespace ownership, duplicate registrations, supported API version, finite positive densities, and color ranges; a failed load changes nothing. It is not yet the authoritative material table for the desktop demo. Numeric IDs and persistent registry remapping require a stable save contract before arbitrary content becomes editable in game.

`NativeMod` receives structure-created and fixed-tick events and emits typed commands. Physics dispatches these events and applies finite acceleration commands at tick boundaries. See `examples/flight_mod.rs`. These mods are compiled with the game and trusted; this is not dynamic-library discovery, a stable binary ABI, or a sandbox.

Future runtime extensions should use a versioned Wasm interface or another explicit stable host ABI with capability-limited commands, execution budgets, transactional registry loading, and save migrations. Existing Forge/Fabric/NeoForge Java mods cannot load into this Rust host without a separate compatibility system.

## References checked

- Minecraft 26.3 release baseline: https://www.minecraft.net/en-us/article/minecraft-java-edition-26-3
- Sable's repository describes its Rapier/Rust physics integration: https://github.com/ryanhcode/sable
- Rapier rigid-body concepts: https://rapier.rs/docs/user_guides/rust/rigid_bodies/

Implementation uses the exact Rapier 0.22 and glow 0.16 APIs resolved in Cargo.lock; current online examples may describe newer versions.
