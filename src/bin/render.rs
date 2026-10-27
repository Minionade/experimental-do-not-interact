//! Software renderer: meshes a patch of terrain and rasterizes it to render.ppm.
use blockengine::mesh::{mesh_chunk, Mesh};
use blockengine::worldgen::Generator;
use rayon::prelude::*;
use std::{collections::HashMap, io::Write};

type V3 = [f32; 3];
fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn norm(a: V3) -> V3 { let l = dot(a, a).sqrt(); [a[0] / l, a[1] / l, a[2] / l] }

fn main() {
    let seed: i64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let (w, h) = (960usize, 540usize);
    let r = 10;
    let g = Generator::new(seed);
    let coords: Vec<(i32, i32)> = (-r..=r).flat_map(|x| (-r..=r).map(move |z| (x, z))).collect();
    let chunks: HashMap<_, _> = coords.par_iter().map(|&(x, z)| ((x, z), g.generate(x, z))).collect();
    let meshes: Vec<(i32, i32, Mesh)> = coords.par_iter().map(|&(x, z)| {
        let n = |dx, dz| chunks.get(&(x + dx, z + dz));
        (x, z, mesh_chunk(&chunks[&(x, z)], [n(-1, 0), n(1, 0), n(0, -1), n(0, 1)]))
    }).collect();
    eprintln!("{} chunks, {} triangles", meshes.len(), meshes.iter().map(|m| m.2.indices.len() / 3).sum::<usize>());

    let ground = g.surface_height(0, 0).max(63) as f32;
    let eye = [0.0, ground + 40.0, -60.0];
    let fwd = norm(sub([0.0, ground, 40.0], eye));
    let right = norm(cross(fwd, [0.0, 1.0, 0.0]));
    let up = cross(right, fwd);
    let f = 1.0 / (60f32.to_radians() / 2.0).tan();
    let aspect = w as f32 / h as f32;

    let mut color = vec![[0.0f32; 3]; w * h];
    let mut depth = vec![f32::INFINITY; w * h];
    for (i, px) in color.iter_mut().enumerate() {
        let t = (i / w) as f32 / h as f32;
        *px = [0.45 + 0.2 * t, 0.65 + 0.15 * t, 0.95];
    }
    for (cx, cz, m) in &meshes {
        let off = [*cx as f32 * 16.0, 0.0, *cz as f32 * 16.0];
        let proj: Vec<Option<(f32, f32, f32)>> = m.vertices.iter().map(|v| {
            let p = sub([v.pos[0] + off[0], v.pos[1], v.pos[2] + off[2]], eye);
            let z = dot(p, fwd);
            if z < 0.1 { return None; }
            Some((w as f32 / 2.0 * (1.0 + f * dot(p, right) / (z * aspect)),
                  h as f32 / 2.0 * (1.0 - f * dot(p, up) / z), z))
        }).collect();
        for t in m.indices.chunks(3) {
            let (Some(a), Some(b), Some(c)) = (proj[t[0] as usize], proj[t[1] as usize], proj[t[2] as usize]) else { continue };
            let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            if area >= 0.0 { continue; } // back-face / degenerate
            let (x0, x1) = (a.0.min(b.0).min(c.0).max(0.0) as usize, (a.0.max(b.0).max(c.0).ceil().min(w as f32 - 1.0)).max(0.0) as usize);
            let (y0, y1) = (a.1.min(b.1).min(c.1).max(0.0) as usize, (a.1.max(b.1).max(c.1).ceil().min(h as f32 - 1.0)).max(0.0) as usize);
            for y in y0..=y1 { for x in x0..=x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b.0 - px) * (c.1 - py) - (b.1 - py) * (c.0 - px)) / area;
                let w1 = ((c.0 - px) * (a.1 - py) - (c.1 - py) * (a.0 - px)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 { continue; }
                let z = w0 * a.2 + w1 * b.2 + w2 * c.2;
                if z < depth[y * w + x] {
                    depth[y * w + x] = z;
                    let col = m.vertices[t[0] as usize].color;
                    let fog = (z / 170.0).clamp(0.0, 1.0).powi(2);
                    color[y * w + x] = [col[0] * (1.0 - fog) + 0.6 * fog, col[1] * (1.0 - fog) + 0.75 * fog, col[2] * (1.0 - fog) + 0.95 * fog];
                }
            }}
        }
    }
    let mut out = std::fs::File::create("render.ppm").unwrap();
    write!(out, "P6\n{w} {h}\n255\n").unwrap();
    out.write_all(&color.iter().flat_map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)).collect::<Vec<_>>()).unwrap();
}
