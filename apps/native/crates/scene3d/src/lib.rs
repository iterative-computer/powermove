//! Studio's document side. No UI, JS engine, platform handles or second writer.
//! Source contracts: 3D source b81b94c66; shipped reference c7f46a056.
pub mod geometry;
pub mod extrusion;
pub mod contract;
pub mod quality;
pub mod import;
pub mod light_tool;
pub mod migration;
pub mod navigation;
pub mod operations;
pub mod part_material;
pub mod presentation;
pub mod render;
pub mod schema;
pub mod service;

use serde_json::Value;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);
pub type Result<T> = std::result::Result<T, Error>;
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}
pub fn to_model(v: &Value) -> pm_model::Value {
    pm_model::json::parse(&v.to_string()).expect("JSON is representable in pm-model")
}
pub fn from_model(v: &pm_model::Value) -> Result<Value> {
    serde_json::from_str(&pm_model::json::to_canonical_string(v)).map_err(|e| Error(e.to_string()))
}
