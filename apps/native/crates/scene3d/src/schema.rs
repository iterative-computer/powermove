//! Strict authored schema and normalized channels, ported from scene3d/schema.ts.
use crate::{Error, Result, fail};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
pub const SCENE: &str = "powermove.3d.scene";
pub const OBJECT: &str = "powermove.3d.object";
pub const LIGHT: &str = "powermove.3d.light";
pub const CAMERA: &str = "powermove.3d.camera";
pub const MAX_DOCUMENT: usize = 8_000_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Object,
    Light,
    Camera,
}
impl Role {
    pub fn definition(self) -> &'static str {
        match self {
            Self::Object => OBJECT,
            Self::Light => LIGHT,
            Self::Camera => CAMERA,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Light => "light",
            Self::Camera => "camera",
        }
    }
}
pub fn role(layer: &Value) -> Option<Role> {
    match layer["d"]["definition"].as_str()? {
        OBJECT => Some(Role::Object),
        LIGHT => Some(Role::Light),
        CAMERA => Some(Role::Camera),
        _ => None,
    }
}
pub const TRANSFORMS: &[(&str, &str)] = &[
    ("x", "position.x"),
    ("y", "position.y"),
    ("z", "position.z"),
    ("rx", "rotation.x"),
    ("ry", "rotation.y"),
    ("rz", "rotation"),
    ("sx", "scale.x"),
    ("sy", "scale.y"),
    ("sz", "scale.z"),
];
#[derive(Clone, Copy)]
enum Kind {
    Number(f64, f64),
    Color,
    Bool,
}
#[derive(Clone)]
struct Field {
    name: &'static str,
    kind: Kind,
    default: Value,
}
fn n(name: &'static str, lo: f64, hi: f64, v: f64) -> Field {
    Field {
        name,
        kind: Kind::Number(lo, hi),
        default: json!(v),
    }
}
fn color(name: &'static str, v: &str) -> Field {
    Field {
        name,
        kind: Kind::Color,
        default: json!(v),
    }
}
fn boolean(name: &'static str, v: bool) -> Field {
    Field {
        name,
        kind: Kind::Bool,
        default: json!(v),
    }
}
fn fields(group: &str) -> Vec<Field> {
    let basic = |k, v| n(k, -100000., 100000., v);
    let unit = |k, v| n(k, 0., 1., v);
    match group {
        "object" => vec![
            basic("x", 0.),
            basic("y", 0.),
            basic("z", 0.),
            n("rx", -360000., 360000., 0.),
            n("ry", -360000., 360000., 0.),
            n("rz", -360000., 360000., 0.),
            n("sx", -10000., 10000., 1.),
            n("sy", -10000., 10000., 1.),
            n("sz", -10000., 10000., 1.),
            boolean("visible", true),
        ],
        "material" => vec![
            color("color", "#C7C4FF"),
            n("roughness", 0.02, 1., 0.4),
            unit("metalness", 0.1),
            color("emissive", "#000000"),
            n("emissiveIntensity", 0., 100., 1.),
            unit("opacity", 1.),
            unit("clearcoat", 0.),
            n("clearcoatRoughness", 0.002, 1., 0.03),
            unit("transmission", 0.),
            n("ior", 1., 2.6, 1.5),
            unit("specular", 1.),
            unit("sheen", 0.),
            n("iridescence", 0., 1200., 0.),
            unit("dispersion", 0.),
            unit("droplets", 0.),
            n("anisotropy", -1., 1., 0.),
            n("subsurface", 0., 1000., 0.),
        ],
        "light" => vec![
            basic("x", -3.),
            basic("y", 5.),
            basic("z", 4.),
            basic("targetX", 0.),
            basic("targetY", 0.),
            basic("targetZ", 0.),
            color("color", "#FFFFFF"),
            n("intensity", 0., 10000., 3.),
            n("distance", 0., 100000., 0.),
            n("decay", 0., 4., 2.),
            n("angle", 1., 89., 45.),
            unit("penumbra", 0.4),
            boolean("visible", true),
            n("width", 0.001, 100000., 2.),
            n("height", 0.001, 100000., 1.2),
        ],
        "camera" => vec![
            basic("x", 3.),
            basic("y", 2.),
            basic("z", 6.),
            basic("targetX", 0.),
            basic("targetY", 0.),
            basic("targetZ", 0.),
            n("fov", 1., 150., 45.),
            n("zoom", 0.001, 1000., 1.),
            n("near", 0.0001, 10000., 0.01),
            n("far", 0.01, 100000., 1000.),
            n("fStop", 0., 64., 0.),
            n("focusDistance", 0., 100000., 0.),
        ],
        "environment" => vec![
            n("ambient", 0., 100., 0.5),
            color("ambientColor", "#FFFFFF"),
            n("exposure", 0.01, 20., 1.),
        ],
        _ => vec![],
    }
}
pub(crate) fn strict(v: &Value, allowed: &[&str]) -> Result<()> {
    let m = v
        .as_object()
        .ok_or_else(|| Error("Expected a JSON object".into()))?;
    if let Some(k) = m.keys().find(|k| !allowed.contains(&k.as_str())) {
        return fail(format!("Unknown 3D field: {k}"));
    }
    Ok(())
}
fn number(v: &Value, lo: f64, hi: f64, integer: bool) -> Result<f64> {
    let x = v
        .as_f64()
        .filter(|x| x.is_finite() && *x >= lo && *x <= hi && (!integer || x.fract() == 0.))
        .ok_or_else(|| {
            Error(format!(
                "Expected {}number in {lo}..={hi}",
                if integer { "integer " } else { "" }
            ))
        })?;
    Ok(x)
}
fn text(v: &Value, min: usize, max: usize) -> Result<&str> {
    v.as_str()
        .filter(|x| (min..=max).contains(&x.encode_utf16().count()))
        .ok_or_else(|| Error("Invalid 3D string".into()))
}
fn id(v: &Value) -> Result<&str> {
    let s = text(v, 1, 64)?;
    if !s.as_bytes()[0].is_ascii_alphabetic()
        || !s
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return fail("Invalid 3D id");
    }
    Ok(s)
}
fn check(v: &Value, k: Kind) -> Result<()> {
    match k {
        Kind::Number(lo, hi) => {
            number(v, lo, hi, false)?;
        }
        Kind::Bool => {
            if !v.is_boolean() {
                return fail("Expected boolean");
            }
        }
        Kind::Color => {
            let c = text(v, 7, 7)?;
            if !c.starts_with('#') || !c[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
                return fail("Expected #RRGGBB");
            }
        }
    }
    Ok(())
}
pub fn channel(v: Value) -> Value {
    json!({"v":v,"kf":[],"expr":null})
}
fn normalize_channel(v: Value, k: Kind) -> Result<Value> {
    if !v.is_object() {
        check(&v, k)?;
        return Ok(channel(v));
    }
    check(&v["v"], k)?;
    let a = v["kf"]
        .as_array()
        .filter(|a| a.len() <= 2400)
        .ok_or_else(|| Error("Invalid keyframes (maximum 2400)".into()))?;
    for f in a {
        number(&f["t"], 0., 100000., false)?;
        check(&f["v"], k)?;
    }
    if !v["expr"].is_null() {
        text(&v["expr"], 0, 2000)?;
    }
    let mut v = v;
    if v.get("expr").is_none() {
        v["expr"] = Value::Null;
    }
    Ok(v)
}
pub fn properties(group: &str, raw: Option<&Value>) -> Result<Value> {
    let empty = json!({});
    let raw = raw.unwrap_or(&empty);
    let fs = fields(group);
    strict(raw, &fs.iter().map(|f| f.name).collect::<Vec<_>>())?;
    let mut out = json!({});
    for f in fs {
        out[f.name] = normalize_channel(raw.get(f.name).cloned().unwrap_or(f.default), f.kind)?;
    }
    Ok(out)
}
fn default(v: &mut Value, k: &str, fallback: Value) {
    if v.get(k).is_none() {
        v[k] = fallback;
    }
}
fn choice(v: &Value, choices: &[&str]) -> Result<()> {
    if !v.as_str().is_some_and(|x| choices.contains(&x)) {
        return fail("Unknown 3D enum value");
    }
    Ok(())
}
fn flag(v: &mut Value, k: &str, d: bool) -> Result<()> {
    default(v, k, json!(d));
    check(&v[k], Kind::Bool)
}
pub fn material(mut v: Value) -> Result<Value> {
    strict(&v, &["p", "maps", "doubleSided", "screen"])?;
    v["p"] = properties("material", v.get("p"))?;
    default(&mut v, "maps", json!({}));
    strict(
        &v["maps"],
        &[
            "color",
            "normal",
            "roughness",
            "metalness",
            "emissive",
            "ao",
        ],
    )?;
    for s in v["maps"].as_object().unwrap().values() {
        text(s, 1, 160)?;
    }
    flag(&mut v, "doubleSided", true)?;
    default(&mut v, "screen", Value::Null);
    if !v["screen"].is_null() {
        text(&v["screen"], 1, 160)?;
    }
    Ok(v)
}
pub fn source(v: &Value) -> Result<usize> {
    if v.get("primitive").is_some() {
        strict(v, &["primitive", "parameters"])?;
        choice(
            &v["primitive"],
            &[
                "box",
                "roundedBox",
                "sphere",
                "plane",
                "cylinder",
                "cone",
                "torus",
                "capsule",
                "icosahedron",
            ],
        )?;
        let e = json!({});
        let p = v.get("parameters").unwrap_or(&e);
        strict(
            p,
            &[
                "width",
                "height",
                "depth",
                "radius",
                "radiusTop",
                "radiusBottom",
                "tube",
                "segments",
                "detail",
            ],
        )?;
        for (k, x) in p.as_object().unwrap() {
            match k.as_str() {
                "segments" => {
                    number(x, 3., 128., true)?;
                }
                "detail" => {
                    number(x, 0., 5., true)?;
                }
                "radiusTop" | "radiusBottom" => {
                    number(x, 0., 10000., false)?;
                }
                _ => {
                    number(x, 0.0001, 10000., false)?;
                }
            }
        }
        let s = p["segments"].as_f64().unwrap_or(48.) as usize;
        return Ok(match v["primitive"].as_str().unwrap() {
            "box" => 24,
            "roundedBox" => 6 * (2 * p["segments"].as_f64().unwrap_or(6.) as usize + 1).pow(2),
            "plane" => 4,
            "icosahedron" => 60 * 4usize.pow(p["detail"].as_f64().unwrap_or(2.) as u32),
            "sphere" | "torus" => (s + 1) * (8.max(s / 2) + 1),
            _ => (s + 1) * 12,
        });
    }
    if v.get("assetId").is_some() {
        strict(v, &["assetId"])?;
        text(&v["assetId"], 1, 160)?;
        return Ok(0);
    }
    if let Some(m) = v.get("mesh") {
        strict(v, &["mesh"])?;
        strict(m, &["positions", "indices", "normals", "uvs"])?;
        let a = m["positions"]
            .as_array()
            .filter(|a| a.len() >= 9 && a.len() <= 300000 && a.len() % 3 == 0)
            .ok_or_else(|| Error("Positions must contain xyz triples".into()))?;
        for n in a {
            number(n, -100000., 100000., false)?;
        }
        let count = a.len() / 3;
        if let Some(indices) = m.get("indices") {
            let a = indices
                .as_array()
                .filter(|a| a.len() >= 3 && a.len() <= 300000 && a.len() % 3 == 0)
                .ok_or_else(|| Error("Invalid triangles".into()))?;
            for i in a {
                number(i, 0., (count - 1) as f64, true)?;
            }
        } else if count % 3 != 0 {
            return fail("Faces must contain valid triangles");
        }
        for (key, len, lo, hi) in [
            ("normals", a.len(), -1., 1.),
            ("uvs", count * 2, -100000., 100000.),
        ] {
            if let Some(v) = m.get(key) {
                let a = v
                    .as_array()
                    .filter(|a| a.len() == len)
                    .ok_or_else(|| Error(format!("Invalid {key} count")))?;
                for n in a {
                    number(n, lo, hi, false)?;
                }
            }
        }
        return Ok(count);
    }
    let (key, min) = if v.get("lathe").is_some() {
        strict(v, &["lathe", "segments"])?;
        ("lathe", 2)
    } else {
        strict(v, &["extrude", "depth", "bevel"])?;
        number(&v["depth"], 0.0001, 10000., false)?;
        if let Some(b) = v.get("bevel") {
            number(b, 0., 100., false)?;
        }
        ("extrude", 3)
    };
    let pts = v[key]
        .as_array()
        .filter(|a| a.len() >= min && a.len() <= 1024)
        .ok_or_else(|| Error("Invalid profile".into()))?;
    for p in pts {
        let p = p
            .as_array()
            .filter(|p| p.len() == 2)
            .ok_or_else(|| Error("Expected profile pair".into()))?;
        number(
            &p[0],
            if key == "lathe" { 0. } else { -100000. },
            100000.,
            false,
        )?;
        number(&p[1], -100000., 100000., false)?;
    }
    if key == "lathe" {
        let s = if let Some(s) = v.get("segments") {
            number(s, 3., 128., true)? as usize
        } else {
            48
        };
        Ok(pts.len() * s)
    } else {
        Ok(pts.len()
            * if v["bevel"].as_f64().unwrap_or(0.) != 0. {
                60
            } else {
                12
            })
    }
}
pub fn object(mut v: Value) -> Result<Value> {
    strict(
        &v,
        &[
            "id",
            "name",
            "parent",
            "source",
            "p",
            "material",
            "useSourceMaterials",
            "castShadow",
            "receiveShadow",
        ],
    )?;
    id(&v["id"])?;
    default(&mut v, "name", json!("Object"));
    text(&v["name"], 1, 160)?;
    default(&mut v, "parent", Value::Null);
    if !v["parent"].is_null() {
        id(&v["parent"])?;
    }
    source(&v["source"])?;
    v["p"] = properties("object", v.get("p"))?;
    v["material"] = material(v.get("material").cloned().unwrap_or(json!({})))?;
    for (k, b) in [
        ("useSourceMaterials", false),
        ("castShadow", true),
        ("receiveShadow", true),
    ] {
        flag(&mut v, k, b)?;
    }
    Ok(v)
}
pub fn light(mut v: Value) -> Result<Value> {
    strict(&v, &["id", "name", "type", "p", "castShadow"])?;
    id(&v["id"])?;
    default(&mut v, "name", json!("Light"));
    text(&v["name"], 1, 160)?;
    default(&mut v, "type", json!("sun"));
    choice(&v["type"], &["sun", "point", "spot", "area"])?;
    v["p"] = properties("light", v.get("p"))?;
    flag(&mut v, "castShadow", true)?;
    Ok(v)
}
pub fn camera(mut v: Value) -> Result<Value> {
    strict(&v, &["projection", "p"])?;
    default(&mut v, "projection", json!("perspective"));
    choice(&v["projection"], &["perspective", "orthographic"])?;
    v["p"] = properties("camera", v.get("p"))?;
    if v["p"]["far"]["v"].as_f64() <= v["p"]["near"]["v"].as_f64() {
        return fail("Camera far plane must be beyond its near plane");
    }
    Ok(v)
}
pub fn environment(mut v: Value) -> Result<Value> {
    strict(
        &v,
        &["p", "background", "shadows", "shadowCatcher", "samples"],
    )?;
    v["p"] = properties("environment", v.get("p"))?;
    default(&mut v, "background", Value::Null);
    if !v["background"].is_null() {
        check(&v["background"], Kind::Color)?;
    }
    flag(&mut v, "shadows", true)?;
    flag(&mut v, "shadowCatcher", false)?;
    default(&mut v, "samples", json!(64));
    number(&v["samples"], 4., 4096., true)?;
    Ok(v)
}
pub fn parse_scene(mut v: Value) -> Result<Value> {
    plain(&v)?;
    strict(
        &v,
        &["version", "objects", "lights", "camera", "environment"],
    )?;
    default(&mut v, "version", json!(1));
    if v["version"].as_f64() != Some(1.) {
        return fail("Unknown 3D scene version");
    }
    for (k, max) in [("objects", 512), ("lights", 16)] {
        default(&mut v, k, json!([]));
        let a = v[k]
            .as_array()
            .filter(|a| a.len() <= max)
            .ok_or_else(|| Error(format!("Too many or invalid {k}")))?
            .clone();
        v[k] = Value::Array(
            a.into_iter()
                .map(|n| if k == "objects" { object(n) } else { light(n) })
                .collect::<Result<_>>()?,
        );
    }
    v["camera"] = camera(v.get("camera").cloned().unwrap_or(json!({})))?;
    v["environment"] = environment(v.get("environment").cloned().unwrap_or(json!({})))?;
    let mut ids = HashSet::from(["camera".to_string()]);
    for n in v["objects"]
        .as_array()
        .unwrap()
        .iter()
        .chain(v["lights"].as_array().unwrap())
    {
        let i = n["id"].as_str().unwrap();
        if !ids.insert(i.into()) {
            return fail(format!("Duplicate 3D id: {i}"));
        }
    }
    let objects = v["objects"].as_array().unwrap();
    let by: HashMap<_, _> = objects
        .iter()
        .map(|n| (n["id"].as_str().unwrap(), n))
        .collect();
    let mut vertices = 0;
    for o in objects {
        vertices += source(&o["source"])?;
        let mut chain = HashSet::from([o["id"].as_str().unwrap()]);
        let mut p = o["parent"].as_str();
        while let Some(parent) = p {
            if !chain.insert(parent) {
                return fail("3D object parenting contains a cycle");
            }
            p = by
                .get(parent)
                .ok_or_else(|| Error(format!("Missing 3D parent: {parent}")))?["parent"]
                .as_str();
        }
    }
    if vertices > 500000 {
        return fail("3D scene exceeds the geometry budget");
    }
    Ok(v)
}
pub fn plain(v: &Value) -> Result<()> {
    if serde_json::to_vec(v)
        .map_err(|e| Error(e.to_string()))?
        .len()
        > MAX_DOCUMENT
    {
        return fail("3D document exceeds the 8 MB geometry budget");
    }
    fn walk(v: &Value, depth: usize) -> Result<()> {
        if depth > 32 {
            return fail("3D edit is too deeply nested");
        }
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    if ["__proto__", "prototype", "constructor"].contains(&k.as_str()) {
                        return fail("Unsafe 3D field");
                    }
                    walk(v, depth + 1)?;
                }
            }
            Value::Array(a) => {
                for v in a {
                    walk(v, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(v, 0)
}
pub fn layer_data(r: Role, raw: Value) -> Result<Value> {
    plain(&raw)?;
    match r {
        Role::Object => {
            strict(&raw, &["object"])?;
            strict(
                &raw["object"],
                &[
                    "source",
                    "material",
                    "useSourceMaterials",
                    "castShadow",
                    "receiveShadow",
                ],
            )?;
            let mut o = raw["object"].clone();
            o["id"] = json!("model");
            let mut o = object(o)?;
            for k in ["id", "name", "parent", "p"] {
                o.as_object_mut().unwrap().remove(k);
            }
            Ok(json!({"object":o}))
        }
        Role::Light => {
            strict(&raw, &["light"])?;
            strict(&raw["light"], &["type", "p", "castShadow"])?;
            for k in ["x", "y", "z", "visible"] {
                if raw["light"]["p"].get(k).is_some() {
                    return fail("Use the light layer transform for position and visibility");
                }
            }
            let mut l = raw["light"].clone();
            l["id"] = json!("light");
            let mut l = light(l)?;
            for k in ["id", "name"] {
                l.as_object_mut().unwrap().remove(k);
            }
            for k in ["x", "y", "z", "visible"] {
                l["p"].as_object_mut().unwrap().remove(k);
            }
            Ok(json!({"light":l}))
        }
        Role::Camera => {
            strict(&raw, &["camera", "environment"])?;
            for k in ["x", "y", "z"] {
                if raw["camera"]["p"].get(k).is_some() {
                    return fail("Use the camera layer transform for position");
                }
            }
            let mut c = camera(raw.get("camera").cloned().unwrap_or(json!({})))?;
            for k in ["x", "y", "z"] {
                c["p"].as_object_mut().unwrap().remove(k);
            }
            Ok(
                json!({"camera":c,"environment":environment(raw.get("environment").cloned().unwrap_or(json!({})))?}),
            )
        }
    }
}
/// Validate an authored channel without losing keyframe metadata.
pub fn validate_channel(group: &str, key: &str, v: Value) -> Result<Value> {
    let f = fields(group)
        .into_iter()
        .find(|f| f.name == key)
        .ok_or_else(|| Error(format!("Unknown 3D channel: {group}.{key}")))?;
    normalize_channel(v, f.kind)
}
