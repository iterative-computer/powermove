//! Refusal boundary for a stored or evaluated 3D scene. Mirrors the shape,
//! ranges and cross-field checks of `core/scene3d/schema.ts::parseScene`.
//! The four app-reachable limits (object/light count, display-name length,
//! serialized 8 MB) are intentionally not refusals in the native engine.

use std::collections::{HashMap, HashSet};

use pm_model::json::{JsString, Map, Value};

type Check = Result<(), String>;

fn bad(path: &str, reason: &str) -> String {
    format!("{path}: {reason}")
}
fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map, String> {
    match value {
        Value::Object(v) => Ok(v),
        _ => Err(bad(path, "expected object")),
    }
}
fn array<'a>(value: &'a Value, path: &str) -> Result<&'a [Value], String> {
    match value {
        Value::Array(v) => Ok(v),
        _ => Err(bad(path, "expected array")),
    }
}
fn string<'a>(
    value: &'a Value,
    path: &str,
    min: usize,
    max: Option<usize>,
) -> Result<&'a str, String> {
    let Value::String(v) = value else {
        return Err(bad(path, "expected string"));
    };
    let len = v.utf16_len();
    if len < min || max.is_some_and(|max| len > max) {
        return Err(bad(path, "invalid string length"));
    }
    // Generic Zod strings accept lone UTF-16 surrogates. Callers that need
    // ASCII syntax (identifiers, colors and enums) reject this stand-in.
    Ok(v.as_str().unwrap_or("\u{fffd}"))
}
fn number(value: &Value, path: &str, lo: f64, hi: f64, integer: bool) -> Result<f64, String> {
    let Value::Number(v) = value else {
        return Err(bad(path, "expected number"));
    };
    if !v.is_finite() || *v < lo || *v > hi || (integer && v.fract() != 0.0) {
        return Err(bad(path, "number outside schema range"));
    }
    Ok(*v)
}
fn boolean(value: &Value, path: &str) -> Check {
    if matches!(value, Value::Bool(_)) {
        Ok(())
    } else {
        Err(bad(path, "expected boolean"))
    }
}
fn keys(map: &Map, path: &str, allowed: &[&str]) -> Check {
    for key in map.keys() {
        if !allowed.iter().any(|allowed| key.as_str() == Some(allowed)) {
            return Err(bad(path, &format!("unknown key {}", key.to_string_lossy())));
        }
    }
    Ok(())
}
fn required<'a>(map: &'a Map, key: &str, path: &str) -> Result<&'a Value, String> {
    map.get(key)
        .ok_or_else(|| bad(&format!("{path}.{key}"), "required"))
}
fn identifier<'a>(value: &'a Value, path: &str) -> Result<&'a str, String> {
    let s = string(value, path, 1, Some(64))?;
    let mut b = s.bytes();
    if !b.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !b.all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(bad(path, "invalid identifier"));
    }
    Ok(s)
}
fn color(value: &Value, path: &str) -> Check {
    let s = string(value, path, 7, Some(7))?;
    if s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(bad(path, "invalid color"))
    }
}
fn enumeration(value: &Value, path: &str, choices: &[&str]) -> Check {
    let s = string(value, path, 1, None)?;
    if choices.contains(&s) {
        Ok(())
    } else {
        Err(bad(path, "unknown enum value"))
    }
}
#[derive(Clone, Copy)]
enum Kind {
    Number(f64, f64),
    Bool,
    Color,
}
fn kind(value: &Value, path: &str, k: Kind) -> Check {
    match k {
        Kind::Number(a, b) => number(value, path, a, b, false).map(|_| ()),
        Kind::Bool => boolean(value, path),
        Kind::Color => color(value, path),
    }
}
fn channel(value: &Value, path: &str, k: Kind, min_time: f64) -> Check {
    if !matches!(value, Value::Object(_)) {
        return kind(value, path, k);
    }
    let m = object(value, path)?;
    kind(required(m, "v", path)?, &format!("{path}.v"), k)?;
    let keyframes = array(required(m, "kf", path)?, &format!("{path}.kf"))?;
    if keyframes.len() > 2400 {
        return Err(bad(path, "too many keyframes"));
    }
    for (i, frame) in keyframes.iter().enumerate() {
        let p = format!("{path}.kf[{i}]");
        let f = object(frame, &p)?;
        number(
            required(f, "t", &p)?,
            &format!("{p}.t"),
            min_time,
            100_000.0,
            false,
        )?;
        kind(required(f, "v", &p)?, &format!("{p}.v"), k)?;
    }
    if let Some(expr) = m.get("expr")
        && !matches!(expr, Value::Null)
    {
        string(expr, &format!("{path}.expr"), 0, Some(2000))?;
    }
    Ok(())
}
fn channels(map: &Map, path: &str, fields: &[(&str, Kind)], min_time: f64) -> Check {
    keys(map, path, &fields.iter().map(|f| f.0).collect::<Vec<_>>())?;
    for &(name, k) in fields {
        if let Some(v) = map.get(name) {
            channel(v, &format!("{path}.{name}"), k, min_time)?;
        }
    }
    Ok(())
}
const BASIC: Kind = Kind::Number(-100_000.0, 100_000.0);
const ANGLE: Kind = Kind::Number(-360_000.0, 360_000.0);
const UNIT: Kind = Kind::Number(0.0, 1.0);
const POSITIVE: Kind = Kind::Number(0.0001, 10_000.0);
const TRANSFORM: &[(&str, Kind)] = &[
    ("x", BASIC),
    ("y", BASIC),
    ("z", BASIC),
    ("rx", ANGLE),
    ("ry", ANGLE),
    ("rz", ANGLE),
    ("sx", Kind::Number(-10_000.0, 10_000.0)),
    ("sy", Kind::Number(-10_000.0, 10_000.0)),
    ("sz", Kind::Number(-10_000.0, 10_000.0)),
    ("visible", Kind::Bool),
];
const MATERIAL: &[(&str, Kind)] = &[
    ("color", Kind::Color),
    ("roughness", UNIT),
    ("metalness", UNIT),
    ("emissive", Kind::Color),
    ("emissiveIntensity", Kind::Number(0.0, 100.0)),
    ("opacity", UNIT),
];
const LIGHT: &[(&str, Kind)] = &[
    ("x", BASIC),
    ("y", BASIC),
    ("z", BASIC),
    ("targetX", BASIC),
    ("targetY", BASIC),
    ("targetZ", BASIC),
    ("color", Kind::Color),
    ("intensity", Kind::Number(0.0, 10_000.0)),
    ("distance", Kind::Number(0.0, 100_000.0)),
    ("decay", Kind::Number(0.0, 4.0)),
    ("angle", Kind::Number(1.0, 89.0)),
    ("penumbra", UNIT),
    ("width", Kind::Number(0.001, 10_000.0)),
    ("height", Kind::Number(0.001, 10_000.0)),
    ("visible", Kind::Bool),
];
const CAMERA: &[(&str, Kind)] = &[
    ("x", BASIC),
    ("y", BASIC),
    ("z", BASIC),
    ("targetX", BASIC),
    ("targetY", BASIC),
    ("targetZ", BASIC),
    ("fov", Kind::Number(1.0, 150.0)),
    ("zoom", Kind::Number(0.001, 1000.0)),
    ("near", POSITIVE),
    ("far", Kind::Number(0.01, 100_000.0)),
];
const ENVIRONMENT: &[(&str, Kind)] = &[
    ("ambient", Kind::Number(0.0, 100.0)),
    ("ambientColor", Kind::Color),
    ("exposure", Kind::Number(0.01, 20.0)),
];

fn points(value: &Value, path: &str, minimum: usize) -> Result<usize, String> {
    let a = array(value, path)?;
    if a.len() < minimum || a.len() > 1024 {
        return Err(bad(path, "invalid point count"));
    }
    for (i, p) in a.iter().enumerate() {
        let pth = format!("{path}[{i}]");
        let xy = array(p, &pth)?;
        if xy.len() != 2 {
            return Err(bad(&pth, "expected two coordinates"));
        }
        for (j, v) in xy.iter().enumerate() {
            number(v, &format!("{pth}[{j}]"), -100_000.0, 100_000.0, false)?;
        }
    }
    Ok(a.len())
}
fn source(value: &Value, path: &str) -> Result<usize, String> {
    let s = object(value, path)?;
    if let Some(v) = s.get("primitive") {
        keys(s, path, &["primitive", "parameters"])?;
        enumeration(
            v,
            &format!("{path}.primitive"),
            &[
                "box",
                "sphere",
                "plane",
                "cylinder",
                "cone",
                "torus",
                "capsule",
                "icosahedron",
            ],
        )?;
        let primitive = string(v, path, 1, None)?;
        let mut segments = 48.0;
        let mut detail = 2.0;
        if let Some(v) = s.get("parameters") {
            let p = object(v, &format!("{path}.parameters"))?;
            keys(
                p,
                path,
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
            for (k, v) in p.iter() {
                let name = k.as_str().unwrap_or("");
                let (lo, hi, int) = match name {
                    "radiusTop" | "radiusBottom" => (0.0, 10_000.0, false),
                    "segments" => (3.0, 128.0, true),
                    "detail" => (0.0, 5.0, true),
                    _ => (0.0001, 10_000.0, false),
                };
                let n = number(v, &format!("{path}.parameters.{name}"), lo, hi, int)?;
                if name == "segments" {
                    segments = n;
                }
                if name == "detail" {
                    detail = n;
                }
            }
        }
        let n = match primitive {
            "box" => 24,
            "plane" => 4,
            "icosahedron" => 60 * 4usize.pow(detail as u32),
            "sphere" | "torus" => {
                ((segments + 1.0) * (8.0f64.max((segments / 2.0).floor()) + 1.0)) as usize
            }
            _ => ((segments + 1.0) * 12.0) as usize,
        };
        return Ok(n);
    }
    if let Some(v) = s.get("assetId") {
        keys(s, path, &["assetId"])?;
        string(v, &format!("{path}.assetId"), 1, Some(160))?;
        return Ok(0);
    }
    if let Some(v) = s.get("lathe") {
        keys(s, path, &["lathe", "segments"])?;
        let n = points(v, &format!("{path}.lathe"), 2)?;
        for p in array(v, path)? {
            if let Value::Array(xy) = p
                && let Value::Number(r) = xy[0]
                && r < 0.0
            {
                return Err("Lathe radii cannot be negative".into());
            }
        }
        let seg = if let Some(v) = s.get("segments") {
            number(v, &format!("{path}.segments"), 3.0, 128.0, true)? as usize
        } else {
            48
        };
        return Ok(n * seg);
    }
    if let Some(v) = s.get("extrude") {
        keys(s, path, &["extrude", "depth", "bevel"])?;
        let n = points(v, &format!("{path}.extrude"), 3)?;
        number(
            required(s, "depth", path)?,
            &format!("{path}.depth"),
            0.0001,
            10_000.0,
            false,
        )?;
        let bevel = if let Some(v) = s.get("bevel") {
            number(v, &format!("{path}.bevel"), 0.0, 100.0, false)?
        } else {
            0.0
        };
        return Ok(n * if bevel != 0.0 { 60 } else { 12 });
    }
    if let Some(v) = s.get("mesh") {
        keys(s, path, &["mesh"])?;
        return mesh(v, &format!("{path}.mesh"));
    }
    Err(bad(path, "invalid source"))
}
fn numeric_array(
    value: &Value,
    path: &str,
    min: usize,
    max: usize,
    lo: f64,
    hi: f64,
    integer: bool,
) -> Result<usize, String> {
    let a = array(value, path)?;
    if a.len() < min || a.len() > max {
        return Err(bad(path, "invalid array length"));
    }
    for (i, v) in a.iter().enumerate() {
        number(v, &format!("{path}[{i}]"), lo, hi, integer)?;
    }
    Ok(a.len())
}
fn mesh(value: &Value, path: &str) -> Result<usize, String> {
    let m = object(value, path)?;
    keys(
        m,
        path,
        &["positions", "indices", "groups", "normals", "uvs"],
    )?;
    let p = numeric_array(
        required(m, "positions", path)?,
        &format!("{path}.positions"),
        9,
        300_000,
        -100_000.0,
        100_000.0,
        false,
    )?;
    if p % 3 != 0 {
        return Err("Positions must contain xyz triples".into());
    }
    let count = p / 3;
    let faces = if let Some(v) = m.get("indices") {
        let a = array(v, &format!("{path}.indices"))?;
        numeric_array(
            v,
            &format!("{path}.indices"),
            3,
            300_000,
            0.0,
            f64::MAX,
            true,
        )?;
        if a.len() % 3 != 0
            || a.iter()
                .any(|v| matches!(v,Value::Number(n) if *n>=count as f64))
        {
            return Err("Faces must contain valid triangle indices".into());
        }
        a.len()
    } else {
        if count % 3 != 0 {
            return Err("Faces must contain valid triangle indices".into());
        }
        count
    };
    if let Some(v) = m.get("normals") {
        let n = numeric_array(v, &format!("{path}.normals"), 0, 300_000, -1.0, 1.0, false)?;
        if n != p {
            return Err("Each vertex needs one normal".into());
        }
    }
    if let Some(v) = m.get("uvs") {
        let n = numeric_array(
            v,
            &format!("{path}.uvs"),
            0,
            200_000,
            -100_000.0,
            100_000.0,
            false,
        )?;
        if n != count * 2 {
            return Err("Each vertex needs one UV pair".into());
        }
    }
    if let Some(v) = m.get("groups") {
        let a = array(v, &format!("{path}.groups"))?;
        if a.len() > 64 {
            return Err(bad(path, "too many groups"));
        }
        for (i, g) in a.iter().enumerate() {
            let p = format!("{path}.groups[{i}]");
            let g = object(g, &p)?;
            keys(g, &p, &["start", "count", "materialIndex"])?;
            let start = number(
                required(g, "start", &p)?,
                &format!("{p}.start"),
                0.0,
                f64::MAX,
                true,
            )? as usize;
            let count = number(
                required(g, "count", &p)?,
                &format!("{p}.count"),
                0.0,
                f64::MAX,
                true,
            )? as usize;
            number(
                required(g, "materialIndex", &p)?,
                &format!("{p}.materialIndex"),
                0.0,
                63.0,
                true,
            )?;
            if !start.is_multiple_of(3)
                || !count.is_multiple_of(3)
                || start.saturating_add(count) > faces
            {
                return Err("Invalid material face ranges".into());
            }
        }
    }
    Ok(count)
}

fn shader_value(v: &Value, path: &str) -> Check {
    match v {
        Value::Number(_) => kind(v, path, BASIC),
        Value::Bool(_) => Ok(()),
        Value::String(_) => color(v, path),
        Value::Array(a) if (2..=4).contains(&a.len()) => {
            for (i, v) in a.iter().enumerate() {
                kind(v, &format!("{path}[{i}]"), BASIC)?;
            }
            Ok(())
        }
        _ => Err(bad(path, "invalid shader value")),
    }
}
fn shader_channel(v: &Value, path: &str) -> Check {
    let m = object(v, path)?;
    shader_value(required(m, "v", path)?, &format!("{path}.v"))?;
    if let Some(v) = m.get("kf") {
        let frames = array(v, &format!("{path}.kf"))?;
        if frames.len() > 2400 {
            return Err(bad(path, "too many keyframes"));
        }
        for (i, f) in frames.iter().enumerate() {
            let p = format!("{path}.kf[{i}]");
            let f = object(f, &p)?;
            number(
                required(f, "t", &p)?,
                &format!("{p}.t"),
                -100_000_000.0,
                100_000_000.0,
                false,
            )?;
            shader_value(required(f, "v", &p)?, &format!("{p}.v"))?;
        }
    }
    if let Some(v) = m.get("expr")
        && !matches!(v, Value::Null)
    {
        string(v, &format!("{path}.expr"), 0, Some(2000))?;
    }
    Ok(())
}
fn socket(v: &Value, path: &str) -> Check {
    match v {
        Value::String(_) => string(v, path, 1, Some(160)).map(|_| ()),
        _ => number(v, path, 0.0, 128.0, true).map(|_| ()),
    }
}
const SHADER_NODES: &[&str] = &[
    "ShaderNodeBsdfPrincipled",
    "ShaderNodeOutputMaterial",
    "ShaderNodeTexCoord",
    "ShaderNodeMapping",
    "ShaderNodeTexNoise",
    "ShaderNodeTexVoronoi",
    "ShaderNodeTexWave",
    "ShaderNodeTexChecker",
    "ShaderNodeTexGradient",
    "ShaderNodeTexImage",
    "ShaderNodeBump",
    "ShaderNodeNormalMap",
    "ShaderNodeMath",
    "ShaderNodeVectorMath",
    "ShaderNodeMixRGB",
    "ShaderNodeMixShader",
    "ShaderNodeBsdfGlass",
    "ShaderNodeBsdfTransparent",
    "ShaderNodeEmission",
    "ShaderNodeValue",
    "ShaderNodeRGB",
    "ShaderNodeSeparateXYZ",
    "ShaderNodeCombineXYZ",
    "ShaderNodeHueSaturation",
    "ShaderNodeInvert",
    "ShaderNodeFresnel",
    "ShaderNodeLayerWeight",
    "ShaderNodeValToRGB",
    "ShaderNodeMapRange",
    "ShaderNodeClamp",
    "ShaderNodeVectorRotate",
];
fn graph(v: &Value, path: &str) -> Result<HashSet<String>, String> {
    let g = object(v, path)?;
    keys(g, path, &["nodes", "links"])?;
    let nodes = array(required(g, "nodes", path)?, &format!("{path}.nodes"))?;
    if nodes.is_empty() || nodes.len() > 128 {
        return Err(bad(path, "invalid shader node count"));
    }
    let mut ids = HashSet::new();
    let mut output = false;
    for (i, n) in nodes.iter().enumerate() {
        let p = format!("{path}.nodes[{i}]");
        let n = object(n, &p)?;
        keys(
            n,
            &p,
            &[
                "id",
                "type",
                "inputs",
                "properties",
                "image",
                "colorSpace",
                "ramp",
            ],
        )?;
        let id = identifier(required(n, "id", &p)?, &format!("{p}.id"))?;
        if !ids.insert(id.to_owned()) {
            return Err("Shader node IDs must be unique".into());
        }
        let ty = required(n, "type", &p)?;
        enumeration(ty, &format!("{p}.type"), SHADER_NODES)?;
        output |= matches!(ty,Value::String(s) if s.as_str()==Some("ShaderNodeOutputMaterial"));
        if let Some(v) = n.get("inputs") {
            let m = object(v, &format!("{p}.inputs"))?;
            for (k, v) in m.iter() {
                if k.utf16_len() == 0 || k.utf16_len() > 160 {
                    return Err(bad(&p, "invalid input key"));
                }
                shader_value(v, &format!("{p}.inputs.{}", k.to_string_lossy()))?;
            }
        }
        if let Some(v) = n.get("properties") {
            let m = object(v, &format!("{p}.properties"))?;
            keys(
                m,
                &p,
                &[
                    "operation",
                    "blend_type",
                    "noise_dimensions",
                    "voronoi_dimensions",
                    "distance",
                    "feature",
                    "wave_type",
                    "bands_direction",
                    "rings_direction",
                    "gradient_type",
                    "interpolation",
                    "extension",
                    "vector_type",
                    "space",
                    "distribution",
                    "use_clamp",
                ],
            )?;
            for (k, v) in m.iter() {
                let name = k.as_str().unwrap_or("");
                let vp = format!("{p}.properties.{name}");
                match name {
                    "use_clamp" => boolean(v, &vp)?,
                    "noise_dimensions" | "voronoi_dimensions" => {
                        enumeration(v, &vp, &["1D", "2D", "3D", "4D"])?
                    }
                    "wave_type" => enumeration(v, &vp, &["BANDS", "RINGS"])?,
                    "bands_direction" => enumeration(v, &vp, &["X", "Y", "Z", "DIAGONAL"])?,
                    "rings_direction" => enumeration(v, &vp, &["X", "Y", "Z", "SPHERICAL"])?,
                    "interpolation" => {
                        enumeration(v, &vp, &["Linear", "Closest", "Cubic", "Smart"])?
                    }
                    "extension" => enumeration(v, &vp, &["REPEAT", "EXTEND", "CLIP", "MIRROR"])?,
                    "vector_type" => {
                        enumeration(v, &vp, &["POINT", "TEXTURE", "VECTOR", "NORMAL"])?
                    }
                    "space" => enumeration(
                        v,
                        &vp,
                        &[
                            "TANGENT",
                            "OBJECT",
                            "WORLD",
                            "BLENDER_OBJECT",
                            "BLENDER_WORLD",
                        ],
                    )?,
                    _ => {
                        string(v, &vp, 0, Some(64))?;
                    }
                }
            }
        }
        if let Some(v) = n.get("image") {
            string(v, &format!("{p}.image"), 1, Some(160))?;
        }
        if let Some(v) = n.get("colorSpace") {
            enumeration(v, &format!("{p}.colorSpace"), &["sRGB", "Non-Color"])?;
        }
        if let Some(v) = n.get("ramp") {
            let rp = format!("{p}.ramp");
            let r = object(v, &rp)?;
            keys(r, &rp, &["interpolation", "stops"])?;
            if let Some(v) = r.get("interpolation") {
                enumeration(
                    v,
                    &format!("{rp}.interpolation"),
                    &["LINEAR", "EASE", "CONSTANT", "CARDINAL", "B_SPLINE"],
                )?;
            }
            let stops = array(required(r, "stops", &rp)?, &format!("{rp}.stops"))?;
            if !(2..=32).contains(&stops.len()) {
                return Err(bad(&rp, "invalid ramp stop count"));
            }
            for (j, s) in stops.iter().enumerate() {
                let sp = format!("{rp}.stops[{j}]");
                let s = object(s, &sp)?;
                keys(s, &sp, &["position", "color"])?;
                number(
                    required(s, "position", &sp)?,
                    &format!("{sp}.position"),
                    0.0,
                    1.0,
                    false,
                )?;
                color(required(s, "color", &sp)?, &format!("{sp}.color"))?;
            }
        }
    }
    if !output {
        return Err("A material needs an output node".into());
    }
    let links = array(required(g, "links", path)?, &format!("{path}.links"))?;
    if links.len() > 256 {
        return Err(bad(path, "too many shader links"));
    }
    let mut targets = HashSet::new();
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for (i, l) in links.iter().enumerate() {
        let p = format!("{path}.links[{i}]");
        let l = object(l, &p)?;
        keys(l, &p, &["from", "output", "to", "input"])?;
        let from = identifier(required(l, "from", &p)?, &format!("{p}.from"))?;
        let to = identifier(required(l, "to", &p)?, &format!("{p}.to"))?;
        let input = required(l, "input", &p)?;
        socket(input, &format!("{p}.input"))?;
        socket(required(l, "output", &p)?, &format!("{p}.output"))?;
        if !ids.contains(from) || !ids.contains(to) {
            return Err("Shader links must address existing nodes".into());
        }
        // The source schema uses JS template-string coercion here, so a
        // numeric socket 0 and a string socket "0" address the same input.
        let socket_name: JsString = match input {
            Value::String(s) => s.clone(),
            Value::Number(n) => pm_model::json::number_to_js_string(*n).into(),
            _ => unreachable!(),
        };
        let key = (to.to_owned(), socket_name);
        if !targets.insert(key) {
            return Err("A shader input can have only one incoming link".into());
        }
        edges
            .entry(from.to_owned())
            .or_default()
            .push(to.to_owned());
    }
    fn visit(
        id: &str,
        edges: &HashMap<String, Vec<String>>,
        active: &mut HashSet<String>,
        seen: &mut HashSet<String>,
    ) -> bool {
        if active.contains(id) {
            return false;
        }
        if seen.contains(id) {
            return true;
        }
        active.insert(id.into());
        if let Some(next) = edges.get(id) {
            for n in next {
                if !visit(n, edges, active, seen) {
                    return false;
                }
            }
        }
        active.remove(id);
        seen.insert(id.into());
        true
    }
    let mut active = HashSet::new();
    let mut seen = HashSet::new();
    for id in &ids {
        if !visit(id, &edges, &mut active, &mut seen) {
            return Err("Shader links cannot contain cycles".into());
        }
    }
    Ok(ids)
}
fn shader(v: &Value, path: &str) -> Check {
    let m = object(v, path)?;
    keys(
        m,
        path,
        &["id", "name", "preset", "source", "graph", "inputs", "p"],
    )?;
    identifier(required(m, "id", path)?, &format!("{path}.id"))?;
    string(required(m, "name", path)?, &format!("{path}.name"), 1, None)?;
    if let Some(v) = m.get("preset") {
        enumeration(
            v,
            &format!("{path}.preset"),
            &["surface", "noise", "checker", "metal", "glass", "emission"],
        )?;
    }
    if let Some(v) = m.get("source") {
        let p = format!("{path}.source");
        let s = object(v, &p)?;
        keys(s, &p, &["assetId", "material"])?;
        string(
            required(s, "assetId", &p)?,
            &format!("{p}.assetId"),
            1,
            Some(160),
        )?;
        string(
            required(s, "material", &p)?,
            &format!("{p}.material"),
            1,
            Some(160),
        )?;
    }
    let nodes = if let Some(v) = m.get("graph") {
        Some(graph(v, &format!("{path}.graph"))?)
    } else {
        None
    };
    if nodes.is_none() && m.get("source").is_none() {
        return Err("Material requires a graph or Blender source".into());
    }
    let empty = Map::new();
    let props = if let Some(v) = m.get("p") {
        object(v, &format!("{path}.p"))?
    } else {
        &empty
    };
    let inputs = if let Some(v) = m.get("inputs") {
        array(v, &format!("{path}.inputs"))?
    } else {
        &[]
    };
    if inputs.len() > 64 {
        return Err(bad(path, "too many material inputs"));
    }
    let mut ids = HashSet::new();
    for (i, input) in inputs.iter().enumerate() {
        let p = format!("{path}.inputs[{i}]");
        let f = object(input, &p)?;
        keys(
            f,
            &p,
            &[
                "id", "label", "kind", "node", "socket", "min", "max", "step",
            ],
        )?;
        let id = identifier(required(f, "id", &p)?, &format!("{p}.id"))?;
        if !ids.insert(id) {
            return Err("Material input IDs must be unique".into());
        }
        string(
            required(f, "label", &p)?,
            &format!("{p}.label"),
            1,
            Some(160),
        )?;
        enumeration(
            required(f, "kind", &p)?,
            &format!("{p}.kind"),
            &["number", "color", "toggle"],
        )?;
        let node = string(required(f, "node", &p)?, &format!("{p}.node"), 1, Some(160))?;
        if nodes.as_ref().is_some_and(|nodes| !nodes.contains(node)) {
            return Err("Exposed input must address a shader node".into());
        }
        socket(required(f, "socket", &p)?, &format!("{p}.socket"))?;
        let min = f
            .get("min")
            .map(|v| number(v, &format!("{p}.min"), -f64::MAX, f64::MAX, false))
            .transpose()?;
        let max = f
            .get("max")
            .map(|v| number(v, &format!("{p}.max"), -f64::MAX, f64::MAX, false))
            .transpose()?;
        if let Some(v) = f.get("step") {
            number(v, &format!("{p}.step"), f64::from_bits(1), f64::MAX, false)?;
        }
        let prop = props
            .get(id)
            .ok_or_else(|| format!("Missing material input {id}"))?;
        shader_channel(prop, &format!("{path}.p.{id}"))?;
        let ch = object(prop, &p)?;
        let kind = string(required(f, "kind", &p)?, &p, 1, None)?;
        let mut values = vec![required(ch, "v", &p)?];
        if let Some(Value::Array(frames)) = ch.get("kf") {
            for frame in frames {
                values.push(required(object(frame, &p)?, "v", &p)?);
            }
        }
        for value in values {
            let valid = match kind {
                "color" => color(value, &p).is_ok(),
                "toggle" => matches!(value, Value::Bool(_)),
                _ => {
                    matches!(value,Value::Number(n) if n.is_finite()&&min.is_none_or(|v|*n>=v)&&max.is_none_or(|v|*n<=v))
                }
            };
            if !valid {
                return Err(format!(
                    "Invalid material input {}",
                    string(required(f, "label", &p)?, &p, 1, None)?
                ));
            }
        }
    }
    for (key, _) in props.iter() {
        let key = key
            .as_str()
            .ok_or_else(|| bad(path, "invalid material input key"))?;
        identifier(&Value::String(key.into()), path)?;
        if !ids.contains(key) {
            return Err("Unknown exposed material input".into());
        }
    }
    Ok(())
}
fn material(v: &Value, path: &str) -> Check {
    let m = object(v, path)?;
    keys(
        m,
        path,
        &["shader", "placement", "p", "maps", "doubleSided"],
    )?;
    if let Some(v) = m.get("shader") {
        shader(v, &format!("{path}.shader"))?;
    }
    if let Some(v) = m.get("placement") {
        let p = format!("{path}.placement");
        let s = object(v, &p)?;
        keys(
            s,
            &p,
            &["scaleX", "scaleY", "offsetX", "offsetY", "rotation"],
        )?;
        for (k, v) in s.iter() {
            let key = k.as_str().unwrap_or("");
            let (lo, hi) = match key {
                "scaleX" | "scaleY" => (0.001, 1000.0),
                "rotation" => (-360_000.0, 360_000.0),
                _ => (-100_000.0, 100_000.0),
            };
            number(v, &format!("{p}.{key}"), lo, hi, false)?;
        }
    }
    if let Some(v) = m.get("p") {
        channels(
            object(v, &format!("{path}.p"))?,
            &format!("{path}.p"),
            MATERIAL,
            -100_000_000.0,
        )?;
    }
    if let Some(v) = m.get("maps") {
        let p = format!("{path}.maps");
        let maps = object(v, &p)?;
        keys(
            maps,
            &p,
            &[
                "color",
                "normal",
                "roughness",
                "metalness",
                "emissive",
                "ao",
            ],
        )?;
        for (k, v) in maps.iter() {
            string(v, &format!("{p}.{}", k.to_string_lossy()), 1, Some(160))?;
        }
    }
    if let Some(v) = m.get("doubleSided") {
        boolean(v, &format!("{path}.doubleSided"))?;
    }
    Ok(())
}

/// Validate a scene object at the same boundary as `parseScene`.
pub fn validate_scene(scene: &Map) -> Check {
    keys(
        scene,
        "scene",
        &["version", "objects", "lights", "camera", "environment"],
    )?;
    if let Some(v) = scene.get("version") {
        number(v, "scene.version", 1.0, 1.0, true)?;
    }
    let objects = if let Some(v) = scene.get("objects") {
        array(v, "scene.objects")?
    } else {
        &[]
    };
    let lights = if let Some(v) = scene.get("lights") {
        array(v, "scene.lights")?
    } else {
        &[]
    };
    let mut ids = HashSet::from([String::from("camera")]);
    let mut parents: HashMap<String, Option<String>> = HashMap::new();
    let mut vertices = 0usize;
    for (i, v) in objects.iter().enumerate() {
        let p = format!("scene.objects[{i}]");
        let o = object(v, &p)?;
        keys(
            o,
            &p,
            &[
                "id",
                "name",
                "parent",
                "source",
                "p",
                "material",
                "useSourceMaterials",
                "blender",
                "generation",
                "slots",
                "castShadow",
                "receiveShadow",
            ],
        )?;
        let id = identifier(required(o, "id", &p)?, &format!("{p}.id"))?.to_owned();
        if !ids.insert(id.clone()) {
            return Err(format!("Duplicate 3D id: {id}"));
        }
        if let Some(v) = o.get("name") {
            string(v, &format!("{p}.name"), 1, None)?;
        }
        let parent = if let Some(v) = o.get("parent") {
            if matches!(v, Value::Null) {
                None
            } else {
                Some(identifier(v, &format!("{p}.parent"))?.to_owned())
            }
        } else {
            None
        };
        parents.insert(id, parent);
        vertices =
            vertices.saturating_add(source(required(o, "source", &p)?, &format!("{p}.source"))?);
        if let Some(v) = o.get("p") {
            channels(
                object(v, &format!("{p}.p"))?,
                &format!("{p}.p"),
                TRANSFORM,
                0.0,
            )?;
        }
        if let Some(v) = o.get("material") {
            material(v, &format!("{p}.material"))?;
        }
        if let Some(v) = o.get("useSourceMaterials") {
            boolean(v, &format!("{p}.useSourceMaterials"))?;
        }
        if let Some(v) = o.get("blender") {
            let b = object(v, &format!("{p}.blender"))?;
            keys(b, &p, &["assetId", "object"])?;
            string(
                required(b, "assetId", &p)?,
                &format!("{p}.blender.assetId"),
                1,
                Some(160),
            )?;
            string(
                required(b, "object", &p)?,
                &format!("{p}.blender.object"),
                1,
                Some(160),
            )?;
        }
        if let Some(v) = o.get("generation") {
            let g = object(v, &format!("{p}.generation"))?;
            keys(g, &p, &["groupId", "partId"])?;
            identifier(
                required(g, "groupId", &p)?,
                &format!("{p}.generation.groupId"),
            )?;
            identifier(
                required(g, "partId", &p)?,
                &format!("{p}.generation.partId"),
            )?;
        }
        if let Some(v) = o.get("slots") {
            let slots = array(v, &format!("{p}.slots"))?;
            if slots.len() > 64 {
                return Err(bad(&p, "too many material slots"));
            }
            for (j, v) in slots.iter().enumerate() {
                let sp = format!("{p}.slots[{j}]");
                let s = object(v, &sp)?;
                keys(s, &sp, &["id", "name", "material"])?;
                identifier(required(s, "id", &sp)?, &format!("{sp}.id"))?;
                string(required(s, "name", &sp)?, &format!("{sp}.name"), 1, None)?;
                material(required(s, "material", &sp)?, &format!("{sp}.material"))?;
            }
        }
        for key in ["castShadow", "receiveShadow"] {
            if let Some(v) = o.get(key) {
                boolean(v, &format!("{p}.{key}"))?;
            }
        }
    }
    for id in parents.keys() {
        let mut chain = HashSet::from([id.as_str()]);
        let mut cursor = id.as_str();
        while let Some(Some(parent)) = parents.get(cursor) {
            if !chain.insert(parent) {
                return Err("3D object parenting contains a cycle".into());
            }
            if !parents.contains_key(parent) {
                return Err(format!("Missing 3D parent: {parent}"));
            }
            cursor = parent;
        }
    }
    for (i, v) in lights.iter().enumerate() {
        let p = format!("scene.lights[{i}]");
        let l = object(v, &p)?;
        keys(l, &p, &["id", "name", "type", "p", "castShadow"])?;
        let id = identifier(required(l, "id", &p)?, &format!("{p}.id"))?;
        if !ids.insert(id.to_owned()) {
            return Err(format!("Duplicate 3D id: {id}"));
        }
        if let Some(v) = l.get("name") {
            string(v, &format!("{p}.name"), 1, None)?;
        }
        if let Some(v) = l.get("type") {
            enumeration(v, &format!("{p}.type"), &["sun", "point", "spot", "area"])?;
        }
        if let Some(v) = l.get("p") {
            channels(object(v, &format!("{p}.p"))?, &format!("{p}.p"), LIGHT, 0.0)?;
        }
        if let Some(v) = l.get("castShadow") {
            boolean(v, &format!("{p}.castShadow"))?;
        }
    }
    if vertices > 500_000 {
        return Err("3D scene exceeds the geometry budget".into());
    }
    if let Some(v) = scene.get("camera") {
        let c = object(v, "scene.camera")?;
        keys(c, "scene.camera", &["projection", "p"])?;
        if let Some(v) = c.get("projection") {
            enumeration(
                v,
                "scene.camera.projection",
                &["perspective", "orthographic"],
            )?;
        }
        if let Some(v) = c.get("p") {
            let p = object(v, "scene.camera.p")?;
            channels(p, "scene.camera.p", CAMERA, 0.0)?;
            let read = |key: &str, fallback: f64| -> f64 {
                p.get(key)
                    .and_then(|v| match v {
                        Value::Number(n) => Some(*n),
                        Value::Object(o) => o.get("v").and_then(|v| match v {
                            Value::Number(n) => Some(*n),
                            _ => None,
                        }),
                        _ => None,
                    })
                    .unwrap_or(fallback)
            };
            if read("far", 1000.0) <= read("near", 0.01) {
                return Err("Camera far plane must be beyond its near plane".into());
            }
        }
    }
    if let Some(v) = scene.get("environment") {
        let e = object(v, "scene.environment")?;
        keys(e, "scene.environment", &["p", "background", "shadows"])?;
        if let Some(v) = e.get("p") {
            channels(
                object(v, "scene.environment.p")?,
                "scene.environment.p",
                ENVIRONMENT,
                0.0,
            )?;
        }
        if let Some(v) = e.get("background")
            && !matches!(v, Value::Null)
        {
            color(v, "scene.environment.background")?;
        }
        if let Some(v) = e.get("shadows") {
            boolean(v, "scene.environment.shadows")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(s: &str) -> Map {
        match pm_model::json::parse(s).unwrap() {
            Value::Object(v) => v,
            _ => panic!("object"),
        }
    }
    #[test]
    fn rejects_foreign_scene_schema_and_mesh_refinements() {
        for (scene, needle) in [
            (
                r#"{"objects":[{"id":"a","source":{"primitive":"nonsense"}}]}"#,
                "enum",
            ),
            (
                r#"{"objects":[{"id":"a","source":{"primitive":"box"},"stray":4}]}"#,
                "unknown key",
            ),
            (
                r#"{"objects":[{"id":"a","source":{"primitive":"box"},"p":{"x":{"v":100001,"kf":[]}}}]}"#,
                "range",
            ),
            (
                r#"{"objects":[{"id":"a","source":{"mesh":{"positions":[0,0,0,1,0,0,0,1,0],"indices":[0,1,3]}}}]}"#,
                "triangle",
            ),
            (
                r#"{"objects":[{"id":"a","source":{"lathe":[[-1,0],[1,1]]}}]}"#,
                "negative",
            ),
        ] {
            assert!(
                validate_scene(&parse(scene)).unwrap_err().contains(needle),
                "{scene}"
            );
        }
    }
    #[test]
    fn keeps_large_app_reachable_scenes() {
        let mut scene = parse(r#"{"objects":[],"lights":[]}"#);
        scene.insert(
            "objects",
            Value::Array(
                (0..513)
                    .map(|i| {
                        Value::Object(parse(&format!(
                            r#"{{"id":"o{i}","name":"{}","source":{{"primitive":"box"}}}}"#,
                            "x".repeat(161)
                        )))
                    })
                    .collect(),
            ),
        );
        scene.insert(
            "lights",
            Value::Array(
                (0..17)
                    .map(|i| Value::Object(parse(&format!(r#"{{"id":"l{i}"}}"#))))
                    .collect(),
            ),
        );
        assert!(validate_scene(&scene).is_ok());
    }

    #[test]
    fn matches_parse_scene_oracle_vectors() {
        let Value::Array(vectors) =
            pm_model::json::parse(include_str!("../tests/fixtures/scene-validation.json")).unwrap()
        else {
            panic!("oracle vector array")
        };
        assert!(vectors.len() >= 61);
        for vector in vectors {
            let Value::Object(v) = vector else {
                panic!("oracle vector object")
            };
            let name = match v.get("name") {
                Some(Value::String(s)) => s.to_string_lossy(),
                _ => panic!("name"),
            };
            let Value::Object(scene) = v.get("scene").expect("scene") else {
                panic!("scene object")
            };
            let Value::Bool(accepted) = v.get("accepted").expect("accepted") else {
                panic!("boolean")
            };
            assert_eq!(validate_scene(scene).is_ok(), *accepted, "{name}");
        }
    }

    #[test]
    fn approved_json_budget_does_not_hide_foreign_corruption() {
        let mut scene = parse(r#"{"objects":[{"id":"a","source":{"primitive":"box"}}]}"#);
        if let Some(Value::Array(objects)) = scene.get_mut("objects")
            && let Value::Object(object) = &mut objects[0]
        {
            object.insert("name", Value::String("x".repeat(8_000_001).into()));
        }
        assert!(
            validate_scene(&scene).is_ok(),
            "the JSON size cap is waived"
        );
        if let Some(Value::Array(objects)) = scene.get_mut("objects")
            && let Value::Object(object) = &mut objects[0]
        {
            object.insert("foreign", Value::Bool(true));
        }
        assert!(validate_scene(&scene).unwrap_err().contains("unknown key"));
    }
}
