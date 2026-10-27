//! Writes a small synthetic resource pack (procedural textures, original model JSON) for testing the pack pipeline.
//! usage: mkpack OUT_DIR
use std::path::Path;

fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263) ^ seed.wrapping_mul(2246822519);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^ (h >> 16)
}

fn png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut e = png::Encoder::new(std::fs::File::create(path).unwrap(), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(rgba).unwrap();
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// 16x16 noisy texture around `base`, with optional per-pixel alpha holes.
fn noisy(base: [u8; 3], spread: i32, seed: u32, holes: u32) -> Vec<u8> {
    (0..256u32).flat_map(|i| {
        let (x, y) = (i % 16, i / 16);
        let n = (hash(x, y, seed) % (2 * spread as u32 + 1)) as i32 - spread;
        let c = |v: u8| (v as i32 + n).clamp(0, 255) as u8;
        let a = if holes > 0 && hash(x, y, seed + 99) % 100 < holes { 0 } else { 255 };
        [c(base[0]), c(base[1]), c(base[2]), a]
    }).collect()
}

fn ore(base: [u8; 3], fleck: [u8; 3], seed: u32) -> Vec<u8> {
    let mut px = noisy(base, 10, seed, 0);
    for i in 0..256u32 {
        let (x, y) = (i % 16, i / 16);
        if hash(x / 3, y / 3, seed + 7) % 5 == 0 && (x + y) % 3 != 0 { px[(i * 4) as usize..(i * 4 + 3) as usize].copy_from_slice(&fleck); }
    }
    px
}

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: mkpack OUT_DIR"));
    let tex = out.join("assets/minecraft/textures/block");
    let models = out.join("assets/minecraft/models/block");
    let states = out.join("assets/minecraft/blockstates");
    write(&out.join("pack.mcmeta"), r#"{"pack":{"pack_format":34,"description":"blockengine synthetic test pack"}}"#);

    // Parent models (original, written against the documented model format).
    let face = |d: &str, t: &str, tint: &str| format!(r##""{d}":{{"texture":"#{t}","cullface":"{d}"{tint}}}"##);
    let cube = |name: &str, tex_for: [&str; 6], tint_up: bool| {
        let dirs = ["down", "up", "north", "south", "west", "east"];
        let faces: Vec<String> = dirs.iter().zip(tex_for).map(|(d, t)| face(d, t, if tint_up && *d == "up" { ",\"tintindex\":0" } else { "" })).collect();
        write(&models.join(format!("{name}.json")), &format!(r#"{{"elements":[{{"from":[0,0,0],"to":[16,16,16],"faces":{{{}}}}}]}}"#, faces.join(",")));
    };
    cube("cube", ["down", "up", "north", "south", "west", "east"], false);
    cube("cube_tinted_top", ["down", "up", "north", "south", "west", "east"], true);
    write(&models.join("cube_all.json"), r##"{"parent":"block/cube","textures":{"particle":"#all","down":"#all","up":"#all","north":"#all","south":"#all","west":"#all","east":"#all"}}"##);
    write(&models.join("cube_column.json"), r##"{"parent":"block/cube","textures":{"particle":"#side","down":"#end","up":"#end","north":"#side","south":"#side","west":"#side","east":"#side"}}"##);
    write(&models.join("cube_bottom_top.json"), r##"{"parent":"block/cube","textures":{"particle":"#side","down":"#bottom","up":"#top","north":"#side","south":"#side","west":"#side","east":"#side"}}"##);
    write(&models.join("grass_like.json"), r##"{"parent":"block/cube_tinted_top","textures":{"particle":"#side","down":"#bottom","up":"#top","north":"#side","south":"#side","west":"#side","east":"#side"}}"##);
    write(&models.join("leaves.json"), r##"{"parent":"block/cube","textures":{"particle":"#all","down":"#all","up":"#all","north":"#all","south":"#all","west":"#all","east":"#all"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{
        "down":{"texture":"#all","cullface":"down","tintindex":0},"up":{"texture":"#all","cullface":"up","tintindex":0},"north":{"texture":"#all","cullface":"north","tintindex":0},
        "south":{"texture":"#all","cullface":"south","tintindex":0},"west":{"texture":"#all","cullface":"west","tintindex":0},"east":{"texture":"#all","cullface":"east","tintindex":0}}}]}"##);
    for (name, tint) in [("cross", ""), ("tinted_cross", ",\"tintindex\":0")] {
        let plane = |from: &str, to: &str, a: &str, b: &str| format!(r##"{{"from":{from},"to":{to},"rotation":{{"origin":[8,8,8],"axis":"y","angle":45,"rescale":true}},"shade":false,"faces":{{"{a}":{{"texture":"#cross"{tint}}},"{b}":{{"texture":"#cross"{tint}}}}}}}"##);
        write(&models.join(format!("{name}.json")), &format!(r##"{{"ambientocclusion":false,"textures":{{"particle":"#cross"}},"elements":[{},{}]}}"##,
            plane("[0.8,0,8]", "[15.2,16,8]", "north", "south"), plane("[8,0,0.8]", "[8,16,15.2]", "west", "east")));
    }

    let simple = |name: &str, px: Vec<u8>| {
        png(&tex.join(format!("{name}.png")), 16, 16, &px);
        write(&states.join(format!("{name}.json")), &format!(r#"{{"variants":{{"":{{"model":"block/{name}"}}}}}}"#));
        write(&models.join(format!("{name}.json")), &format!(r#"{{"parent":"block/cube_all","textures":{{"all":"block/{name}"}}}}"#));
    };
    simple("stone", noisy([125, 125, 128], 12, 1, 0));
    simple("dirt", noisy([134, 96, 67], 14, 2, 0));
    simple("sand", noisy([219, 207, 142], 8, 3, 0));
    simple("bedrock", noisy([60, 60, 62], 25, 4, 0));
    simple("snow_block", noisy([245, 250, 252], 4, 5, 0));
    simple("cobblestone", noisy([118, 118, 118], 30, 6, 0));
    simple("obsidian", noisy([22, 18, 34], 6, 7, 0));
    simple("coal_ore", ore([125, 125, 128], [30, 30, 30], 8));
    simple("iron_ore", ore([125, 125, 128], [210, 170, 140], 9));
    simple("gold_ore", ore([125, 125, 128], [250, 215, 60], 10));
    simple("diamond_ore", ore([125, 125, 128], [90, 235, 235], 11));
    simple("glowstone", noisy([255, 205, 120], 25, 12, 0));
    simple("sandstone", noisy([216, 203, 155], 6, 13, 0));

    // Grass: grayscale top (tinted by the biome colormap), dirt side with a green fringe, dirt bottom.
    png(&tex.join("grass_block_top.png"), 16, 16, &noisy([150, 150, 150], 14, 20, 0));
    let mut side = noisy([134, 96, 67], 14, 21, 0);
    for i in 0..256u32 { if i / 16 < 3 + hash(i % 16, 0, 22) % 2 { let o = (i * 4) as usize; side[o..o + 3].copy_from_slice(&[96, 160, 56]); } }
    png(&tex.join("grass_block_side.png"), 16, 16, &side);
    write(&states.join("grass_block.json"), r#"{"variants":{"":{"model":"block/grass_block"}}}"#);
    write(&models.join("grass_block.json"), r#"{"parent":"block/grass_like","textures":{"bottom":"block/dirt","top":"block/grass_block_top","side":"block/grass_block_side"}}"#);

    // Logs, leaves (alpha holes, tinted).
    png(&tex.join("oak_log.png"), 16, 16, &noisy([102, 81, 50], 14, 30, 0));
    png(&tex.join("oak_log_top.png"), 16, 16, &noisy([162, 130, 78], 10, 31, 0));
    write(&states.join("oak_log.json"), r#"{"variants":{"axis=y":{"model":"block/oak_log"},"axis=x":{"model":"block/oak_log","x":90,"y":90},"axis=z":{"model":"block/oak_log","x":90}}}"#);
    write(&models.join("oak_log.json"), r#"{"parent":"block/cube_column","textures":{"end":"block/oak_log_top","side":"block/oak_log"}}"#);
    png(&tex.join("oak_leaves.png"), 16, 16, &noisy([170, 170, 170], 30, 32, 28));
    write(&states.join("oak_leaves.json"), r#"{"variants":{"":{"model":"block/oak_leaves"}}}"#);
    write(&models.join("oak_leaves.json"), r#"{"parent":"block/leaves","textures":{"all":"block/oak_leaves"}}"#);

    // Plants.
    let mut flower = vec![0u8; 16 * 16 * 4];
    for y in 0..16u32 { for x in 0..16u32 {
        let o = ((y * 16 + x) * 4) as usize;
        if (7..=8).contains(&x) && y >= 6 { flower[o..o + 4].copy_from_slice(&[40, 140, 40, 255]); }
        if (5..=10).contains(&x) && (2..=7).contains(&y) && (x as i32 - 7).abs() + (y as i32 - 4).abs() < 5 { flower[o..o + 4].copy_from_slice(&[210, 30, 40, 255]); }
    }}
    png(&tex.join("poppy.png"), 16, 16, &flower);
    write(&states.join("poppy.json"), r#"{"variants":{"":{"model":"block/poppy"}}}"#);
    write(&models.join("poppy.json"), r#"{"parent":"block/cross","textures":{"cross":"block/poppy"}}"#);
    let grass: Vec<u8> = (0..256u32).flat_map(|i| { let (x, y) = (i % 16, i / 16); if x % 3 == 1 && y > 3 + (x * 7) % 5 { [150, 150, 150, 255] } else { [0, 0, 0, 0] } }).collect();
    png(&tex.join("short_grass.png"), 16, 16, &grass);
    write(&states.join("short_grass.json"), r#"{"variants":{"":{"model":"block/short_grass"}}}"#);
    write(&models.join("short_grass.json"), r#"{"parent":"block/tinted_cross","textures":{"cross":"block/short_grass"}}"#);

    // Animated fluids: 8 frames stacked vertically; water steps every 2 ticks, lava blends smoothly.
    let fluid = |name: &str, base: [f32; 3], speed: f32| {
        let mut px = Vec::new();
        for f in 0..8u32 { for y in 0..16u32 { for x in 0..16u32 {
            let t = f as f32 / 8.0 * std::f32::consts::TAU;
            let w = ((x as f32 * 0.7 + t * speed).sin() + (y as f32 * 0.9 - t * speed).cos()) * 0.5;
            let k = 0.75 + 0.25 * w;
            px.extend([(base[0] * k) as u8, (base[1] * k) as u8, (base[2] * k) as u8, 255]);
        }}}
        png(&tex.join(format!("{name}.png")), 16, 128, &px);
    };
    fluid("water_still", [70.0, 110.0, 235.0], 1.0);
    write(&tex.join("water_still.png.mcmeta"), r#"{"animation":{"frametime":2}}"#);
    fluid("lava_still", [255.0, 120.0, 20.0], 1.0);
    write(&tex.join("lava_still.png.mcmeta"), r#"{"animation":{"frametime":4,"interpolate":true}}"#);

    // Block-break crack stages: speckle that grows with the stage, on a transparent tile.
    for stage in 0..10u32 {
        let px: Vec<u8> = (0..256u32).flat_map(|i| {
            let (x, y) = (i % 16, i / 16);
            if hash(x / 2, y / 2, 55) % 10 < stage { [20, 20, 20, 235] } else { [0, 0, 0, 0] }
        }).collect();
        png(&tex.join(format!("destroy_stage_{stage}.png")), 16, 16, &px);
    }

    // Item icons: sticks, coal, and five tiers of five tools drawn from a diagonal handle plus a head.
    let items = out.join("assets/minecraft/textures/item");
    let draw = |set: &dyn Fn(i32, i32) -> Option<[u8; 4]>| -> Vec<u8> {
        (0..256i32).flat_map(|i| set(i % 16, i / 16).unwrap_or([0, 0, 0, 0])).collect()
    };
    let handle = |x: i32, y: i32| (x + y == 15 && (3..=13).contains(&x)).then_some([130, 95, 50, 255]);
    png(&items.join("stick.png"), 16, 16, &draw(&|x, y| handle(x, y)));
    png(&items.join("coal.png"), 16, 16, &draw(&|x, y| ((x - 8).pow(2) + (y - 8).pow(2) < 26).then_some([35 + (hash(x as u32, y as u32, 3) % 25) as u8, 35, 38, 255])));
    for (tier, c) in [("wooden", [160, 125, 70]), ("stone", [125, 125, 128]), ("iron", [215, 215, 220]), ("golden", [250, 215, 60]), ("diamond", [90, 235, 235])] {
        let head = [c[0], c[1], c[2], 255];
        let kinds: [(&str, Box<dyn Fn(i32, i32) -> bool>); 5] = [
            ("pickaxe", Box::new(|x, y| (y == 3 && (3..=12).contains(&x)) || (y == 4 && (x == 3 || x == 12)) || (y == 5 && (x == 2 || x == 13)))),
            ("axe", Box::new(|x, y| (2..=5).contains(&y) && (6..=11).contains(&x) && x - 6 <= 5 - y + 2)),
            ("shovel", Box::new(|x, y| (2..=4).contains(&y) && (11..=13).contains(&x))),
            ("hoe", Box::new(|x, y| (y == 3 && (5..=11).contains(&x)) || (y == 4 && x == 5))),
            ("sword", Box::new(|x, y| x + y == 15 && (5..=12).contains(&x) || (x + y == 14 && (6..=12).contains(&x)))),
        ];
        for (kind, is_head) in kinds {
            png(&items.join(format!("{tier}_{kind}.png")), 16, 16, &draw(&|x, y| if is_head(x, y) { Some(head) } else { handle(x, y) }));
        }
    }

    // Biome colormaps (the documented 256x256 triangle lookup): cold = bluish green, hot/dry = yellowish.
    for (name, cold, hot) in [("grass", [110.0, 190.0, 110.0], [190.0, 180.0, 70.0]), ("foliage", [70.0, 140.0, 90.0], [160.0, 160.0, 40.0])] {
        let px: Vec<u8> = (0..256u32 * 256).flat_map(|i| {
            let (x, y) = ((i % 256) as f32 / 255.0, (i / 256) as f32 / 255.0);
            let t = 1.0 - x; let wet = 1.0 - y;
            let mix = |a: f32, b: f32| (a + (b - a) * t * (1.0 - 0.5 * wet)).clamp(0.0, 255.0) as u8;
            [mix(cold[0], hot[0]), mix(cold[1], hot[1]), mix(cold[2], hot[2]), 255]
        }).collect();
        png(&out.join(format!("assets/minecraft/textures/colormap/{name}.png")), 256, 256, &px);
    }
    eprintln!("wrote synthetic pack to {}", out.display());
}
