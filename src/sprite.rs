//! Original PNG pixels and animation metadata; no resampling or replacement art.
use crate::vanilla::{ClientAssets, ResourceId};
use serde_json::Value;
use std::io::Cursor;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    Cutout,
    Translucent,
}
pub struct Sprite {
    pub width: u32,
    pub height: u32,
    pub alpha: AlphaMode,
    pub interpolate: bool,
    frames: Vec<Vec<u8>>,
    timeline: Vec<(usize, u32)>,
    duration: u64,
}
impl Sprite {
    pub fn load(assets: &mut ClientAssets, id: &ResourceId) -> Result<Self, String> {
        Self::decode(
            &assets.texture_png(id)?,
            assets.texture_metadata(id)?.as_ref(),
        )
    }
    pub fn decode(bytes: &[u8], metadata: Option<&Value>) -> Result<Self, String> {
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        if reader.info().width > 8192 || reader.info().height > 8192 {
            return Err("sprite dimensions exceed limit".into());
        }
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
            _ => return Err("unsupported PNG type".into()),
        };
        let animation = metadata.and_then(|m| m.get("animation"));
        let (width, height) = if let Some(a) = animation {
            if !a.is_object() {
                return Err("animation must be object".into());
            }
            let dimension = |key: &str| {
                a.get(key)
                    .map(|v| {
                        v.as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .filter(|n| *n > 0)
                            .ok_or("invalid frame dimension")
                    })
                    .transpose()
            };
            match (dimension("width")?, dimension("height")?) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, info.height),
                (None, Some(h)) => (info.width, h),
                (None, None) => {
                    let side = info.width.min(info.height);
                    (side, side)
                }
            }
        } else {
            (info.width, info.height)
        };
        if info.width % width != 0 || info.height % height != 0 {
            return Err("frame dimensions do not tile sprite".into());
        }
        let columns = info.width / width;
        let rows = info.height / height;
        let mut frames = Vec::new();
        for index in 0..columns * rows {
            let mut frame = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height {
                let start = (((index / columns * height + y) * info.width
                    + index % columns * width)
                    * 4) as usize;
                frame.extend_from_slice(&rgba[start..start + (width * 4) as usize]);
            }
            frames.push(frame);
        }
        let mut timeline = Vec::new();
        let mut interpolate = false;
        if let Some(a) = animation {
            let time = |v: &Value| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0)
                    .ok_or("invalid frame time")
            };
            let default_time = a.get("frametime").map(time).transpose()?.unwrap_or(1);
            interpolate = a
                .get("interpolate")
                .map(|v| v.as_bool().ok_or("interpolate must be boolean"))
                .transpose()?
                .unwrap_or(false);
            if let Some(list) = a.get("frames") {
                for entry in list.as_array().ok_or("frames must be array")? {
                    let (index, ticks) = if entry.is_object() {
                        (
                            entry["index"].as_u64(),
                            entry
                                .get("time")
                                .map(time)
                                .transpose()?
                                .unwrap_or(default_time),
                        )
                    } else {
                        (entry.as_u64(), default_time)
                    };
                    let index = index
                        .and_then(|n| usize::try_from(n).ok())
                        .filter(|n| *n < frames.len())
                        .ok_or("animation frame index outside sprite")?;
                    timeline.push((index, ticks));
                }
            }
            if timeline.is_empty() {
                timeline = (0..frames.len()).map(|i| (i, default_time)).collect();
            }
        } else {
            timeline.push((0, 1));
        }
        let duration = timeline.iter().map(|(_, ticks)| *ticks as u64).sum();
        let mut alpha = AlphaMode::Opaque;
        for &(index, _) in &timeline {
            for p in frames[index].as_chunks::<4>().0 {
                if p[3] != 0 && p[3] != 255 {
                    alpha = AlphaMode::Translucent;
                    break;
                }
                if p[3] == 0 && alpha == AlphaMode::Opaque {
                    alpha = AlphaMode::Cutout;
                }
            }
        }
        Ok(Self {
            width,
            height,
            alpha,
            interpolate,
            frames,
            timeline,
            duration,
        })
    }
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
    pub fn sample(&self, ticks: f64) -> (&[u8], &[u8], f32) {
        let mut time = if ticks.is_finite() {
            ticks.max(0.) % self.duration as f64
        } else {
            0.
        };
        for (i, &(index, duration)) in self.timeline.iter().enumerate() {
            if time < duration as f64 {
                let next = self.timeline[(i + 1) % self.timeline.len()].0;
                return (
                    &self.frames[index],
                    &self.frames[next],
                    if self.interpolate {
                        (time / duration as f64) as f32
                    } else {
                        0.
                    },
                );
            }
            time -= duration as f64;
        }
        unreachable!()
    }
    pub fn pixels(&self, ticks: f64) -> Vec<u8> {
        let (a, b, t) = self.sample(ticks);
        if t == 0. {
            return a.to_vec();
        }
        a.iter()
            .zip(b)
            .enumerate()
            .map(|(i, (&a, &b))| {
                if i % 4 == 3 {
                    a
                } else {
                    (a as f32 * (1. - t) + b as f32 * t) as u8
                }
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn png() -> Vec<u8> {
        let mut b = Vec::new();
        {
            let mut e = png::Encoder::new(&mut b, 1, 2);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 128])
                .unwrap();
        }
        b
    }
    #[test]
    fn respects_frame_order_and_duration() {
        let s = Sprite::decode(
            &png(),
            Some(&json!({"animation":{"frames":[{"index":1,"time":2},0]}})),
        )
        .unwrap();
        assert_eq!(s.frame_count(), 2);
        assert_eq!(s.pixels(0.), [0, 0, 255, 128]);
        assert_eq!(s.pixels(2.), [255, 0, 0, 255]);
        assert_eq!(s.pixels(3.), [0, 0, 255, 128]);
        assert_eq!(s.alpha, AlphaMode::Translucent);
    }
    #[test]
    fn validates_metadata_and_preserves_static_rectangles() {
        assert!(Sprite::decode(&png(), Some(&json!({"animation":{"frames":[99]}}))).is_err());
        assert!(Sprite::decode(&png(), Some(&json!({"animation":{"frametime":0}}))).is_err());
        let s = Sprite::decode(&png(), None).unwrap();
        assert_eq!((s.width, s.height), (1, 2));
        let s = Sprite::decode(&png(), Some(&json!({"animation":{"interpolate":true}}))).unwrap();
        assert_eq!(s.pixels(0.5), [127, 0, 127, 255]);
    }
}
