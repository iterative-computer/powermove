//! Placement is part of the same import transaction, using worker-measured bounds.
use super::*;
use pm_scene3d::{
    geometry,
    navigation::{self as nav, Bounds, Vec3},
};
use serde_json::{Value as Json, json};

fn snapshot(cx: &Cx<'_>) -> crate::Snapshot {
    crate::Snapshot {
        doc_id: 0,
        revision: 0,
        document: std::sync::Arc::new(pm_model::Document::from_root(Some(cx.root().clone()), 0)),
    }
}
pub(super) struct AssetBounds;
impl geometry::AssetGeometry for AssetBounds {
    fn bounds(&self, _id: &str, record: &Json, _time: f64) -> pm_scene3d::Result<Bounds> {
        if record["kind"] != "model" {
            return Err(pm_scene3d::Error("Model geometry is not retained".into()));
        }
        if record["native3dBounds"]["version"] == 1 {
            if let Some(array) = record["native3dBounds"]["bounds"]
                .as_array()
                .filter(|a| a.len() == 6)
            {
                let mut v = [0.; 6];
                for (n, item) in array.iter().enumerate() {
                    v[n] = item
                        .as_f64()
                        .ok_or_else(|| pm_scene3d::Error("Invalid retained model bounds".into()))?;
                }
                let b = Bounds {
                    min: [v[0], v[1], v[2]],
                    max: [v[3], v[4], v[5]],
                };
                b.validate()?;
                return Ok(b);
            }
        }
        // Older native/Electron imports fit their initial geometry inside a
        // unit-radius envelope. Keep their files offline and use that envelope.
        Ok(Bounds {
            min: [-1.; 3],
            max: [1.; 3],
        })
    }
}
fn interval(b: Bounds, axis: Vec3) -> (f64, f64) {
    let c = nav::dot(b.center(), axis);
    let r = (0..3)
        .map(|i| (b.max[i] - b.min[i]) * 0.5 * axis[i].abs())
        .sum::<f64>();
    (c - r, c + r)
}
fn union(a: Bounds, b: Bounds) -> Bounds {
    Bounds {
        min: std::array::from_fn(|i| a.min[i].min(b.min[i])),
        max: std::array::from_fn(|i| a.max[i].max(b.max[i])),
    }
}
fn world_camera(r: &pm_scene3d::service::ResolvedScene) -> Option<(String, nav::Pose, nav::Lens)> {
    let id = r.camera_id.clone()?;
    let camera = &r.scene["camera"];
    let p = &camera["p"];
    let num = |key: &str, default: f64| {
        p[key]
            .get("v")
            .unwrap_or(&p[key])
            .as_f64()
            .unwrap_or(default)
    };
    let position = r.camera_position?;
    let target = *r.targets.get(&id)?;
    let pose = nav::Pose {
        position,
        target,
        zoom: num("zoom", 1.),
    };
    let lens = if camera["projection"] == "orthographic" {
        nav::Lens::Orthographic {
            width: 6.,
            height: 6.,
        }
    } else {
        nav::Lens::Perspective {
            fov: num("fov", 45.),
            aspect: 1.,
            near: num("near", 0.01),
            far: num("far", 1000.),
        }
    };
    Some((id, pose, lens))
}
pub(super) fn place(
    cx: &mut Cx<'_>,
    comp: Option<&str>,
    id: &str,
    receipt: &pm_scene3d::import::archive::Receipt,
    time: f64,
    frame: bool,
) -> Result<(), CommandError> {
    let frozen = snapshot(cx);
    let root = frozen.root().ok_or_else(|| invalid("No import project"))?;
    let comp_id = comp
        .map(str::to_owned)
        .or_else(|| frozen.open_comp())
        .ok_or_else(|| invalid("No import composition"))?;
    let node = crate::model::comp_node(root, Some(&comp_id))
        .ok_or_else(|| invalid("No import composition"))?;
    let json = pm_scene3d::from_model(&node.to_value()).map_err(invalid)?;
    let layers = json["layers"]
        .as_array()
        .ok_or_else(|| invalid("No import layers"))?;
    let records = pm_scene3d::from_model(
        &root
            .get("assets")
            .map(pm_model::Node::to_value)
            .unwrap_or(Value::Null),
    )
    .map_err(invalid)?;
    let resolved = super::resolve_scene(&frozen, Some(&comp_id), time).ok();
    let camera = resolved.as_ref().and_then(world_camera);
    let axis = camera
        .as_ref()
        .map(|(_, pose, _)| {
            let mut axis = nav::cross(nav::sub(pose.target, pose.position), [0., 1., 0.]);
            axis[1] = 0.;
            nav::normalize(axis)
        })
        .filter(|a| nav::length(*a) > 1e-6)
        .unwrap_or([1., 0., 0.]);
    let mut queries = crate::queries::QueryContext::new(&frozen.document);
    let mut existing = vec![];
    for (index, layer) in layers.iter().enumerate() {
        if layer["id"] == id || schema::role(layer) != Some(schema::Role::Object) {
            continue;
        }
        let object = &layer["d"]["data"]["object"];
        if ["world", "fog", "terrain", "scatter"]
            .iter()
            .any(|k| object.get(k).is_some())
            || object["source"].get("nature").is_some()
        {
            continue;
        }
        if schema::source(&object["source"]).is_err() {
            continue;
        }
        let request = |op: &str| {
            pm_scene3d::to_model(
                &json!({"op":op,"comp":comp_id,"open":comp_id,"layer":index,"t":time}),
            )
        };
        if queries.query(&request("active")).ok() != Some(Value::Bool(true)) {
            continue;
        }
        let local = match geometry::local_bounds(
            &object["source"],
            &AssetBounds,
            &records,
            time - layer["from"].as_f64().unwrap_or(0.),
        ) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let Some(world) = queries
            .query(&request("world3D"))
            .ok()
            .and_then(|v| pm_scene3d::from_model(&v).ok())
            .and_then(|v| geometry::matrix(&v).ok())
        else {
            continue;
        };
        if let Ok(bounds) = geometry::transform(local, world) {
            existing.push(bounds);
        }
    }
    let local = receipt.bounds();
    let center = local.center();
    let (_, new_right) = interval(local, axis);
    let half = new_right - nav::dot(center, axis);
    let gap = (half * 0.3).max(0.15);
    let distance = existing
        .iter()
        .map(|b| interval(*b, axis).1)
        .max_by(f64::total_cmp)
        .map_or(0., |right| right + half + gap);
    let position = [
        axis[0] * distance - center[0],
        -local.min[1],
        axis[2] * distance - center[2],
    ];
    super::apply(
        cx,
        comp,
        &pm_scene3d::to_model(
            &json!({"operation":"update_object","target":id,"patch":{"p":{"x":position[0],"y":position[1],"z":position[2]}}}),
        ),
        &[],
        time,
        false,
        None,
    )?;
    if !frame {
        return Ok(());
    }
    let Some((camera_id, pose, mut lens)) = camera else {
        return Ok(());
    };
    let (_, camera_layer) = cx.layer(&camera_id)?;
    if camera_layer.get("lock").and_then(Value::as_bool) == Some(true)
        || (cx.origin().is_generated()
            && camera_layer
                .get("locked_intent")
                .and_then(Value::as_object)
                .is_some_and(|m| m.values().any(Value::is_truthy)))
    {
        return Ok(());
    }
    for path in [
        "position.x",
        "position.y",
        "position.z",
        "camera.targetX",
        "camera.targetY",
        "camera.targetZ",
        "camera.zoom",
        "camera.far",
    ] {
        if super::guard(cx, &camera_id, path).is_err() {
            return Ok(());
        }
    }
    let imported = Bounds {
        min: nav::add(local.min, position),
        max: nav::add(local.max, position),
    };
    let bounds = existing.into_iter().fold(imported, union);
    let aspect = json["w"].as_f64().unwrap_or(1.) / json["h"].as_f64().unwrap_or(1.).max(1.);
    match &mut lens {
        nav::Lens::Perspective { aspect: a, .. } => *a = aspect,
        nav::Lens::Orthographic { width, .. } => *width = 6. * aspect,
    };
    let next = nav::frame(pose, bounds, lens);
    let index = layers
        .iter()
        .position(|l| l["id"] == camera_id)
        .ok_or_else(|| invalid("Import camera disappeared"))?;
    let request = pm_scene3d::to_model(
        &json!({"op":"parent3D","comp":comp_id,"open":comp_id,"layer":index,"t":time}),
    );
    let parent = geometry::matrix(
        &pm_scene3d::from_model(&queries.query(&request).map_err(invalid)?).map_err(invalid)?,
    )
    .map_err(invalid)?;
    let inverse = geometry::inverse(parent).map_err(invalid)?;
    let eye = pm_scene3d::render::gltf::math::apply(&inverse, next.position);
    let target = pm_scene3d::render::gltf::math::apply(&inverse, next.target);
    let mut params = json!({"x":eye[0],"y":eye[1],"z":eye[2],"targetX":target[0],"targetY":target[1],"targetZ":target[2],"zoom":next.zoom});
    if let nav::Lens::Perspective { far, .. } = lens {
        params["far"] = json!(
            far.max(nav::length(nav::sub(next.position, next.target)) + bounds.radius() * 2.)
        );
    }
    super::apply(
        cx,
        comp,
        &pm_scene3d::to_model(
            &json!({"operation":"set_camera","target":camera_id,"patch":{"p":params}}),
        ),
        &[],
        time,
        false,
        None,
    )
}
