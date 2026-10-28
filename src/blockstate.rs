//! Blockstate selection and transforms. Random tickets are supplied by the caller;
//! matching vanilla's position-seeded random sequence is a separate parity task.
use crate::{
    vanilla::{ClientAssets, ResourceId},
    vanilla_mesh::{bake_model, ModelQuad},
};
use glam::{Mat3, Vec3};
use serde_json::Value;
use std::collections::BTreeMap;
pub type Properties = BTreeMap<String, String>;

#[derive(Debug)]
pub struct ModelChoice {
    pub model: ResourceId,
    pub x: u32,
    pub y: u32,
    pub z: u32,
    pub uvlock: bool,
}

pub fn parse_properties(text: &str) -> Result<Properties, String> {
    let mut result = Properties::new();
    if text.is_empty() {
        return Ok(result);
    }
    for pair in text.split(',') {
        let (key, value) = pair.split_once('=').ok_or("property needs key=value")?;
        if key.is_empty()
            || value.is_empty()
            || value.contains('=')
            || result.insert(key.into(), value.into()).is_some()
        {
            return Err("empty or duplicate property".into());
        }
    }
    Ok(result)
}

fn matches(condition: &Value, state: &Properties, depth: usize) -> Result<bool, String> {
    if depth > 64 {
        return Err("multipart condition depth limit".into());
    }
    let object = condition.as_object().ok_or("condition must be an object")?;
    let mut result = true;
    for (key, value) in object {
        let matched = if key == "OR" || key == "AND" {
            let children = value.as_array().ok_or("logical condition needs array")?;
            if children.is_empty() {
                return Err("empty logical condition".into());
            }
            let values: Vec<bool> = children
                .iter()
                .map(|c| matches(c, state, depth + 1))
                .collect::<Result<_, _>>()?;
            if key == "OR" {
                values.into_iter().any(|b| b)
            } else {
                values.into_iter().all(|b| b)
            }
        } else {
            let pattern = value.as_str().ok_or("property condition must be string")?;
            let actual = state
                .get(key)
                .ok_or_else(|| format!("missing property {key}"))?;
            let (negated, pattern) = pattern
                .strip_prefix('!')
                .map_or((false, pattern), |p| (true, p));
            if pattern.split('|').any(str::is_empty) {
                return Err("empty property alternative".into());
            }
            pattern.split('|').any(|p| p == actual) != negated
        };
        result &= matched;
    }
    Ok(result)
}
fn choice(value: &Value) -> Result<ModelChoice, String> {
    let angle = |key: &str| -> Result<u32, String> {
        let n = value
            .get(key)
            .map(|v| v.as_u64().ok_or("invalid blockstate rotation"))
            .transpose()?
            .unwrap_or(0);
        if ![0, 90, 180, 270].contains(&n) {
            return Err("blockstate rotation must be a quarter turn".into());
        }
        Ok(n as u32)
    };
    Ok(ModelChoice {
        model: ResourceId::parse(value["model"].as_str().ok_or("missing model ID")?)?,
        x: angle("x")?,
        y: angle("y")?,
        z: angle("z")?,
        uvlock: value
            .get("uvlock")
            .map(|v| v.as_bool().ok_or("uvlock must be boolean"))
            .transpose()?
            .unwrap_or(false),
    })
}
fn choose(value: &Value, ticket: u64) -> Result<ModelChoice, String> {
    let Some(array) = value.as_array() else {
        return choice(value);
    };
    if array.is_empty() {
        return Err("empty weighted model list".into());
    }
    let mut choices = Vec::new();
    let mut total = 0u64;
    for value in array {
        let weight = value
            .get("weight")
            .map(|v| v.as_u64().ok_or("invalid model weight"))
            .transpose()?
            .unwrap_or(1);
        if weight == 0 {
            return Err("model weight must be positive".into());
        }
        total = total.checked_add(weight).ok_or("model weight overflow")?;
        choices.push((total, choice(value)?));
    }
    let index = ticket % total;
    Ok(choices.into_iter().find(|(end, _)| index < *end).unwrap().1)
}
/// Select one weighted variant, or all matching multipart contributions.
/// Properties must be a complete valid state from the registry; this is not a registry validator.
pub fn select(
    definition: &Value,
    state: &Properties,
    mut ticket: impl FnMut() -> u64,
) -> Result<Vec<ModelChoice>, String> {
    match (definition.get("variants"), definition.get("multipart")) {
        (Some(variants), None) => {
            let mut selected = None;
            for (key, value) in variants.as_object().ok_or("variants must be object")? {
                let required = parse_properties(key)?;
                let mut matched = true;
                for (k, v) in required {
                    matched &= state
                        .get(&k)
                        .ok_or_else(|| format!("missing property {k}"))?
                        == &v;
                }
                if matched {
                    if selected.is_some() {
                        return Err("ambiguous blockstate variants".into());
                    }
                    selected = Some(value);
                }
            }
            Ok(vec![choose(
                selected.ok_or("no matching variant")?,
                ticket(),
            )?])
        }
        (None, Some(parts)) => {
            let mut result = Vec::new();
            for part in parts.as_array().ok_or("multipart must be array")? {
                if part
                    .get("when")
                    .map(|c| matches(c, state, 0))
                    .transpose()?
                    .unwrap_or(true)
                {
                    result.push(choose(
                        part.get("apply").ok_or("multipart needs apply")?,
                        ticket(),
                    )?);
                }
            }
            Ok(result)
        }
        _ => Err("blockstate requires exactly one of variants or multipart".into()),
    }
}
fn basis(direction: &str) -> (Vec3, Vec3, Vec3) {
    match direction {
        "down" => (Vec3::X, -Vec3::Z, -Vec3::Y),
        "up" => (Vec3::X, Vec3::Z, Vec3::Y),
        "north" => (-Vec3::X, -Vec3::Y, -Vec3::Z),
        "south" => (Vec3::X, -Vec3::Y, Vec3::Z),
        "west" => (Vec3::Z, -Vec3::Y, -Vec3::X),
        "east" => (-Vec3::Z, -Vec3::Y, Vec3::X),
        _ => unreachable!("baker validates face directions"),
    }
}
fn direction(normal: Vec3) -> &'static str {
    ["down", "up", "north", "south", "west", "east"]
        .into_iter()
        .max_by(|a, b| basis(a).2.dot(normal).total_cmp(&basis(b).2.dot(normal)))
        .unwrap()
}
pub fn bake_choice(
    assets: &mut ClientAssets,
    choice: &ModelChoice,
) -> Result<Vec<ModelQuad>, String> {
    let mut quads = bake_model(&assets.resolve_model(&choice.model)?)?;
    transform_quads(&mut quads, choice);
    Ok(quads)
}
fn transform_quads(quads: &mut [ModelQuad], choice: &ModelChoice) {
    let rotation = Mat3::from_rotation_z(-(choice.z as f32).to_radians())
        * Mat3::from_rotation_y(-(choice.y as f32).to_radians())
        * Mat3::from_rotation_x(-(choice.x as f32).to_radians());
    for quad in quads {
        let (u, v, n) = basis(&quad.face_direction);
        let new_direction = direction(rotation * n);
        let (new_u, new_v, _) = basis(new_direction);
        for vertex in &mut quad.vertices {
            vertex.position = (rotation * (Vec3::from_array(vertex.position) - Vec3::splat(0.5))
                + Vec3::splat(0.5))
            .to_array();
            if choice.uvlock {
                let offset = rotation * (u * (vertex.uv[0] - 0.5) + v * (vertex.uv[1] - 0.5));
                vertex.uv = [new_u.dot(offset) + 0.5, new_v.dot(offset) + 0.5];
            }
        }
        quad.normal = (rotation * Vec3::from_array(quad.normal)).to_array();
        quad.face_direction = new_direction.into();
        if let Some(face) = &mut quad.cull_face {
            *face = direction(rotation * basis(face).2).into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn variants_select_exact_properties_and_rotation() {
        let d = json!({"variants":{"facing=east,lit=true":{"model":"block/furnace_on","y":90},"facing=north,lit=false":{"model":"block/furnace"}}});
        let c = select(&d, &parse_properties("lit=true,facing=east").unwrap(), || 0).unwrap();
        assert_eq!(c[0].model.key(), "minecraft:block/furnace_on");
        assert_eq!(c[0].y, 90);
        assert!(select(&d, &Properties::new(), || 0).is_err());
    }
    #[test]
    fn weighted_boundaries_are_exact() {
        let d = json!([{"model":"block/a","weight":2},{"model":"block/b","weight":3}]);
        for (ticket, path) in [(0, "a"), (1, "a"), (2, "b"), (4, "b"), (5, "a")] {
            assert_eq!(
                choose(&d, ticket).unwrap().model.key(),
                format!("minecraft:block/{path}")
            );
        }
        assert!(choose(&json!([{"model":"a","weight":0}]), 0).is_err());
    }
    #[test]
    fn multipart_combines_nested_conditions() {
        let d = json!({"multipart":[{"apply":{"model":"block/post"}},{"when":{"OR":[{"north":"true"},{"AND":[{"east":"true"},{"shape":"!inner|outer"}]}]},"apply":{"model":"block/side"}}]});
        let state = parse_properties("north=false,east=true,shape=straight").unwrap();
        assert_eq!(select(&d, &state, || 0).unwrap().len(), 2);
        assert!(parse_properties("x=1,x=2").is_err());
        assert!(select(
            &json!({"variants":{"":{"model":"a"},"x=1":{"model":"b"}}}),
            &parse_properties("x=1").unwrap(),
            || 0
        )
        .is_err());
    }
    #[test]
    fn uv_lock_tracks_world_axes_for_all_quarter_turns() {
        let mut faces = serde_json::Map::new();
        for face in ["down", "up", "north", "south", "west", "east"] {
            faces.insert(
                face.into(),
                json!({"texture":"block/stone","cullface":face}),
            );
        }
        let model = crate::vanilla::ResolvedModel {
            elements: Some(vec![json!({"from":[0,0,0],"to":[16,16,16],"faces":faces})]),
            ..Default::default()
        };
        for x in [0, 90, 180, 270] {
            for y in [0, 90, 180, 270] {
                for z in [0, 90, 180, 270] {
                    let mut quads = bake_model(&model).unwrap();
                    transform_quads(
                        &mut quads,
                        &ModelChoice {
                            model: ResourceId::parse("test").unwrap(),
                            x,
                            y,
                            z,
                            uvlock: true,
                        },
                    );
                    for q in quads {
                        let (u, v, n) = basis(&q.face_direction);
                        assert!((Vec3::from_array(q.normal) - n).length() < 1e-5);
                        assert_eq!(q.cull_face.as_deref(), Some(q.face_direction.as_str()));
                        for vertex in q.vertices {
                            let p = Vec3::from_array(vertex.position) - Vec3::splat(0.5);
                            assert!((vertex.uv[0] - (p.dot(u) + 0.5)).abs() < 1e-5);
                            assert!((vertex.uv[1] - (p.dot(v) + 0.5)).abs() < 1e-5);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rotation_moves_north_to_east_and_locks_top_uvs() {
        let model = crate::vanilla::ResolvedModel {
            elements: Some(vec![
                json!({"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"block/stone","cullface":"north"},"up":{"texture":"block/stone"}}}),
            ]),
            ..Default::default()
        };
        let mut quads = bake_model(&model).unwrap();
        transform_quads(
            &mut quads,
            &ModelChoice {
                model: ResourceId::parse("test").unwrap(),
                x: 0,
                y: 90,
                z: 0,
                uvlock: true,
            },
        );
        let east = quads.iter().find(|q| q.face_direction == "east").unwrap();
        assert_eq!(east.cull_face.as_deref(), Some("east"));
        assert!((Vec3::from_array(east.normal) - Vec3::X).length() < 1e-6);
        let top = quads.iter().find(|q| q.face_direction == "up").unwrap();
        for v in &top.vertices {
            assert!((v.position[0] - v.uv[0]).abs() < 1e-6);
            assert!((v.position[2] - v.uv[1]).abs() < 1e-6);
        }
    }
}
