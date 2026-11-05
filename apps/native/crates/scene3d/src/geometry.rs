//! Geometry preflight runs on a worker at an immutable document snapshot. The
//! actor verifies the snapshot before accepting any geometry-derived edits.
use crate::{Error, Result, fail, navigation::*, schema};
use pm_geom3d::{
    gltf::{self, math},
    primitives::Parameters,
    source::{MeshSource, Source},
};
use serde_json::{Value, json};
use std::collections::HashMap;
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedGeometry {
    pub source: pm_model::Node,
    pub comp: Option<String>,
    pub time: f64,
    pub bounds: Value,
    pub parent_inverse: Value,
}
pub trait AssetGeometry: Send + Sync {
    fn bounds(&self, id: &str, record: &Value, time: f64) -> Result<Bounds>;
}
pub struct NoAssets;
impl AssetGeometry for NoAssets {
    fn bounds(&self, id: &str, _: &Value, _: f64) -> Result<Bounds> {
        fail(format!("3D model geometry is not loaded: {id}"))
    }
}
#[derive(Default)]
pub struct LoadedAssets {
    models: HashMap<String, (Value, std::sync::Arc<crate::import::Model>)>,
}
impl LoadedAssets {
    pub fn insert(&mut self, id: String, record: Value, model: crate::import::Model) {
        self.models.insert(id, (record, std::sync::Arc::new(model)));
    }
    pub fn insert_shared(
        &mut self,
        id: String,
        record: Value,
        model: std::sync::Arc<crate::import::Model>,
    ) {
        self.models.insert(id, (record, model));
    }
    pub fn remove(&mut self, id: &str) {
        self.models.remove(id);
    }
}
impl AssetGeometry for LoadedAssets {
    fn bounds(&self, id: &str, record: &Value, time: f64) -> Result<Bounds> {
        let (loaded, model) = self
            .models
            .get(id)
            .ok_or_else(|| Error(format!("3D model geometry is not loaded: {id}")))?;
        if loaded != record {
            return fail("3D model source changed; reload geometry");
        }
        let b = match model.as_ref() {
            crate::import::Model::Gltf(m) => {
                let pose = gltf::pose::Pose::at(m, time);
                let (min, max) = gltf::pose::scene_box(m, &pose)
                    .ok_or_else(|| Error("Model has no geometry".into()))?;
                Bounds {
                    min: std::array::from_fn(|i| (min[i] - m.fit.center[i]) / m.fit.radius),
                    max: std::array::from_fn(|i| (max[i] - m.fit.center[i]) / m.fit.radius),
                }
            }
            crate::import::Model::Obj(m) => points(
                m.positions
                    .chunks_exact(3)
                    .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]),
            )?,
            crate::import::Model::Splats(d) => {
                let (center, r) = d.normalization()?;
                let core = d.dense_core()?;
                Bounds {
                    min: [
                        (core.min[0] - center[0]) / r,
                        -(core.max[1] - center[1]) / r,
                        -(core.max[2] - center[2]) / r,
                    ],
                    max: [
                        (core.max[0] - center[0]) / r,
                        -(core.min[1] - center[1]) / r,
                        -(core.min[2] - center[2]) / r,
                    ],
                }
            }
        };
        b.validate()?;
        Ok(b)
    }
}
fn points(points: impl Iterator<Item = Vec3>) -> Result<Bounds> {
    let mut b = Bounds {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };
    for p in points {
        for (i, v) in p.into_iter().enumerate() {
            b.min[i] = b.min[i].min(v);
            b.max[i] = b.max[i].max(v);
        }
    }
    b.validate()?;
    Ok(b)
}
pub fn local_bounds(
    source: &Value,
    assets: &dyn AssetGeometry,
    records: &Value,
    time: f64,
) -> Result<Bounds> {
    schema::source(source)?;
    if let Some(id) = source["assetId"].as_str() {
        return assets.bounds(id, &records[id], time);
    }
    let n = |k: &str| source["parameters"][k].as_f64();
    if source["primitive"] == "roundedBox" {
        let size = [
            n("width").unwrap_or(1.),
            n("height").unwrap_or(1.),
            n("depth").unwrap_or(1.),
        ];
        return Ok(Bounds {
            min: size.map(|x| -x / 2.),
            max: size.map(|x| x / 2.),
        });
    }
    let source = if let Some(name) = source["primitive"].as_str() {
        Source::primitive(
            name,
            &Parameters {
                width: n("width"),
                height: n("height"),
                depth: n("depth"),
                radius: n("radius"),
                radius_top: n("radiusTop"),
                radius_bottom: n("radiusBottom"),
                tube: n("tube"),
                segments: n("segments"),
                detail: n("detail"),
            },
        )
        .ok_or_else(|| Error("Unknown primitive".into()))?
    } else if source.get("mesh").is_some() {
        let m = &source["mesh"];
        let floats = |k: &str| {
            m[k].as_array()
                .map(|a| a.iter().map(|x| x.as_f64().unwrap()).collect())
        };
        Source::Mesh(MeshSource {
            positions: floats("positions").unwrap(),
            indices: m["indices"]
                .as_array()
                .map(|a| a.iter().map(|x| x.as_f64().unwrap() as u32).collect()),
            normals: floats("normals"),
            uvs: floats("uvs"),
            groups: vec![],
        })
    } else {
        let key = if source.get("lathe").is_some() {
            "lathe"
        } else {
            "extrude"
        };
        let points = source[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
            .collect();
        if key == "lathe" {
            Source::Lathe {
                points,
                segments: source["segments"].as_f64().unwrap_or(48.),
            }
        } else {
            Source::Extrude {
                points,
                depth: source["depth"].as_f64().unwrap(),
                bevel: source["bevel"].as_f64().unwrap_or(0.),
            }
        }
    };
    let g = source.geometry();
    points(
        g.positions
            .chunks_exact(3)
            .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]),
    )
}
pub fn transform(b: Bounds, m: [f64; 16]) -> Result<Bounds> {
    b.validate()?;
    if !m.iter().all(|v| v.is_finite()) {
        return fail("Invalid world transform");
    }
    points((0..8).map(|n| {
        math::apply(
            &m,
            std::array::from_fn(|i| {
                if n & (1 << i) == 0 {
                    b.min[i]
                } else {
                    b.max[i]
                }
            }),
        )
    }))
}
pub fn inverse(m: [f64; 16]) -> Result<[f64; 16]> {
    if !m.iter().all(|v| v.is_finite()) || math::determinant(&m).abs() < 1e-15 {
        return fail("Parent transform is singular");
    }
    Ok(math::invert(&m))
}
pub fn parent_delta(comp: &Value, l: &Value, delta: Vec3) -> Result<Vec3> {
    if l["parent"].is_null() && l["group"].is_null() {
        return Ok(delta);
    }
    let id = l["id"]
        .as_str()
        .ok_or_else(|| Error("Missing layer id".into()))?;
    let m = matrix(&comp["_scene3dParentInverse"][id])?;
    let origin = math::apply(&m, [0.; 3]);
    Ok(sub(math::apply(&m, delta), origin))
}
pub fn matrix(v: &Value) -> Result<[f64; 16]> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 16)
        .ok_or_else(|| Error("Evaluated parent transform unavailable".into()))?;
    let mut m = [0.; 16];
    for (i, v) in a.iter().enumerate() {
        m[i] = v
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| Error("Invalid parent matrix".into()))?;
    }
    Ok(m)
}
pub fn needs_geometry(args: &Value) -> bool {
    matches!(
        args["operation"].as_str(),
        Some("explode" | "drop_to_floor" | "place_on" | "auto_studio")
    )
}
pub fn descendants(l: &Value, group: &str, layers: &[Value]) -> bool {
    let mut current = l;
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = current["group"].as_str() {
        if id == group {
            return true;
        }
        if !seen.insert(id) {
            return false;
        }
        let Some(next) = layers.iter().find(|l| l["id"] == id) else {
            return false;
        };
        current = next;
    }
    false
}
pub fn required_ids(comp: &Value, args: &Value, selection: &[String]) -> Result<Vec<String>> {
    let ls = comp["layers"]
        .as_array()
        .ok_or_else(|| Error("No composition layers".into()))?;
    let target = args["target"]
        .as_str()
        .or(args["id"].as_str())
        .or_else(|| selection.first().map(String::as_str));
    let resolve = |name: &str| -> Result<String> {
        let mut found = ls.iter().filter(|l| l["id"] == name);
        if let Some(l) = found.next() {
            return Ok(l["id"].as_str().unwrap().into());
        }
        let matches = ls.iter().filter(|l| l["name"] == name).collect::<Vec<_>>();
        if matches.len() != 1 {
            return fail("Choose an unambiguous layer ID");
        }
        Ok(matches[0]["id"].as_str().unwrap().into())
    };
    match args["operation"].as_str() {
        Some("auto_studio") => Ok(ls
            .iter()
            .filter(|l| schema::role(l) == Some(schema::Role::Object))
            .filter_map(|l| l["id"].as_str().map(str::to_owned))
            .collect()),
        Some("explode") => {
            let group = resolve(target.ok_or_else(|| Error("Choose a model group".into()))?)?;
            Ok(ls
                .iter()
                .filter(|l| {
                    schema::role(l) == Some(schema::Role::Object) && descendants(l, &group, ls)
                })
                .filter_map(|l| l["id"].as_str().map(str::to_owned))
                .collect())
        }
        Some("place_on") => Ok(vec![
            resolve(target.ok_or_else(|| Error("Choose a model".into()))?)?,
            resolve(
                args["on_target"]
                    .as_str()
                    .ok_or_else(|| Error("Choose a model to place it on".into()))?,
            )?,
        ]),
        Some("drop_to_floor") => Ok(vec![resolve(
            target.ok_or_else(|| Error("Choose a model".into()))?,
        )?]),
        _ => Ok(vec![]),
    }
}
pub fn bounds_json(b: Bounds) -> Value {
    json!([b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2]])
}
