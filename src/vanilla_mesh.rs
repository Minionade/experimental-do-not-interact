//! CPU geometry for resolved vanilla element models, independent of GPU storage.
//! Special renderers and element rotations are rejected, never approximated.
use crate::vanilla::{ResolvedModel, ResolvedTexture};
use glam::Vec3;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Debug, Serialize)]
pub struct ModelQuad {
    /// Counter-clockwise when viewed from outside; indices are 0,1,2,0,2,3.
    pub vertices: [ModelVertex; 4],
    pub normal: [f32; 3],
    pub texture: ResolvedTexture,
    pub cull_face: Option<String>,
    pub tint_index: Option<u32>,
    pub shade: bool,
    /// Preserve future version-specific fields for later rendering stages.
    pub source_face: Value,
}

fn numbers<const N: usize>(value: &Value) -> Result<[f32; N], String> {
    let array = value.as_array().ok_or("expected numeric array")?;
    if array.len() != N {
        return Err(format!("expected {N} coordinates"));
    }
    let mut result = [0.; N];
    for (out, value) in result.iter_mut().zip(array) {
        *out = value.as_f64().ok_or("expected numeric coordinate")? as f32;
        if !out.is_finite() {
            return Err("non-finite model coordinate".into());
        }
    }
    Ok(result)
}

/// Bake axis-aligned elements in block units, keeping each sprite's UVs separate.
/// UVs are normalized to a single sprite, with V increasing down the source PNG.
/// Blockstate transforms, UV-lock, atlas placement, tint and lighting are later stages.
pub fn bake_model(model: &ResolvedModel) -> Result<Vec<ModelQuad>, String> {
    if let Some(parent) = &model.builtin_parent {
        return Err(format!("special model renderer not implemented: {parent}"));
    }
    let mut quads = Vec::new();
    for element in model.elements.as_deref().unwrap_or_default() {
        if element.get("rotation").is_some() {
            return Err("element rotation not implemented".into());
        }
        let [x0, y0, z0] = numbers::<3>(&element["from"])?;
        let [x1, y1, z1] = numbers::<3>(&element["to"])?;
        if x0 > x1 || y0 > y1 || z0 > z1 {
            return Err("inverted element bounds".into());
        }
        let shade = match element.get("shade") {
            Some(value) => value.as_bool().ok_or("shade must be boolean")?,
            None => true,
        };
        let faces = element["faces"].as_object().ok_or("element needs faces")?;
        for (direction, face) in faces {
            // Corners are top-left, bottom-left, bottom-right, top-right in UV space.
            let (positions, default_uv) = match direction.as_str() {
                "down" => (
                    [[x0, y0, z1], [x0, y0, z0], [x1, y0, z0], [x1, y0, z1]],
                    [x0, 16. - z1, x1, 16. - z0],
                ),
                "up" => (
                    [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
                    [x0, z0, x1, z1],
                ),
                "north" => (
                    [[x1, y1, z0], [x1, y0, z0], [x0, y0, z0], [x0, y1, z0]],
                    [16. - x1, 16. - y1, 16. - x0, 16. - y0],
                ),
                "south" => (
                    [[x0, y1, z1], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1]],
                    [x0, 16. - y1, x1, 16. - y0],
                ),
                "west" => (
                    [[x0, y1, z0], [x0, y0, z0], [x0, y0, z1], [x0, y1, z1]],
                    [z0, 16. - y1, z1, 16. - y0],
                ),
                "east" => (
                    [[x1, y1, z1], [x1, y0, z1], [x1, y0, z0], [x1, y1, z0]],
                    [16. - z1, 16. - y1, 16. - z0, 16. - y0],
                ),
                _ => return Err(format!("invalid face direction {direction}")),
            };
            let uv = face
                .get("uv")
                .map(numbers::<4>)
                .transpose()?
                .unwrap_or(default_uv);
            let rotation = match face.get("rotation") {
                Some(value) => value.as_u64().ok_or("invalid UV rotation")?,
                None => 0,
            };
            if ![0, 90, 180, 270].contains(&rotation) {
                return Err("UV rotation must be 0, 90, 180 or 270".into());
            }
            let cull_face = face
                .get("cullface")
                .map(|v| {
                    let s = v.as_str().ok_or("cullface must be a direction")?;
                    if !["down", "up", "north", "south", "west", "east"].contains(&s) {
                        return Err("invalid cullface direction");
                    }
                    Ok(s.to_string())
                })
                .transpose()?;
            let tint_index = match face.get("tintindex") {
                None => None,
                Some(v) if v.as_i64() == Some(-1) => None,
                Some(v) => Some(
                    u32::try_from(v.as_u64().ok_or("invalid tintindex")?)
                        .map_err(|_| "tintindex overflow")?,
                ),
            };
            let texture =
                model.texture_binding(face["texture"].as_str().ok_or("face needs texture")?)?;
            let coords = [
                [uv[0], uv[1]],
                [uv[0], uv[3]],
                [uv[2], uv[3]],
                [uv[2], uv[1]],
            ];
            let vertices = std::array::from_fn(|i| ModelVertex {
                position: positions[i].map(|v| v / 16.),
                uv: coords[(i + rotation as usize / 90) % 4].map(|v| v / 16.),
            });
            let normal = (Vec3::from_array(positions[1]) - Vec3::from_array(positions[0]))
                .cross(Vec3::from_array(positions[2]) - Vec3::from_array(positions[0]))
                .normalize_or_zero()
                .to_array();
            quads.push(ModelQuad {
                vertices,
                normal,
                texture,
                cull_face,
                tint_index,
                shade,
                source_face: face.clone(),
            });
        }
    }
    Ok(quads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn cube() -> ResolvedModel {
        let mut faces = serde_json::Map::new();
        for direction in ["down", "up", "north", "south", "west", "east"] {
            faces.insert(
                direction.into(),
                json!({"texture":"#all", "cullface": direction}),
            );
        }
        ResolvedModel {
            textures: [(
                "all".into(),
                json!({"sprite":"minecraft:block/stone","force_translucent":true}),
            )]
            .into(),
            elements: Some(vec![json!({"from":[0,0,0],"to":[16,16,16],"faces":faces})]),
            ..Default::default()
        }
    }
    #[test]
    fn cube_winds_outward_and_preserves_sprite_metadata() {
        let quads = bake_model(&cube()).unwrap();
        assert_eq!(quads.len(), 6);
        for q in quads {
            let center = q
                .vertices
                .iter()
                .map(|v| Vec3::from_array(v.position))
                .sum::<Vec3>()
                / 4.;
            assert!(Vec3::from_array(q.normal).dot(center - Vec3::splat(0.5)) > 0.);
            assert_eq!(q.texture.sprite.key(), "minecraft:block/stone");
            assert_eq!(q.texture.metadata["force_translucent"], true);
            assert!(q
                .vertices
                .iter()
                .all(|v| v.uv.iter().all(|c| *c == 0. || *c == 1.)));
        }
    }
    #[test]
    fn partial_element_default_uv_and_rotation() {
        let mut model = cube();
        model.elements = Some(vec![
            json!({"from":[2,4,6],"to":[10,12,14],"faces":{"south":{"texture":"#all"}}}),
        ]);
        let plain = bake_model(&model).unwrap();
        assert_eq!(plain[0].vertices[0].uv, [2. / 16., 4. / 16.]);
        assert_eq!(plain[0].vertices[2].uv, [10. / 16., 12. / 16.]);
        model.elements.as_mut().unwrap()[0]["faces"]["south"]["rotation"] = json!(90);
        let rotated = bake_model(&model).unwrap();
        assert_eq!(rotated[0].vertices[0].uv, plain[0].vertices[1].uv);
    }
    #[test]
    fn unsupported_and_malformed_models_fail_closed() {
        let mut model = cube();
        model.builtin_parent = Some("minecraft:builtin/entity".into());
        assert!(bake_model(&model).is_err());
        model.builtin_parent = None;
        model.elements.as_mut().unwrap()[0]["rotation"] = json!({"angle":22.5});
        assert!(bake_model(&model).is_err());
        model.elements.as_mut().unwrap()[0]
            .as_object_mut()
            .unwrap()
            .remove("rotation");
        model.elements.as_mut().unwrap()[0]["faces"]["south"]["rotation"] = json!(45);
        assert!(bake_model(&model).is_err());
    }
}
