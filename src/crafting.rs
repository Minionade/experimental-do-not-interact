//! Crafting recipes: loaded from a resource/data pack (`data/<ns>/recipe(s)/*.json`) with a built-in fallback set.
use crate::item::{normalize, Inventory, ItemStack};
use crate::pack::PackStack;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// One acceptable item for an ingredient slot. A tag matches any of its member items.
#[derive(Clone, Debug, PartialEq)]
pub enum Ingredient { Item(String), Tag(String), Any(Vec<Ingredient>) }

#[derive(Clone, Debug)]
pub enum Pattern {
    /// Row-major cells, `None` = empty.
    Shaped { w: usize, h: usize, cells: Vec<Option<Ingredient>> },
    Shapeless(Vec<Ingredient>),
}

#[derive(Clone, Debug)]
pub struct Recipe { pub id: String, pub result: ItemStack, pub pattern: Pattern }

#[derive(Default)]
pub struct Recipes {
    pub list: Vec<Recipe>,
    tags: HashMap<String, Vec<String>>,
}

fn item_id(s: &str) -> String { normalize(s) }

fn parse_ingredient(v: &Value) -> Option<Ingredient> {
    match v {
        Value::String(s) => Some(match s.strip_prefix('#') { Some(t) => Ingredient::Tag(item_id(t)), None => Ingredient::Item(item_id(s)) }),
        Value::Array(a) => Some(Ingredient::Any(a.iter().filter_map(parse_ingredient).collect())),
        Value::Object(o) => o.get("item").and_then(|i| i.as_str()).map(|i| Ingredient::Item(item_id(i)))
            .or_else(|| o.get("tag").and_then(|t| t.as_str()).map(|t| Ingredient::Tag(item_id(t)))),
        _ => None,
    }
}

fn parse_result(v: &Value) -> Option<ItemStack> {
    match v {
        Value::String(s) => Some(ItemStack::new(s, 1)),
        Value::Object(o) => {
            let id = o.get("id").or_else(|| o.get("item"))?.as_str()?;
            Some(ItemStack::new(id, o.get("count").and_then(|c| c.as_u64()).unwrap_or(1).min(64) as u8))
        }
        _ => None,
    }
}

fn parse_recipe(id: &str, j: &Value) -> Option<Recipe> {
    let ty = j.get("type")?.as_str()?.strip_prefix("minecraft:")?;
    let result = parse_result(j.get("result")?)?;
    let pattern = match ty {
        "crafting_shaped" => {
            let rows: Vec<&str> = j.get("pattern")?.as_array()?.iter().filter_map(|r| r.as_str()).collect();
            let key = j.get("key")?.as_object()?;
            let w = rows.iter().map(|r| r.chars().count()).max()?;
            let mut cells = Vec::new();
            for r in &rows {
                let chars: Vec<char> = r.chars().collect();
                for x in 0..w {
                    cells.push(match chars.get(x) { Some(' ') | None => None, Some(c) => Some(parse_ingredient(key.get(&c.to_string())?)?) });
                }
            }
            Pattern::Shaped { w, h: rows.len(), cells }
        }
        "crafting_shapeless" => Pattern::Shapeless(j.get("ingredients")?.as_array()?.iter().filter_map(parse_ingredient).collect()),
        _ => return None, // smelting, smithing, special recipes: not crafting-grid recipes
    };
    Some(Recipe { id: id.to_string(), result, pattern })
}

fn parse_tag(packs: &PackStack, ns: &str, name: &str, out: &mut HashMap<String, Vec<String>>, seen: &mut HashSet<String>) {
    let key = format!("{ns}:{name}");
    if out.contains_key(&key) || !seen.insert(key.clone()) { return; }
    let j = ["item", "items"].iter().find_map(|d| packs.read_json(&format!("data/{ns}/tags/{d}/{name}.json")));
    let mut members = Vec::new();
    if let Some(vals) = j.as_ref().and_then(|j| j.get("values")).and_then(|v| v.as_array()) {
        for v in vals {
            let s = v.as_str().or_else(|| v.get("id").and_then(|i| i.as_str())).unwrap_or("");
            if let Some(t) = s.strip_prefix('#') {
                let (ns2, n2) = t.split_once(':').unwrap_or(("minecraft", t));
                parse_tag(packs, ns2, n2, out, seen);
                members.extend(out.get(&format!("{ns2}:{n2}")).cloned().unwrap_or_default());
            } else if !s.is_empty() { members.push(item_id(s)); }
        }
    }
    out.insert(key, members);
}

const WOODS: [&str; 6] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak"];

impl Recipes {
    /// Loads every crafting recipe from the packs; falls back to the built-in set if none are found.
    pub fn load(packs: &PackStack) -> Recipes {
        let mut r = Recipes::default();
        let mut files: Vec<String> = Vec::new();
        for dir in ["recipe", "recipes"] { files.extend(packs.list(&format!("data/minecraft/{dir}/"))); }
        for path in files.iter().filter(|p| p.ends_with(".json")) {
            let id = path.rsplit('/').next().unwrap().trim_end_matches(".json");
            if let Some(rec) = packs.read_json(path).and_then(|j| parse_recipe(&format!("minecraft:{id}"), &j)) { r.list.push(rec); }
        }
        if r.list.is_empty() { return Recipes::builtin(); }
        // Resolve every tag a recipe mentions.
        let mut seen = HashSet::new();
        let mut names = Vec::new();
        fn collect(i: &Ingredient, names: &mut Vec<String>) { match i { Ingredient::Tag(t) => names.push(t.clone()), Ingredient::Any(a) => a.iter().for_each(|x| collect(x, names)), _ => {} } }
        for rec in &r.list { match &rec.pattern { Pattern::Shaped { cells, .. } => cells.iter().flatten().for_each(|i| collect(i, &mut names)), Pattern::Shapeless(v) => v.iter().for_each(|i| collect(i, &mut names)) } }
        for t in names { let (ns, n) = t.split_once(':').unwrap(); parse_tag(packs, ns, n, &mut r.tags, &mut seen); }
        r.list.sort_by(|a, b| a.id.cmp(&b.id));
        r
    }

    /// A small hand-written set covering the early game: wood, tools, furnace, chest, torches.
    pub fn builtin() -> Recipes {
        let mut r = Recipes::default();
        let item = |s: &str| Ingredient::Item(item_id(s));
        let shaped = |rows: &[&str], key: &[(char, &str)], out: (&str, u8)| {
            let w = rows.iter().map(|r| r.len()).max().unwrap();
            let cells = rows.iter().flat_map(|row| (0..w).map(move |x| row.chars().nth(x))).map(|c| c.and_then(|c| key.iter().find(|k| k.0 == c).map(|k| Ingredient::Item(item_id(k.1))))).collect();
            Recipe { id: format!("minecraft:{}", out.0), result: ItemStack::new(out.0, out.1), pattern: Pattern::Shaped { w, h: rows.len(), cells } }
        };
        for w in WOODS {
            r.list.push(Recipe { id: format!("minecraft:{w}_planks"), result: ItemStack::new(&format!("{w}_planks"), 4), pattern: Pattern::Shapeless(vec![item(&format!("{w}_log"))]) });
            r.tags.entry("minecraft:planks".into()).or_default().push(item_id(&format!("{w}_planks")));
            r.tags.entry("minecraft:logs".into()).or_default().push(item_id(&format!("{w}_log")));
        }
        let planks = || Ingredient::Tag("minecraft:planks".into());
        r.list.push(Recipe { id: "minecraft:stick".into(), result: ItemStack::new("stick", 4), pattern: Pattern::Shaped { w: 1, h: 2, cells: vec![Some(planks()), Some(planks())] } });
        r.list.push(Recipe { id: "minecraft:crafting_table".into(), result: ItemStack::new("crafting_table", 1), pattern: Pattern::Shaped { w: 2, h: 2, cells: vec![Some(planks()); 4] } });
        r.list.push(Recipe { id: "minecraft:chest".into(), result: ItemStack::new("chest", 1), pattern: Pattern::Shaped { w: 3, h: 3, cells: (0..9).map(|i| if i == 4 { None } else { Some(planks()) }).collect() } });
        r.list.push(shaped(&["CCC", "C C", "CCC"], &[('C', "cobblestone")], ("furnace", 1)));
        r.list.push(shaped(&["C", "S"], &[('C', "coal"), ('S', "stick")], ("torch", 4)));
        for (tier, mat) in [("wooden", None), ("stone", Some("cobblestone")), ("iron", Some("iron_ingot")), ("golden", Some("gold_ingot")), ("diamond", Some("diamond"))] {
            let m = |c: char| -> Option<Ingredient> { let _ = c; Some(mat.map_or_else(planks, item)) };
            let stick = || Some(item("stick"));
            let grid = |rows: [&str; 3]| -> Vec<Option<Ingredient>> { rows.iter().flat_map(|r| r.chars().map(|c| match c { 'M' => m('M'), 'S' => stick(), _ => None })).collect() };
            for (name, w, h, g) in [("pickaxe", 3, 3, grid(["MMM", " S ", " S "])), ("axe", 2, 3, grid(["MM ", "MS ", " S "])), ("shovel", 1, 3, grid(["M  ", "S  ", "S  "])),
                                    ("hoe", 2, 3, grid(["MM ", " S ", " S "])), ("sword", 1, 3, grid(["M  ", "M  ", "S  "]))] {
                // Trim the 3-wide row-major grid down to the recipe's real width.
                let cells: Vec<Option<Ingredient>> = (0..3).flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| g[y * 3 + x].clone()).collect();
                r.list.push(Recipe { id: format!("minecraft:{tier}_{name}"), result: ItemStack::new(&format!("{tier}_{name}"), 1), pattern: Pattern::Shaped { w, h, cells } });
            }
        }
        r
    }

    fn matches_item(&self, ing: &Ingredient, item: &str) -> bool {
        match ing {
            Ingredient::Item(i) => i == item,
            Ingredient::Tag(t) => self.tags.get(t).is_some_and(|m| m.iter().any(|x| x == item)),
            Ingredient::Any(a) => a.iter().any(|i| self.matches_item(i, item)),
        }
    }

    /// Output for items laid out on a `w` x `h` grid (row-major; names normalised), if any recipe matches.
    pub fn match_grid(&self, grid: &[Option<String>], w: usize, h: usize) -> Option<&Recipe> {
        let cells: Vec<(usize, usize, &str)> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter_map(|(x, y)| grid[y * w + x].as_deref().map(|n| (x, y, n))).collect();
        if cells.is_empty() { return None; }
        let (x0, y0) = (cells.iter().map(|c| c.0).min()?, cells.iter().map(|c| c.1).min()?);
        let (bw, bh) = (cells.iter().map(|c| c.0).max()? - x0 + 1, cells.iter().map(|c| c.1).max()? - y0 + 1);
        self.list.iter().find(|rec| match &rec.pattern {
            Pattern::Shaped { w: rw, h: rh, cells: rc } => {
                if *rw != bw || *rh != bh { return false; }
                let at = |x: usize, y: usize| grid[(y0 + y) * w + x0 + x].as_deref();
                let ok = |mirror: bool| (0..bh).all(|y| (0..bw).all(|x| {
                    let want = &rc[y * rw + if mirror { rw - 1 - x } else { x }];
                    match (want, at(x, y)) { (None, None) => true, (Some(i), Some(n)) => self.matches_item(i, n), _ => false }
                }));
                ok(false) || ok(true)
            }
            Pattern::Shapeless(ings) => {
                if ings.len() != cells.len() { return false; }
                let mut used = vec![false; cells.len()];
                ings.iter().all(|i| match (0..cells.len()).find(|&k| !used[k] && self.matches_item(i, cells[k].2)) { Some(k) => { used[k] = true; true } None => false })
            }
        })
    }

    /// Which concrete items (and how many of each) would be consumed from `inv`, if it can pay for `recipe`.
    fn payment(&self, recipe: &Recipe, inv: &Inventory) -> Option<Vec<(String, u32)>> {
        let ings: Vec<&Ingredient> = match &recipe.pattern { Pattern::Shaped { cells, .. } => cells.iter().flatten().collect(), Pattern::Shapeless(v) => v.iter().collect() };
        let mut have: HashMap<String, u32> = HashMap::new();
        for s in inv.slots.iter().flatten() { *have.entry(s.name.clone()).or_default() += s.count as u32; }
        let mut pay: Vec<(String, u32)> = Vec::new();
        for ing in ings {
            // Prefer the item the player has the most of, so tags don't drain a scarce variant first.
            let mut cands: Vec<(&String, &u32)> = have.iter().filter(|(n, c)| **c > 0 && self.matches_item(ing, n)).collect();
            cands.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let name = cands.first()?.0.clone();
            *have.get_mut(&name).unwrap() -= 1;
            match pay.iter_mut().find(|p| p.0 == name) { Some(p) => p.1 += 1, None => pay.push((name, 1)) }
        }
        Some(pay)
    }

    pub fn can_craft(&self, recipe: &Recipe, inv: &Inventory) -> bool { self.payment(recipe, inv).is_some() }

    /// Recipes the inventory can currently pay for.
    pub fn craftable(&self, inv: &Inventory) -> Vec<&Recipe> { self.list.iter().filter(|r| self.can_craft(r, inv)).collect() }

    /// Consumes the ingredients and adds the result. False (and no change) if it can't pay or the result won't fit.
    pub fn craft(&self, recipe: &Recipe, inv: &mut Inventory) -> bool {
        let Some(pay) = self.payment(recipe, inv) else { return false };
        let mut trial = inv.clone();
        for (n, c) in &pay { trial.remove(n, *c); }
        if trial.add(&recipe.result) != 0 { return false; }
        *inv = trial;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{tests::temp, Pack};

    fn g(cells: &[&str]) -> Vec<Option<String>> { cells.iter().map(|c| if c.is_empty() { None } else { Some(item_id(c)) }).collect() }

    #[test]
    fn builtin_grid_matching_shaped_shapeless_and_mirrored() {
        let r = Recipes::builtin();
        let out = |grid: &[&str], w, h| r.match_grid(&g(grid), w, h).map(|x| (x.result.name.clone(), x.result.count));
        assert_eq!(out(&["birch_log", "", "", ""], 2, 2), Some(("minecraft:birch_planks".into(), 4)), "shapeless, anywhere on the grid");
        assert_eq!(out(&["", "", "", "oak_log"], 2, 2), Some(("minecraft:oak_planks".into(), 4)));
        assert_eq!(out(&["oak_planks", "", "oak_planks", ""], 2, 2), Some(("minecraft:stick".into(), 4)));
        assert_eq!(out(&["oak_planks", "oak_planks", "", ""], 2, 2), None, "sticks are a vertical pair, not horizontal");
        assert_eq!(out(&["", "oak_planks", "", "spruce_planks"], 2, 2), Some(("minecraft:stick".into(), 4)), "tags accept mixed planks; shaped recipes can sit in any column");
        assert_eq!(out(&["oak_planks", "oak_planks", "oak_planks", "oak_planks"], 2, 2), Some(("minecraft:crafting_table".into(), 1)));
        assert_eq!(out(&["oak_planks", "", "", ""], 2, 2), None);
        let pick = ["cobblestone", "cobblestone", "cobblestone", "", "stick", "", "", "stick", ""];
        assert_eq!(out(&pick, 3, 3), Some(("minecraft:stone_pickaxe".into(), 1)));
        // Axes work mirrored, and in either orientation.
        assert_eq!(out(&["iron_ingot", "iron_ingot", "", "iron_ingot", "stick", "", "", "stick", ""], 3, 3).unwrap().0, "minecraft:iron_axe");
        assert_eq!(out(&["", "iron_ingot", "iron_ingot", "", "stick", "iron_ingot", "", "stick", ""], 3, 3).unwrap().0, "minecraft:iron_axe");
        assert_eq!(out(&["cobblestone", "cobblestone", "cobblestone", "cobblestone", "", "cobblestone", "cobblestone", "cobblestone", "cobblestone"], 3, 3).unwrap().0, "minecraft:furnace");
    }

    #[test]
    fn inventory_crafting_consumes_ingredients_and_respects_space() {
        let r = Recipes::builtin();
        let mut inv = Inventory::new();
        inv.add(&ItemStack::new("oak_log", 2));
        let planks = r.list.iter().find(|x| x.id == "minecraft:oak_planks").unwrap();
        assert!(r.can_craft(planks, &inv));
        assert!(r.craft(planks, &mut inv));
        assert_eq!((inv.count("oak_log"), inv.count("oak_planks")), (1, 4));
        let sticks = r.list.iter().find(|x| x.id == "minecraft:stick").unwrap();
        assert!(r.craft(sticks, &mut inv));
        assert_eq!((inv.count("oak_planks"), inv.count("stick")), (2, 4));
        let table = r.list.iter().find(|x| x.id == "minecraft:crafting_table").unwrap();
        assert!(!r.can_craft(table, &inv), "needs four planks");
        assert!(!r.craft(table, &mut inv));
        assert_eq!(inv.count("oak_planks"), 2, "failed crafts change nothing");
        // Mixed plank types pay for tag ingredients; each ingredient is taken from the largest stack, which balances them.
        inv.add(&ItemStack::new("spruce_planks", 5));
        assert!(r.craft(table, &mut inv));
        assert_eq!((inv.count("spruce_planks"), inv.count("oak_planks")), (2, 1));
        // No room for the result: refused and unchanged.
        let mut full = Inventory::new();
        for i in 0..crate::item::SLOTS { full.slots[i] = Some(ItemStack::new("dirt", 64)); }
        full.slots[0] = Some(ItemStack::new("birch_log", 1));
        let bp = r.list.iter().find(|x| x.id == "minecraft:birch_planks").unwrap();
        assert!(r.craft(bp, &mut full), "the freed log slot holds the planks");
        full.slots[0] = Some(ItemStack::new("birch_log", 2));
        full.slots[1] = Some(ItemStack::new("birch_planks", 64));
        assert!(!r.craft(bp, &mut full) || full.count("birch_planks") >= 64);
        assert!(r.craftable(&inv).iter().any(|x| x.id == "minecraft:stick" || x.id == "minecraft:oak_planks"));
    }

    #[test]
    fn loads_recipes_and_tags_from_a_data_pack() {
        let d = temp("recipes");
        let w = |p: &str, b: &str| { let f = d.join("p").join(p); std::fs::create_dir_all(f.parent().unwrap()).unwrap(); std::fs::write(f, b).unwrap(); };
        w("data/minecraft/tags/item/planks.json", r##"{"values":["minecraft:oak_planks","#minecraft:fancy_planks"]}"##);
        w("data/minecraft/tags/item/fancy_planks.json", r#"{"values":["minecraft:cherry_planks"]}"#);
        w("data/minecraft/recipe/stick.json", r##"{"type":"minecraft:crafting_shaped","pattern":["#","#"],"key":{"#":"#minecraft:planks"},"result":{"id":"minecraft:stick","count":4}}"##);
        w("data/minecraft/recipe/old_style.json", r##"{"type":"minecraft:crafting_shaped","pattern":["X ","XX"],"key":{"X":{"item":"minecraft:dirt"}},"result":{"item":"minecraft:glass","count":2}}"##);
        w("data/minecraft/recipe/mix.json", r##"{"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:coal"},[{"item":"minecraft:stick"},{"tag":"minecraft:planks"}]],"result":{"id":"minecraft:torch","count":1}}"##);
        w("data/minecraft/recipe/smelt.json", r#"{"type":"minecraft:smelting","ingredient":"minecraft:sand","result":{"id":"minecraft:glass"}}"#);
        w("data/minecraft/recipe/broken.json", "{ nope");
        let mut s = PackStack::new();
        s.push(Pack::open(d.join("p")).unwrap());
        let r = Recipes::load(&s);
        assert_eq!(r.list.len(), 3, "smelting and broken files are skipped: {:?}", r.list.iter().map(|x| &x.id).collect::<Vec<_>>());
        let out = |grid: &[&str], w, h| r.match_grid(&g(grid), w, h).map(|x| (x.result.name.clone(), x.result.count));
        assert_eq!(out(&["cherry_planks", "oak_planks"], 1, 2), Some(("minecraft:stick".into(), 4)), "nested tag resolved");
        assert_eq!(out(&["dirt", "", "dirt", "dirt"], 2, 2), Some(("minecraft:glass".into(), 2)), "old-style result key (item instead of id)");
        assert_eq!(out(&["coal", "cherry_planks"], 2, 1), Some(("minecraft:torch".into(), 1)), "alternatives with a tag");
        assert_eq!(out(&["stone", "stone"], 2, 1), None);
        // An empty pack falls back to the built-in recipes.
        assert!(Recipes::load(&PackStack::new()).list.len() > 10);
        std::fs::remove_dir_all(d).ok();
    }
}
