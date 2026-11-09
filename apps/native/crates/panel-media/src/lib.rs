//! Project media browser: immutable kernel queries, id-addressed input and
//! guarded undoable edits. No window, JS runtime, decoder or filesystem IO.
//! Build `Catalog` and `PreparedEdit` on a worker; publish the result to the UI.
//! See `docs/native/panels/media.md` for the frozen behaviour inventory.

mod catalog;
mod edit;
mod input;
mod model;
mod preview;
pub mod readiness;

pub use catalog::*;
pub use edit::*;
pub use input::*;
pub use model::*;
pub use preview::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    MissingItem,
    InvalidTarget,
    FolderCycle,
    Stale,
    InvalidMedia,
    ConfirmationRequired,
    Kernel(String),
}

pub(crate) fn text<'a>(v: &'a pm_model::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(pm_model::Value::as_str)
}
pub(crate) fn number(v: &pm_model::Value, key: &str) -> Option<f64> {
    v.get(key)
        .and_then(pm_model::Value::as_f64)
        .filter(|v| v.is_finite())
}
pub(crate) fn object<'a>(v: &'a pm_model::Value, key: &str) -> Vec<(&'a str, &'a pm_model::Value)> {
    v.get(key)
        .and_then(pm_model::Value::as_object)
        .into_iter()
        .flat_map(|m| m.iter())
        .filter_map(|(k, v)| k.as_str().map(|k| (k, v)))
        .collect()
}
pub(crate) trait ValueMut {
    fn get_mut(&mut self, key: &str) -> Option<&mut pm_model::Value>;
}
impl ValueMut for pm_model::Value {
    fn get_mut(&mut self, key: &str) -> Option<&mut pm_model::Value> {
        self.as_object_mut()?.get_mut(key)
    }
}

pub mod v4;
