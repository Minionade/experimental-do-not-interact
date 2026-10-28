//! Diagnostic preview of original, pinned vanilla model geometry and textures.
use glam::{Mat4, Vec3};
use glow::HasContext;
use orbital_voxel::{
    vanilla::{ClientAssets, ResourceId},
    vanilla_mesh::bake_model,
};
use std::{collections::BTreeMap, io::Cursor, path::PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut jar = None;
    let mut model = None;
    let mut block = None;
    let mut properties = orbital_voxel::blockstate::Properties::new();
    let mut ticket = 0u64;
    let mut frames = None;
    let mut screenshot = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--minecraft-jar" => jar = Some(PathBuf::from(args.next().ok_or("missing JAR path")?)),
            "--model" => model = Some(ResourceId::parse(&args.next().ok_or("missing model ID")?)?),
            "--block" => block = Some(ResourceId::parse(&args.next().ok_or("missing block ID")?)?),
            "--state" => {
                properties = orbital_voxel::blockstate::parse_properties(
                    &args.next().ok_or("missing properties")?,
                )?
            }
            "--ticket" => {
                ticket = args
                    .next()
                    .ok_or("missing ticket")?
                    .parse::<u64>()
                    .map_err(|e| e.to_string())?
            }
            "--frames" => {
                frames = Some(
                    args.next()
                        .ok_or("missing frame count")?
                        .parse::<u64>()
                        .map_err(|e| e.to_string())?,
                )
            }
            "--screenshot" => {
                screenshot = Some(PathBuf::from(args.next().ok_or("missing screenshot path")?))
            }
            "--help" | "-h" => {
                println!("vanilla-preview --minecraft-jar FILE [--model minecraft:block/stone | --block minecraft:furnace --state facing=east,lit=true] [--ticket N] [--frames N] [--screenshot FILE.png]\nUnlit diagnostic preview, not gameplay. No tint, animation or special renderers. Ticket controls weighted choice; vanilla position RNG is not implemented.");
                return Ok(());
            }
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if frames == Some(0) {
        return Err("frame count must be positive".into());
    }
    let mut assets = ClientAssets::open(&jar.ok_or("--minecraft-jar is required")?)?;
    if model.is_some() && block.is_some() {
        return Err("use --model or --block, not both".into());
    }
    if block.is_none() && !properties.is_empty() {
        return Err("--state requires --block".into());
    }
    let (label, quads) = if let Some(block) = block {
        let definition = assets.blockstate(&block)?;
        let choices = orbital_voxel::blockstate::select(&definition, &properties, || ticket)?;
        let mut quads = Vec::new();
        for choice in &choices {
            quads.extend(orbital_voxel::blockstate::bake_choice(&mut assets, choice)?);
        }
        (format!("{} {:?}", block.key(), properties), quads)
    } else {
        let model = model.unwrap_or(ResourceId::parse("minecraft:block/stone")?);
        (model.key(), bake_model(&assets.resolve_model(&model)?)?)
    };
    if quads.is_empty() {
        return Err("model has no element faces".into());
    }
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let attr = video.gl_attr();
    attr.set_context_profile(sdl2::video::GLProfile::Core);
    attr.set_context_version(3, 3);
    attr.set_depth_size(24);
    let window = video
        .window(&format!("{} — original asset preview", label), 640, 640)
        .opengl()
        .build()
        .map_err(|e| e.to_string())?;
    let _context = window.gl_create_context()?;
    let gl = unsafe {
        glow::Context::from_loader_function(|s| video.gl_get_proc_address(s) as *const _)
    };
    let mut events = sdl.event_pump()?;
    unsafe {
        let program = gl.create_program()?;
        for (kind, source) in [
            (glow::VERTEX_SHADER, "#version 330 core\nlayout(location=0) in vec3 p; layout(location=1) in vec2 uv; uniform mat4 mvp; out vec2 tex; void main(){tex=uv;gl_Position=mvp*vec4(p,1);}"),
            (glow::FRAGMENT_SHADER, "#version 330 core\nin vec2 tex; uniform sampler2D sprite; out vec4 color; void main(){color=texture(sprite,tex);if(color.a<0.1)discard;}"),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) { return Err(gl.get_shader_info_log(shader)); }
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(gl.get_program_info_log(program));
        }
        gl.use_program(Some(program));
        let mvp = Mat4::perspective_rh_gl(40f32.to_radians(), 1., 0.1, 100.)
            * Mat4::look_at_rh(Vec3::new(2.4, 2., 3.), Vec3::splat(0.5), Vec3::Y);
        gl.uniform_matrix_4_f32_slice(
            gl.get_uniform_location(program, "mvp").as_ref(),
            false,
            &mvp.to_cols_array(),
        );
        gl.uniform_1_i32(gl.get_uniform_location(program, "sprite").as_ref(), 0);
        gl.enable(glow::DEPTH_TEST);
        gl.enable(glow::CULL_FACE);
        gl.active_texture(glow::TEXTURE0);
        let mut groups: BTreeMap<ResourceId, Vec<f32>> = BTreeMap::new();
        for quad in &quads {
            let data = groups.entry(quad.texture.sprite.clone()).or_default();
            for i in [0, 1, 2, 0, 2, 3] {
                data.extend(quad.vertices[i].position);
                data.extend(quad.vertices[i].uv);
            }
        }
        let mut draws = Vec::new();
        for (sprite, vertices) in groups {
            let bytes = assets.texture_png(&sprite)?;
            let mut decoder = png::Decoder::new(Cursor::new(bytes));
            decoder
                .set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
            let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
            let mut pixels = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut pixels).map_err(|e| e.to_string())?;
            let pixels = &pixels[..info.buffer_size()];
            let rgba: Vec<u8> = match info.color_type {
                png::ColorType::Rgba => pixels.to_vec(),
                png::ColorType::Rgb => pixels
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .flat_map(|c| [c[0], c[1], c[2], 255])
                    .collect(),
                png::ColorType::Grayscale => pixels.iter().flat_map(|&c| [c, c, c, 255]).collect(),
                png::ColorType::GrayscaleAlpha => pixels
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .flat_map(|c| [c[0], c[0], c[0], c[1]])
                    .collect(),
                _ => return Err("unsupported PNG format".into()),
            };
            // Animated strips need metadata and a sprite-frame scheduler, not stretching.
            if info.width != info.height {
                return Err(format!(
                    "non-square/animated sprite not supported in preview: {}",
                    sprite.key()
                ));
            }
            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                info.width as i32,
                info.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&rgba)),
            );
            for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32);
            }
            for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32);
            }
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&vertices),
                glow::STATIC_DRAW,
            );
            for (index, size, offset) in [(0, 3, 0), (1, 2, 12)] {
                gl.enable_vertex_attrib_array(index);
                gl.vertex_attrib_pointer_f32(index, size, glow::FLOAT, false, 20, offset);
            }
            draws.push((vao, vbo, texture, (vertices.len() / 5) as i32));
        }
        let mut frame = 0;
        'render: loop {
            for event in events.poll_iter() {
                if matches!(
                    event,
                    sdl2::event::Event::Quit { .. }
                        | sdl2::event::Event::KeyDown {
                            keycode: Some(sdl2::keyboard::Keycode::Escape),
                            ..
                        }
                ) {
                    break 'render;
                }
            }
            gl.viewport(0, 0, 640, 640);
            gl.clear_color(0.09, 0.11, 0.15, 1.);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            for &(vao, _, texture, count) in &draws {
                gl.bind_vertex_array(Some(vao));
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.draw_arrays(glow::TRIANGLES, 0, count);
            }
            frame += 1;
            if frame == frames.unwrap_or(1) {
                if let Some(path) = &screenshot {
                    let mut pixels = vec![0u8; 640 * 640 * 4];
                    gl.read_pixels(
                        0,
                        0,
                        640,
                        640,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelPackData::Slice(Some(&mut pixels)),
                    );
                    let flipped: Vec<u8> = pixels
                        .as_chunks::<{ 640 * 4 }>()
                        .0
                        .iter()
                        .rev()
                        .flatten()
                        .copied()
                        .collect();
                    let mut encoder = png::Encoder::new(
                        std::fs::File::create(path).map_err(|e| e.to_string())?,
                        640,
                        640,
                    );
                    encoder.set_color(png::ColorType::Rgba);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder
                        .write_header()
                        .map_err(|e| e.to_string())?
                        .write_image_data(&flipped)
                        .map_err(|e| e.to_string())?;
                }
            }
            window.gl_swap_window();
            if frames.is_some_and(|limit| frame >= limit) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        for (vao, vbo, texture, _) in draws {
            gl.delete_vertex_array(vao);
            gl.delete_buffer(vbo);
            gl.delete_texture(texture);
        }
        gl.delete_program(program);
    }
    println!(
        "Rendered {}: {} quads using original PNGs",
        label,
        quads.len()
    );
    Ok(())
}
