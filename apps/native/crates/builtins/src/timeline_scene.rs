//! Native-only paint data. Stable model ids join the v4 outline to the GPUI
//! track renderer; no authored paint commands or pixel geometry cross JS.
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct Key {
    pub id: String,
    pub channel: String,
    pub time: f64,
    pub selected: bool,
    pub interpolation: String,
}
#[derive(Clone, Debug)]
pub struct Row {
    pub layer: String,
    pub name: String,
    pub kind: String,
    pub from: f64,
    pub duration: f64,
    pub property: bool,
    pub selected: bool,
    pub keys: Vec<Key>,
}
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub duration: f64,
    pub fps: f64,
    pub time: f64,
    pub rows: BTreeMap<String, Row>,
}
