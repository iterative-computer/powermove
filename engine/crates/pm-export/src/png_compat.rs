//! Select the encoded PNG compatibility boundary from the frozen document.
use pm_eval::node;
use pm_model::{Document, Node, NodeKind};

pub(crate) fn has_level1_content(document: &Document) -> bool {
    fn has_level1(comp: &Node) -> bool {
        node::array(node::member(Some(comp), "layers"))
            .unwrap_or(&[])
            .iter()
            .any(|layer| {
                !matches!(
                    node::to_number(node::member(Some(layer), "compat")).partial_cmp(&2.0),
                    Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                )
            })
    }
    let Some(root) = document.root() else {
        return false;
    };
    has_level1(root)
        || match node::member(Some(root), "comps").map(Node::kind) {
            Some(NodeKind::Object(comps)) => comps.keys().any(|id| {
                pm_eval::Comp::resolve(document, &id.to_string_lossy())
                    .is_ok_and(|comp| has_level1(comp.node()))
            }),
            _ => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn considers_all_compositions_and_defaults_old_layers_to_level1() {
        for (json, expected) in [
            (r#"{}"#, false),
            (r#"{"layers":[]}"#, false),
            (r#"{"layers":[{"type":"solid"}]}"#, true),
            (r#"{"layers":[{"compat":1},{"compat":2}]}"#, true),
            (r#"{"layers":[{"compat":2}]}"#, false),
            (
                r#"{"id":"a","layers":[{"compat":2}],"comps":{"a":{"layers":[{}]}}}"#,
                false,
            ),
            (
                r#"{"comps":{"a":{"layers":[{"compat":2}]},"b":{"layers":[{}]}}}"#,
                true,
            ),
            (
                r#"{"comps":{"a":{"layers":[{"compat":2}]},"b":{"layers":[{"compat":2}]}}}"#,
                false,
            ),
            (
                r#"{"layers":[{"compat":2}],"assets":{"x":{"layers":[{}]}}}"#,
                false,
            ),
        ] {
            let document = Document::new(Some(pm_model::json::parse(json).unwrap()), 1);
            assert_eq!(has_level1_content(&document), expected, "{json}");
        }
    }
}
