//! Generated test documents in the shape the app stores (`loaded.json`):
//! a root composition, layers with every transform channel, assets.

#![allow(dead_code)]

use serde_json::{Value, json};

pub fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

/// Linear keyframes `(t, v)`.
pub fn keys(v: Value, keys: &[(f64, Value)]) -> Value {
    let kf: Vec<Value> = keys
        .iter()
        .enumerate()
        .map(|(i, (t, v))| {
            json!({ "t": t, "v": v, "i": format!("k{i}"), "eo": [0.333, 0.0], "ei": [0.667, 1.0], "hold": false })
        })
        .collect();
    json!({ "v": v, "kf": kf, "expr": null })
}

/// The transform channels of a layer (`mkLayer` defaults).
pub fn transform(x: f64, y: f64) -> Value {
    json!({
        "perspective": ch(json!(50)), "position.z": ch(json!(0)), "anchor.z": ch(json!(0)),
        "scale.z": ch(json!(100)), "rotation.x": ch(json!(0)), "rotation.y": ch(json!(0)),
        "orientation.x": ch(json!(0)), "orientation.y": ch(json!(0)), "orientation.z": ch(json!(0)),
        "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(x)),
        "position.y": ch(json!(y)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
        "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
    })
}

/// A layer of `ty` with content `d`, at `(x, y)`.
pub fn layer(id: &str, ty: &str, d: Value, x: f64, y: f64, dur: f64) -> Value {
    json!({
        "id": id, "type": ty, "name": id, "from": 0, "dur": dur, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": transform(x, y), "fx": [], "transitionIn": null, "transitionOut": null,
        "masks": [], "d": d, "locked_intent": {}
    })
}

/// Sets a channel of a layer.
pub fn set(layer: &mut Value, path: &str, v: Value) {
    layer["p"][path] = if v.is_object() { v } else { ch(v) };
}

/// A project whose open composition is `w` × `h`.
pub fn project(w: u32, h: u32, dur: f64, background: Value, layers: Vec<Value>) -> Value {
    json!({
        "id": "Ptest", "name": "test", "w": w, "h": h, "fps": 30, "dur": dur, "bg": "#000000",
        "backgroundFill": background, "compId": "Ptest-root", "compName": "test",
        "markers": [], "work": [0, dur], "params": {}, "revision": 0, "edits": [],
        "created": 0, "shutter": 0.5, "layers": layers, "assets": {}, "comps": {}
    })
}

pub fn solid_fill(color: &str) -> Value {
    json!({ "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": color, "position": 0 }] })
}

pub fn no_fill() -> Value {
    json!({ "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] })
}

/// Adds an asset record.
pub fn asset(doc: &mut Value, id: &str, kind: &str, format: &str, w: u32, h: u32, dur: f64) {
    doc["assets"][id] = json!({
        "id": id, "name": id, "kind": kind, "format": format, "w": w, "h": h, "dur": dur,
        "fps": 30, "persisted": true
    });
}

/// Adds a nested composition.
pub fn composition(doc: &mut Value, id: &str, w: u32, h: u32, dur: f64, layers: Vec<Value>) {
    doc["comps"][id] = json!({
        "id": id, "name": id, "w": w, "h": h, "fps": 30, "dur": dur, "bg": "#000000",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "layers": layers, "markers": []
    });
}

/// A 3D object layer with a primitive (`parseLayer3DData` shape).
pub fn object3d(id: &str, primitive: &str, parameters: Value, color: &str, w: u32, h: u32, dur: f64) -> Value {
    let mut l = layer(
        id,
        "extension",
        json!({
            "definition": "powermove.3d.object", "version": 1, "w": w, "h": h, "params": {},
            "data": { "object": {
                "source": { "primitive": primitive, "parameters": parameters },
                "material": { "p": {
                    "color": ch(json!(color)), "roughness": ch(json!(0.4)), "metalness": ch(json!(0.1)),
                    "emissive": ch(json!("#000000")), "emissiveIntensity": ch(json!(1)), "opacity": ch(json!(1))
                }, "maps": {}, "doubleSided": true },
                "useSourceMaterials": false, "slots": [], "castShadow": true, "receiveShadow": true
            } }
        }),
        0.0,
        0.0,
        dur,
    );
    l["threeD"] = json!(true);
    for k in ["scale.x", "scale.y", "scale.z"] {
        set(&mut l, k, json!(100));
    }
    l
}

/// A sun light layer at `position`.
pub fn sun(id: &str, position: [f64; 3], cast: bool, dur: f64) -> Value {
    let mut l = layer(
        id,
        "extension",
        json!({
            "definition": "powermove.3d.light", "version": 1, "w": 0, "h": 0, "params": {},
            "data": { "light": { "type": "sun", "castShadow": cast, "p": {
                "targetX": ch(json!(0)), "targetY": ch(json!(0)), "targetZ": ch(json!(0)),
                "color": ch(json!("#FFFFFF")), "intensity": ch(json!(3)), "distance": ch(json!(0)),
                "decay": ch(json!(2)), "angle": ch(json!(45)), "penumbra": ch(json!(0.4)),
                "width": ch(json!(1)), "height": ch(json!(1))
            } } }
        }),
        position[0],
        position[1],
        dur,
    );
    l["threeD"] = json!(true);
    set(&mut l, "position.z", json!(position[2]));
    l
}

/// A camera layer at `position` aimed at the origin; `shadows` for the environment.
pub fn camera(id: &str, position: [f64; 3], shadows: bool, dur: f64) -> Value {
    let mut l = layer(
        id,
        "extension",
        json!({
            "definition": "powermove.3d.camera", "version": 1, "w": 0, "h": 0, "params": {},
            "data": {
                "camera": { "projection": "perspective", "p": {
                    "targetX": ch(json!(0)), "targetY": ch(json!(0)), "targetZ": ch(json!(0)),
                    "fov": ch(json!(45)), "zoom": ch(json!(1)), "near": ch(json!(0.01)), "far": ch(json!(1000))
                } },
                "environment": { "p": {
                    "ambient": ch(json!(0.5)), "ambientColor": ch(json!("#FFFFFF")), "exposure": ch(json!(1))
                }, "background": null, "shadows": shadows }
            }
        }),
        position[0],
        position[1],
        dur,
    );
    l["threeD"] = json!(true);
    set(&mut l, "position.z", json!(position[2]));
    l
}

/// The document as the renderer loads it.
pub fn load(doc: &Value) -> pm_model::Document {
    let value = pm_model::json::parse(&doc.to_string()).unwrap();
    pm_model::Document::new(Some(value), 1)
}
