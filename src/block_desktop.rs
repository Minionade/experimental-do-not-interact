//! Interactive original-asset octree test world, using registered Minecraft states.
use crate::{
    block_collision,
    block_world::BlockWorld,
    blockstate::parse_properties,
    collision::{self, Aabb},
    oracle::PhysicsCatalog,
    state_registry::StateRegistry,
    textured_renderer::TexturedRenderer,
    vanilla::{ClientAssets, RegistryReports, ResourceId},
    world_mesh::{self, ModelCache},
};
use glam::{DVec3, IVec3};
use sdl2::{
    event::Event,
    keyboard::{Keycode, Scancode},
    mouse::MouseButton,
    video::GLProfile,
};
use std::{path::PathBuf, rc::Rc, sync::Arc, time::Instant};
pub struct Options {
    pub jar: PathBuf,
    pub reports: PathBuf,
    pub frames: Option<u64>,
    pub screenshot: Option<PathBuf>,
    pub save: Option<PathBuf>,
    pub load: Option<PathBuf>,
    pub headless: bool,
}
const PALETTE: [&str; 9] = [
    "stone",
    "dirt",
    "oak_planks",
    "bricks",
    "glass",
    "crafting_table",
    "furnace",
    "oak_stairs",
    "oak_fence",
];
fn showcase(registry: Arc<StateRegistry>) -> Result<BlockWorld, String> {
    let mut w = BlockWorld::new(registry, IVec3::splat(-32), 64)?;
    let stone = w.registry.resolve("stone", &Default::default())?;
    let planks = w.registry.resolve("oak_planks", &Default::default())?;
    for z in -12..12 {
        for x in -12..12 {
            w.set(
                IVec3::new(x, -1, z),
                if (x + z) % 7 == 0 { planks } else { stone },
            )?;
        }
    }
    for (i, block) in [
        "crafting_table",
        "furnace",
        "oak_stairs",
        "bricks",
        "oak_planks",
        "glass",
    ]
    .into_iter()
    .enumerate()
    {
        let id = w.registry.resolve(
            block,
            &if block == "furnace" {
                parse_properties("facing=south,lit=true")?
            } else {
                Default::default()
            },
        )?;
        w.set(IVec3::new(-5 + i as i32 * 2, 0, 0), id)?;
    }
    for x in -5..6 {
        let id = w.registry.resolve(
            "oak_fence",
            &parse_properties(&format!("east={},west={}", x < 5, x > -5))?,
        )?;
        w.set(IVec3::new(x, 0, -4), id)?;
    }
    let bricks = w.registry.resolve("bricks", &Default::default())?;
    for y in 0..4 {
        for x in -3i32..4 {
            if y == 3 || x.abs() == 3 {
                w.set(IVec3::new(x, y, -7), bricks)?;
            }
        }
    }
    Ok(w)
}
fn save(world: &BlockWorld, path: &std::path::Path) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, world.to_json()?).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, path).map_err(|e| e.to_string())
}
pub fn run(options: Options) -> Result<(), String> {
    let mut assets = ClientAssets::open(&options.jar)?;
    let reports = RegistryReports::load_pinned(&options.reports)?;
    let registry = Arc::new(StateRegistry::from_reports(&reports)?);
    let physics = PhysicsCatalog::load_pinned(&options.reports.join("physics.json"), &registry)?;
    let mut world = if let Some(path) = &options.load {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
            return Err("save exceeds load limit".into());
        }
        BlockWorld::from_json(
            registry.clone(),
            &std::io::read_to_string(file).map_err(|e| e.to_string())?,
        )?
    } else {
        showcase(registry.clone())?
    };
    let mut cache = ModelCache::default();
    let mut meshes = Vec::new();
    for key in world.active_chunks(16384)? {
        cache.prepare_chunk(&world, key, &mut assets)?;
        meshes.push(world_mesh::mesh_chunk(&world, key, &cache)?);
    }
    world.take_dirty();
    let triangles: usize = meshes
        .iter()
        .flat_map(|m| m.batches.values())
        .map(|v| v.len() / 3)
        .sum();
    println!("{} official states; {} octree nodes; {} chunks; {triangles} triangles; {} original sprites",registry.len(),world.node_count(),meshes.len(),cache.sprites.len());
    if options.headless {
        if let Some(path) = &options.save {
            save(&world, path)?;
            let reloaded = BlockWorld::from_json(
                registry,
                &std::fs::read_to_string(path).map_err(|e| e.to_string())?,
            )?;
            if reloaded.to_json()? != world.to_json()? {
                return Err("world save roundtrip mismatch".into());
            }
        }
        return Ok(());
    }
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let attr = video.gl_attr();
    attr.set_context_profile(GLProfile::Core);
    attr.set_context_version(3, 3);
    attr.set_depth_size(24);
    let mut window = video
        .window("Minecraft 26.3 assets | cubic octree world", 1280, 720)
        .opengl()
        .resizable()
        .build()
        .map_err(|e| e.to_string())?;
    let _context = window.gl_create_context()?;
    let gl = Rc::new(unsafe {
        glow::Context::from_loader_function(|s| video.gl_get_proc_address(s) as *const _)
    });
    let _ = video.gl_set_swap_interval(sdl2::video::SwapInterval::VSync);
    let mut renderer = TexturedRenderer::new(gl)?;
    for mesh in meshes {
        renderer.upload(mesh)?;
    }
    let palette = PALETTE
        .iter()
        .map(|name| registry.resolve(name, &Default::default()))
        .collect::<Result<Vec<_>, _>>()?;
    let names = PALETTE
        .iter()
        .map(|name| {
            assets
                .display_name("block", &ResourceId::parse(name)?)
                .map(str::to_string)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut selected = 0usize;
    let mut walking = false;
    let mut vertical_velocity = 0.0;
    let mut grounded = false;
    let mut accumulator = 0.0;
    let body_at = |eye: DVec3| {
        Aabb::new(
            eye + DVec3::new(-0.3, -1.62, -0.3),
            eye + DVec3::new(0.3, 0.18, 0.3),
        )
    };
    let mut camera = DVec3::new(10., 8., 16.);
    let (mut yaw, mut pitch) = (0.48f64, -0.35f64);
    let mut events = sdl.event_pump()?;
    let mut captured = false;
    let start = Instant::now();
    let mut previous = start;
    let mut title_time = start;
    let mut frames = 0u64;
    println!("Click to capture; F toggles walk/fly, Space jumps in walk mode; WASD fly, Space/Ctrl vertical, Shift fast; 1-9 select; left break, right place, middle pick; F5 save; Esc release/quit. Creative test controls, not survival parity.");
    'running: loop {
        let now = Instant::now();
        let dt = (now - previous).as_secs_f64().min(0.1);
        previous = now;
        let forward = |yaw: f64, pitch: f64| {
            DVec3::new(
                -yaw.sin() * pitch.cos(),
                pitch.sin(),
                -yaw.cos() * pitch.cos(),
            )
        };
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. } => break 'running,
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    repeat: false,
                    ..
                } => {
                    if captured {
                        captured = false;
                        sdl.mouse().set_relative_mouse_mode(false);
                    } else {
                        break 'running;
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::F),
                    repeat: false,
                    ..
                } => {
                    if walking {
                        walking = false;
                    } else {
                        let body = body_at(camera)?;
                        match block_collision::colliders(&world, &physics, body) {
                            Ok(boxes) if boxes.is_empty() => {
                                walking = true;
                            }
                            Ok(_) => eprintln!("Cannot enter walk mode inside a block"),
                            Err(e) => eprintln!("Cannot enter walk mode: {e}"),
                        }
                    }
                    vertical_velocity = 0.0;
                    grounded = false;
                    accumulator = 0.0;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::F5),
                    repeat: false,
                    ..
                } => {
                    if let Some(path) = &options.save {
                        save(&world, path)?;
                        println!("Saved {}", path.display());
                    }
                }
                Event::KeyDown {
                    keycode: Some(key),
                    repeat: false,
                    ..
                } => {
                    if let Some(i) = [
                        Keycode::Num1,
                        Keycode::Num2,
                        Keycode::Num3,
                        Keycode::Num4,
                        Keycode::Num5,
                        Keycode::Num6,
                        Keycode::Num7,
                        Keycode::Num8,
                        Keycode::Num9,
                    ]
                    .iter()
                    .position(|k| *k == key)
                    {
                        selected = i;
                    }
                }
                Event::MouseMotion { xrel, yrel, .. } if captured => {
                    yaw -= xrel as f64 * 0.003;
                    pitch = (pitch - yrel as f64 * 0.003).clamp(-1.54, 1.54);
                }
                Event::MouseButtonDown { mouse_btn, .. } => {
                    if !captured {
                        captured = true;
                        sdl.mouse().set_relative_mouse_mode(true);
                        continue;
                    }
                    if let Some(hit) =
                        block_collision::raycast(&world, &physics, camera, forward(yaw, pitch), 6.)?
                    {
                        match mouse_btn {
                            MouseButton::Left => world.set(hit.cell, 0)?,
                            MouseButton::Right => {
                                let p = hit.cell + hit.normal;
                                if hit.normal != IVec3::ZERO
                                    && world.contains(p)
                                    && world.get(p).is_some_and(|id| registry.is_air(id))
                                {
                                    cache.ensure(palette[selected], &registry, &mut assets)?;
                                    let body = body_at(camera)?;
                                    let intersects_player =
                                        physics.static_boxes(palette[selected], false)?.iter().any(
                                            |shape| shape.translated(p.as_dvec3()).intersects(body),
                                        );
                                    if !intersects_player {
                                        world.set(p, palette[selected])?;
                                    }
                                }
                            }
                            MouseButton::Middle => {
                                if let Some(i) = palette.iter().position(|id| *id == hit.state) {
                                    selected = i;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        let keys = events.keyboard_state();
        let axis = |a, b| {
            keys.is_scancode_pressed(a) as i32 as f64 - keys.is_scancode_pressed(b) as i32 as f64
        };
        let look = forward(yaw, pitch);
        let movement = look * axis(Scancode::W, Scancode::S)
            + look.cross(DVec3::Y).normalize() * axis(Scancode::D, Scancode::A)
            + DVec3::Y * axis(Scancode::Space, Scancode::LCtrl);
        if captured && walking {
            accumulator += dt;
            const TICK: f64 = 1.0 / 120.0;
            while accumulator >= TICK {
                accumulator -= TICK;
                if grounded && keys.is_scancode_pressed(Scancode::Space) {
                    vertical_velocity = 8.4;
                }
                vertical_velocity = (vertical_velocity - 32.0 * TICK).max(-78.4);
                let flat = forward(yaw, 0.0);
                let horizontal = (flat * axis(Scancode::W, Scancode::S)
                    + flat.cross(DVec3::Y) * axis(Scancode::D, Scancode::A))
                .normalize_or_zero()
                    * 4.317;
                let desired = (horizontal + DVec3::Y * vertical_velocity) * TICK;
                let body = body_at(camera)?;
                let result =
                    block_collision::colliders(&world, &physics, body.swept_bounds(desired, 0.6))
                        .and_then(|boxes| {
                            collision::move_body(body, &boxes, desired, 0.6, grounded)
                        });
                match result {
                    Ok(motion) => {
                        camera += motion.delta;
                        grounded = motion.grounded;
                        if motion.blocked[1] {
                            vertical_velocity = 0.0;
                        }
                    }
                    Err(e) => {
                        eprintln!("Walking paused: {e}");
                        walking = false;
                        accumulator = 0.0;
                        break;
                    }
                }
            }
        } else if captured {
            camera += movement.normalize_or_zero()
                * dt
                * if keys.is_scancode_pressed(Scancode::LShift) {
                    20.
                } else {
                    6.
                };
        }
        for key in world.take_dirty() {
            cache.prepare_chunk(&world, key, &mut assets)?;
            renderer.upload(world_mesh::mesh_chunk(&world, key, &cache)?)?;
        }
        let (w, h) = window.drawable_size();
        if w == 0 || h == 0 {
            std::thread::sleep(std::time::Duration::from_millis(16));
            continue;
        }
        renderer.sync_sprites(&cache, start.elapsed().as_secs_f64() * 20.)?;
        let (draws, triangles) = renderer.draw(camera, look, w, h)?;
        frames += 1;
        if options.frames.is_some_and(|n| frames >= n) {
            if let Some(path) = &options.screenshot {
                renderer.screenshot(path, w, h)?;
            }
        }
        window.gl_swap_window();
        if title_time.elapsed().as_secs_f32() > 0.25 {
            let hit = block_collision::raycast(&world, &physics, camera, look, 6.)?
                .and_then(|h| registry.get(h.state))
                .map(|s| s.block.as_str())
                .unwrap_or("air");
            let _ = window.set_title(&format!(
                "{} | target {hit} | {draws} draws {triangles} triangles | {} | F walk/fly / F5 save",
                names[selected], if walking { "walk" } else { "fly" }
            ));
            title_time = now;
        }
        if options.frames.is_some_and(|n| frames >= n) {
            println!("Rendered {frames} frames; {draws} draws; {triangles} triangles");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    if let Some(path) = &options.save {
        save(&world, path)?;
    }
    Ok(())
}
