//! Pinned Studio document additions. These are ordinary layer data/channels;
//! the kernel remains the only writer and retains its existing inverse journal.
use crate::{Error, Result, fail, schema};
use serde_json::{Value, json};
pub const GROUPS: &[&str] = &["world", "fog", "terrain", "scatter", "materialOptions"];
pub fn source(value: &Value, nullable: bool) -> Result<()> {
    if nullable && value.is_null() {
        return Ok(());
    }
    let text = |v: &Value| v.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 160);
    if text(value) {
        return Ok(());
    }
    let Some(m) = value.as_object() else {
        return fail("Expected image, composition or local layer source");
    };
    if m.len() == 1
        && ["compositionId", "layerId"]
            .iter()
            .any(|key| m.get(*key).is_some_and(text))
    {
        return Ok(());
    }
    fail("Invalid Studio source reference")
}
fn choose(v: &Value, values: &[&str]) -> Result<()> {
    if v.as_str().is_some_and(|s| values.contains(&s)) {
        Ok(())
    } else {
        fail("Invalid Studio enum value")
    }
}
pub fn node(kind: &str, mut v: Value) -> Result<Value> {
    let allowed: &[&str] = match kind {
        "world" => &["mode", "preset", "source", "p"],
        "terrain" => &["heightSource", "heights", "regions", "p"],
        "scatter" => &["model", "surface", "p"],
        "fog" | "materialOptions" | "region" => &["p"],
        _ => return fail("Unknown Studio node kind"),
    };
    schema::strict(&v, allowed)?;
    v["p"] = schema::properties(kind, v.get("p"))?;
    match kind {
        "world" => {
            if v.get("mode").is_none() {
                v["mode"] = json!("sky");
            }
            if v.get("preset").is_none() {
                v["preset"] = json!("clear-noon");
            }
            if v.get("source").is_none() {
                v["source"] = Value::Null;
            }
            choose(&v["mode"], &["none", "sky", "hdri"])?;
            choose(
                &v["preset"],
                &[
                    "clear-noon",
                    "golden-hour",
                    "overcast",
                    "blue-hour",
                    "night",
                ],
            )?;
            source(&v["source"], true)?;
        }
        "terrain" => {
            if v.get("heightSource").is_none() {
                v["heightSource"] = Value::Null;
            }
            source(&v["heightSource"], true)?;
            if v.get("heights").is_none() {
                v["heights"] = json!([]);
            }
            let heights = v["heights"]
                .as_array()
                .ok_or_else(|| Error("Invalid terrain heights".into()))?;
            let resolution = v["p"]["resolution"]["v"].as_f64().unwrap().round() as usize;
            if heights.len() > 66049
                || (!heights.is_empty() && heights.len() != (resolution + 1).pow(2))
                || heights
                    .iter()
                    .any(|n| n.as_f64().is_none_or(|n| !(-1. ..=1.).contains(&n)))
            {
                return fail("Terrain heights must match the grid and stay in -1..1");
            }
            if v.get("regions").is_none() {
                v["regions"] = json!([]);
            }
            let regions = v["regions"]
                .as_array()
                .filter(|r| r.len() <= 8)
                .ok_or_else(|| Error("At most eight terrain regions".into()))?;
            v["regions"] = Value::Array(
                regions
                    .iter()
                    .cloned()
                    .map(|r| node("region", r))
                    .collect::<Result<_>>()?,
            );
        }
        "scatter" => {
            for key in ["model", "surface"] {
                if !v[key]
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 160)
                {
                    return fail("Scatter needs model and surface layer references");
                }
            }
        }
        _ => {}
    }
    if kind == "scatter" || kind == "region" {
        ranges(&v["p"])?;
    }
    Ok(v)
}
fn ranges(p: &Value) -> Result<()> {
    for (a, b) in [
        ("minSlope", "maxSlope"),
        ("minHeight", "maxHeight"),
        ("scaleMin", "scaleMax"),
        ("rotationMin", "rotationMax"),
    ] {
        if let (Some(a), Some(b)) = (p[a]["v"].as_f64(), p[b]["v"].as_f64()) {
            if a > b {
                return fail("Studio minimum must not exceed maximum");
            }
        }
    }
    Ok(())
}
fn sparse(v: &Value) -> Result<Value> {
    let mut out = json!({});
    for (k, v) in v
        .as_object()
        .ok_or_else(|| Error("Expected material properties".into()))?
    {
        out[k] = schema::validate_channel("intrinsic", k, v.clone())?;
    }
    Ok(out)
}
fn material_maps(v: &Value) -> Result<()> {
    schema::strict(
        v,
        &[
            "color",
            "normal",
            "roughness",
            "metalness",
            "emissive",
            "ao",
        ],
    )?;
    for (slot, v) in v.as_object().unwrap() {
        source(v, false)?;
        if v.is_object() && !matches!(slot.as_str(), "color" | "emissive") {
            return fail("Live sources support color and emissive maps");
        }
    }
    Ok(())
}
fn material_index(key: &str) -> bool {
    key.parse::<u32>()
        .is_ok_and(|i| i < 100000 && i.to_string() == key)
}
pub fn normalize_object(v: &mut Value) -> Result<()> {
    if ["world", "fog", "terrain", "scatter"]
        .iter()
        .filter(|k| v.get(**k).is_some())
        .count()
        > 1
    {
        return fail("Choose one environment layer kind");
    }
    for kind in GROUPS {
        if let Some(raw) = v.get(*kind).cloned() {
            v[*kind] = node(kind, raw)?;
        }
    }
    if let Some(p) = v.get("materialProperties").cloned() {
        v["materialProperties"] = sparse(&p)?;
    }
    for key in ["sourceMaterialProperties", "sourceMaterials"] {
        if let Some(entries) = v.get(key).cloned() {
            for (index, value) in entries
                .as_object()
                .ok_or_else(|| Error("Expected indexed material records".into()))?
            {
                if !material_index(index) {
                    return fail("Invalid source material index");
                }
                if key == "sourceMaterialProperties" {
                    v[key][index] = sparse(value)?;
                } else {
                    material_maps(value)?;
                }
            }
        }
    }
    if let Some(parts) = v.get("parts").cloned() {
        let parts = parts
            .as_object()
            .ok_or_else(|| Error("Expected model parts".into()))?;
        for (key, part) in parts {
            let components: Vec<_> = key.split('_').collect();
            if components.len() > 64
                || components.iter().any(|s| {
                    !s.parse::<u32>()
                        .is_ok_and(|n| n <= 9999 && n.to_string() == *s)
                })
            {
                return fail("Invalid model part path");
            }
            schema::strict(part, &["name", "p"])?;
            let name = part.get("name").cloned().unwrap_or(json!("Part"));
            if !name
                .as_str()
                .is_some_and(|s| !s.is_empty() && s.len() <= 160)
            {
                return fail("Invalid part name");
            }
            v["parts"][key] = json!({"name":name,"p":schema::properties("part",part.get("p"))?});
        }
    }
    Ok(())
}
/// Flatten all additional document channels with their stable timeline paths.
pub fn channels(object: &Value) -> Vec<(String, &Value)> {
    let mut out = vec![];
    for kind in GROUPS {
        if let Some(p) = object[*kind]["p"].as_object() {
            for (k, v) in p {
                out.push((format!("{kind}.{k}"), v));
            }
        }
    }
    if let Some(regions) = object["terrain"]["regions"].as_array() {
        for (i, r) in regions.iter().enumerate() {
            if let Some(p) = r["p"].as_object() {
                for (k, v) in p {
                    out.push((format!("terrain.region{i}.{k}"), v));
                }
            }
        }
    }
    if let Some(p) = object["materialProperties"].as_object() {
        for (k, v) in p {
            out.push((format!("mi.{k}"), v));
        }
    }
    if let Some(entries) = object["sourceMaterialProperties"].as_object() {
        for (i, p) in entries {
            if let Some(p) = p.as_object() {
                for (k, v) in p {
                    out.push((format!("mi.{i}.{k}"), v));
                }
            }
        }
    }
    if let Some(parts) = object["parts"].as_object() {
        for (i, part) in parts {
            if let Some(p) = part["p"].as_object() {
                for (k, v) in p {
                    out.push((format!("pt.{i}.{k}"), v));
                }
            }
        }
    }
    out
}

/// Typed actor plan for environment layers and composition quality.
pub fn plan(
    comp: &Value,
    args: &Value,
    selection: &[String],
    uid: &mut impl FnMut(&str) -> String,
) -> Result<crate::operations::Plan> {
    use crate::{
        operations::{Action, Plan},
        schema::Role,
    };
    let operation = args["operation"].as_str().unwrap_or("");
    if operation == "set_render_quality" {
        schema::strict(args, &["operation", "preset", "patch"])?;
        let current = crate::quality::get(comp)?;
        let mut update = args.get("patch").cloned().unwrap_or(json!({}));
        if !update.is_object() {
            return fail("Expected Studio quality patch");
        }
        if let Some(preset) = args.get("preset") {
            update["preset"] = preset.clone();
        }
        if update.as_object().unwrap().is_empty() {
            return fail("Choose a quality preset or patch");
        }
        let quality = crate::quality::resolve(&current, &update)?;
        return Ok(Plan {
            actions: vec![Action::RenderQuality { quality }],
            id: None,
        });
    }
    schema::strict(
        args,
        &[
            "operation",
            "id",
            "target",
            "name",
            "patch",
            "world",
            "fog",
            "terrain",
            "scatter",
        ],
    )?;
    let kind = match operation {
        "set_world" => "world",
        "add_fog" | "update_fog" => "fog",
        "add_terrain" | "update_terrain" => "terrain",
        "add_scatter" | "update_scatter" => "scatter",
        "set_material_options" => "materialOptions",
        _ => return fail("Unknown Studio environment operation"),
    };
    let layers = comp["layers"]
        .as_array()
        .ok_or_else(|| Error("No composition layers".into()))?;
    let selected = args["target"]
        .as_str()
        .or(args["id"].as_str())
        .or_else(|| selection.first().map(String::as_str));
    let existing = if kind == "world" {
        layers
            .iter()
            .find(|l| l["d"]["data"]["object"].get("world").is_some())
    } else {
        selected.and_then(|id| layers.iter().find(|l| l["id"] == id || l["name"] == id))
    };
    let adding = operation.starts_with("add_") || kind == "world" && existing.is_none();
    let mut patch = args
        .get(kind)
        .or_else(|| args.get("patch"))
        .cloned()
        .unwrap_or(json!({}));
    if kind == "materialOptions" && patch.get("p").is_none() {
        patch = json!({"p":patch});
    }
    // Match the pinned command's preset channel writes; parsing saved data
    // never substitutes preset defaults over authored/keyframed channels.
    if kind == "world" && patch.get("preset").is_some() {
        let values = match patch["preset"].as_str() {
            Some("clear-noon") => (45., 2., 0., 1.),
            Some("golden-hour") => (6., 3., 0., 1.),
            Some("overcast") => (45., 4., 1., 0.1),
            Some("blue-hour") => (-5., 2., 0., 0.),
            Some("night") => (-20., 2., 0., 0.),
            _ => return fail("Invalid Studio sky preset"),
        };
        let authored = patch.get("p").cloned().unwrap_or(json!({}));
        let mut p = json!({"elevation":values.0,"haze":values.1,"overcast":values.2,"sunIntensity":values.3});
        for (key, value) in authored
            .as_object()
            .ok_or_else(|| Error("Expected World channels".into()))?
        {
            p[key] = value.clone();
        }
        patch["p"] = p;
    }
    if adding {
        let id = args["id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| uid(kind));
        if layers.iter().any(|l| l["id"] == id) {
            return fail("Layer ID already exists");
        }
        let mut actions = vec![];
        if kind == "scatter" {
            if let Some(species) = patch["model"]
                .as_str()
                .and_then(|s| s.strip_prefix("nature:"))
                .map(str::to_owned)
            {
                let source_id = uid("nature");
                let mut action = crate::operations::add(
                    Role::Object,
                    json!({"source":{"nature":species,"seed":1}}),
                    source_id.clone(),
                    Some("Nature"),
                    None,
                )?;
                if let Action::Add { visible, .. } = &mut action {
                    *visible = false;
                }
                actions.push(action);
                patch["model"] = json!(source_id);
            }
        }
        let value = node(kind, patch)?;
        let title = args["name"].as_str().unwrap_or(match kind {
            "world" => "World",
            "fog" => "Fog Volume",
            "terrain" => "Terrain",
            _ => "Scatter",
        });
        actions.push(crate::operations::add(
            Role::Object,
            json!({"source":{"primitive":"plane"},kind:value}),
            id.clone(),
            Some(title),
            None,
        )?);
        return Ok(Plan {
            actions,
            id: Some(id),
        });
    }
    let layer = existing.ok_or_else(|| Error("Choose the Studio layer to update".into()))?;
    if schema::role(layer) != Some(Role::Object) {
        return fail("Choose a model or environment layer");
    }
    let id = layer["id"].as_str().unwrap().to_owned();
    let mut data = layer["d"]["data"].clone();
    let previous = data["object"].get(kind).cloned().unwrap_or(json!({}));
    let mut final_value = previous.clone();
    if !final_value.is_object() {
        return fail("Invalid Studio layer data");
    }
    for (k, value) in patch
        .as_object()
        .ok_or_else(|| Error("Expected Studio patch".into()))?
    {
        if k == "p" {
            if final_value.get("p").is_none() {
                final_value["p"] = json!({});
            }
            for (k, v) in value
                .as_object()
                .ok_or_else(|| Error("Expected Studio channels".into()))?
            {
                final_value["p"][k] = v.clone();
            }
        } else {
            final_value[k] = value.clone();
        }
    }
    final_value = node(kind, final_value)?;
    // Validate the final coupled state, then retain original base/key data while
    // ordinary property commands apply the requested channel writes/autokeys.
    let mut staged = final_value.clone();
    let mut actions = vec![];
    if let Some(changes) = patch["p"].as_object() {
        for (k, value) in changes {
            if let Some(old) = previous["p"].get(k) {
                staged["p"][k] = old.clone();
            }
            actions.push(Action::Channel {
                id: id.clone(),
                path: format!("{kind}.{k}"),
                value: if value.is_object() {
                    final_value["p"][k].clone()
                } else {
                    value.clone()
                },
            });
        }
    }
    data["object"][kind] = staged;
    if data != layer["d"]["data"] {
        actions.insert(
            0,
            Action::StagedData {
                id: id.clone(),
                data,
            },
        );
    }
    Ok(Plan {
        actions,
        id: Some(id),
    })
}

pub fn validate_composition(comp: &Value) -> Result<()> {
    let layers = comp["layers"]
        .as_array()
        .ok_or_else(|| Error("No composition layers".into()))?;
    let objects: Vec<_> = layers
        .iter()
        .filter_map(|l| l["d"]["data"].get("object").map(|o| (l, o)))
        .collect();
    if objects
        .iter()
        .filter(|(_, o)| o.get("world").is_some())
        .count()
        > 1
    {
        return fail("A composition has one World layer");
    }
    if objects
        .iter()
        .filter(|(_, o)| o.get("fog").is_some())
        .count()
        > 32
    {
        return fail("At most 32 fog volumes");
    }
    if objects
        .iter()
        .filter_map(|(_, o)| o["scatter"]["p"]["count"]["v"].as_f64())
        .sum::<f64>()
        > 1000000.
    {
        return fail("At most one million scattered placements per composition");
    }
    fn check_source(source: &Value, receiver: &str, comp: &Value, layers: &[Value]) -> Result<()> {
        if source["compositionId"]
            .as_str()
            .is_some_and(|id| comp["id"] == id)
        {
            return fail("Studio source creates a composition cycle");
        }
        let Some(id) = source["layerId"].as_str() else {
            return Ok(());
        };
        if id == receiver {
            return fail("Studio source cannot refer to itself");
        }
        // Keep missing/deleted sources serialized; their resource state is
        // reported separately. Existing roots/groups must be raster artwork.
        let mut pending = vec![id.to_owned()];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id.clone()) {
                return fail("Studio group source contains a cycle");
            }
            if let Some(layer) = layers.iter().find(|l| l["id"] == id) {
                if schema::role(layer).is_some() || layer["threeD"] == true {
                    return fail("Studio source cannot contain 3D descendants");
                }
                if layer["type"] == "group" {
                    pending.extend(
                        layers
                            .iter()
                            .filter(|l| l["group"] == id)
                            .filter_map(|l| l["id"].as_str().map(str::to_owned)),
                    );
                }
            }
        }
        Ok(())
    }
    for (layer, o) in objects {
        let id = layer["id"].as_str().unwrap_or("");
        check_source(&o["world"]["source"], id, comp, layers)?;
        check_source(&o["terrain"]["heightSource"], id, comp, layers)?;
        for maps in std::iter::once(&o["material"]["maps"]).chain(
            o["sourceMaterials"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values()),
        ) {
            if let Some(maps) = maps.as_object() {
                for source in maps.values() {
                    check_source(source, id, comp, layers)?;
                }
            }
        }
    }
    Ok(())
}
