use orbital_voxel::{
    crafting::{ItemTags, RecipeBook},
    items::{Inventory, ItemCatalog},
    vanilla::{ClientAssets, RegistryReports},
};
use std::path::PathBuf;
#[test]
#[ignore = "requires original hash-pinned client JAR and reports"]
fn every_original_stonecutting_recipe_executes_atomically() {
    let directory =
        PathBuf::from(std::env::var("MINECRAFT_REPORTS").expect("set MINECRAFT_REPORTS"));
    let jar =
        PathBuf::from(std::env::var("MINECRAFT_CLIENT_JAR").expect("set MINECRAFT_CLIENT_JAR"));
    let reports = RegistryReports::load_pinned(&directory).unwrap();
    let items = ItemCatalog::load_pinned(&reports, &directory).unwrap();
    let mut assets = ClientAssets::open(&jar).unwrap();
    let tags = ItemTags::load(&mut assets, &items).unwrap();
    let book = RecipeBook::load(&mut assets, &tags, &items).unwrap();
    assert_eq!(book.stonecutting.len(), 351);
    for (id, recipe) in &book.stonecutting {
        let raw = &book.raw[id];
        let expected_item = raw["result"]["id"].as_str().unwrap();
        let expected_count = raw["result"]["count"].as_u64().unwrap_or(1) as u32;
        let input_id = items
            .ids()
            .filter(|id| *id != "minecraft:air")
            .find(|id| recipe.accepts(&items.stack(id, 1).unwrap()))
            .expect("recipe input");
        let mut input = Some(items.stack(input_id, 1).unwrap());
        let mut inventory = Inventory::new(1).unwrap();
        assert!(book
            .find_stonecutting(input.as_ref().unwrap(), &items)
            .unwrap()
            .contains(&id.as_str()));
        assert!(
            book.stonecut_into(id, &mut input, &mut inventory, &items)
                .unwrap(),
            "{id}"
        );
        assert!(input.is_none(), "{id}");
        let output = inventory.slots()[0].as_ref().unwrap();
        assert_eq!(output.item, expected_item, "{id}");
        assert_eq!(output.count, expected_count, "{id}");
        let mut input = Some(items.stack(input_id, 1).unwrap());
        let mut inventory = Inventory::new(1).unwrap();
        inventory
            .set(0, Some(items.stack("bedrock", 64).unwrap()), &items)
            .unwrap();
        let before = inventory.clone();
        assert!(
            !book
                .stonecut_into(id, &mut input, &mut inventory, &items)
                .unwrap(),
            "{id}"
        );
        assert_eq!(inventory, before, "{id}");
        assert_eq!(input.as_ref().unwrap().count, 1, "{id}");
    }
}

#[test]
#[ignore = "requires original hash-pinned client JAR and reports"]
fn all_original_smithing_transforms_preserve_base_changes() {
    use orbital_voxel::smithing::SmithingInput;
    use serde_json::json;
    let directory =
        PathBuf::from(std::env::var("MINECRAFT_REPORTS").expect("set MINECRAFT_REPORTS"));
    let jar =
        PathBuf::from(std::env::var("MINECRAFT_CLIENT_JAR").expect("set MINECRAFT_CLIENT_JAR"));
    let reports = RegistryReports::load_pinned(&directory).unwrap();
    let items = ItemCatalog::load_pinned(&reports, &directory).unwrap();
    let mut assets = ClientAssets::open(&jar).unwrap();
    let tags = ItemTags::load(&mut assets, &items).unwrap();
    let book = RecipeBook::load(&mut assets, &tags, &items).unwrap();
    assert_eq!(book.smithing.len(), 12);
    for (id, recipe) in &book.smithing {
        let raw = &book.raw[id];
        let mut base = items.stack(raw["base"].as_str().unwrap(), 1).unwrap();
        base.patch
            .insert("minecraft:custom_name".into(), Some(json!("Kept name")));
        base.patch
            .insert("minecraft:custom_data".into(), Some(json!({"test":123})));
        base.patch.insert("minecraft:repair_cost".into(), None);
        if base.component(&items, "minecraft:max_damage").is_some() {
            base.patch
                .insert("minecraft:damage".into(), Some(json!(123)));
        }
        let expected_patch = base.patch.clone();
        if let Some(default) = base.component(&items, "minecraft:max_damage").cloned() {
            base.patch
                .insert("minecraft:max_damage".into(), Some(default));
        }
        let mut input = SmithingInput {
            template: Some(items.stack(raw["template"].as_str().unwrap(), 2).unwrap()),
            base: Some(base),
            addition: Some(items.stack("netherite_ingot", 2).unwrap()),
        };
        let preview = recipe.preview(&input, &items).unwrap().unwrap();
        assert_eq!(preview.item, raw["result"]["id"].as_str().unwrap());
        assert_eq!(preview.patch, expected_patch);
        assert_eq!(
            preview.component(&items, "minecraft:max_damage"),
            items
                .get(&preview.item)
                .unwrap()
                .components
                .get("minecraft:max_damage")
        );
        let mut inventory = Inventory::new(1).unwrap();
        inventory
            .set(0, Some(items.stack("bedrock", 64).unwrap()), &items)
            .unwrap();
        let before_input = input.clone();
        let before_inventory = inventory.clone();
        assert!(
            !recipe
                .craft_into(&mut input, &mut inventory, &items)
                .unwrap(),
            "{id}"
        );
        assert_eq!(input, before_input);
        assert_eq!(inventory, before_inventory);
        inventory.remove(0, 64).unwrap();
        assert!(
            recipe
                .craft_into(&mut input, &mut inventory, &items)
                .unwrap(),
            "{id}"
        );
        assert_eq!(inventory.slots()[0], Some(preview));
        assert!(input.base.is_none());
        assert_eq!(input.template.as_ref().unwrap().count, 1);
        assert_eq!(input.addition.as_ref().unwrap().count, 1);
        let mut wrong = before_input;
        wrong.addition = Some(items.stack("iron_ingot", 1).unwrap());
        assert!(recipe.preview(&wrong, &items).unwrap().is_none());
    }
}

#[test]
#[ignore = "requires original hash-pinned client JAR and reports"]
fn all_transmute_recipes_preserve_contents_and_clone_map_slots() {
    use orbital_voxel::crafting::CraftingGrid;
    use serde_json::json;
    let directory =
        PathBuf::from(std::env::var("MINECRAFT_REPORTS").expect("set MINECRAFT_REPORTS"));
    let jar =
        PathBuf::from(std::env::var("MINECRAFT_CLIENT_JAR").expect("set MINECRAFT_CLIENT_JAR"));
    let reports = RegistryReports::load_pinned(&directory).unwrap();
    let items = ItemCatalog::load_pinned(&reports, &directory).unwrap();
    let mut assets = ClientAssets::open(&jar).unwrap();
    let tags = ItemTags::load(&mut assets, &items).unwrap();
    let book = RecipeBook::load(&mut assets, &tags, &items).unwrap();
    assert_eq!(book.transmute.len(), 33);
    for (id, recipe) in &book.transmute {
        let raw = &book.raw[id];
        let map = id == "minecraft:map_cloning";
        let bundle = id.ends_with("_bundle");
        let base_id = if map {
            "filled_map"
        } else if bundle {
            "bundle"
        } else {
            "shulker_box"
        };
        let mut base = items.stack(base_id, if map { 3 } else { 1 }).unwrap();
        base.patch
            .insert("minecraft:custom_name".into(), Some(json!("Contents stay")));
        let (key, value) = if map {
            ("minecraft:map_id", json!(42))
        } else if bundle {
            (
                "minecraft:bundle_contents",
                json!([{"id":"minecraft:diamond","count":7}]),
            )
        } else {
            (
                "minecraft:container",
                json!([{"slot":5,"item":{"id":"minecraft:diamond","count":7}}]),
            )
        };
        base.patch.insert(key.into(), Some(value));
        let mut grid = CraftingGrid {
            width: 3,
            height: 3,
            slots: vec![None; 9],
        };
        grid.slots[0] = Some(base.clone());
        grid.slots[8] = Some(items.stack(raw["material"].as_str().unwrap(), 64).unwrap());
        if map {
            grid.slots[4] = Some(items.stack("map", 12).unwrap());
        }
        let original = grid.clone();
        let (_, preview) = recipe.preview(&grid, &items).unwrap().unwrap();
        assert_eq!(preview.patch, base.patch, "{id}");
        assert_eq!(preview.count, if map { 3 } else { 1 }, "{id}");
        assert_eq!(
            preview.item,
            if map {
                "minecraft:filled_map"
            } else {
                raw["result"]["id"].as_str().unwrap()
            }
        );
        assert_eq!(
            book.find_crafting(&grid, &items).unwrap(),
            Some(id.as_str())
        );
        let mut inventory = Inventory::new(1).unwrap();
        inventory
            .set(0, Some(items.stack("bedrock", 64).unwrap()), &items)
            .unwrap();
        let before = inventory.clone();
        assert!(!book
            .craft_into(id, &mut grid, &mut inventory, &items)
            .unwrap());
        assert_eq!(grid, original);
        assert_eq!(inventory, before);
        inventory.remove(0, 64).unwrap();
        assert!(book
            .craft_into(id, &mut grid, &mut inventory, &items)
            .unwrap());
        assert_eq!(inventory.slots()[0], Some(preview.clone()));
        assert_eq!(grid.slots[8].as_ref().unwrap().count, 63);
        if map {
            assert_eq!(grid.slots[0].as_ref().unwrap().count, 2);
            assert_eq!(grid.slots[4].as_ref().unwrap().count, 11);
        } else {
            assert!(grid.slots[0].is_none());
            let mut same_color = original;
            same_color.slots[0] = Some(preview);
            assert!(recipe.preview(&same_color, &items).unwrap().is_none());
        }
    }
    let map = &book.transmute["minecraft:map_cloning"];
    let mut grid = CraftingGrid {
        width: 3,
        height: 3,
        slots: vec![Some(items.stack("map", 64).unwrap()); 9],
    };
    grid.slots[0] = Some(items.stack("filled_map", 1).unwrap());
    assert_eq!(map.preview(&grid, &items).unwrap().unwrap().1.count, 9);
    grid.slots[1] = Some(items.stack("filled_map", 1).unwrap());
    assert!(map.preview(&grid, &items).unwrap().is_none());
}
