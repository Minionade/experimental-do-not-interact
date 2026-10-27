//! Windowed wgpu viewer with free-fly camera and streamed chunks.
//! HDR: auto-selects scRGB or HDR10/PQ output when the surface supports it; `--sdr` forces SDR.
//! Controls: click to grab mouse, WASD move, Space/Shift up/down, Ctrl sprint, Esc release/quit.
use blockengine::chunk::Chunk;
use blockengine::mesh::{mesh_chunk, Mesh, Vertex};
use blockengine::world::{level_seed, read_level_dat, World};
use blockengine::worldgen::Generator;
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

const RADIUS: i32 = 12;
const IN_FLIGHT: usize = 16;
const DEPTH_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVertex { pos: [f32; 3], color: [f32; 3] }

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms { view_proj: [[f32; 4]; 4], eye: [f32; 4], hdr: [f32; 4] }

/// How the swapchain expects pixels. Chosen once at startup from the surface's capabilities.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Output {
    /// sRGB surface; hardware applies the transfer function. Values above 1.0 clip.
    Sdr,
    /// Rgba16Float + extended linear sRGB (scRGB). 1.0 = SDR white, values above 1.0 are brighter than white.
    ScRgb,
    /// Rgb10a2Unorm + BT.2100 PQ (HDR10). Shader converts to BT.2020 and PQ-encodes absolute nits.
    Pq,
}

impl Output {
    fn mode(self) -> f32 { match self { Output::Sdr => 0.0, Output::ScRgb => 1.0, Output::Pq => 2.0 } }
}

fn pq_encode(nits: f32) -> f32 {
    let y = (nits / 10000.0).max(0.0).powf(0.1593017578125);
    ((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y)).powf(78.84375)
}

/// Mirrors the shader's output encoding, for the clear colour.
fn encode_for_output(lin: [f64; 3], out: Output, paper_white: f32) -> [f64; 3] {
    match out {
        Output::Sdr | Output::ScRgb => lin,
        Output::Pq => {
            let (r, g, b) = (lin[0] as f32 * paper_white, lin[1] as f32 * paper_white, lin[2] as f32 * paper_white);
            let c = [0.6274 * r + 0.3293 * g + 0.0433 * b, 0.0691 * r + 0.9195 * g + 0.0114 * b, 0.0164 * r + 0.0880 * g + 0.8956 * b];
            [pq_encode(c[0]) as f64, pq_encode(c[1]) as f64, pq_encode(c[2]) as f64]
        }
    }
}

const SHADER: &str = r#"
struct U { view_proj: mat4x4<f32>, eye: vec4<f32>, hdr: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct VOut { @builtin(position) p: vec4<f32>, @location(0) c: vec3<f32>, @location(1) d: f32 };
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) c: vec3<f32>) -> VOut {
    var o: VOut;
    o.p = u.view_proj * vec4<f32>(pos, 1.0);
    o.c = c;
    o.d = distance(pos, u.eye.xyz);
    return o;
}
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
    let fog = clamp(i.d / (u.eye.w), 0.0, 1.0);
    let sky = vec3<f32>(0.6, 0.75, 0.95);
    // Colours are authored in sRGB; decode to linear light. Emissive blocks exceed 1.0.
    let lin = pow(mix(i.c, sky, fog * fog), vec3<f32>(2.2));
    if (u.hdr.x < 1.5) {
        // Sdr: sRGB surface encodes for us. scRGB: linear passthrough, >1.0 is HDR.
        return vec4<f32>(lin, 1.0);
    }
    // PQ: linear (1.0 = paper white) -> nits -> BT.2020 -> ST 2084.
    let nits = lin * u.hdr.y;
    let rgb2020 = vec3<f32>(
        dot(nits, vec3<f32>(0.6274, 0.3293, 0.0433)),
        dot(nits, vec3<f32>(0.0691, 0.9195, 0.0114)),
        dot(nits, vec3<f32>(0.0164, 0.0880, 0.8956)));
    let y = pow(clamp(rgb2020 / 10000.0, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(0.1593017578125));
    let pq = pow((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y), vec3<f32>(78.84375));
    return vec4<f32>(pq, 1.0);
}
"#;

type M4 = [[f32; 4]; 4];
fn mul(a: M4, b: M4) -> M4 {
    let mut r = [[0.0; 4]; 4];
    for c in 0..4 { for rr in 0..4 { for k in 0..4 { r[c][rr] += a[k][rr] * b[c][k]; } } }
    r
}
fn view_proj(eye: [f32; 3], fwd: [f32; 3], aspect: f32) -> M4 {
    let norm = |v: [f32; 3]| { let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt(); [v[0] / l, v[1] / l, v[2] / l] };
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let f = norm(fwd);
    let s = norm(cross(f, [0.0, 1.0, 0.0]));
    let u = cross(s, f);
    let view = [[s[0], u[0], -f[0], 0.0], [s[1], u[1], -f[1], 0.0], [s[2], u[2], -f[2], 0.0],
                [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0]];
    let (near, far) = (0.1f32, 1000.0f32);
    let t = 1.0 / (70f32.to_radians() / 2.0).tan();
    let proj = [[t / aspect, 0.0, 0.0, 0.0], [0.0, t, 0.0, 0.0],
                [0.0, 0.0, far / (near - far), -1.0], [0.0, 0.0, near * far / (near - far), 0.0]];
    mul(proj, view)
}

struct ChunkGpu { vb: wgpu::Buffer, ib: wgpu::Buffer, n: u32 }

struct Gfx {
    window: Arc<Window>, surface: wgpu::Surface<'static>, device: wgpu::Device, queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration, pipeline: wgpu::RenderPipeline, depth: wgpu::TextureView,
    ubuf: wgpu::Buffer, bind: wgpu::BindGroup, output: Output,
}

struct Streamer {
    generator: Arc<Generator>,
    saved: Arc<Mutex<World>>,
    cache: Arc<Mutex<HashMap<(i32, i32), Arc<Chunk>>>>,
    tx: mpsc::Sender<(i32, i32, Mesh)>,
    rx: mpsc::Receiver<(i32, i32, Mesh)>,
    pending: HashSet<(i32, i32)>,
}

fn get_chunk(s: &Arc<Mutex<HashMap<(i32, i32), Arc<Chunk>>>>, generator: &Generator, world: &Mutex<World>, x: i32, z: i32) -> Arc<Chunk> {
    if let Some(c) = s.lock().unwrap().get(&(x, z)) { return c.clone(); }
    let loaded = { let mut w = world.lock().unwrap(); if w.load_chunk(x, z).unwrap_or(false) { w.chunks.remove(&(x, z)) } else { None } };
    let c = Arc::new(loaded.unwrap_or_else(|| generator.generate(x, z)));
    s.lock().unwrap().entry((x, z)).or_insert(c).clone()
}

struct App {
    gfx: Option<Gfx>,
    st: Streamer,
    chunks: HashMap<(i32, i32), ChunkGpu>,
    pos: [f32; 3], yaw: f32, pitch: f32,
    keys: HashSet<KeyCode>, grabbed: bool, last: Instant,
    want_hdr: bool, paper_white: f32, output: Output,
}

impl App {
    fn init_gfx(&mut self, el: &ActiveEventLoop) {
        let window = Arc::new(el.create_window(Window::default_attributes().with_title("blockengine")).unwrap());
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).unwrap();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface), ..Default::default() })).expect("no GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let size = window.inner_size();
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).unwrap();
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // Pick the best HDR output the surface offers: scRGB first (simplest, linear), then HDR10/PQ.
        let caps = surface.get_capabilities(&adapter);
        let mut output = Output::Sdr;
        if self.want_hdr {
            use wgpu::{SurfaceColorSpace as Cs, SurfaceColorSpaces as Css, TextureFormat as Tf};
            if caps.color_spaces(Tf::Rgba16Float).contains(Css::EXTENDED_SRGB_LINEAR) {
                config.format = Tf::Rgba16Float; config.color_space = Cs::ExtendedSrgbLinear; output = Output::ScRgb;
            } else if caps.color_spaces(Tf::Rgb10a2Unorm).contains(Css::BT2100_PQ) {
                config.format = Tf::Rgb10a2Unorm; config.color_space = Cs::Bt2100Pq; output = Output::Pq;
            }
        }
        eprintln!("output: {:?} ({:?}, {:?}); display info: {:?}", output, config.format, config.color_space, surface.display_hdr_info(&adapter));
        self.output = output;
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None, source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None }],
        });
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }] });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None, layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 24, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), compilation_options: Default::default(),
                targets: &[Some(config.format.into())] }),
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FMT, depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(), multiview_mask: None, cache: None,
        });
        let depth = Self::depth_view(&device, &config);
        self.gfx = Some(Gfx { window, surface, device, queue, config, pipeline, depth, ubuf, bind, output });
    }

    fn depth_view(device: &wgpu::Device, c: &wgpu::SurfaceConfiguration) -> wgpu::TextureView {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: c.width, height: c.height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: DEPTH_FMT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] }).create_view(&Default::default())
    }

    fn forward(&self) -> [f32; 3] {
        [self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), self.yaw.cos() * self.pitch.cos()]
    }

    fn update(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        let speed = if self.keys.contains(&KeyCode::ControlLeft) { 60.0 } else { 20.0 } * dt;
        let f = self.forward();
        let flat = { let l = (f[0] * f[0] + f[2] * f[2]).sqrt().max(1e-4); [f[0] / l, f[2] / l] };
        let mut d = [0.0f32; 3];
        let k = |c| self.keys.contains(&c);
        if k(KeyCode::KeyW) { d[0] += flat[0]; d[2] += flat[1]; }
        if k(KeyCode::KeyS) { d[0] -= flat[0]; d[2] -= flat[1]; }
        if k(KeyCode::KeyD) { d[0] -= flat[1]; d[2] += flat[0]; }
        if k(KeyCode::KeyA) { d[0] += flat[1]; d[2] -= flat[0]; }
        if k(KeyCode::Space) { d[1] += 1.0; }
        if k(KeyCode::ShiftLeft) { d[1] -= 1.0; }
        for i in 0..3 { self.pos[i] += d[i] * speed; }

        // Stream: drain finished meshes, request nearest missing chunks.
        let Some(g) = &self.gfx else { return };
        while let Ok((x, z, m)) = self.st.rx.try_recv() {
            self.st.pending.remove(&(x, z));
            if m.indices.is_empty() { self.chunks.remove(&(x, z)); continue; }
            let verts: Vec<GpuVertex> = m.vertices.iter().map(|v: &Vertex| GpuVertex { pos: [v.pos[0] + x as f32 * 16.0, v.pos[1], v.pos[2] + z as f32 * 16.0], color: v.color }).collect();
            let vb = g.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
            let ib = g.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&m.indices), usage: wgpu::BufferUsages::INDEX });
            self.chunks.insert((x, z), ChunkGpu { vb, ib, n: m.indices.len() as u32 });
        }
        let (pcx, pcz) = ((self.pos[0] / 16.0).floor() as i32, (self.pos[2] / 16.0).floor() as i32);
        self.chunks.retain(|&(x, z), _| (x - pcx).abs() <= RADIUS + 2 && (z - pcz).abs() <= RADIUS + 2);
        if self.st.pending.len() >= IN_FLIGHT { return; }
        let mut want: Vec<(i32, i32)> = (-RADIUS..=RADIUS).flat_map(|dx| (-RADIUS..=RADIUS).map(move |dz| (pcx + dx, pcz + dz)))
            .filter(|c| !self.chunks.contains_key(c) && !self.st.pending.contains(c)).collect();
        want.sort_by_key(|&(x, z)| (x - pcx).pow(2) + (z - pcz).pow(2));
        for (x, z) in want.into_iter().take(IN_FLIGHT - self.st.pending.len()) {
            self.st.pending.insert((x, z));
            let (generator, cache, tx, world) = (self.st.generator.clone(), self.st.cache.clone(), self.st.tx.clone(), self.st.saved.clone());
            rayon::spawn(move || {
                // Clone the 3x3 neighbourhood so light can be stitched across borders without mutating the cache.
                let mut map = blockengine::access::ChunkMap::new();
                for dx in -1..=1 { for dz in -1..=1 {
                    map.insert((x + dx, z + dz), (*get_chunk(&cache, &generator, &world, x + dx, z + dz)).clone());
                }}
                blockengine::light::stitch(&mut map, x, z);
                let n = |dx, dz| map.get(&(x + dx, z + dz));
                let m = mesh_chunk(&map[&(x, z)], [n(-1, 0), n(1, 0), n(0, -1), n(0, 1)]);
                // keep the cache bounded
                let mut cc = cache.lock().unwrap();
                if cc.len() > 4000 { cc.clear(); }
                drop(cc);
                let _ = tx.send((x, z, m));
            });
        }
    }

    fn render(&mut self) {
        let Some(g) = &self.gfx else { return };
        let aspect = g.config.width as f32 / g.config.height as f32;
        let u = Uniforms { view_proj: view_proj(self.pos, self.forward(), aspect), eye: [self.pos[0], self.pos[1], self.pos[2], (RADIUS * 16) as f32],
            hdr: [g.output.mode(), self.paper_white, 0.0, 0.0] };
        let sky = encode_for_output([0.6f64.powf(2.2), 0.75f64.powf(2.2), 0.95f64.powf(2.2)], g.output, self.paper_white);
        g.queue.write_buffer(&g.ubuf, 0, bytemuck::bytes_of(&u));
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => { g.surface.configure(&g.device, &g.config); return }
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        let mut enc = g.device.create_command_encoder(&Default::default());
        {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: sky[0], g: sky[1], b: sky[2], a: 1.0 }), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &g.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                ..Default::default()
            });
            p.set_pipeline(&g.pipeline);
            p.set_bind_group(0, &g.bind, &[]);
            for c in self.chunks.values() {
                p.set_vertex_buffer(0, c.vb.slice(..));
                p.set_index_buffer(c.ib.slice(..), wgpu::IndexFormat::Uint32);
                p.draw_indexed(0..c.n, 0, 0..1);
            }
        }
        g.queue.submit([enc.finish()]);
        g.window.pre_present_notify();
        g.queue.present(frame);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) { if self.gfx.is_none() { self.init_gfx(el); } }
    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => if let Some(g) = &mut self.gfx {
                g.config.width = s.width.max(1); g.config.height = s.height.max(1);
                g.surface.configure(&g.device, &g.config);
                g.depth = Self::depth_view(&g.device, &g.config);
            },
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => if let Some(g) = &self.gfx {
                self.grabbed = g.window.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| g.window.set_cursor_grab(CursorGrabMode::Confined)).is_ok();
                g.window.set_cursor_visible(!self.grabbed);
            },
            WindowEvent::KeyboardInput { event, .. } => if let PhysicalKey::Code(k) = event.physical_key {
                if event.state == ElementState::Pressed {
                    if k == KeyCode::Escape {
                        if self.grabbed { if let Some(g) = &self.gfx { let _ = g.window.set_cursor_grab(CursorGrabMode::None); g.window.set_cursor_visible(true); } self.grabbed = false; }
                        else { el.exit(); }
                    }
                    self.keys.insert(k);
                } else { self.keys.remove(&k); }
            },
            WindowEvent::RedrawRequested => { self.update(); self.render(); if let Some(g) = &self.gfx { g.window.request_redraw(); } }
            _ => {}
        }
    }
    fn device_event(&mut self, _: &ActiveEventLoop, _: winit::event::DeviceId, ev: DeviceEvent) {
        if let (DeviceEvent::MouseMotion { delta }, true) = (ev, self.grabbed) {
            self.yaw -= delta.0 as f32 * 0.0025;
            self.pitch = (self.pitch - delta.1 as f32 * 0.0025).clamp(-1.55, 1.55);
        }
    }
}

fn main() {
    // usage: viewer [seed | path/to/java/world] [--sdr] [--paper-white=NITS]
    let args: Vec<String> = std::env::args().skip(1).collect();
    let want_hdr = !args.iter().any(|a| a == "--sdr");
    let paper_white = args.iter().find_map(|a| a.strip_prefix("--paper-white=")?.parse().ok()).unwrap_or(203.0f32);
    let arg = args.iter().find(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "1".into());
    let path = std::path::Path::new(&arg);
    let (seed, world) = if path.join("level.dat").exists() {
        let seed = read_level_dat(path).ok().and_then(|l| level_seed(&l)).unwrap_or(0);
        eprintln!("loading Java world {arg} (seed {seed}); missing chunks use the built-in generator");
        (seed, World::open(path))
    } else { (arg.parse().unwrap_or(1), World::open("/nonexistent")) };
    let generator = Arc::new(Generator::new(seed));
    let h = generator.surface_height(0, 0).max(63) as f32;
    let (tx, rx) = mpsc::channel();
    let mut app = App {
        gfx: None, chunks: HashMap::new(), pos: [0.0, h + 20.0, 0.0], yaw: 0.0, pitch: -0.3,
        keys: HashSet::new(), grabbed: false, last: Instant::now(), want_hdr, paper_white, output: Output::Sdr,
        st: Streamer { generator, saved: Arc::new(Mutex::new(world)), cache: Arc::default(), tx, rx, pending: HashSet::new() },
    };
    let el = EventLoop::new().unwrap();
    el.set_control_flow(ControlFlow::Poll);
    el.run_app(&mut app).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pq_matches_st2084_reference_points() {
        assert!(pq_encode(0.0) < 1e-5); // the ST 2084 formula bottoms out at ~7e-7, not exactly 0
        assert!((pq_encode(10000.0) - 1.0).abs() < 1e-4);
        // 100 nits ~ 0.508, 203 nits (HDR reference white) ~ 0.581 per BT.2408.
        assert!((pq_encode(100.0) - 0.508).abs() < 1e-3);
        assert!((pq_encode(203.0) - 0.581).abs() < 2e-3);
    }
}
