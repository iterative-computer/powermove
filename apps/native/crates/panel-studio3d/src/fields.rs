//! Opus 3d-studio.md §8. Stored keys are never renamed to match visible copy.
use serde::Serialize;
use serde_json::Value;
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
pub(crate) fn channel<'a>(data: &'a Value, path: &str) -> &'a Value {
    let (prefix, key) = path.split_once('.').expect("catalog path");
    match prefix {
        "m" => &data["object"]["material"]["p"][key],
        _ => &data[prefix]["p"][key],
    }
}
