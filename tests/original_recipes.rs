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
