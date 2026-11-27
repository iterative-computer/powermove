#![allow(dead_code)]
use pm_kernel::{Command, DocumentStore, Origin};
use pm_model::Value;
use serde_json::{Value as Json, json};
pub fn clip(id: &str, from: f64, dur: f64, track: i32) -> Json {
    json!({"id":id,"name":id,"type":"video","from":from,"dur":dur,"track":track,"on":true,
        "d":{"asset":"src","trim":{"v":from+2.0,"kf":[],"expr":null},"speed":1.0,"opaque":{"retain":true}},
        "p":{"position.x":{"v":0,"kf":[{"t":1.0,"v":3,"i":format!("{id}-k1")},{"t":3.0,"v":9,"i":format!("{id}-k2")}],"expr":null}},
        "fx":[],"masks":[],"extensionData":{"authored":"keep"}})
}
pub fn audio(id: &str, from: f64, dur: f64, track: i32) -> Json {
    let mut c = clip(id, from, dur, track);
    c["type"] = json!("audio");
    c
}
/// The source asset is shaped as a v1 one is after opening: no `timing`, and
/// the v1 video reading marker opening adds (so a save and reopen keeps it).
pub fn store(layers: Vec<Json>) -> DocumentStore {
    let p = json!({"id":"P","name":"Pro fixture","compId":"C1","fps":30,"dur":40,"w":100,"h":100,"bg":"#000000",
        "assets":{"src":{"id":"src","kind":"video","name":"Synthetic","dur":100,"hasAudio":true,"videoColor":"electron"}},"layers":layers});
    DocumentStore::new_untitled(pm_model::json::parse(&p.to_string()).unwrap())
}
pub fn abc() -> DocumentStore {
    store(vec![
        clip("a", 0.0, 4.0, 0),
        clip("b", 4.0, 4.0, 0),
        clip("c", 8.0, 4.0, 0),
    ])
}
pub fn root(s: &DocumentStore) -> Value {
    s.root().unwrap().to_value()
}
pub fn layer(s: &DocumentStore, id: &str) -> Value {
    let root = s.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, id).unwrap_or_else(|| panic!("missing {id}"));
    pm_kernel::model::layer_at(root, &loc).unwrap().to_value()
}
pub fn n(v: &Value, path: &[&str]) -> f64 {
    let mut v = v;
    for p in path {
        v = if let Ok(i) = p.parse::<usize>() {
            &v.as_array().unwrap()[i]
        } else {
            v.get(p).unwrap_or_else(|| panic!("missing {p}"))
        };
    }
    v.as_f64().unwrap()
}
pub fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}
pub fn run(s: &mut DocumentStore, c: impl Into<Command>) {
    s.execute(c.into(), Origin::Interface).unwrap();
}
pub fn reject(s: &mut DocumentStore, c: impl Into<Command>) {
    let before = root(s);
    let rev = s.revision();
    assert!(s.execute(c.into(), Origin::Interface).is_err());
    assert_eq!(root(s), before);
    assert_eq!(s.revision(), rev);
}
pub fn roundtrip(s: &mut DocumentStore, c: impl Into<Command>) {
    let before = root(s);
    run(s, c);
    let after = root(s);
    assert_ne!(before, after);
    s.undo().unwrap();
    assert_eq!(root(s), before);
    s.redo().unwrap();
    assert_eq!(root(s), after);
}
