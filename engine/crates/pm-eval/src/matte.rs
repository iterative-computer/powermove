//! Track mattes (`legacy/core/matte.ts`).

use pm_model::Node;

use crate::js_sem::{self, JsValue};
use crate::node;

/// The four track matte modes (`MATTE_MODES`).
pub const MATTE_MODES: [&str; 4] = ["alpha", "alpha-inverted", "luma", "luma-inverted"];

/// `validMatteSource(layers, layer, id)`: whether `id` names a chain of
/// matte sources (each the first layer with that id) that never revisits a
/// layer and never reaches a missing layer or an audio, null, adjustment, or
/// group layer. A falsy `id` (no matte) is valid.
pub fn valid_matte_source(layers: &[Node], layer: &Node, id: &JsValue) -> bool {
    let mut seen: Vec<JsValue> = vec![node::get(layer, b"id").to_js()];
    let mut cursor = id.clone();
    while cursor.truthy() {
        if seen.iter().any(|s| same_value_zero(s, &cursor)) {
            return false;
        }
        seen.push(cursor.clone());
        let Some(source) = layers
            .iter()
            .find(|l| js_sem::strict_eq(&node::get(l, b"id").to_js(), &cursor))
        else {
            return false;
        };
        let ty = node::member(Some(source), "type");
        if ["audio", "null", "adjustment", "group"]
            .iter()
            .any(|t| node::is_str(ty, t))
        {
            return false;
        }
        cursor = node::get(source, b"matteSource").to_js();
    }
    true
}

/// `Set.has` equality.
fn same_value_zero(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Number(x), JsValue::Number(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => js_sem::strict_eq(a, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::json::parse;

    fn layers(text: &str) -> Vec<Node> {
        match Node::from_value(parse(text).unwrap(), 1).kind() {
            pm_model::NodeKind::Array(items) => items.to_vec(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn chains_and_cycles() {
        let l = layers(
            r#"[{"id":"a","type":"solid","matteSource":"b"},{"id":"b","type":"shape","matteSource":"c"},
                {"id":"c","type":"text"},{"id":"d","type":"null"},{"id":"e","type":"solid","matteSource":"a"}]"#,
        );
        assert!(valid_matte_source(&l, &l[0], &JsValue::from("b")));
        assert!(valid_matte_source(&l, &l[0], &JsValue::Null));
        assert!(!valid_matte_source(&l, &l[0], &JsValue::from("d")), "null layers are not sources");
        assert!(!valid_matte_source(&l, &l[0], &JsValue::from("x")), "missing");
        assert!(!valid_matte_source(&l, &l[0], &JsValue::from("e")), "e -> a is a cycle");
        assert!(!valid_matte_source(&l, &l[2], &JsValue::from("a")), "a -> b -> c revisits c");
    }
}
