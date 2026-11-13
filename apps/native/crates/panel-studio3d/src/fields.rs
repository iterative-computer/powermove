//! Opus 3d-studio.md §8. Stored keys are never renamed to match visible copy.
use serde::Serialize;
use serde_json::Value;
/// One keyframeable 3D field: stored channel, visible label and range.
#[derive(Clone, Debug, Serialize)]
pub struct Field {
    pub id: String,
    pub group: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub value: Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub unit: &'static str,
    pub animated: bool,
    pub keyed: bool,
    pub channel: Option<String>,
}
pub(crate) fn catalog(role: &str, data: &Value) -> Vec<Field> {
    let mut out = vec![];
    let mut add = |group, prefix: &str, key: &str, label, kind, min, max, unit| {
        out.push(Field {
            id: format!("{prefix}.{key}"),
            group,
            label,
            kind,
            value: Value::Null,
            min,
            max,
            unit,
            animated: false,
            keyed: false,
            channel: Some(format!("{prefix}.{key}")),
        });
    };
    // Environment layers are model-role layers whose object carries one of the
    // Studio environment nodes. They never show material rows.
    if role == "object"
        && let Some(kind) = environment(data)
    {
        for (key, label, group, kind_, min, max, unit) in environment_fields(kind) {
            add(group, kind, key, label, kind_, min, max, unit);
        }
        return out;
    }
    match role {
        "object" if data["object"]["useSourceMaterials"] != true => {
            for (key, label, group, min, max, unit) in [
                ("color", "Color", "Material", None, None, ""),
                (
                    "roughness",
                    "Roughness",
                    "Material",
                    Some(0.02),
                    Some(1.),
                    "",
                ),
                ("metalness", "Metalness", "Material", Some(0.), Some(1.), ""),
                ("opacity", "Opacity", "Material", Some(0.), Some(1.), ""),
                (
                    "clearcoat",
                    "Clear Coat",
                    "Coat and Glass",
                    Some(0.),
                    Some(1.),
                    "",
                ),
                (
                    "clearcoatRoughness",
                    "Coat Roughness",
                    "Coat and Glass",
                    Some(0.002),
                    Some(1.),
                    "",
                ),
                (
                    "transmission",
                    "Transmission",
                    "Coat and Glass",
                    Some(0.),
                    Some(1.),
                    "",
                ),
                (
                    "ior",
                    "Index of Refraction",
                    "Coat and Glass",
                    Some(1.),
                    Some(2.6),
                    "",
                ),
                (
                    "specular",
                    "Specular",
                    "Coat and Glass",
                    Some(0.),
                    Some(1.),
                    "",
                ),
                (
                    "dispersion",
                    "Dispersion",
                    "Coat and Glass",
                    Some(0.),
                    Some(1.),
                    "",
                ),
                ("sheen", "Sheen", "Surface", Some(0.), Some(1.), ""),
                (
                    "iridescence",
                    "Thin Film",
                    "Surface",
                    Some(0.),
                    Some(1200.),
                    "nm",
                ),
                (
                    "droplets",
                    "Water Droplets",
                    "Surface",
                    Some(0.),
                    Some(1.),
                    "",
                ),
                ("anisotropy", "Brushed", "Surface", Some(-1.), Some(1.), ""),
                ("subsurface", "Subsurface", "Surface", Some(0.), None, ""),
                ("emissive", "Emission", "Emission", None, None, ""),
                (
                    "emissiveIntensity",
                    "Emission Strength",
                    "Emission",
                    Some(0.),
                    None,
                    "",
                ),
            ] {
                add(
                    group,
                    "m",
                    key,
                    label,
                    if min.is_none() { "color" } else { "number" },
                    min,
                    max,
                    unit,
                );
            }
        }
        "light" => {
            add("Light", "light", "color", "Color", "color", None, None, "");
            add(
                "Light",
                "light",
                "intensity",
                "Brightness",
                "number",
                Some(0.),
                None,
                "",
            );
            match data["light"]["type"].as_str() {
                Some("area") => {
                    for (k, l) in [("width", "Width"), ("height", "Height")] {
                        add("Light", "light", k, l, "number", Some(0.001), None, "");
                    }
                }
                Some("point" | "spot") => {
                    add(
                        "Light",
                        "light",
                        "distance",
                        "Range",
                        "number",
                        Some(0.),
                        None,
                        "",
                    );
                    add(
                        "Light",
                        "light",
                        "decay",
                        "Falloff",
                        "number",
                        Some(0.),
                        None,
                        "",
                    );
                    if data["light"]["type"] == "spot" {
                        add(
                            "Light",
                            "light",
                            "angle",
                            "Spread",
                            "number",
                            Some(1.),
                            Some(89.),
                            "°",
                        );
                        add(
                            "Light",
                            "light",
                            "penumbra",
                            "Softness",
                            "number",
                            Some(0.),
                            Some(1.),
                            "",
                        );
                    }
                }
                _ => {}
            }
        }
        "camera" => {
            if data["camera"]["projection"] == "orthographic" {
                add(
                    "Camera",
                    "camera",
                    "zoom",
                    "Zoom",
                    "number",
                    Some(0.001),
                    None,
                    "",
                );
            } else {
                add(
                    "Camera",
                    "camera",
                    "fov",
                    "Field of View",
                    "number",
                    Some(1.),
                    Some(150.),
                    "°",
                );
            }
            for (k, l, min, max) in [
                ("near", "Clip Start", 0.0001, None),
                ("far", "Clip End", 0.01, None),
                ("fStop", "f-Stop", 0., Some(64.)),
                ("focusDistance", "Focus", 0., None),
            ] {
                add("Camera", "camera", k, l, "number", Some(min), max, "");
            }
            add(
                "Environment",
                "environment",
                "ambient",
                "Ambient",
                "number",
                Some(0.),
                None,
                "",
            );
            add(
                "Environment",
                "environment",
                "ambientColor",
                "Ambient Color",
                "color",
                None,
                None,
                "",
            );
            add(
                "Environment",
                "environment",
                "exposure",
                "Exposure",
                "number",
                None,
                None,
                "",
            );
        }
        _ => {}
    }
    out
}
/// The Studio environment node a model-role layer carries, if any.
pub fn environment(data: &Value) -> Option<&'static str> {
    ["world", "fog", "terrain", "scatter"]
        .into_iter()
        .find(|k| data["object"].get(*k).is_some_and(Value::is_object))
}
type Row = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<f64>,
    Option<f64>,
    &'static str,
);
/// Keyframeable environment channels in the order a non-expert reads them:
/// the look first, then the physical detail in closed groups.
fn environment_fields(kind: &str) -> Vec<Row> {
    let num = |key, label, group, min, max, unit| (key, label, group, "number", min, max, unit);
    let col = |key, label, group| (key, label, group, "color", None, None, "");
    match kind {
        "world" => vec![
            num("elevation", "Sun Height", "Sky", Some(-90.), Some(90.), "°"),
            num("azimuth", "Sun Direction", "Sky", None, None, "°"),
            num("sunIntensity", "Sun Strength", "Sky", Some(0.), None, ""),
            num("sunRadius", "Sun Size", "Sky", Some(0.05), Some(10.), "°"),
            num("haze", "Haze", "Sky", Some(1.), Some(10.), ""),
            num("overcast", "Clouds", "Sky", Some(0.), Some(1.), ""),
            col("groundColor", "Ground", "Sky"),
            num("intensity", "Brightness", "Sky", Some(0.), None, ""),
            num("rotation", "Rotation", "Sky", None, None, "°"),
            num(
                "fogDensity",
                "Density",
                "Atmosphere",
                Some(0.),
                Some(100.),
                "",
            ),
            col("fogColor", "Color", "Atmosphere"),
            num(
                "fogAnisotropy",
                "Glow Around Sun",
                "Atmosphere",
                Some(-0.95),
                Some(0.95),
                "",
            ),
            num("fogBaseHeight", "Base Height", "Atmosphere", None, None, ""),
            num(
                "fogHeightFalloff",
                "Height Falloff",
                "Atmosphere",
                Some(0.),
                Some(100.),
                "",
            ),
            num(
                "fogDistance",
                "Visibility",
                "Atmosphere",
                Some(0.001),
                None,
                "",
            ),
            num(
                "metersPerUnit",
                "Meters per Unit",
                "Atmosphere",
                Some(0.000001),
                Some(1000.),
                "m",
            ),
        ],
        "fog" => vec![
            num("density", "Density", "Fog", Some(0.), Some(100.), ""),
            col("color", "Color", "Fog"),
            num(
                "anisotropy",
                "Glow Around Lights",
                "Fog",
                Some(-0.95),
                Some(0.95),
                "",
            ),
            num("width", "Width", "Fog", Some(0.0001), None, ""),
            num("height", "Height", "Fog", Some(0.0001), None, ""),
            num("depth", "Depth", "Fog", Some(0.0001), None, ""),
        ],
        "terrain" => vec![
            num("amplitude", "Height", "Terrain", Some(0.), None, ""),
            num(
                "frequency",
                "Feature Size",
                "Terrain",
                Some(0.00001),
                None,
                "",
            ),
            num(
                "octaves",
                "Detail Levels",
                "Terrain",
                Some(1.),
                Some(12.),
                "",
            ),
            num("erosion", "Erosion", "Terrain", Some(0.), Some(64.), ""),
            num("detail", "Fine Detail", "Terrain", Some(0.), Some(0.25), ""),
            num("seed", "Seed", "Terrain", Some(0.), None, ""),
            num("width", "Width", "Size", Some(0.001), Some(10000.), ""),
            num("depth", "Depth", "Size", Some(0.001), Some(10000.), ""),
            num("resolution", "Grid", "Size", Some(2.), Some(256.), ""),
            num("blend", "Region Blend", "Ground", Some(0.), Some(1.), ""),
            num("waterLevel", "Water Level", "Ground", None, None, ""),
            num("snowLevel", "Snow Line", "Ground", None, None, ""),
        ],
        "scatter" => vec![
            num("count", "Count", "Scatter", Some(0.), Some(100000.), ""),
            num("density", "Density", "Scatter", Some(0.), Some(1.), ""),
            num("seed", "Seed", "Scatter", Some(0.), None, ""),
            num(
                "scaleMin",
                "Smallest",
                "Variation",
                Some(0.0001),
                Some(1000.),
                "",
            ),
            num(
                "scaleMax",
                "Largest",
                "Variation",
                Some(0.0001),
                Some(1000.),
                "",
            ),
            num("rotationMin", "Turn From", "Variation", None, None, "°"),
            num("rotationMax", "Turn To", "Variation", None, None, "°"),
            num(
                "minSlope",
                "Flattest",
                "Placement",
                Some(0.),
                Some(90.),
                "°",
            ),
            num(
                "maxSlope",
                "Steepest",
                "Placement",
                Some(0.),
                Some(90.),
                "°",
            ),
            num("minHeight", "Lowest", "Placement", None, None, ""),
            num("maxHeight", "Highest", "Placement", None, None, ""),
        ],
        _ => vec![],
    }
}
pub(crate) fn channel<'a>(data: &'a Value, path: &str) -> &'a Value {
    let (prefix, key) = path.split_once('.').expect("catalog path");
    match prefix {
        "m" => &data["object"]["material"]["p"][key],
        "world" | "fog" | "terrain" | "scatter" => &data["object"][prefix]["p"][key],
        _ => &data[prefix]["p"][key],
    }
}
