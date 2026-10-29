//! Chunk-local VBOs, shared original sprites, camera-relative transforms and animation.
use crate::{
    block_world::ChunkKey,
    sprite::AlphaMode,
    vanilla::ResourceId,
    world_mesh::{ChunkMesh, ModelCache},
};
use glam::{DVec3, IVec3, Mat4, Vec3};
use glow::HasContext;
use std::{collections::BTreeMap, rc::Rc};
struct Batch {
    vao: glow::NativeVertexArray,
    vbo: glow::NativeBuffer,
    count: i32,
    sprite: ResourceId,
}
struct Texture {
    id: glow::NativeTexture,
    alpha: AlphaMode,
    last_pixels: Vec<u8>,
}
pub struct TexturedRenderer {
    gl: Rc<glow::Context>,
    program: glow::NativeProgram,
    textures: BTreeMap<ResourceId, Texture>,
    chunks: BTreeMap<ChunkKey, Vec<Batch>>,
}
impl TexturedRenderer {
    /// The caller keeps the GL context current until this renderer is dropped.
    pub fn new(gl: Rc<glow::Context>) -> Result<Self, String> {
        unsafe {
            let program = gl.create_program()?;
            for (kind, source) in [
                (
                    glow::VERTEX_SHADER,
                    include_str!("../assets/shaders/blocks.vert"),
                ),
                (
                    glow::FRAGMENT_SHADER,
                    include_str!("../assets/shaders/blocks.frag"),
                ),
            ] {
                let s = gl.create_shader(kind)?;
                gl.shader_source(s, source);
                gl.compile_shader(s);
                if !gl.get_shader_compile_status(s) {
                    let error = gl.get_shader_info_log(s);
                    gl.delete_shader(s);
                    gl.delete_program(program);
                    return Err(error);
                }
                gl.attach_shader(program, s);
                gl.delete_shader(s);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                let e = gl.get_program_info_log(program);
                gl.delete_program(program);
                return Err(e);
            }
            Ok(Self {
                gl,
                program,
                textures: BTreeMap::new(),
                chunks: BTreeMap::new(),
            })
        }
    }
    pub fn upload(&mut self, mesh: ChunkMesh) -> Result<(), String> {
        unsafe {
            let gl = &self.gl;
            let mut batches = Vec::new();
            for (sprite, vertices) in mesh.batches {
                if vertices.is_empty() {
                    continue;
                }
                let count = i32::try_from(vertices.len()).map_err(|_| "vertex count overflow")?;
                let vao = gl.create_vertex_array()?;
                let vbo = gl.create_buffer()?;
                gl.bind_vertex_array(Some(vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                gl.buffer_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    bytemuck::cast_slice(&vertices),
                    glow::STATIC_DRAW,
                );
                for (index, size, offset) in [(0, 3, 0), (1, 2, 12), (2, 3, 20)] {
                    gl.enable_vertex_attrib_array(index);
                    gl.vertex_attrib_pointer_f32(index, size, glow::FLOAT, false, 32, offset);
                }
                batches.push(Batch {
                    vao,
                    vbo,
                    count,
                    sprite,
                });
            }
            if let Some(old) = self.chunks.remove(&mesh.key) {
                for b in old {
                    gl.delete_vertex_array(b.vao);
                    gl.delete_buffer(b.vbo);
                }
            }
            if !batches.is_empty() {
                self.chunks.insert(mesh.key, batches);
            }
            Ok(())
        }
    }
    pub fn sync_sprites(&mut self, cache: &ModelCache, ticks: f64) -> Result<(), String> {
        unsafe {
            let gl = &self.gl;
            gl.active_texture(glow::TEXTURE0);
            for (id, sprite) in &cache.sprites {
                if let Some(texture) = self.textures.get_mut(id) {
                    if sprite.frame_count() > 1 {
                        let pixels = sprite.pixels(ticks);
                        if pixels != texture.last_pixels {
                            gl.bind_texture(glow::TEXTURE_2D, Some(texture.id));
                            gl.tex_sub_image_2d(
                                glow::TEXTURE_2D,
                                0,
                                0,
                                0,
                                sprite.width as i32,
                                sprite.height as i32,
                                glow::RGBA,
                                glow::UNSIGNED_BYTE,
                                glow::PixelUnpackData::Slice(Some(&pixels)),
                            );
                            texture.last_pixels = pixels;
                        }
                    }
                    continue;
                }
                let texture = gl.create_texture()?;
                let pixels = sprite.pixels(ticks);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    sprite.width as i32,
                    sprite.height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&pixels)),
                );
                for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32);
                }
                for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32);
                }
                self.textures.insert(
                    id.clone(),
                    Texture {
                        id: texture,
                        alpha: sprite.alpha,
                        last_pixels: pixels,
                    },
                );
            }
            Ok(())
        }
    }
    pub fn draw(
        &self,
        camera: DVec3,
        forward: DVec3,
        width: u32,
        height: u32,
    ) -> Result<(usize, usize), String> {
        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, width as i32, height as i32);
            gl.clear_color(0.45, 0.65, 0.85, 1.);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.enable(glow::DEPTH_TEST);
            gl.enable(glow::CULL_FACE);
            gl.disable(glow::BLEND);
            gl.depth_mask(true);
            gl.use_program(Some(self.program));
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_1_i32(gl.get_uniform_location(self.program, "sprite").as_ref(), 0);
            let view = Mat4::look_at_rh(Vec3::ZERO, forward.as_vec3(), Vec3::Y);
            let projection = Mat4::perspective_rh_gl(
                65f32.to_radians(),
                width as f32 / height.max(1) as f32,
                0.05,
                2048.,
            );
            let location = gl.get_uniform_location(self.program, "mvp");
            let mut draws = 0;
            let mut triangles = 0;
            let mut transparent = Vec::new();
            for (key, batches) in &self.chunks {
                let origin = (IVec3::from_array(*key) * 16).as_dvec3();
                let center = origin + DVec3::splat(8.);
                let relative = center - camera;
                let p = view.transform_point3(relative.as_vec3());
                let radius = 16.;
                let depth = -p.z;
                let tan = (65f32.to_radians() / 2.).tan();
                let aspect = width as f32 / height.max(1) as f32;
                if depth + radius < 0.05
                    || depth - radius > 2048.
                    || p.y.abs() > depth * tan + radius * (1. + tan * tan).sqrt()
                    || p.x.abs()
                        > depth * tan * aspect + radius * (1. + (tan * aspect).powi(2)).sqrt()
                {
                    continue;
                }
                let mvp = projection * view * Mat4::from_translation((origin - camera).as_vec3());
                for batch in batches {
                    let texture = self
                        .textures
                        .get(&batch.sprite)
                        .ok_or("GPU sprite not uploaded")?;
                    if texture.alpha == AlphaMode::Translucent {
                        transparent.push((relative.length_squared(), mvp, batch, texture));
                        continue;
                    }
                    gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &mvp.to_cols_array());
                    gl.bind_vertex_array(Some(batch.vao));
                    gl.bind_texture(glow::TEXTURE_2D, Some(texture.id));
                    gl.draw_arrays(glow::TRIANGLES, 0, batch.count);
                    draws += 1;
                    triangles += batch.count as usize / 3;
                }
            }
            // Coarse batch sorting; intersecting translucent surfaces still need per-face sorting.
            transparent.sort_by(|a, b| b.0.total_cmp(&a.0));
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(false);
            for (_, mvp, batch, texture) in transparent {
                gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &mvp.to_cols_array());
                gl.bind_vertex_array(Some(batch.vao));
                gl.bind_texture(glow::TEXTURE_2D, Some(texture.id));
                gl.draw_arrays(glow::TRIANGLES, 0, batch.count);
                draws += 1;
                triangles += batch.count as usize / 3;
            }
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            // Small screen-space crosshair without extra meshes.
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::SCISSOR_TEST);
            gl.clear_color(1., 1., 1., 1.);
            for (w, h) in [(12, 2), (2, 12)] {
                gl.scissor((width as i32 - w) / 2, (height as i32 - h) / 2, w, h);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            gl.disable(glow::SCISSOR_TEST);
            let error = gl.get_error();
            if error != glow::NO_ERROR {
                return Err(format!("OpenGL error {error:x}"));
            }
            Ok((draws, triangles))
        }
    }
    pub fn screenshot(
        &self,
        path: &std::path::Path,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        unsafe {
            let mut pixels = vec![0; (width * height * 4) as usize];
            self.gl.read_pixels(
                0,
                0,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            let flipped: Vec<u8> = pixels
                .chunks_exact(width as usize * 4)
                .rev()
                .flatten()
                .copied()
                .collect();
            let mut e = png::Encoder::new(
                std::fs::File::create(path).map_err(|e| e.to_string())?,
                width,
                height,
            );
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .map_err(|e| e.to_string())?
                .write_image_data(&flipped)
                .map_err(|e| e.to_string())
        }
    }
}
impl Drop for TexturedRenderer {
    fn drop(&mut self) {
        unsafe {
            for batches in self.chunks.values() {
                for b in batches {
                    self.gl.delete_vertex_array(b.vao);
                    self.gl.delete_buffer(b.vbo);
                }
            }
            for t in self.textures.values() {
                self.gl.delete_texture(t.id);
            }
            self.gl.delete_program(self.program);
        }
    }
}
