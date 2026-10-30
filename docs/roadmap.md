# Minecraft 26.3 target and milestones

The product goal is 1:1 Minecraft 26.3 content and behavior implemented in Rust, with extensive modification support, octree world storage, built-in structure physics, and spherical planets connected through space. This initial commit establishes engine components; it does not implement Minecraft 26.3 parity.

## Parity ledger

| Area | Current status | Required acceptance criteria |
| --- | --- | --- |
| Native application | Desktop prototype | Settings, input rebinding, accessibility, audio, UI and platform packaging |
| World storage | Cubic block octrees; separate spherical density octrees | Editable terrain, block-state schemas, streaming, persistence, crash-safe saves, migrations |
| World generation | Two original analytic spheres | 26.3 generation reference fixtures plus explicit spherical-generation deviations |
| Blocks and items | Verified vanilla resource importer; diagnostic scene IDs | Full versioned registries, state transitions, inventories, placement, mining, drops, tools |
| Survival | Absent | Health, hunger, damage, status effects, progression and game modes |
| Entities and AI | Rapier test bodies only | Entity registries, navigation under radial gravity, AI, spawning, interactions, animation |
| Recipes and content packs | TOML block-definition groundwork | Recipes, tags, loot, predicates, functions, commands and data-pack loading |
| Redstone | Absent | Scheduling, update ordering, block entities and reference-world behavioral tests |
| Fluids and lighting | Color-only terrain | Radial flow rules, block/fluid simulation, sky/block light, propagation across chunks |
| Dimensions | Two planets in one scene | Explicit dimension/planet rules, portals, generation, transitions and persistence |
| Physics structures | Compound rigid bodies | Live edits, split/merge, joints, riders, relative collision, constraints, mass changes |
| Space | Continuous radial gravity and bounded test orbit | Transfer flight, atmosphere, stable reference frames, celestial time and far rendering |
| Renderer | OpenGL terrain/final pipeline, culling | Textures, transparency, lighting, local adaptive LOD, upload budgets, profiling |
| Shaders | Engine-owned GLSL | Native materials, lighting and post-processing; no Iris compatibility requirement |
| Saves and resources | Octree JSON; pinned client resources and report parser | Tested import/export adapters for selected formats; source assets from user installation |
| Multiplayer | Absent | Versioned protocol, authoritative simulation, synchronization, prediction, authentication |
| Mods | Compiled-in event/command host | Stable runtime ABI, registries, serialization hooks, migrations, diagnostics, hot reload policy |

The official client, server and asset-index hashes are pinned in `config/minecraft-26.3.json`. Registry reports and behavior fixtures still need verified provenance before claiming compatibility. This repository contains no extracted Mojang game source or assets. Every subsystem needs explicit observable-behavior fixtures rather than assuming a Rust rewrite will preserve Java behavior.

## Next implementation order

1. **Vanilla content and editable octree world.** Connect the exact registries and original model/texture importer to the renderer, then implement ray queries, placing/breaking, dirty ancestor propagation, background mesh generation, saves and reloads. Acceptance: edit across all axes and chunk edges, unload/reload, preserve geometry and collision.
2. **Local adaptive LOD.** Balanced cuts and transition meshes, streaming priorities, budgets, material-aware coarse summaries. Acceptance: crack-free motion across each LOD boundary and no hidden loss of edited structures.
3. **Unified player physics.** Radial capsule controller, jumping, ship-relative movement and riding, stable transitions between planet and structure frames. Acceptance: walk around a planet, board a rotating ship, take off and land without teleporting or resetting velocity.
4. **Native rendering.** Indexed meshes, asynchronous uploads, textures/materials, transparency, atmosphere and lighting. Establish CPU/GPU frame budgets and compare representative near-surface, ship and space scenes.
5. **Gameplay loop.** Inventory, mining, placing, items, crafting, lighting, fluids and initial entities. Keep content registries separate from engine behavior and expose tick-safe mod hooks.
6. **26.3 parity expansion.** Build the ledger above into versioned fixtures and implement each system against them. Spherical-world deviations must be documented instead of labeled vanilla-compatible.
7. **Large worlds and multiplayer.** Region migration, deterministic snapshots where practical, authoritative simulation, streaming replication and scalability tests.

These are successive development milestones, not an estimate that the entire game can be ported in one session. No completion date or performance multiplier is asserted.

## Decisions already made

- All terrain chunks are 3D cubes; no fixed-height column chunk design.
- Terrain and moving structures use octree spatial layouts.
- Planets are geometric spheres with continuous radial gravity, not six gravity faces.
- Physics is an engine subsystem and continues into space.
- The renderer uses native shaders; Iris shaderpack compatibility is not a requirement.
- Rust modding interfaces are explicit and versioned; Java mod compatibility is a distinct project.

Base-game content must use the exact Minecraft 26.3 roster, identities, names, textures and behavior. Diagnostic hulls and colored spheres are not proposed replacement content.
