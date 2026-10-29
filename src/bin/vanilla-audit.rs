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
            "--inspect-item" | "--inspect-block" | "--inspect-entity" | "--inspect-texture"
            | "--bake-model" => inspections.push((
                arg,
                ResourceId::parse(&args.next().ok_or("inspection needs an ID")?)?,
            )),
            "--help" | "-h" => {
                println!("vanilla-audit --minecraft-jar FILE.jar [--audit-models] [--inspect-item ID] [--inspect-block ID] [--inspect-entity ID] [--inspect-texture ID] [--bake-model MODEL_ID] [--reports DIR | --pinned-reports DIR]\nReads exact, hash-pinned Minecraft 26.3 resources. Does not launch the game or certify gameplay parity.");
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
