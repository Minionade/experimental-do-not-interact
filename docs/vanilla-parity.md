# Minecraft 26.3 parity contract

## Required outcome

Base-game identities, visible names, textures, models, animations, sound associations, recipes, block states, item components, tool statistics, entity behavior, drops, interactions and game rules must match Minecraft Java 26.3. Do not invent substitutes, rename content, infer the roster from image filenames, or silently omit difficult systems.

The requested changes are cubic octree storage/LOD, spherical worlds with continuous traversal and radial gravity, and moving-structure physics that extends into space. Adapt coordinates at the engine boundary. Preserve content and behavior wherever those requested changes do not inherently require a different rule; additional deviations need explicit decisions and fixtures.

Colored spheres, hull IDs and free-flight controls are diagnostic fixtures, not proposed base-game replacements.

## Pinned reference

`config/minecraft-26.3.json` records hashes resolved from the official launcher manifest at https://piston-meta.mojang.com/mc/game/version_manifest_v2.json and fields inspected in the official client.

The client is 41,483,720 bytes with SHA-1 `e877b6a07acd633fb3bb475002175cec036e7b87`. Embedded fields: version `26.3`, world version `5023`, protocol `777`, resource pack `97.1`, data pack `121.0`. The server and external asset-index hashes are pinned too. The importer does not silently follow the latest release.

## Implemented

- Check client size, full archive SHA-1 and embedded version before import.
- Read original English translation strings, including the block-name fallback for block items; missing names are errors.
- Preserve original blockstate alternatives and multipart JSON, item-definition graphs, data JSON, model layers and unknown fields.
- Resolve model inheritance and texture aliases. Preserve sprite objects and their rendering metadata, including `force_translucent`.
- Identify special `builtin/*` model roots instead of silently converting them to ordinary cube meshes.
- Return original PNG bytes by exact resource location, including entity textures. No generated replacement textures or image compression.
- Parse generated `registries.json`, `blocks.json` and optional `items.json`; validate duplicate IDs and block roster consistency. Registry protocol IDs and global block-state IDs are distinct. Octree cells support 32-bit IDs.

The verified client contains 1,288 blockstate JSON files, 1,658 item-definition JSON files, 4,192 model JSON files, 3,964 PNG textures (743 under entity textures), and 9,883 `data/minecraft/` files. These are resource-file counts, **not registry sizes or implemented gameplay counts**.

## Inspect a local installation

```powershell
cargo run --locked --no-default-features --bin vanilla-audit -- --minecraft-jar "$env:APPDATA\.minecraft\versions\26.3\26.3.jar" --audit-models --inspect-item minecraft:diamond_pickaxe --inspect-block minecraft:stone --inspect-entity minecraft:zombie --inspect-texture minecraft:entity/zombie/zombie
```

Other launchers can supply their official client JAR path. Optional `--reports PATH` reads generated report files. The parser checks structure but does not certify their version/provenance. The importer is read-only; it does not modify the Minecraft installation or redistribute assets in the repository.

## Remaining gates

1. Generate authoritative registries/components from the pinned distribution and store checksums and provenance. Current report tests use constructed fixtures, not a verified official report snapshot.
2. Connect vanilla data to the renderer: state variants, multipart rules, item graphs, special renderers, model/UV baking, atlases, tint, animation metadata and entity geometry. The current desktop renderer does not consume `ClientAssets` yet.
3. Load hash-verified external asset-index objects for sound and additional languages; only client-JAR resources and English are currently read.
4. Implement gameplay behavior, including exact item/tool components and statistics. Resources alone do not contain all behavior or entity-model code.
5. Compare against the same 26.3 baseline using inventory, mining, durability, damage, recipe, drop, entity, block update, redstone, fluid, lighting and save fixtures. Separate approved planetary adaptations from unintended discrepancies.
6. Make missing content, incorrect names, texture mismatches and unsupported behaviors explicit failures. No unannounced placeholder fallback in parity mode.

Full parity is the acceptance target, not a claim about this foundation milestone.
