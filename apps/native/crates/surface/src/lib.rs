//! Atomic, inspectable v4 native UI state. No JS, DOM, renderer, or platform dependency.
pub mod binding;
pub mod interaction;
pub mod style;
pub mod tree;
pub use tree::*;
pub mod controls;
pub mod kit;

pub mod events;
pub mod host_api;
pub mod host_data;
pub mod schema;

/// Deterministic inspection transport. Cargo's global serde_json feature
/// unification must not change wire bytes when a document importer enables
/// preserve_order. Array order (including six-op commands) stays authored.
pub fn inspection_json(mut value: serde_json::Value) -> String {
    value.sort_all_objects();
    value.to_string()
}

#[cfg(test)]
mod inspection_tests {
    #[test]
    fn object_order_is_stable_without_reordering_arrays() {
        let value = serde_json::from_str(r#"{"z":[{"b":2,"a":1},3,1],"a":0}"#).unwrap();
        assert_eq!(
            super::inspection_json(value),
            r#"{"a":0,"z":[{"a":1,"b":2},3,1]}"#
        );
    }
}
