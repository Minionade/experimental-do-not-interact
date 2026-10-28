use glam::{DVec3, IVec3};
use orbital_voxel::{
    mesh,
    mods::Registry,
    octree::BlockOctree,
    physics::{example_ship, Physics, FIXED_DT},
    planet::{Planet, TerrainOctree, Walker},
};
use std::{path::PathBuf, time::Instant};
fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut headless = false;
    let mut benchmark = false;
    let mut frames = None;
    let mut screenshot = None;
    let mut save = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => headless = true,
            "--mesh-bench" => benchmark = true,
            "--frames" => {
                let n = args
                    .next()
                    .ok_or("--frames needs a number")?
                    .parse::<u64>()
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("frames must be positive".into());
                }
                frames = Some(n);
            }
            "--screenshot" => {
                screenshot = Some(PathBuf::from(
                    args.next().ok_or("--screenshot needs a path")?,
                ))
            }
            "--save-demo" => {
                save = Some(PathBuf::from(
                    args.next().ok_or("--save-demo needs a path")?,
                ));
                headless = true;
            }
            "--help" | "-h" => {
                println!("Orbital Voxel — experimental Rust octree engine, not a complete Minecraft port.\n\nRun without flags for desktop.\n  --headless                 Validate planet traversal and simulate orbit\n  --mesh-bench               Generate and mesh an octree terrain cut\n  --frames N                Stop desktop rendering after N frames\n  --screenshot FILE.png      Capture the last frame (requires --frames)\n  --save-demo FILE.json      Save and reload an octree structure\n\nDesktop: click to capture mouse; WASD; Space/Ctrl; Shift; F walk/free; G ship; E spawn; Esc release/quit.");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}; use --help")),
        }
    }
    if screenshot.is_some() && frames.is_none() {
        return Err("--screenshot requires --frames".into());
    }
    if headless || benchmark {
        let planet = Planet::new(DVec3::ZERO, 96., 9.81);
        let mut walker = Walker::new(&planet);
        let initial = walker.position;
        for _ in 0..4096 {
            walker.walk(&planet, std::f64::consts::TAU * planet.radius / 4096., 0.);
        }
        println!(
            "circumnavigation position error: {:.3e} m",
            walker.position.distance(initial)
        );
        let mut physics = Physics::new(vec![planet.clone()]);
        let altitude = 48.;
        let radius = planet.radius + altitude;
        let speed = planet.circular_speed(altitude);
        let period = std::f64::consts::TAU * radius / speed;
        let body = physics.spawn_ball(DVec3::X * radius, DVec3::Y * speed, 0.25);
        for _ in 0..(period / FIXED_DT).round() as usize {
            physics.step(None);
        }
        println!(
            "orbit radial drift after one period: {:.6} m",
            physics.pose(body).0.length() - radius
        );
        let mut registry = Registry::default();
        registry.load_toml(include_str!("../examples/mods/example.toml"))?;
        println!("validated {} data-driven block definitions", registry.len());
        if let Some(path) = save {
            let tree = example_ship();
            std::fs::write(&path, tree.to_json()?).map_err(|e| e.to_string())?;
            let loaded = BlockOctree::from_json(
                &std::fs::read_to_string(&path).map_err(|e| e.to_string())?,
            )?;
            println!(
                "saved and reloaded {} octree nodes; hull block id {:?}",
                loaded.node_count(),
                loaded.get(IVec3::ZERO)
            );
        }
        if benchmark {
            let t = Instant::now();
            let tree = TerrainOctree::generate(&planet, 3);
            let generated = t.elapsed();
            let t = Instant::now();
            let meshes = mesh::mesh_terrain(&tree, &planet);
            println!(
                "{} leaf chunks, {} triangles, generation {:?}, parallel meshing {:?}",
                meshes.len(),
                meshes.iter().map(|m| m.vertices.len() / 3).sum::<usize>(),
                generated,
                t.elapsed()
            );
        }
        return Ok(());
    }
    #[cfg(feature = "desktop")]
    {
        orbital_voxel::desktop::run(orbital_voxel::desktop::Options { frames, screenshot })
    }
    #[cfg(not(feature = "desktop"))]
    {
        Err("desktop is disabled; use --headless or compile with default features".into())
    }
}
