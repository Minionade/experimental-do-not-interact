use orbital_voxel::vanilla::{ClientAssets, RegistryReports, ResourceId};
use sha1::{Digest, Sha1};
use std::path::PathBuf;
fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut jar = None;
    let mut reports = None;
    let mut pinned_reports = false;
    let mut audit = false;
    let mut audit_recipes = false;
    let mut craft_grid = None;
    let mut recipe_id = None;
    let mut inspections = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--minecraft-jar" => {
                jar = Some(PathBuf::from(
                    args.next().ok_or("--minecraft-jar needs a path")?,
                ))
            }
            "--reports" | "--pinned-reports" => {
                pinned_reports = arg == "--pinned-reports";
                reports = Some(PathBuf::from(
                    args.next().ok_or("--reports needs a directory")?,
                ))
            }
            "--audit-models" => audit = true,
            "--audit-recipes" => audit_recipes = true,
            "--craft-grid" => {
                craft_grid = Some(PathBuf::from(
                    args.next().ok_or("--craft-grid needs JSON path")?,
                ))
            }
            "--recipe" => recipe_id = Some(args.next().ok_or("--recipe needs ID")?),
            "--inspect-item" | "--inspect-block" | "--inspect-entity" | "--inspect-texture"
            | "--bake-model" => inspections.push((
                arg,
                ResourceId::parse(&args.next().ok_or("inspection needs an ID")?)?,
            )),
            "--help" | "-h" => {
                println!("vanilla-audit --minecraft-jar FILE.jar [--audit-models] [--inspect-item ID] [--inspect-block ID] [--inspect-entity ID] [--inspect-texture ID] [--bake-model MODEL_ID] [--reports DIR | --pinned-reports DIR] [--audit-recipes] [--craft-grid FILE.json --recipe ID]\nReads exact, hash-pinned Minecraft 26.3 resources. Does not launch the game or certify gameplay parity.");
                return Ok(());
            }
            _ => return Err(format!("unknown option {arg}; use --help")),
        }
    }
    let mut assets = ClientAssets::open(&jar.ok_or("--minecraft-jar is required")?)?;
    println!(
        "Verified official Minecraft 26.3 client. Asset file counts (not registry counts):\n{}",
        serde_json::to_string_pretty(&assets.counts).map_err(|e| e.to_string())?
    );
    if audit {
        let (total, special) = assets.audit_models()?;
        println!("Resolved {total} model inheritance chains; {special} require a built-in special renderer (not implemented).");
    }
    let report_directory = reports.clone();
    let reports = reports
        .map(|path| {
            if pinned_reports {
                RegistryReports::load_pinned(&path)
            } else {
                RegistryReports::load(&path)
            }
        })
        .transpose()?;
    if let Some(reports) = &reports {
        let registry = orbital_voxel::state_registry::StateRegistry::from_reports(reports)?;
        println!(
            "Loaded {} global states; fingerprint {}. {}",
            registry.len(),
            registry.fingerprint(),
            if pinned_reports {
                "Exact 26.3 report hashes verified."
            } else {
                "Report provenance not verified."
            }
        );
    }
    let items = if pinned_reports {
        Some(orbital_voxel::items::ItemCatalog::load_pinned(
            reports.as_ref().ok_or("missing reports")?,
            report_directory.as_ref().unwrap(),
        )?)
    } else {
        None
    };
    if let Some(items) = &items {
        println!(
            "Verified {} original item component definitions",
            items.len()
        );
    }
    if audit_recipes || craft_grid.is_some() {
        let items = items
            .as_ref()
            .ok_or("recipe operations require --pinned-reports")?;
        let tags = orbital_voxel::crafting::ItemTags::load(&mut assets, items)?;
        let book = orbital_voxel::crafting::RecipeBook::load(&mut assets, &tags, items)?;
        println!("Loaded {} item tags, {} ordinary crafting recipes, {} cooking recipes; {} specialized recipes retained but not executed",tags.len(),book.crafting.len(),book.cooking.len(),book.unsupported.len());
        if let Some(path) = craft_grid {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            if file.metadata().map_err(|e| e.to_string())?.len() > 1024 * 1024 {
                return Err("craft grid exceeds size limit".into());
            }
            let mut grid: orbital_voxel::crafting::CraftingGrid =
                serde_json::from_reader(file).map_err(|e| e.to_string())?;
            let recipe = recipe_id
                .or(book.find_crafting(&grid, items)?.map(str::to_string))
                .ok_or("no ordinary recipe matches grid")?;
            let mut inventory = orbital_voxel::items::Inventory::new(36)?;
            if !book.craft_into(&recipe, &mut grid, &mut inventory, items)? {
                return Err("recipe did not craft".into());
            }
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"recipe":recipe,"grid":grid,"inventory":inventory})
                )
                .map_err(|e| e.to_string())?
            );
        }
    }
    for (kind, id) in inspections {
        match kind.as_str() {
            "--bake-model" => {
                let model = assets.resolve_model(&id)?;
                let quads = orbital_voxel::vanilla_mesh::bake_model(&model)?;
                // Verify every referenced original PNG exists before reporting success.
                let textures: std::collections::BTreeSet<_> =
                    quads.iter().map(|q| q.texture.sprite.clone()).collect();
                for texture in &textures {
                    assets.texture_png(texture)?;
                }
                println!(
                    "Baked {}: {} quads, {} original textures",
                    id.key(),
                    quads.len(),
                    textures.len()
                );
                println!(
                    "{}",
                    serde_json::to_string_pretty(&quads).map_err(|e| e.to_string())?
                );
            }
            "--inspect-item" => {
                println!("{} = {}", id.key(), assets.display_name("item", &id)?);
                if let Some(items) = &items {
                    println!(
                        "Original item components: {}",
                        serde_json::to_string_pretty(
                            &items.get(&id.key()).ok_or("unregistered item")?.components
                        )
                        .map_err(|e| e.to_string())?
                    );
                }
                let definition = assets.item_definition(&id)?;
                println!(
                    "Item definition: {}",
                    serde_json::to_string_pretty(&definition).map_err(|e| e.to_string())?
                );
                if let Some(name) = definition["model"]["model"].as_str() {
                    let model = assets.resolve_model(&ResourceId::parse(name)?)?;
                    println!(
                        "Model textures: {:?}; built-in renderer: {:?}",
                        model.textures, model.builtin_parent
                    );
                    if model.textures.contains_key("layer0") {
                        let id = model.texture("#layer0")?;
                        let bytes = assets.texture_png(&id)?;
                        println!(
                            "Original {}: {} bytes, SHA-1 {:x}",
                            id.key(),
                            bytes.len(),
                            Sha1::digest(&bytes)
                        );
                    }
                }
                if let Some(r) = &reports {
                    println!(
                        "Registry item ID: {:?}",
                        r.protocol_id("minecraft:item", &id)
                    );
                }
            }
            "--inspect-block" => {
                println!("{} = {}", id.key(), assets.display_name("block", &id)?);
                println!(
                    "Blockstate: {}",
                    serde_json::to_string_pretty(&assets.blockstate(&id)?)
                        .map_err(|e| e.to_string())?
                );
                if let Some(r) = &reports {
                    println!(
                        "Default global block-state ID: {:?}",
                        r.default_block_state(&id)
                    );
                }
            }
            "--inspect-entity" => {
                println!("{} = {}", id.key(), assets.display_name("entity", &id)?)
            }
            "--inspect-texture" => {
                let bytes = assets.texture_png(&id)?;
                println!(
                    "Original {}: {} bytes, SHA-1 {:x}",
                    id.key(),
                    bytes.len(),
                    Sha1::digest(&bytes)
                );
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}
