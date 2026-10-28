use crate::{
    mesh::{self, Mesh, Vertex},
    octree::Bounds,
    physics::{self, BodyHandle, Physics},
    planet::{self, Planet, TerrainOctree, Walker},
};
use glam::{DVec3, Mat4, Vec3};
use glow::HasContext;
use sdl2::{
    event::Event,
    keyboard::{Keycode, Scancode},
    video::GLProfile,
};
use std::{collections::HashMap, path::PathBuf, time::Instant};

#[derive(Default)]
pub struct Options {
    pub frames: Option<u64>,
    pub screenshot: Option<PathBuf>,
}
struct Program {
    id: glow::NativeProgram,
    uniforms: HashMap<String, glow::NativeUniformLocation>,
}
impl Program {
    unsafe fn create(
        gl: &glow::Context,
        vertex: &str,
        fragment: &str,
        final_pass: bool,
    ) -> Result<Self, String> {
        let id = gl.create_program()?;
        for (kind, source) in [
            (glow::VERTEX_SHADER, vertex),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let s = gl.create_shader(kind)?;
            gl.shader_source(s, source);
            gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) {
                let log = gl.get_shader_info_log(s);
                gl.delete_shader(s);
                gl.delete_program(id);
                return Err(format!("shader compilation: {log}"));
            }
            gl.attach_shader(id, s);
            gl.delete_shader(s);
        }
        gl.bind_attrib_location(id, 0, "a_position");
        gl.bind_attrib_location(id, 1, if final_pass { "a_uv" } else { "a_normal" });
        if !final_pass {
            gl.bind_attrib_location(id, 2, "a_color");
        }
        gl.link_program(id);
        if !gl.get_program_link_status(id) {
            let log = gl.get_program_info_log(id);
            gl.delete_program(id);
            return Err(format!("shader link: {log}"));
        }
        let common = [
            ("u_view_width", glow::FLOAT),
            ("u_view_height", glow::FLOAT),
            ("u_time", glow::FLOAT),
            ("u_camera_position", glow::FLOAT_VEC3),
        ];
        let stage = if final_pass {
            vec![("u_scene_color", glow::SAMPLER_2D)]
        } else {
            vec![
                ("u_model_view", glow::FLOAT_MAT4),
                ("u_projection", glow::FLOAT_MAT4),
                ("u_sun_direction", glow::FLOAT_VEC3),
            ]
        };
        let known: HashMap<_, _> = common.into_iter().chain(stage).collect();
        let mut uniforms = HashMap::new();
        for index in 0..gl.get_active_uniforms(id) {
            if let Some(u) = gl.get_active_uniform(id, index) {
                if known.get(u.name.as_str()) != Some(&u.utype) || u.size != 1 {
                    gl.delete_program(id);
                    return Err(format!("unsupported active uniform {}", u.name));
                }
                if let Some(location) = gl.get_uniform_location(id, &u.name) {
                    uniforms.insert(u.name, location);
                }
            }
        }
        let attrs: &[(&str, u32, u32)] = if final_pass {
            &[
                ("a_position", glow::FLOAT_VEC3, 0),
                ("a_uv", glow::FLOAT_VEC2, 1),
            ]
        } else {
            &[
                ("a_position", glow::FLOAT_VEC3, 0),
                ("a_normal", glow::FLOAT_VEC3, 1),
                ("a_color", glow::FLOAT_VEC4, 2),
            ]
        };
        for i in 0..gl.get_active_attributes(id) {
            if let Some(a) = gl.get_active_attribute(id, i) {
                if !attrs.iter().any(|(n, t, l)| {
                    *n == a.name && *t == a.atype && gl.get_attrib_location(id, n) == Some(*l)
                }) {
                    gl.delete_program(id);
                    return Err(format!("unsupported vertex input {} or location", a.name));
                }
            }
        }
        Ok(Self { id, uniforms })
    }
    fn u(&self, name: &str) -> Option<&glow::NativeUniformLocation> {
        self.uniforms.get(name)
    }
    unsafe fn matrix(&self, gl: &glow::Context, name: &str, m: Mat4) {
        gl.uniform_matrix_4_f32_slice(self.u(name), false, &m.to_cols_array());
    }
}
struct GpuMesh {
    vao: glow::NativeVertexArray,
    vbo: glow::NativeBuffer,
    count: i32,
    bounds: Bounds,
}
impl GpuMesh {
    unsafe fn upload(gl: &glow::Context, m: &Mesh) -> Result<Self, String> {
        let vao = gl.create_vertex_array()?;
        let vbo = gl.create_buffer()?;
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(&m.vertices),
            glow::STATIC_DRAW,
        );
        for i in 0..3 {
            gl.enable_vertex_attrib_array(i);
            gl.vertex_attrib_pointer_f32(
                i,
                3,
                glow::FLOAT,
                false,
                std::mem::size_of::<Vertex>() as i32,
                (i * 12) as i32,
            );
        }
        Ok(Self {
            vao,
            vbo,
            count: m.vertices.len() as i32,
            bounds: m.bounds,
        })
    }
    unsafe fn delete(&self, gl: &glow::Context) {
        gl.delete_vertex_array(self.vao);
        gl.delete_buffer(self.vbo);
    }
}
struct Targets {
    fbo: glow::NativeFramebuffer,
    color: glow::NativeTexture,
    depth: glow::NativeRenderbuffer,
    width: u32,
    height: u32,
}
impl Targets {
    unsafe fn new(gl: &glow::Context, width: u32, height: u32) -> Result<Self, String> {
        let fbo = gl.create_framebuffer()?;
        let color = gl.create_texture()?;
        let depth = gl.create_renderbuffer()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(color));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA16F as i32,
            width as i32,
            height as i32,
            0,
            glow::RGBA,
            glow::FLOAT,
            glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
        gl.renderbuffer_storage(
            glow::RENDERBUFFER,
            glow::DEPTH_COMPONENT24,
            width as i32,
            height as i32,
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(color),
            0,
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(depth),
        );
        if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
            return Err("incomplete offscreen framebuffer".into());
        }
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        Ok(Self {
            fbo,
            color,
            depth,
            width,
            height,
        })
    }
    unsafe fn delete(&self, gl: &glow::Context) {
        gl.delete_framebuffer(self.fbo);
        gl.delete_texture(self.color);
        gl.delete_renderbuffer(self.depth);
    }
}
struct Renderer {
    gl: glow::Context,
    terrain: Program,
    final_pass: Program,
    targets: Targets,
    quad_vao: glow::NativeVertexArray,
    quad_vbo: glow::NativeBuffer,
}
impl Renderer {
    unsafe fn new(gl: glow::Context, w: u32, h: u32) -> Result<Self, String> {
        let terrain = Program::create(
            &gl,
            include_str!("../assets/shaders/terrain.vert"),
            include_str!("../assets/shaders/terrain.frag"),
            false,
        )?;
        let final_pass = Program::create(
            &gl,
            include_str!("../assets/shaders/final.vert"),
            include_str!("../assets/shaders/final.frag"),
            true,
        )?;
        let targets = Targets::new(&gl, w, h)?;
        let quad_vao = gl.create_vertex_array()?;
        let quad_vbo = gl.create_buffer()?;
        gl.bind_vertex_array(Some(quad_vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(quad_vbo));
        let quad: [f32; 30] = [
            -1., -1., 0., 0., 0., 1., -1., 0., 1., 0., 1., 1., 0., 1., 1., -1., -1., 0., 0., 0.,
            1., 1., 0., 1., 1., -1., 1., 0., 0., 1.,
        ];
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(&quad),
            glow::STATIC_DRAW,
        );
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 20, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 20, 12);
        Ok(Self {
            gl,
            terrain,
            final_pass,
            targets,
            quad_vao,
            quad_vbo,
        })
    }
    unsafe fn resize(&mut self, w: u32, h: u32) -> Result<(), String> {
        if self.targets.width == w && self.targets.height == h {
            return Ok(());
        }
        let new = Targets::new(&self.gl, w, h)?;
        self.targets.delete(&self.gl);
        self.targets = new;
        Ok(())
    }
    unsafe fn begin(&self, projection: Mat4, view: Mat4, camera: DVec3, seconds: f32) {
        let gl = &self.gl;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.targets.fbo));
        gl.viewport(0, 0, self.targets.width as i32, self.targets.height as i32);
        gl.enable(glow::DEPTH_TEST);
        gl.enable(glow::CULL_FACE);
        gl.cull_face(glow::BACK);
        gl.front_face(glow::CCW);
        gl.clear_color(0.004, 0.008, 0.018, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        gl.use_program(Some(self.terrain.id));
        self.terrain.matrix(gl, "u_projection", projection);
        let sun = view
            .transform_vector3(Vec3::new(-0.6, 0.8, 0.5))
            .normalize();
        gl.uniform_3_f32(self.terrain.u("u_sun_direction"), sun.x, sun.y, sun.z);
        gl.uniform_3_f32(
            self.terrain.u("u_camera_position"),
            camera.x as f32,
            camera.y as f32,
            camera.z as f32,
        );
        gl.uniform_1_f32(self.terrain.u("u_time"), seconds);
        gl.uniform_1_f32(self.terrain.u("u_view_width"), self.targets.width as f32);
        gl.uniform_1_f32(self.terrain.u("u_view_height"), self.targets.height as f32);
    }
    unsafe fn draw(&self, mesh: &GpuMesh, model_view: Mat4) {
        self.terrain.matrix(&self.gl, "u_model_view", model_view);
        self.gl.bind_vertex_array(Some(mesh.vao));
        self.gl.draw_arrays(glow::TRIANGLES, 0, mesh.count);
    }
    unsafe fn finish(&self, seconds: f32, camera: DVec3) {
        let gl = &self.gl;
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.use_program(Some(self.final_pass.id));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(self.targets.color));
        gl.uniform_1_i32(self.final_pass.u("u_scene_color"), 0);
        gl.uniform_1_f32(self.final_pass.u("u_view_width"), self.targets.width as f32);
        gl.uniform_1_f32(
            self.final_pass.u("u_view_height"),
            self.targets.height as f32,
        );
        gl.uniform_1_f32(self.final_pass.u("u_time"), seconds);
        gl.uniform_3_f32(
            self.final_pass.u("u_camera_position"),
            camera.x as f32,
            camera.y as f32,
            camera.z as f32,
        );
        gl.bind_vertex_array(Some(self.quad_vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 6);
    }
    unsafe fn screenshot(&self, path: &std::path::Path) -> Result<(), String> {
        let (w, h) = (self.targets.width, self.targets.height);
        let mut raw = vec![0u8; (w * h * 4) as usize];
        self.gl.read_pixels(
            0,
            0,
            w as i32,
            h as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut raw)),
        );
        let mut flipped = vec![0u8; raw.len()];
        let stride = w as usize * 4;
        for y in 0..h as usize {
            flipped[y * stride..(y + 1) * stride]
                .copy_from_slice(&raw[(h as usize - 1 - y) * stride..(h as usize - y) * stride]);
        }
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut encoder = png::Encoder::new(file, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .map_err(|e| e.to_string())?
            .write_image_data(&flipped)
            .map_err(|e| e.to_string())
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.targets.delete(&self.gl);
            self.gl.delete_program(self.terrain.id);
            self.gl.delete_program(self.final_pass.id);
            self.gl.delete_vertex_array(self.quad_vao);
            self.gl.delete_buffer(self.quad_vbo);
        }
    }
}

struct PlanetDraw {
    depth: u8,
    meshes: Vec<GpuMesh>,
}
unsafe fn make_planet(
    gl: &glow::Context,
    planet: &Planet,
    depth: u8,
) -> Result<PlanetDraw, String> {
    let start = Instant::now();
    let tree = TerrainOctree::generate(planet, depth);
    let meshes = mesh::mesh_terrain(&tree, planet);
    println!(
        "octree depth {depth}: {} surface chunks, {} triangles, {:?} generation + mesh",
        meshes.len(),
        meshes.iter().map(|m| m.vertices.len() / 3).sum::<usize>(),
        start.elapsed()
    );
    Ok(PlanetDraw {
        depth,
        meshes: meshes
            .iter()
            .map(|m| GpuMesh::upload(gl, m))
            .collect::<Result<_, _>>()?,
    })
}
fn visible(bounds: Bounds, camera: DVec3, view: Mat4, aspect: f32) -> bool {
    let center = view.transform_point3((bounds.center() - camera).as_vec3());
    let radius = (bounds.size * 0.8661) as f32;
    let depth = -center.z;
    let tan = (65f32.to_radians() * 0.5).tan();
    if depth + radius < 0.05 || depth - radius > 20000.0 {
        return false;
    }
    center.y.abs() <= depth * tan + radius * (1.0 + tan * tan).sqrt()
        && center.x.abs() <= depth * tan * aspect + radius * (1.0 + (tan * aspect).powi(2)).sqrt()
}
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Free,
    Walk,
    Ship,
}

pub fn run(options: Options) -> Result<(), String> {
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let attr = video.gl_attr();
    attr.set_context_profile(GLProfile::Core);
    attr.set_context_version(3, 3);
    attr.set_depth_size(24);
    attr.set_double_buffer(true);
    let mut window = video
        .window("Orbital Voxel | octree prototype", 1280, 720)
        .opengl()
        .resizable()
        .position_centered()
        .build()
        .map_err(|e| e.to_string())?;
    let _context = window.gl_create_context()?;
    let gl = unsafe {
        glow::Context::from_loader_function(|s| video.gl_get_proc_address(s) as *const _)
    };
    println!(
        "GPU: {} | OpenGL {}",
        unsafe { gl.get_parameter_string(glow::RENDERER) },
        unsafe { gl.get_parameter_string(glow::VERSION) }
    );
    let _ = video.gl_set_swap_interval(sdl2::video::SwapInterval::VSync);
    let mut renderer = unsafe { Renderer::new(gl, 1280, 720)? };
    let planets = vec![
        Planet::new(DVec3::ZERO, 96.0, 9.81),
        Planet::new(DVec3::new(280., 50., -150.), 42., 1.62),
    ];
    let mut planet_draws = planets
        .iter()
        .map(|p| unsafe { make_planet(&renderer.gl, p, 2) })
        .collect::<Result<Vec<_>, _>>()?;
    let structure = physics::example_ship();
    let structure_mesh =
        mesh::mesh_blocks(&structure, glam::IVec3::splat(-4), glam::IVec3::splat(5));
    let gpu_ship = unsafe { GpuMesh::upload(&renderer.gl, &structure_mesh)? };
    let mut physics = Physics::new(planets.clone());
    let mut ships: Vec<BodyHandle> = vec![physics.spawn_structure(
        &structure,
        glam::IVec3::splat(-4),
        glam::IVec3::splat(5),
        DVec3::new(0., 106., 0.),
    )?];
    let mut walker = Walker::new(&planets[0]);
    let mut mode = Mode::Free;
    let mut camera = DVec3::new(0., 160., 245.);
    let (mut yaw, mut pitch) = (0.0_f64, -0.57_f64);
    let mut events = sdl.event_pump()?;
    let start = Instant::now();
    let mut previous = Instant::now();
    let mut title_time = Instant::now();
    let mut frames = 0u64;
    let mut title_frames = 0;
    let mut captured = false;
    println!("WASD move; mouse look after left click; F walk/free; G ship thrust; Space/Ctrl vertical; Shift fast; E spawn; Esc release/quit");
    'running: loop {
        let now = Instant::now();
        let dt = (now - previous).as_secs_f64().min(0.1);
        previous = now;
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
                Event::MouseButtonDown {
                    mouse_btn: sdl2::mouse::MouseButton::Left,
                    ..
                } => {
                    captured = true;
                    sdl.mouse().set_relative_mouse_mode(true);
                }
                Event::MouseMotion { xrel, yrel, .. } if captured => {
                    if mode == Mode::Walk {
                        walker.yaw(&planets[0], -xrel as f64 * 0.003);
                    } else {
                        yaw -= xrel as f64 * 0.003;
                    }
                    pitch = (pitch - yrel as f64 * 0.003).clamp(-1.5, 1.5);
                }
                Event::KeyDown {
                    keycode: Some(Keycode::F),
                    repeat: false,
                    ..
                } => {
                    mode = if mode == Mode::Walk {
                        Mode::Free
                    } else {
                        Mode::Walk
                    };
                    pitch = 0.;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::G),
                    repeat: false,
                    ..
                } => {
                    mode = if mode == Mode::Ship {
                        Mode::Free
                    } else {
                        Mode::Ship
                    };
                }
                Event::KeyDown {
                    keycode: Some(Keycode::E),
                    repeat: false,
                    ..
                } if ships.len() < 64 => {
                    let p = camera + DVec3::new(0., 0., -8.);
                    ships.push(physics.spawn_structure(
                        &structure,
                        glam::IVec3::splat(-4),
                        glam::IVec3::splat(5),
                        p,
                    )?);
                }
                _ => {}
            }
        }
        let keys = events.keyboard_state();
        let axis = |a, b| {
            keys.is_scancode_pressed(a) as i32 as f64 - keys.is_scancode_pressed(b) as i32 as f64
        };
        let forward_input = axis(Scancode::W, Scancode::S);
        let strafe = axis(Scancode::D, Scancode::A);
        let vertical = axis(Scancode::Space, Scancode::LCtrl);
        let fast = keys.is_scancode_pressed(Scancode::LShift);
        let mut forward = DVec3::new(
            -yaw.sin() * pitch.cos(),
            pitch.sin(),
            -yaw.cos() * pitch.cos(),
        );
        let mut up = DVec3::Y;
        let mut thrust = None;
        match mode {
            Mode::Walk => {
                walker.walk(
                    &planets[0],
                    forward_input * dt * if fast { 22.0 } else { 6.0 },
                    strafe * dt * if fast { 22.0 } else { 6.0 },
                );
                camera = walker.position;
                up = planets[0].up(camera);
                forward = walker.forward * pitch.cos() + up * pitch.sin();
            }
            Mode::Free => {
                camera += (forward * forward_input
                    + forward.cross(up).normalize() * strafe
                    + up * vertical)
                    * dt
                    * if fast { 220.0 } else { 28.0 };
            }
            Mode::Ship => {
                thrust = Some((
                    ships[0],
                    (forward * forward_input
                        + forward.cross(up).normalize() * strafe
                        + up * vertical)
                        * if fast { 45.0 } else { 18.0 },
                ));
            }
        }
        physics.advance(dt, thrust);
        if mode == Mode::Ship {
            camera = physics.pose(ships[0]).0 - forward * 18.0 + up * 6.0;
        }
        let (w, h) = window.drawable_size();
        if w == 0 || h == 0 {
            std::thread::sleep(std::time::Duration::from_millis(16));
            continue;
        }
        unsafe {
            renderer.resize(w, h)?;
        }
        for (planet, draw) in planets.iter().zip(&mut planet_draws) {
            let next = planet::lod_depth(planet, camera, draw.depth);
            if next != draw.depth {
                let replacement = unsafe { make_planet(&renderer.gl, planet, next)? };
                for m in &draw.meshes {
                    unsafe {
                        m.delete(&renderer.gl);
                    }
                }
                *draw = replacement;
            }
        }
        let view = Mat4::look_at_rh(Vec3::ZERO, forward.as_vec3(), up.as_vec3());
        let aspect = w as f32 / h as f32;
        let projection = Mat4::perspective_rh_gl(65f32.to_radians(), aspect, 0.05, 20000.0);
        let mut draws = 0;
        let mut triangles = 0;
        unsafe {
            renderer.begin(projection, view, camera, start.elapsed().as_secs_f32());
            for planet in &planet_draws {
                for m in &planet.meshes {
                    if visible(m.bounds, camera, view, aspect) {
                        renderer.draw(
                            m,
                            view * Mat4::from_translation((m.bounds.min - camera).as_vec3()),
                        );
                        draws += 1;
                        triangles += m.count / 3;
                    }
                }
            }
            for body in &ships {
                let (position, rotation) = physics.pose(*body);
                renderer.draw(
                    &gpu_ship,
                    view * Mat4::from_rotation_translation(
                        rotation.as_quat(),
                        (position - camera).as_vec3(),
                    ),
                );
                draws += 1;
                triangles += gpu_ship.count / 3;
            }
            renderer.finish(start.elapsed().as_secs_f32(), camera);
            if options.frames.is_some_and(|n| frames + 1 >= n) {
                if let Some(path) = &options.screenshot {
                    renderer.screenshot(path)?;
                    println!("screenshot: {}", path.display());
                }
            }
            let error = renderer.gl.get_error();
            if error != glow::NO_ERROR {
                return Err(format!("OpenGL error 0x{error:x}"));
            }
        }
        window.gl_swap_window();
        frames += 1;
        title_frames += 1;
        if title_time.elapsed().as_secs_f64() >= 0.5 {
            let fps = title_frames as f64 / title_time.elapsed().as_secs_f64();
            let label = match mode {
                Mode::Free => "SPACE / FREE",
                Mode::Walk => "SURFACE WALK",
                Mode::Ship => "SHIP THRUST",
            };
            let _=window.set_title(&format!("Orbital Voxel | {label} | {fps:.0} FPS | {draws} draws | {triangles} tris | altitude {:.1} m | F walk G ship E spawn",planets[0].sdf(camera)));
            title_time = Instant::now();
            title_frames = 0;
        }
        if options.frames.is_some_and(|n| frames >= n) {
            println!("rendered {frames} frames; {draws} draws; {triangles} triangles");
            break;
        }
    }
    unsafe {
        gpu_ship.delete(&renderer.gl);
        for p in &planet_draws {
            for m in &p.meshes {
                m.delete(&renderer.gl);
            }
        }
    }
    Ok(())
}
