//! Windowed wgpu viewer with free-fly camera and streamed chunks.
//! HDR: auto-selects scRGB or HDR10/PQ output when the surface supports it; `--sdr` forces SDR.
//! Controls: click to grab mouse; WASD move, Space jump, Shift sneak, Ctrl sprint, F fly (Space up / Shift down),
//! left click break, right click place, 1-9 or wheel select block, F5 save, T skip 6000 ticks of time, Esc release/quit.
use blockengine::access::SharedChunkMap;
use blockengine::chunk::Chunk;
use blockengine::crafting::Recipes;
use blockengine::font;
use blockengine::game::{Game, GameMode, Input, MAX_AIR};
use blockengine::item::{ItemStack, HOTBAR as HOTBAR_SLOTS};
use blockengine::mesh::{block_color, entity_model, mesh_chunk_grid, outline_mesh, overlay_mesh, Mesh, Vertex};
use blockengine::sky::{rotate_star, sky_at, stars};
use blockengine::model::Models;
use blockengine::pack::{Pack, PackStack};
use blockengine::world::{level_seed, read_level_dat, World};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
const DYN_VERTS: usize = 1 << 16;
const DEPTH_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVertex { pos: [f32; 3], color: [f32; 3], uv: [f32; 2], light: [f32; 2], alpha: f32 }

fn gv(v: &Vertex, off: [f32; 3]) -> GpuVertex {
    GpuVertex { pos: [v.pos[0] + off[0], v.pos[1] + off[1], v.pos[2] + off[2]], color: v.color, uv: v.uv, light: v.light, alpha: v.alpha }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms { view_proj: [[f32; 4]; 4], eye: [f32; 4], hdr: [f32; 4], sky: [f32; 4] }

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
struct U { view_proj: mat4x4<f32>, eye: vec4<f32>, hdr: vec4<f32>, sky: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_smp: sampler;
struct VOut {
    @builtin(position) p: vec4<f32>, @location(0) c: vec3<f32>, @location(1) d: f32,
    @location(2) uv: vec2<f32>, @location(3) l: vec2<f32>, @location(4) a: f32,
};
@vertex fn vs(@location(0) pos: vec3<f32>, @location(1) c: vec3<f32>, @location(2) uv: vec2<f32>,
              @location(3) l: vec2<f32>, @location(4) a: f32) -> VOut {
    var o: VOut;
    o.p = u.view_proj * vec4<f32>(pos, 1.0);
    o.c = c; o.uv = uv; o.l = l; o.a = a;
    o.d = distance(pos, u.eye.xyz);
    return o;
}

// Lit, fogged colour in linear light. u.sky = (sky/fog colour sRGB, daylight).
fn lit(i: VOut, t: vec4<f32>) -> vec3<f32> {
    let fog = clamp(i.d / u.eye.w, 0.0, 1.0);
    let day = u.sky.w;
    // Open sky keeps a moonlight floor at night (0.25 of full), so outdoors stays readable while caves go dark.
    let sky_l = i.l.x * (0.25 + 0.75 * day);
    let lvl = max(sky_l, i.l.y);
    let warm = clamp(i.l.y - sky_l, 0.0, 1.0);
    let moon = (1.0 - day) * i.l.x * (1.0 - warm);
    let lf = 0.15 + 0.85 * lvl;
    // Light and shade are authored in sRGB-ish space like the colours; the atlas decodes to linear on sampling.
    var tint = mix(vec3<f32>(1.0), vec3<f32>(1.0, 0.86, 0.68), warm * 0.6);
    tint = mix(tint, vec3<f32>(0.78, 0.86, 1.12), moon * 0.7);
    let shaded = i.c * lf * tint;
    return mix(pow(shaded, vec3<f32>(2.2)) * t.rgb, pow(u.sky.rgb, vec3<f32>(2.2)), fog * fog);
}

// Output encoding: SDR (sRGB surface), scRGB (linear, >1.0 is HDR) or HDR10 PQ.
fn encode(c: vec3<f32>) -> vec3<f32> {
    var lin = c;
    if (u.hdr.x < 0.5) {
        // SDR can't show >1.0: scale by the brightest channel so emissive blocks keep their hue instead of clipping to yellow.
        let m = max(max(lin.r, lin.g), lin.b);
        if (m > 1.0) { lin = lin / m; }
    }
    if (u.hdr.x < 1.5) { return lin; }
    // PQ: linear (1.0 = paper white) -> nits -> BT.2020 -> ST 2084.
    let nits = lin * u.hdr.y;
    let rgb2020 = vec3<f32>(
        dot(nits, vec3<f32>(0.6274, 0.3293, 0.0433)),
        dot(nits, vec3<f32>(0.0691, 0.9195, 0.0114)),
        dot(nits, vec3<f32>(0.0164, 0.0880, 0.8956)));
    let y = pow(clamp(rgb2020 / 10000.0, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(0.1593017578125));
    return pow((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y), vec3<f32>(78.84375));
}

@fragment fn fs_opaque(i: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(atlas, atlas_smp, i.uv);
    if (t.a < 0.1) { discard; }
    return vec4<f32>(encode(lit(i, t)), 1.0);
}

@fragment fn fs_trans(i: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(atlas, atlas_smp, i.uv);
    let a = t.a * i.a;
    if (a < 0.02) { discard; }
    return vec4<f32>(encode(lit(i, t)), a);
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

struct ChunkGpu { vb: wgpu::Buffer, ib: wgpu::Buffer, n: u32, ib_t: Option<wgpu::Buffer>, n_t: u32 }

const SKY_STARS: usize = 300;
const HUD_VERTS: usize = 1 << 16;

struct Gfx {
    window: Arc<Window>, surface: wgpu::Surface<'static>, device: wgpu::Device, queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration, pipeline: wgpu::RenderPipeline, depth: wgpu::TextureView,
    ubuf: wgpu::Buffer, bind: wgpu::BindGroup, ubuf_ui: wgpu::Buffer, bind_ui: wgpu::BindGroup, output: Output,
    atlas_tex: wgpu::Texture, atlas_bind: wgpu::BindGroup,
    pipeline_t: wgpu::RenderPipeline, ubuf_sky: wgpu::Buffer, bind_sky: wgpu::BindGroup, sky_vb: wgpu::Buffer, sky_ib: wgpu::Buffer,
    ovl_vb: wgpu::Buffer, ovl_ib: wgpu::Buffer, hud_vb: wgpu::Buffer, hud_ib: wgpu::Buffer,
    dyn_vb: wgpu::Buffer, dyn_ib: wgpu::Buffer, ui_vb: wgpu::Buffer, ui_ib: wgpu::Buffer,
}

enum Msg {
    Chunk(Chunk),
    Mesh { x: i32, z: i32, seq: u64, mesh: Mesh },
}

/// Where chunks come from and where edits go.
struct Source { read: Arc<Mutex<World>>, save_dir: Option<PathBuf> }

/// Creative-mode hotbar contents (name, block id, props, is_fluid).
const CREATIVE_HOTBAR: [(&str, &str, &str, u8); 9] = [
    ("Stone", "minecraft:stone", "", 0), ("Dirt", "minecraft:dirt", "", 0), ("Grass Block", "minecraft:grass_block", "", 0),
    ("Oak Log", "minecraft:oak_log", "axis=y", 0), ("Oak Leaves", "minecraft:oak_leaves", "distance=1,persistent=true,waterlogged=false", 0),
    ("Sand", "minecraft:sand", "", 0), ("Glowstone", "minecraft:glowstone", "", 0),
    ("Water", "minecraft:water", "", 1), ("Lava", "minecraft:lava", "", 1),
];

struct App {
    gfx: Option<Gfx>,
    game: Game,
    src: Source,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    pending_gen: HashSet<(i32, i32)>,
    mesh_seq: HashMap<(i32, i32), u64>,
    next_seq: u64,
    in_flight_meshes: usize,
    chunks: HashMap<(i32, i32), ChunkGpu>,
    yaw: f32, pitch: f32,
    keys: HashSet<KeyCode>, grabbed: bool, last: Instant,
    want_hdr: bool, paper_white: f32, output: Output,
    models: Arc<Models>, textured: bool, anim_state: Vec<u64>, t0: Instant,
    recipes: Arc<Recipes>, screen_open: bool, craft_sel: usize, fps: f32, name_t: f32, last_sel: usize,
    mouse_left: bool, mouse_right: bool, left_hold: f32, click_right: bool, act_cd: f32, title_t: f32,
}

/// "minecraft:oak_planks" -> "oak planks"
fn pretty(name: &str) -> String { name.strip_prefix("minecraft:").unwrap_or(name).replace('_', " ") }

/// Immediate-mode HUD geometry in pixel coordinates (origin top-left), emitted as NDC quads.
struct Hud { v: Vec<GpuVertex>, i: Vec<u32>, w: f32, h: f32, white: [f32; 2] }

impl Hud {
    fn new(w: f32, h: f32, white: [f32; 2]) -> Self { Self { v: Vec::new(), i: Vec::new(), w, h, white } }
    fn quad(&mut self, x: f32, y: f32, w: f32, h: f32, uv: (f32, f32, f32, f32), color: [f32; 3], alpha: f32) {
        if self.v.len() + 4 > HUD_VERTS { return; }
        let ndc = |px: f32, py: f32| [px / self.w * 2.0 - 1.0, 1.0 - py / self.h * 2.0, 0.0];
        let base = self.v.len() as u32;
        // BL, BR, TR, TL: counter-clockwise in NDC.
        for (px, py, u, vv) in [(x, y + h, uv.0, uv.3), (x + w, y + h, uv.2, uv.3), (x + w, y, uv.2, uv.1), (x, y, uv.0, uv.1)] {
            self.v.push(GpuVertex { pos: ndc(px, py), color, uv: [u, vv], light: [1.0, 1.0], alpha });
        }
        self.i.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 3], alpha: f32) {
        let u = self.white;
        self.quad(x, y, w, h, (u[0], u[1], u[0], u[1]), color, alpha);
    }
    /// Text with a one-pixel drop shadow. `px` is the on-screen size of one glyph pixel.
    fn text(&mut self, x: f32, y: f32, px: f32, color: [f32; 3], s: &str) {
        let pixels = font::text_pixels(s);
        for (dx, dy, c) in [(px, px, [color[0] * 0.2, color[1] * 0.2, color[2] * 0.2]), (0.0, 0.0, color)] {
            for &(gx, gy) in &pixels { self.rect(x + dx + gx as f32 * px, y + dy + gy as f32 * px, px, px, c, 1.0); }
        }
    }
    fn text_centered(&mut self, cx: f32, y: f32, px: f32, color: [f32; 3], s: &str) {
        self.text(cx - font::text_width(s) as f32 * px / 2.0, y, px, color, s);
    }
}

impl App {
    /// Draws an item's icon: its pack texture if it has one, otherwise a coloured square with its initial.
    fn icon(&self, hud: &mut Hud, name: &str, x: f32, y: f32, size: f32) {
        if let Some(r) = self.models.item_icon(name) { hud.quad(x, y, size, size, r, [1.0; 3], 1.0); return; }
        let c = block_color(name);
        hud.rect(x, y, size, size, [c[0] * 0.55, c[1] * 0.55, c[2] * 0.55], 1.0);
        hud.rect(x + size * 0.08, y + size * 0.08, size * 0.84, size * 0.84, c, 1.0);
        let letter: String = pretty(name).chars().next().into_iter().collect();
        let px = (size / 10.0).max(1.0);
        hud.text(x + size / 2.0 - 2.5 * px, y + size / 2.0 - 3.5 * px, px, [1.0, 1.0, 1.0], &letter);
    }

    fn slot(&self, hud: &mut Hud, stack: Option<&ItemStack>, x: f32, y: f32, gui: f32, selected: bool) {
        let s = 20.0 * gui;
        if selected { hud.rect(x - gui, y - gui, s + 2.0 * gui, s + 2.0 * gui, [1.0, 1.0, 1.0], 0.95); }
        // Slots sit inside their cell with a one-pixel gutter so a grid reads as separate squares.
        hud.rect(x + 0.5 * gui, y + 0.5 * gui, s - gui, s - gui, [0.16, 0.16, 0.16], 0.8);
        if let Some(st) = stack {
            self.icon(hud, &st.name, x + 2.0 * gui, y + 2.0 * gui, 16.0 * gui);
            if st.count > 1 {
                let t = st.count.to_string();
                let px = gui;
                hud.text(x + s - font::text_width(&t) as f32 * px - gui, y + s - 8.0 * px, px, [1.0, 1.0, 1.0], &t);
            }
        }
    }

    fn build_hud(&self, w: f32, h: f32) -> Hud {
        let mut hud = Hud::new(w, h, self.models.atlas.white_uv());
        let gui = (w / 320.0).min(h / 240.0).floor().max(1.0);
        let slot = 20.0 * gui;
        let (bx, by) = ((w - HOTBAR_SLOTS as f32 * slot) / 2.0, h - slot - 2.0 * gui);
        let inv = &self.game.inventory;

        // Hotbar.
        hud.rect(bx - 2.0 * gui, by - 2.0 * gui, HOTBAR_SLOTS as f32 * slot + 4.0 * gui, slot + 4.0 * gui, [0.0, 0.0, 0.0], 0.45);
        for i in 0..HOTBAR_SLOTS { self.slot(&mut hud, inv.slots[i].as_ref(), bx + i as f32 * slot, by, gui, i == inv.selected); }

        if self.name_t > 0.0 {
            if let Some(st) = inv.selected_stack() {
                let ny = by - (if self.game.mode == GameMode::Survival { 36.0 } else { 16.0 }) * gui;
                hud.text_centered(w / 2.0, ny, gui, [1.0, 1.0, 1.0], &pretty(&st.name));
            }
        }

        // Hearts and breath (survival only).
        if self.game.mode == GameMode::Survival {
            let hy = by - 12.0 * gui;
            for k in 0..10 {
                let hp = self.game.health - 2.0 * k as f32;
                let (hx, flash) = (bx + k as f32 * 8.0 * gui, self.game.since_damage < 10 && self.game.since_damage % 4 < 2);
                for (row, bits) in font::HEART.iter().enumerate() {
                    for col in 0..9 {
                        if bits & (1 << (8 - col)) == 0 { continue; }
                        let full = hp >= 2.0 || (hp >= 1.0 && col < 5);
                        let c = if flash { [1.0, 0.8, 0.8] } else if full { [0.9, 0.12, 0.12] } else { [0.22, 0.05, 0.05] };
                        hud.rect(hx + col as f32 * gui * 0.85, hy + row as f32 * gui * 0.85, gui * 0.85, gui * 0.85, c, 1.0);
                    }
                }
            }
            if self.game.air < MAX_AIR {
                let n = (self.game.air as f32 / 30.0).ceil() as i32;
                for k in 0..n.min(10) {
                    let (x, y) = (bx + HOTBAR_SLOTS as f32 * slot - (k + 1) as f32 * 8.0 * gui, hy - 10.0 * gui);
                    hud.rect(x, y, 7.0 * gui, 7.0 * gui, [0.35, 0.6, 0.95], 1.0);
                    hud.rect(x + 2.0 * gui, y + 2.0 * gui, 2.0 * gui, 2.0 * gui, [0.85, 0.95, 1.0], 1.0);
                }
            }
            if self.game.since_damage < 10 {
                hud.rect(0.0, 0.0, w, h, [0.85, 0.0, 0.0], 0.35 * (1.0 - self.game.since_damage as f32 / 10.0));
            }
        }

        // Debug text.
        let p = self.game.player_pos();
        let clock = (self.game.time % 24000) as f64 / 1000.0 + 6.0;
        let lines = [format!("XYZ {:.1} {:.1} {:.1}", p[0], p[1], p[2]), format!("{:?} {:02}:{:02}", self.game.mode, (clock as u32) % 24, (clock.fract() * 60.0) as u32),
            format!("FPS {:.0}", self.fps)];
        for (i, l) in lines.iter().enumerate() { hud.text(4.0 * gui, (4.0 + i as f32 * 9.0) * gui, gui, [1.0, 1.0, 1.0], l); }

        if self.screen_open { self.build_inventory_screen(&mut hud, gui); }
        hud
    }

    fn build_inventory_screen(&self, hud: &mut Hud, gui: f32) {
        let (w, h) = (hud.w, hud.h);
        let slot = 20.0 * gui;
        hud.rect(0.0, 0.0, w, h, [0.0, 0.0, 0.0], 0.55);
        let (gx, gy) = ((w * 0.05).max(8.0 * gui), (h * 0.18).max(20.0 * gui));
        hud.text(gx, gy - 12.0 * gui, gui * 1.2, [1.0, 1.0, 1.0], "INVENTORY");
        let inv = &self.game.inventory;
        for row in 0..3 { for col in 0..9 {
            self.slot(hud, inv.slots[9 + row * 9 + col].as_ref(), gx + col as f32 * slot, gy + row as f32 * slot, gui, false);
        }}
        for col in 0..HOTBAR_SLOTS { self.slot(hud, inv.slots[col].as_ref(), gx + col as f32 * slot, gy + 3.0 * slot + 6.0 * gui, gui, col == inv.selected); }

        // Crafting list: everything the inventory can pay for right now.
        let cx = gx + 9.0 * slot + 16.0 * gui;
        let craftable = self.recipes.craftable(inv);
        hud.text(cx, gy - 12.0 * gui, gui * 1.2, [1.0, 1.0, 1.0], &format!("CRAFTING ({})", craftable.len()));
        let row_h = 22.0 * gui;
        let visible = (((h - gy - 20.0 * gui) / row_h).floor() as usize).clamp(1, 12);
        let sel = self.craft_sel.min(craftable.len().saturating_sub(1));
        let scroll = sel.saturating_sub(visible - 1);
        if craftable.is_empty() { hud.text(cx, gy, gui, [0.75, 0.75, 0.75], "NOTHING TO CRAFT YET"); }
        for (n, rec) in craftable.iter().enumerate().skip(scroll).take(visible) {
            let y = gy + (n - scroll) as f32 * row_h;
            if n == sel { hud.rect(cx - 2.0 * gui, y - 1.0 * gui, w - cx - 8.0 * gui, row_h - 2.0 * gui, [0.35, 0.55, 0.95], 0.6); }
            self.icon(hud, &rec.result.name, cx, y, 16.0 * gui);
            let title = format!("{}X {}", rec.result.count, pretty(&rec.result.name));
            hud.text(cx + 20.0 * gui, y, gui, [1.0, 1.0, 1.0], &title);
            let needs = rec.ingredient_summary().iter().map(|(c, n)| format!("{c} {}", n.replace('_', " "))).collect::<Vec<_>>().join(", ");
            hud.text(cx + 20.0 * gui, y + 9.0 * gui, gui * 0.6, [0.8, 0.8, 0.8], &needs);
        }
        hud.text(gx, gy + 4.0 * slot + 14.0 * gui, gui * 0.8, [0.85, 0.85, 0.85], "UP/DOWN SELECT   ENTER CRAFT   Q DROP   E OR ESC CLOSE");
    }

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
        let mk_ubuf = || device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let (ubuf, ubuf_ui, ubuf_sky) = (mk_ubuf(), mk_ubuf(), mk_ubuf());
        let mk_bind = |b: &wgpu::Buffer| device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() }] });
        let (bind, bind_ui, bind_sky) = (mk_bind(&ubuf), mk_bind(&ubuf_ui), mk_bind(&ubuf_sky));
        // Block atlas: nearest-sampled, sRGB. Animated tiles are rewritten in place as the tick advances.
        let a = &self.models.atlas;
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: a.width, height: a.height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &atlas_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &a.pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(a.width * 4), rows_per_image: Some(a.height) },
            wgpu::Extent3d { width: a.width, height: a.height, depth_or_array_layers: 1 });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Nearest, min_filter: wgpu::FilterMode::Nearest, ..Default::default() });
        let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, count: None,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false } },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, count: None, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering) },
        ] });
        let atlas_view = atlas_tex.create_view(&Default::default());
        let atlas_bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &tex_bgl, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&atlas_view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
        ] });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None, bind_group_layouts: &[Some(&bgl), Some(&tex_bgl)], immediate_size: 0 });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x2, 4 => Float32];
        let make_pipeline = |fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None, layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 44, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: config.format, blend, write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FMT, depth_write_enabled: Some(depth_write),
                depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(), multiview_mask: None, cache: None,
        });
        let pipeline = make_pipeline("fs_opaque", None, true);
        // Translucent geometry blends over what is behind it and does not write depth.
        let pipeline_t = make_pipeline("fs_trans", Some(wgpu::BlendState::ALPHA_BLENDING), false);
        let depth = Self::depth_view(&device, &config);
        let mk_buf = |size: u64, usage| device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage, mapped_at_creation: false });
        let vu = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST;
        let iu = wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST;
        let (dyn_vb, dyn_ib) = (mk_buf((DYN_VERTS * 44) as u64, vu), mk_buf((DYN_VERTS * 6 * 4) as u64, iu));
        let (sky_vb, sky_ib) = (mk_buf((44 * 4 * (SKY_STARS + 2)) as u64, vu), mk_buf((4 * 6 * (SKY_STARS + 2)) as u64, iu));
        let (ovl_vb, ovl_ib) = (mk_buf(44 * 64, vu), mk_buf(4 * 64, iu));
        let (hud_vb, hud_ib) = (mk_buf((44 * HUD_VERTS) as u64, vu), mk_buf((HUD_VERTS * 6 / 4 * 4) as u64, iu));
        let (ui_vb, ui_ib) = (mk_buf(44 * 16, vu), mk_buf(4 * 16, iu));
        self.gfx = Some(Gfx { window, surface, device, queue, config, pipeline, depth, ubuf, bind, ubuf_ui, bind_ui, output, atlas_tex, atlas_bind,
            pipeline_t, ubuf_sky, bind_sky, sky_vb, sky_ib,
            ovl_vb, ovl_ib, hud_vb, hud_ib,
            dyn_vb, dyn_ib, ui_vb, ui_ib });
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

    fn pos(&self) -> [f32; 3] { let e = self.game.eye(); [e[0] as f32, e[1] as f32, e[2] as f32] }

    fn key(&self, k: KeyCode) -> bool { self.keys.contains(&k) }

    fn save(&mut self) {
        let Some(dir) = self.src.save_dir.clone() else { eprintln!("world is read-only (pass --write to save into a Java world)"); return };
        match self.game.save(&dir) {
            Ok(n) => eprintln!("saved {n} chunks to {}", dir.display()),
            Err(e) => eprintln!("save failed: {e}"),
        }
    }

    fn update(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.25);
        self.last = now;

        self.fps = self.fps * 0.9 + (1.0 / dt.max(1e-4)) * 0.1;
        if self.game.inventory.selected != self.last_sel { self.last_sel = self.game.inventory.selected; self.name_t = 2.0; }
        self.name_t = (self.name_t - dt).max(0.0);

        // Input -> player. Menus swallow movement and clicks.
        let menu = self.screen_open;
        let input = if menu { Input::default() } else { Input { forward: self.key(KeyCode::KeyW), back: self.key(KeyCode::KeyS), left: self.key(KeyCode::KeyA),
            right: self.key(KeyCode::KeyD), jump: self.key(KeyCode::Space), sneak: self.key(KeyCode::ShiftLeft),
            sprint: self.key(KeyCode::ControlLeft) } };
        self.game.set_input(input, self.yaw as f64);

        // Mining: hold the left button on a block. A click shorter than a frame still holds for a moment so a tick sees it.
        let f = self.forward();
        let target = self.game.target([f[0] as f64, f[1] as f64, f[2] as f64]);
        self.left_hold = (self.left_hold - dt).max(0.0);
        let holding = self.grabbed && !menu && (self.mouse_left || self.left_hold > 0.0);
        self.game.set_mining(if holding { target.map(|h| h.pos) } else { None });
        self.game.update(dt as f64);

        // Placing: right click, rate limited while held.
        self.act_cd -= dt;
        let fresh = std::mem::take(&mut self.click_right);
        if self.grabbed && !menu && (self.mouse_right || fresh) && (self.act_cd <= 0.0 || fresh) {
            if let Some(hit) = target {
                let at = [hit.pos[0] + hit.normal[0], hit.pos[1] + hit.normal[1], hit.pos[2] + hit.normal[2]];
                if self.game.place_from_inventory(at) { self.act_cd = 0.2; }
            }
        }

        self.title_t -= dt;
        if self.title_t <= 0.0 {
            self.title_t = 0.5;
            if let Some(g) = &self.gfx {
                let p = self.game.player_pos();
                // Tick 0 is sunrise = 06:00 on the in-game clock.
                let clock = (self.game.time % 24000) as f64 / 1000.0 + 6.0;
                let held = self.game.inventory.selected_stack().map_or("empty hand".to_string(), |s| pretty(&s.name));
                g.window.set_title(&format!("blockengine | {:?} | {} | {:.0} {:.0} {:.0} | {:02}:{:02} | {} chunks",
                    self.game.mode, held, p[0], p[1], p[2], (clock as u32) % 24, ((clock.fract()) * 60.0) as u32, self.chunks.len()));
            }
        }
        self.animate_textures();
        self.stream();
    }

    /// Rewrites animated atlas tiles whose frame changed (animations run on a 20 Hz client tick).
    fn animate_textures(&mut self) {
        let Some(g) = &self.gfx else { return };
        let tick = (self.t0.elapsed().as_secs_f64() * 20.0) as u64;
        let anims = &self.models.atlas.animations;
        if self.anim_state.len() != anims.len() { self.anim_state = vec![u64::MAX; anims.len()]; }
        let tile = self.models.atlas.tile;
        for (a, last) in anims.iter().zip(self.anim_state.iter_mut()) {
            let st = a.state_at(tick);
            if *last == st { continue; }
            *last = st;
            g.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &g.atlas_tex, mip_level: 0, origin: wgpu::Origin3d { x: a.origin.0, y: a.origin.1, z: 0 }, aspect: wgpu::TextureAspect::All },
                &a.pixels_at(tick),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(tile * 4), rows_per_image: Some(tile) },
                wgpu::Extent3d { width: tile, height: tile, depth_or_array_layers: 1 });
        }
    }

    /// Chunk loading, meshing jobs, GPU uploads and unloading.
    fn stream(&mut self) {
        let Some(g) = &self.gfx else { return };
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Chunk(c) => { self.pending_gen.remove(&(c.x, c.z)); self.game.insert_chunk(c); }
                Msg::Mesh { x, z, seq, mesh } => {
                    self.in_flight_meshes -= 1;
                    if self.mesh_seq.get(&(x, z)) != Some(&seq) { continue; } // superseded by a newer job
                    if mesh.indices.is_empty() && mesh.translucent.is_empty() { self.chunks.remove(&(x, z)); continue; }
                    let off = [x as f32 * 16.0, 0.0, z as f32 * 16.0];
                    let verts: Vec<GpuVertex> = mesh.vertices.iter().map(|v: &Vertex| gv(v, off)).collect();
                    let vb = g.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
                    let mk_ib = |idx: &[u32]| g.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(idx), usage: wgpu::BufferUsages::INDEX });
                    let ib = mk_ib(&mesh.indices);
                    let ib_t = (!mesh.translucent.is_empty()).then(|| mk_ib(&mesh.translucent));
                    self.chunks.insert((x, z), ChunkGpu { vb, ib, n: mesh.indices.len() as u32, ib_t, n_t: mesh.translucent.len() as u32 });
                }
            }
        }
        let p = self.game.player_pos();
        let (pcx, pcz) = ((p[0] / 16.0).floor() as i32, (p[2] / 16.0).floor() as i32);
        let dist = |&(x, z): &(i32, i32)| (x - pcx).pow(2) + (z - pcz).pow(2);

        // Unload far chunks (never ones holding unsaved edits).
        let far = |&(x, z): &(i32, i32)| (x - pcx).abs() > RADIUS + 3 || (z - pcz).abs() > RADIUS + 3;
        let drop: Vec<_> = self.game.blocks.chunks.keys().filter(|k| far(k) && !self.game.blocks.modified.contains(*k)).copied().collect();
        for k in drop { self.game.blocks.chunks.remove(&k); self.chunks.remove(&k); self.mesh_seq.remove(&k); self.game.blocks.dirty.remove(&k); }
        self.chunks.retain(|k, _| !far(k));

        // Request chunks out to RADIUS+1 so the visible ring always has all 8 neighbours.
        if self.pending_gen.len() < IN_FLIGHT {
            let r = RADIUS + 1;
            let mut want: Vec<(i32, i32)> = (-r..=r).flat_map(|dx| (-r..=r).map(move |dz| (pcx + dx, pcz + dz)))
                .filter(|c| !self.game.blocks.chunks.contains_key(c) && !self.pending_gen.contains(c)).collect();
            want.sort_by_key(dist);
            for (x, z) in want.into_iter().take(IN_FLIGHT - self.pending_gen.len()) {
                self.pending_gen.insert((x, z));
                let (generator, tx, read) = (self.game.generator.clone(), self.tx.clone(), self.src.read.clone());
                rayon::spawn(move || {
                    let loaded = { let mut w = read.lock().unwrap(); if w.load_chunk(x, z).unwrap_or(false) { w.chunks.remove(&(x, z)) } else { None } };
                    let c = match loaded {
                        Some(mut c) => { blockengine::light::compute(&mut c); c }
                        None => generator.generate(x, z),
                    };
                    let _ = tx.send(Msg::Chunk(c));
                });
            }
        }

        // Mesh dirty chunks (nearest first) once all 8 neighbours are present.
        let mut todo: Vec<(i32, i32)> = self.game.blocks.dirty.iter().copied()
            .filter(|c| self.game.blocks.chunks.contains_key(c) && dist(c) <= RADIUS * RADIUS).collect();
        todo.sort_by_key(dist);
        self.game.blocks.dirty.retain(|c| self.game.blocks.chunks.contains_key(c));
        for (x, z) in todo {
            if self.in_flight_meshes >= IN_FLIGHT { break; }
            let snapshot: Vec<_> = (-1..=1).flat_map(|dx| (-1..=1).map(move |dz| (x + dx, z + dz)))
                .filter_map(|k| self.game.blocks.chunks.get(&k).map(|c| (k, c.clone()))).collect();
            if snapshot.len() < 9 { continue; } // stays dirty until neighbours arrive
            self.game.blocks.dirty.remove(&(x, z));
            self.next_seq += 1;
            let seq = self.next_seq;
            self.mesh_seq.insert((x, z), seq);
            self.in_flight_meshes += 1;
            let (tx, models) = (self.tx.clone(), self.textured.then(|| self.models.clone()));
            rayon::spawn(move || {
                // Private copy-on-write view so stitching light never disturbs the live world.
                let mut map: SharedChunkMap = snapshot.into_iter().collect();
                blockengine::light::stitch(&mut map, x, z);
                let grid: [[Option<&blockengine::chunk::Chunk>; 3]; 3] =
                    std::array::from_fn(|i| std::array::from_fn(|j| map.get(&(x + i as i32 - 1, z + j as i32 - 1)).map(|c| &**c)));
                let mesh = mesh_chunk_grid(grid, models.as_deref(), true);
                let _ = tx.send(Msg::Mesh { x, z, seq, mesh });
            });
        }
    }

    /// Sun, moon and stars as camera-facing quads far from the eye. Drawn first so the world covers them.
    fn sky_mesh(&self, eye: [f32; 3], sky: &blockengine::sky::Sky) -> (Vec<GpuVertex>, Vec<u32>) {
        let (mut v, mut idx): (Vec<GpuVertex>, Vec<u32>) = (Vec::new(), Vec::new());
        let mut quad = |dir: [f32; 3], dist: f32, size: f32, color: [f32; 3]| {
            let up_ref = if dir[1].abs() > 0.99 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
            let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
            let norm = |a: [f32; 3]| { let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt(); [a[0] / l, a[1] / l, a[2] / l] };
            let right = norm(cross(dir, up_ref));
            let up = cross(right, dir);
            let c = [eye[0] + dir[0] * dist, eye[1] + dir[1] * dist, eye[2] + dir[2] * dist];
            let base = v.len() as u32;
            for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = [c[0] + (right[0] * sx + up[0] * sy) * size, c[1] + (right[1] * sx + up[1] * sy) * size, c[2] + (right[2] * sx + up[2] * sy) * size];
                v.push(GpuVertex { pos: p, color, uv: [0.0, 0.0], light: [1.0, 1.0], alpha: 1.0 });
            }
            idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        };
        // Sun is HDR-bright; moon is pale. Both are flat squares like the originals' billboards.
        quad(sky.sun, 450.0, 45.0, [2.0, 1.9, 1.5]);
        quad(sky.moon, 450.0, 36.0, [0.8, 0.85, 1.0]);
        if sky.stars > 0.02 {
            let t = self.game.time as f64;
            for (i, s) in stars(SKY_STARS, 42).into_iter().enumerate() {
                let d = rotate_star(s, t);
                if d[1] < -0.05 { continue; }
                let twinkle = 0.55 + 0.45 * ((i * 7919) % 100) as f32 / 100.0;
                quad(d, 440.0, 1.1 + (i % 3) as f32 * 0.5, [sky.stars * twinkle; 3]);
            }
        }
        (v, idx)
    }

    fn render(&mut self) {
        let Some(g) = &self.gfx else { return };
        let aspect = g.config.width as f32 / g.config.height as f32;
        let eye = self.pos();
        let sky = sky_at(self.game.time as f64);
        let hdr = [g.output.mode(), self.paper_white, 0.0, 0.0];
        let sky_u = [sky.color[0], sky.color[1], sky.color[2], sky.daylight];
        let vp = view_proj(eye, self.forward(), aspect);
        g.queue.write_buffer(&g.ubuf, 0, bytemuck::bytes_of(&Uniforms { view_proj: vp, eye: [eye[0], eye[1], eye[2], (RADIUS * 16) as f32], hdr, sky: sky_u }));
        // Sky pass: real camera matrix, fog disabled by a huge fog distance.
        g.queue.write_buffer(&g.ubuf_sky, 0, bytemuck::bytes_of(&Uniforms { view_proj: vp, eye: [eye[0], eye[1], eye[2], 1e9], hdr, sky: sky_u }));
        // UI pass uses clip-space vertices: identity matrix, fog disabled.
        let ident = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        g.queue.write_buffer(&g.ubuf_ui, 0, bytemuck::bytes_of(&Uniforms { view_proj: ident, eye: [0.0, 0.0, 0.0, 1e9], hdr, sky: sky_u }));

        let (sky_v, sky_i) = self.sky_mesh(eye, &sky);
        g.queue.write_buffer(&g.sky_vb, 0, bytemuck::cast_slice(&sky_v));
        g.queue.write_buffer(&g.sky_ib, 0, bytemuck::cast_slice(&sky_i));

        // Entities + block outline, rebuilt every frame. Entities take the light of the cell they stand in.
        let mut dynm = Mesh::default();
        for e in &self.game.entities.entities {
            if e.id == self.game.player { continue; }
            let (sk, bl) = self.game.light_at(e.pos[0].floor() as i32, (e.pos[1] + 0.5).floor() as i32, e.pos[2].floor() as i32).unwrap_or((15, 0));
            let mut m = entity_model(e);
            for v in &mut m.vertices { v.light = [sk as f32 / 15.0, bl as f32 / 15.0]; }
            let base = dynm.vertices.len() as u32;
            dynm.vertices.extend(m.vertices);
            dynm.indices.extend(m.indices.into_iter().map(|i| i + base));
        }
        let f = self.forward();
        if let Some(hit) = self.game.target([f[0] as f64, f[1] as f64, f[2] as f64]) {
            let mut m = outline_mesh(hit.pos);
            for v in &mut m.vertices { v.light = [1.0, 1.0]; }
            let base = dynm.vertices.len() as u32;
            dynm.vertices.extend(m.vertices);
            dynm.indices.extend(m.indices.into_iter().map(|i| i + base));
        }
        dynm.vertices.truncate(DYN_VERTS);
        dynm.indices.retain(|&i| (i as usize) < dynm.vertices.len());
        dynm.indices.truncate(dynm.indices.len() / 3 * 3);
        let dverts: Vec<GpuVertex> = dynm.vertices.iter().map(|v| gv(v, [0.0; 3])).collect();
        g.queue.write_buffer(&g.dyn_vb, 0, bytemuck::cast_slice(&dverts));
        g.queue.write_buffer(&g.dyn_ib, 0, bytemuck::cast_slice(&dynm.indices));
        // Crosshair: two quads in NDC, aspect-corrected.
        let (a, t) = (0.018f32, 0.0025f32);
        let ax = a / aspect; let tx = t / aspect;
        let white = [1.0f32, 1.0, 1.0];
        let quad = |x0: f32, y0: f32, x1: f32, y1: f32| [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        let mut ui: Vec<GpuVertex> = Vec::new();
        for q in [quad(-ax, -t, ax, t), quad(-tx, -a, tx, a)] { for p in q { ui.push(GpuVertex { pos: [p[0], p[1], 0.0], color: white, uv: [0.0, 0.0], light: [1.0, 1.0], alpha: 1.0 }); } }
        g.queue.write_buffer(&g.ui_vb, 0, bytemuck::cast_slice(&ui));
        g.queue.write_buffer(&g.ui_ib, 0, bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]));

        // Crack overlay on the block being mined.
        let mut ovl_n = 0u32;
        if let Some((pos, p)) = self.game.mining_progress() {
            let id = format!("block/destroy_stage_{}", ((p * 10.0) as usize).min(9));
            let m = if self.models.atlas.contains(&id) { overlay_mesh(pos, Some(self.models.atlas.rect(&id)), 1.0) } else { overlay_mesh(pos, None, p * 0.55) };
            let verts: Vec<GpuVertex> = m.vertices.iter().map(|v| gv(v, [0.0; 3])).collect();
            g.queue.write_buffer(&g.ovl_vb, 0, bytemuck::cast_slice(&verts));
            g.queue.write_buffer(&g.ovl_ib, 0, bytemuck::cast_slice(&m.translucent));
            ovl_n = m.translucent.len() as u32;
        }
        let hud = self.build_hud(g.config.width as f32, g.config.height as f32);
        g.queue.write_buffer(&g.hud_vb, 0, bytemuck::cast_slice(&hud.v));
        g.queue.write_buffer(&g.hud_ib, 0, bytemuck::cast_slice(&hud.i));

        let clear = encode_for_output([(sky.color[0] as f64).powf(2.2), (sky.color[1] as f64).powf(2.2), (sky.color[2] as f64).powf(2.2)], g.output, self.paper_white);
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
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0], g: clear[1], b: clear[2], a: 1.0 }), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &g.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                ..Default::default()
            });
            p.set_pipeline(&g.pipeline);
            p.set_bind_group(1, &g.atlas_bind, &[]);
            // 1. Sky bodies (far away, covered by everything else).
            p.set_bind_group(0, &g.bind_sky, &[]);
            p.set_vertex_buffer(0, g.sky_vb.slice(..));
            p.set_index_buffer(g.sky_ib.slice(..), wgpu::IndexFormat::Uint32);
            p.draw_indexed(0..sky_i.len() as u32, 0, 0..1);
            // 2. Opaque and cutout world geometry, entities and the selection outline.
            p.set_bind_group(0, &g.bind, &[]);
            for c in self.chunks.values() {
                if c.n == 0 { continue; }
                p.set_vertex_buffer(0, c.vb.slice(..));
                p.set_index_buffer(c.ib.slice(..), wgpu::IndexFormat::Uint32);
                p.draw_indexed(0..c.n, 0, 0..1);
            }
            if !dynm.indices.is_empty() {
                p.set_vertex_buffer(0, g.dyn_vb.slice(..));
                p.set_index_buffer(g.dyn_ib.slice(..), wgpu::IndexFormat::Uint32);
                p.draw_indexed(0..dynm.indices.len() as u32, 0, 0..1);
            }
            // 3. Translucent geometry (water, ice, stained glass), far chunks first so blending composes correctly.
            p.set_pipeline(&g.pipeline_t);
            let (ex, ez) = (eye[0] / 16.0, eye[2] / 16.0);
            let mut order: Vec<(f32, &ChunkGpu)> = self.chunks.iter().filter(|(_, c)| c.n_t > 0)
                .map(|(&(x, z), c)| ((x as f32 + 0.5 - ex).powi(2) + (z as f32 + 0.5 - ez).powi(2), c)).collect();
            order.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (_, c) in order {
                p.set_vertex_buffer(0, c.vb.slice(..));
                p.set_index_buffer(c.ib_t.as_ref().unwrap().slice(..), wgpu::IndexFormat::Uint32);
                p.draw_indexed(0..c.n_t, 0, 0..1);
            }
            if ovl_n > 0 {
                p.set_vertex_buffer(0, g.ovl_vb.slice(..));
                p.set_index_buffer(g.ovl_ib.slice(..), wgpu::IndexFormat::Uint32);
                p.draw_indexed(0..ovl_n, 0, 0..1);
            }
            // 4. Crosshair, then the HUD (alpha-blended, screen space).
            p.set_pipeline(&g.pipeline);
            p.set_bind_group(0, &g.bind_ui, &[]);
            p.set_vertex_buffer(0, g.ui_vb.slice(..));
            p.set_index_buffer(g.ui_ib.slice(..), wgpu::IndexFormat::Uint32);
            p.draw_indexed(0..12, 0, 0..1);
            if !hud.i.is_empty() {
                p.set_pipeline(&g.pipeline_t);
                p.set_vertex_buffer(0, g.hud_vb.slice(..));
                p.set_index_buffer(g.hud_ib.slice(..), wgpu::IndexFormat::Uint32);
                p.draw_indexed(0..hud.i.len() as u32, 0, 0..1);
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
            WindowEvent::CloseRequested => { self.save(); el.exit() }
            WindowEvent::Resized(s) => if let Some(g) = &mut self.gfx {
                g.config.width = s.width.max(1); g.config.height = s.height.max(1);
                g.surface.configure(&g.device, &g.config);
                g.depth = Self::depth_view(&g.device, &g.config);
            },
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                if down && !self.grabbed && button == MouseButton::Left {
                    if let Some(g) = &self.gfx {
                        self.grabbed = g.window.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| g.window.set_cursor_grab(CursorGrabMode::Confined)).is_ok();
                        g.window.set_cursor_visible(!self.grabbed);
                    }
                } else {
                    match button {
                        MouseButton::Left => { self.mouse_left = down; if down { self.left_hold = 0.12; } }
                        MouseButton::Right => { self.mouse_right = down; if down { self.click_right = true; } }
                        _ => {}
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta { winit::event::MouseScrollDelta::LineDelta(_, y) => y, winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 30.0 };
                if dy != 0.0 && !self.screen_open { let inv = &mut self.game.inventory; inv.selected = (inv.selected as i32 - dy.signum() as i32).rem_euclid(9) as usize; }
            }
            WindowEvent::KeyboardInput { event, .. } => if let PhysicalKey::Code(k) = event.physical_key {
                let pressed = event.state == ElementState::Pressed;
                // Menu navigation repeats while held.
                if pressed && self.screen_open {
                    match k {
                        KeyCode::ArrowUp => self.craft_sel = self.craft_sel.saturating_sub(1),
                        KeyCode::ArrowDown => self.craft_sel += 1,
                        KeyCode::Enter | KeyCode::NumpadEnter => {
                            let recipes = self.recipes.clone();
                            let pick = recipes.craftable(&self.game.inventory).get(self.craft_sel).map(|r| (*r).clone());
                            if let Some(r) = pick { recipes.craft(&r, &mut self.game.inventory); }
                        }
                        _ => {}
                    }
                }
                if pressed && !event.repeat {
                    match k {
                        KeyCode::Escape => {
                            if self.screen_open { self.screen_open = false; }
                            else if self.grabbed {
                                if let Some(g) = &self.gfx { let _ = g.window.set_cursor_grab(CursorGrabMode::None); g.window.set_cursor_visible(true); }
                                self.grabbed = false; self.mouse_left = false; self.mouse_right = false;
                            } else { self.save(); el.exit(); }
                        }
                        KeyCode::KeyE => { self.screen_open = !self.screen_open; self.craft_sel = 0; }
                        KeyCode::KeyQ => { let whole = self.key(KeyCode::ControlLeft); self.game.drop_selected(whole, self.yaw as f64); }
                        KeyCode::KeyF => self.game.toggle_flying(),
                        KeyCode::F5 => self.save(),
                        KeyCode::KeyT => self.game.time += 6000, // skip a quarter day
                        KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5
                        | KeyCode::Digit6 | KeyCode::Digit7 | KeyCode::Digit8 | KeyCode::Digit9 => {
                            self.game.inventory.selected = match k { KeyCode::Digit1 => 0, KeyCode::Digit2 => 1, KeyCode::Digit3 => 2, KeyCode::Digit4 => 3,
                                KeyCode::Digit5 => 4, KeyCode::Digit6 => 5, KeyCode::Digit7 => 6, KeyCode::Digit8 => 7, _ => 8 };
                        }
                        _ => {}
                    }
                }
                if pressed { self.keys.insert(k); } else { self.keys.remove(&k); }
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
    // usage: viewer [seed | path/to/java/world] [--sdr] [--paper-white=NITS] [--write]
    let args: Vec<String> = std::env::args().skip(1).collect();
    let want_hdr = !args.iter().any(|a| a == "--sdr");
    let allow_write = args.iter().any(|a| a == "--write");
    let mut packs = PackStack::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--pack" {
            let Some(path) = it.next() else { eprintln!("--pack needs a path"); std::process::exit(2) };
            match Pack::open(path) { Ok(p) => { eprintln!("resource pack: {}", p.name); packs.push(p); } Err(e) => { eprintln!("cannot open pack {path}: {e}"); std::process::exit(2) } }
        }
    }
    let textured = !packs.is_empty();
    let recipes = Recipes::load(&packs); // recipes come from the same packs (a vanilla jar ships them); built-ins otherwise
    let models = Arc::new(Models::new(packs));
    if textured { eprintln!("atlas {}x{} (tile {}px), {} animated textures", models.atlas.width, models.atlas.height, models.atlas.tile, models.atlas.animations.len()); }
    let paper_white = args.iter().find_map(|a| a.strip_prefix("--paper-white=")?.parse().ok()).unwrap_or(203.0f32);
    let arg = { let mut skip = false; args.iter().find(|a| { if skip { skip = false; return false; } if a.as_str() == "--pack" { skip = true; return false; } !a.starts_with("--") }) }.cloned().unwrap_or_else(|| "1".into());
    let path = std::path::Path::new(&arg);
    let (seed, read_dir, save_dir) = if path.join("level.dat").exists() {
        let seed = read_level_dat(path).ok().and_then(|l| level_seed(&l)).unwrap_or(0);
        eprintln!("loading Java world {arg} (seed {seed}); missing chunks use the built-in generator");
        // Writing re-encodes chunks in our format, which drops data we don't model (block entities, entities, structures).
        let save = if allow_write { Some(path.to_path_buf()) } else { None };
        (seed, path.to_path_buf(), save)
    } else {
        let seed: i64 = arg.parse().unwrap_or(1);
        let dir = PathBuf::from(format!("saves/seed_{seed}"));
        (seed, dir.clone(), Some(dir))
    };
    let (tx, rx) = mpsc::channel();
    let mut game = Game::new(seed);
    let creative = args.iter().any(|a| a == "--creative");
    if creative {
        game.mode = GameMode::Creative;
        for (i, (_, name, _, _)) in CREATIVE_HOTBAR.iter().enumerate() {
            game.inventory.slots[i] = Some(ItemStack::new(name, 64));
        }
    } else if args.iter().any(|a| a == "--kit") {
        // Starter kit for trying survival without punching trees first.
        for (n, c) in [("wooden_pickaxe", 1), ("wooden_axe", 1), ("wooden_shovel", 1), ("oak_log", 8), ("cobblestone", 16), ("coal", 4), ("stick", 8), ("torch", 8)] {
            game.inventory.add(&ItemStack::new(n, c));
        }
    }
    // A few pigs around spawn; they freeze until their chunk loads, then drop onto the terrain.
    for i in 0..6 {
        let (x, z) = (4 + i * 3, -6 + (i * 5) % 11);
        let h = game.generator.surface_height(x, z).max(blockengine::worldgen::SEA_LEVEL);
        game.entities.spawn(blockengine::entity::Kind::Pig, [x as f64 + 0.5, h as f64 + 2.0, z as f64 + 0.5]);
    }
    let mut app = App {
        recipes: Arc::new(recipes), gfx: None, game, src: Source { read: Arc::new(Mutex::new(World::open(read_dir))), save_dir },
        tx, rx, pending_gen: HashSet::new(), mesh_seq: HashMap::new(), next_seq: 0, in_flight_meshes: 0, chunks: HashMap::new(),
        yaw: 0.0, pitch: -0.3, keys: HashSet::new(), grabbed: false, last: Instant::now(),
        want_hdr, paper_white, output: Output::Sdr, models, textured, anim_state: Vec::new(), t0: Instant::now(),
        screen_open: false, craft_sel: 0, fps: 60.0, name_t: 0.0, last_sel: 0,
        mouse_left: false, mouse_right: false, left_hold: 0.0, click_right: false, act_cd: 0.0, title_t: 0.0,
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
