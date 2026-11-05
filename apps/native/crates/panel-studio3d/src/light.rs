//! Light-tool values and absolute adjustments. Picking stays an explicit host
//! capability; no guessed hit, fake reflection or hidden Fast fallback.
use crate::{Result, invalid};
use pm_kernel::{Published, QueryMirror};
use pm_scene3d::{
    from_model,
    light_tool::{self, Adjust, Kind, Snapshot},
    navigation::length,
    to_model,
};
use serde_json::{Value, json};
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Settings {
    pub aim: &'static str,
    pub add: bool,
    pub kind: &'static str,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            aim: "reflect",
            add: false,
            kind: "area",
        }
    }
}
fn current(p: &Published, time: f64) -> Result<Option<(Value, Snapshot, Kind)>> {
    if p.selection.layers().len() != 1 {
        return Ok(None);
    }
    let root = p.snapshot.root().ok_or_else(|| invalid("Project closed"))?;
    let comp = pm_kernel::model::comp_node(root, p.snapshot.open_comp().as_deref())
        .ok_or_else(|| invalid("Composition closed"))?;
    let comp = from_model(&comp.to_value())?;
    let Some((index, l)) = comp["layers"]
        .as_array()
        .ok_or_else(|| invalid("No layers"))?
        .iter()
        .enumerate()
        .find(|(_, l)| l["id"] == p.selection.layers()[0])
    else {
        return Ok(None);
    };
    if pm_scene3d::schema::role(l) != Some(pm_scene3d::schema::Role::Light) {
        return Ok(None);
    }
    let data = &l["d"]["data"]["light"];
    let kind = match data["type"].as_str() {
        Some("area") => Kind::Area,
        Some("spot") => Kind::Spot,
        Some("point") => Kind::Point,
        Some("sun") => Kind::Sun,
        _ => return Err(invalid("Unknown light type")),
    };
    let mut mirror = QueryMirror::new(p);
    let mut value = |path: &str, c: &Value| -> Result<f64> {
        from_model(&mirror.queries().query(&to_model(&json!({"op":"evP","comp":p.snapshot.open_comp(),"open":p.snapshot.open_comp(),"layer":index,"t":time,"key":path,"prop":c}))).map_err(|e|invalid(e.to_string()))?)?
            .as_f64().ok_or_else(||invalid("Non-numeric light channel"))
    };
    let mut position = [0.; 3];
    let mut target = [0.; 3];
    for (a, key) in ["x", "y", "z"].iter().enumerate() {
        let path = format!("position.{key}");
        position[a] = value(&path, &l["p"][&path])?;
    }
    for (a, key) in ["targetX", "targetY", "targetZ"].iter().enumerate() {
        target[a] = value(&format!("light.{key}"), &data["p"][key])?;
    }
    let mut n = |key: &str| value(&format!("light.{key}"), &data["p"][key]);
    let state = Snapshot {
        position,
        target,
        intensity: n("intensity")?,
        width: n("width")?,
        height: n("height")?,
        angle: n("angle")?,
    };
    Ok(Some((l.clone(), state, kind)))
}
pub(crate) fn projection(p: &Published, time: f64, settings: &Settings) -> Result<Value> {
    let Some((l, s, kind)) = current(p, time)? else {
        return Ok(
            json!({"settings":settings,"hint":"Select a light to aim it, or Add one.","rows":[]}),
        );
    };
    if settings.add {
        return Ok(
            json!({"settings":settings,"hint":format!("Click a model to add a {} there.",pm_scene3d::presentation::light_name(match settings.kind {"spot"=>Kind::Spot,"point"=>Kind::Point,"sun"=>Kind::Sun,_=>Kind::Area})),"rows":[]}),
        );
    }
    let mut rows = vec![
        json!({"id":"brightness","label":"Brightness","value":s.intensity,"min":0.05,"max":2000,"log":true,"tooltip":"How much light it gives."}),
    ];
    if kind == Kind::Area {
        rows.push(json!({"id":"size","label":"Size","value":s.width.max(s.height),"min":0.02,"max":40,"log":true,"tooltip":"Bigger softboxes give softer light and broader highlights."}));
    }
    if kind != Kind::Sun {
        rows.push(json!({"id":"distance","label":"Distance","value":length(pm_scene3d::navigation::sub(s.position,s.target)),"min":0.1,"max":200,"log":true,"tooltip":"How far it sits from what it lights. Brightness on the product stays the same."}));
    }
    if kind == Kind::Spot {
        rows.push(json!({"id":"spread","label":"Spread","value":s.angle,"min":1,"max":89,"log":false,"tooltip":"How wide the spotlight’s beam is."}));
    }
    Ok(
        json!({"settings":settings,"name":l["name"],"kind":pm_scene3d::presentation::light_name(kind),"locked":l["lock"]==true,"rows":rows}),
    )
}
pub(crate) fn adjustment(p: &Published, time: f64, id: &str, arg: Value) -> Result<Value> {
    let (l, s, kind) = current(p, time)?.ok_or_else(|| invalid("Choose a light"))?;
    if l["lock"] == true {
        return Err(invalid("Layer is locked"));
    }
    let (adjust, min, max) = match id {
        "brightness" => (Adjust::Power, 0.05, 2000.),
        "size" if kind == Kind::Area => (Adjust::Size, 0.02, 40.),
        "distance" if kind != Kind::Sun => (Adjust::Distance, 0.1, 200.),
        "spread" if kind == Kind::Spot => (Adjust::Spread, 1., 89.),
        _ => return Err(invalid("Unavailable light adjustment")),
    };
    let n = arg
        .as_f64()
        .filter(|n| n.is_finite() && *n >= min && *n <= max)
        .ok_or_else(|| invalid("Light adjustment outside range"))?;
    let after = light_tool::adjust(s, kind, adjust, n)?;
    let patch = match adjust {
        Adjust::Power => json!({"intensity":after.intensity}),
        Adjust::Size => json!({"width":after.width,"height":after.height}),
        Adjust::Spread => json!({"angle":after.angle}),
        Adjust::Distance => {
            json!({"x":after.position[0],"y":after.position[1],"z":after.position[2],"intensity":after.intensity})
        }
    };
    Ok(json!({"operation":"update_light","target":l["id"],"patch":{"p":patch}}))
}
