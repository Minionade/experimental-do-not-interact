//! Block texture atlas built from a resource pack stack, including animated textures (.mcmeta).
use crate::pack::{split_id, PackStack};
use std::collections::HashMap;
use std::io::Cursor;

const MAX_ATLAS: u32 = 8192;
pub const WHITE: &str = "blockengine:white";
pub const MISSING: &str = "blockengine:missing";

/// An animated texture: pre-scaled frames and the playback sequence.
pub struct Animation {
    /// Tile origin in the atlas, in pixels.
    pub origin: (u32, u32),
    frames: Vec<Vec<u8>>,
    /// (frame index, duration in ticks)
    seq: Vec<(usize, u32)>,
    total: u32,
    interpolate: bool,
}

pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub tile: u32,
    /// RGBA8, sRGB-encoded.
    pub pixels: Vec<u8>,
    index: HashMap<String, (u32, u32)>,
    pub animations: Vec<Animation>,
}

pub struct Image { pub w: u32, pub h: u32, pub rgba: Vec<u8> }

pub fn decode_png(bytes: &[u8]) -> Option<Image> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Indexed => return None, // EXPAND removes this
    };
    Some(Image { w, h, rgba })
}

fn scale_nearest(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    for y in 0..dh { for x in 0..dw {
        let (sx, sy) = (x * sw / dw, y * sh / dh);
        let (s, d) = (((sy * sw + sx) * 4) as usize, ((y * dw + x) * 4) as usize);
        out[d..d + 4].copy_from_slice(&src[s..s + 4]);
    }}
    out
}

/// Cuts a sprite sheet (frames stacked vertically) into frames and returns the playback sequence.
fn parse_animation(img: &Image, meta: &serde_json::Value) -> Option<(Vec<Image>, Vec<(usize, u32)>, bool)> {
    let a = meta.get("animation")?;
    let fw = a.get("width").and_then(|v| v.as_u64()).map_or(img.w, |v| v as u32).max(1);
    let fh = a.get("height").and_then(|v| v.as_u64()).map_or(if a.get("width").is_some() { fw } else { img.w }, |v| v as u32).max(1);
    if fw > img.w || fh > img.h { return None; }
    // Frames are laid out left-to-right then top-to-bottom; vanilla sheets are a single column.
    let cols = (img.w / fw).max(1);
    let count = cols * (img.h / fh);
    let frames: Vec<Image> = (0..count).map(|i| {
        let (ox, oy) = ((i % cols) * fw, (i / cols) * fh);
        let mut rgba = Vec::with_capacity((fw * fh * 4) as usize);
        for y in 0..fh { let s = (((oy + y) * img.w + ox) * 4) as usize; rgba.extend_from_slice(&img.rgba[s..s + (fw * 4) as usize]); }
        Image { w: fw, h: fh, rgba }
    }).collect();
    let default_time = a.get("frametime").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as u32;
    let seq: Vec<(usize, u32)> = match a.get("frames").and_then(|v| v.as_array()) {
        Some(list) => list.iter().filter_map(|f| match f {
            serde_json::Value::Number(n) => Some((n.as_u64()? as usize, default_time)),
            o => Some((o.get("index")?.as_u64()? as usize, o.get("time").and_then(|t| t.as_u64()).unwrap_or(default_time as u64).max(1) as u32)),
        }).filter(|(i, _)| *i < frames.len()).collect(),
        None => (0..frames.len()).map(|i| (i, default_time)).collect(),
    };
    if seq.is_empty() { return None; }
    Some((frames, seq, a.get("interpolate").and_then(|v| v.as_bool()).unwrap_or(false)))
}

/// Normalises "block/stone" and "minecraft:block/stone" to the same key.
pub fn texture_key(id: &str) -> String { let (ns, p) = split_id(id); format!("{ns}:{p}") }

impl Atlas {
    pub fn build(packs: &PackStack) -> Atlas {
        struct Entry { key: String, frames: Vec<Image>, seq: Vec<(usize, u32)>, interpolate: bool }
        let mut entries: Vec<Entry> = Vec::new();
        for path in packs.list("assets/") {
            let Some(rest) = path.strip_prefix("assets/") else { continue };
            let Some((ns, rest)) = rest.split_once('/') else { continue };
            // Block textures feed the models; item textures feed inventory icons.
            let Some((kind, name)) = ["block", "item"].iter().find_map(|k| Some((*k, rest.strip_prefix(&format!("textures/{k}/"))?.strip_suffix(".png")?))) else { continue };
            let Some(img) = packs.read(&path).and_then(|b| decode_png(&b)) else { continue };
            let meta = packs.read_json(&format!("{path}.mcmeta"));
            let key = format!("{ns}:{kind}/{name}");
            match meta.as_ref().and_then(|m| parse_animation(&img, m)) {
                Some((frames, seq, interpolate)) => entries.push(Entry { key, frames, seq, interpolate }),
                None => {
                    // Static texture. Non-square images contribute their top square.
                    let side = img.w.min(img.h);
                    let rgba = img.rgba[..(side * img.w * 4) as usize].chunks(img.w as usize * 4)
                        .flat_map(|row| row[..side as usize * 4].to_vec()).collect();
                    entries.push(Entry { key, frames: vec![Image { w: side, h: side, rgba }], seq: vec![(0, 1)], interpolate: false });
                }
            }
        }
        entries.sort_by(|a, b| a.key.cmp(&b.key));

        let n = entries.len() as u32 + 2; // + white + missing
        let mut tile = entries.iter().map(|e| e.frames[0].w.max(e.frames[0].h)).max().unwrap_or(16).max(1);
        let (mut cols, mut rows);
        loop {
            cols = (MAX_ATLAS / tile).min(n).max(1);
            rows = n.div_ceil(cols);
            if rows * tile <= MAX_ATLAS || tile <= 1 { break; }
            tile /= 2; // enormous packs: shrink tiles to fit
        }
        let (width, height) = (cols * tile, rows * tile);
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut index = HashMap::new();
        let mut animations = Vec::new();

        let blit = |pixels: &mut Vec<u8>, slot: u32, tile_rgba: &[u8]| {
            let (ox, oy) = ((slot % cols) * tile, (slot / cols) * tile);
            for y in 0..tile { let d = (((oy + y) * width + ox) * 4) as usize; let s = (y * tile * 4) as usize;
                pixels[d..d + (tile * 4) as usize].copy_from_slice(&tile_rgba[s..s + (tile * 4) as usize]); }
            (ox, oy)
        };
        let white = vec![255u8; (tile * tile * 4) as usize];
        index.insert(WHITE.to_string(), blit(&mut pixels, 0, &white));
        let missing: Vec<u8> = (0..tile * tile).flat_map(|i| {
            let (x, y) = (i % tile, i / tile);
            if (x * 2 / tile + y * 2 / tile) % 2 == 0 { [248, 0, 248, 255] } else { [0, 0, 0, 255] }
        }).collect();
        index.insert(MISSING.to_string(), blit(&mut pixels, 1, &missing));

        for (i, e) in entries.into_iter().enumerate() {
            let scaled: Vec<Vec<u8>> = e.frames.iter().map(|f| if f.w == tile && f.h == tile { f.rgba.clone() } else { scale_nearest(&f.rgba, f.w, f.h, tile, tile) }).collect();
            let origin = blit(&mut pixels, i as u32 + 2, &scaled[(e.seq[0].0).min(scaled.len() - 1)]);
            index.insert(e.key, origin);
            if e.seq.len() > 1 || scaled.len() > 1 && e.seq.iter().any(|s| s.0 != e.seq[0].0) {
                let total = e.seq.iter().map(|s| s.1).sum();
                animations.push(Animation { origin, frames: scaled, seq: e.seq, total, interpolate: e.interpolate });
            }
        }
        Atlas { width, height, tile, pixels, index, animations }
    }

    /// Normalised (u0, v0, u1, v1) of a texture's tile, or the missing-texture tile.
    pub fn rect(&self, id: &str) -> (f32, f32, f32, f32) {
        let (ox, oy) = self.index.get(&texture_key(id)).or_else(|| self.index.get(id)).copied().unwrap_or(self.index[MISSING]);
        let (w, h, t) = (self.width as f32, self.height as f32, self.tile as f32);
        (ox as f32 / w, oy as f32 / h, (ox as f32 + t) / w, (oy as f32 + t) / h)
    }

    pub fn contains(&self, id: &str) -> bool { self.index.contains_key(&texture_key(id)) }
    pub fn white_uv(&self) -> [f32; 2] { let r = self.rect(WHITE); [(r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0] }
}

impl Animation {
    /// Pixels (tile x tile RGBA) to display at game tick `tick` (20 per second).
    pub fn pixels_at(&self, tick: u64) -> Vec<u8> {
        let t = (tick % self.total as u64) as u32;
        let (mut acc, mut cur) = (0, 0);
        for (i, &(_, d)) in self.seq.iter().enumerate() { if t < acc + d { cur = i; break; } acc += d; }
        let (fi, dur) = self.seq[cur];
        if !self.interpolate { return self.frames[fi].clone(); }
        let next = self.frames[self.seq[(cur + 1) % self.seq.len()].0].as_slice();
        let k = (t - acc) as f32 / dur as f32;
        self.frames[fi].iter().zip(next).map(|(&a, &b)| (a as f32 + (b as f32 - a as f32) * k).round() as u8).collect()
    }

    /// Index into the sequence for `tick`, so callers can skip uploads when nothing changed.
    pub fn state_at(&self, tick: u64) -> u64 {
        let t = (tick % self.total as u64) as u32;
        if self.interpolate { return t as u64; }
        let mut acc = 0;
        for (i, &(_, d)) in self.seq.iter().enumerate() { if t < acc + d { return i as u64; } acc += d; }
        0
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::pack::{tests::temp, Pack};

    pub fn png_bytes(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(rgba).unwrap();
        out
    }
    pub fn solid(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> { (0..w * h).flat_map(|_| c).collect() }

    fn stack(files: &[(&str, Vec<u8>)]) -> (PackStack, std::path::PathBuf) {
        let d = temp("atlas");
        for (p, b) in files { let f = d.join("p").join(p); std::fs::create_dir_all(f.parent().unwrap()).unwrap(); std::fs::write(f, b).unwrap(); }
        let mut s = PackStack::new();
        s.push(Pack::open(d.join("p")).unwrap());
        (s, d)
    }

    #[test]
    fn static_textures_share_a_uniform_tile_size() {
        let (s, d) = stack(&[
            ("assets/minecraft/textures/block/small.png", png_bytes(2, 2, &solid(2, 2, [255, 0, 0, 255]))),
            ("assets/minecraft/textures/block/big.png", png_bytes(8, 8, &solid(8, 8, [0, 255, 0, 255]))),
        ]);
        let a = Atlas::build(&s);
        assert_eq!(a.tile, 8);
        assert!(a.contains("block/small") && a.contains("minecraft:block/big") && !a.contains("block/nope"));
        let px = |id: &str| { let r = a.rect(id); let (x, y) = ((r.0 * a.width as f32) as u32 + 3, (r.1 * a.height as f32) as u32 + 3); let i = ((y * a.width + x) * 4) as usize; [a.pixels[i], a.pixels[i + 1], a.pixels[i + 2]] };
        assert_eq!(px("block/small"), [255, 0, 0], "2x2 scaled up to the 8x8 tile");
        assert_eq!(px("block/big"), [0, 255, 0]);
        assert_eq!(px("block/unknown"), px(MISSING), "unknown ids map to the missing tile");
        assert_eq!(px(WHITE), [255, 255, 255]);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn animated_texture_follows_sequence_and_custom_times() {
        // 4 frames stacked vertically: red, green, blue, yellow (each 2x2).
        let mut sheet = vec![];
        for c in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255]] { sheet.extend(solid(2, 2, c)); }
        let (s, d) = stack(&[
            ("assets/minecraft/textures/block/anim.png", png_bytes(2, 8, &sheet)),
            ("assets/minecraft/textures/block/anim.png.mcmeta", br#"{"animation":{"frametime":2,"frames":[0,1,{"index":3,"time":4}]}}"#.to_vec()),
        ]);
        let a = Atlas::build(&s);
        assert_eq!(a.animations.len(), 1);
        let an = &a.animations[0];
        let first = |t: u64| { let p = an.pixels_at(t); [p[0], p[1], p[2]] };
        assert_eq!(first(0), [255, 0, 0]);
        assert_eq!(first(1), [255, 0, 0]);
        assert_eq!(first(2), [0, 255, 0]);
        assert_eq!(first(3), [0, 255, 0]);
        assert_eq!(first(4), [255, 255, 0], "frame index 3 holds for 4 ticks");
        assert_eq!(first(7), [255, 255, 0]);
        assert_eq!(first(8), [255, 0, 0], "loops after total = 8 ticks");
        assert_eq!(first(8 * 1000 + 2), [0, 255, 0]);
        assert_ne!(an.state_at(1), an.state_at(2));
        assert_eq!(an.state_at(0), an.state_at(1), "no redundant uploads within a frame");
        // The atlas initially shows the first frame of the sequence.
        let r = a.rect("block/anim");
        let i = (((r.1 * a.height as f32) as u32 * a.width + (r.0 * a.width as f32) as u32) * 4) as usize;
        assert_eq!(&a.pixels[i..i + 3], &[255, 0, 0]);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn interpolated_animation_blends_between_frames() {
        let mut sheet = solid(2, 2, [0, 0, 0, 255]);
        sheet.extend(solid(2, 2, [200, 100, 0, 255]));
        let (s, d) = stack(&[
            ("assets/minecraft/textures/block/fade.png", png_bytes(2, 4, &sheet)),
            ("assets/minecraft/textures/block/fade.png.mcmeta", br#"{"animation":{"frametime":4,"interpolate":true}}"#.to_vec()),
        ]);
        let a = Atlas::build(&s);
        let an = &a.animations[0];
        let px = |t| an.pixels_at(t)[0];
        assert_eq!(px(0), 0);
        assert_eq!(px(2), 100, "halfway through frame 0 -> halfway to frame 1");
        assert_eq!(px(4), 200);
        assert_eq!(px(6), 100, "frame 1 blends back toward frame 0 (wraps)");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn custom_frame_size_and_missing_mcmeta_fields() {
        // 4x2 sheet with width=2,height=2 => two side-by-side frames (not stacked).
        let mut rgba = vec![];
        for _y in 0..2 { rgba.extend([255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 255]); }
        let (s, d) = stack(&[
            ("assets/minecraft/textures/block/wide.png", png_bytes(4, 2, &rgba)),
            ("assets/minecraft/textures/block/wide.png.mcmeta", br#"{"animation":{"width":2,"height":2}}"#.to_vec()),
            ("assets/minecraft/textures/block/broken.png", png_bytes(2, 2, &solid(2, 2, [9, 9, 9, 255]))),
            ("assets/minecraft/textures/block/broken.png.mcmeta", b"{ this is not json".to_vec()),
        ]);
        let a = Atlas::build(&s);
        assert_eq!(a.animations.len(), 1, "broken mcmeta falls back to a static texture");
        let an = &a.animations[0];
        assert_eq!(an.pixels_at(0)[..3], [255, 0, 0]);
        assert_eq!(an.pixels_at(1)[..3], [0, 0, 255]);
        std::fs::remove_dir_all(d).ok();
    }
}
