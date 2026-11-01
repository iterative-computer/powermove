//! Streams the PMV4 document record straight from the canonical tree.
//!
//! The old app wrote `{"v":..,"proj":..,"history":..,"ws":..}` with
//! `JSON.stringify` (`legacy/app.ts`, `saveProject`). This writes the same
//! members in the same order, each value in the engine's canonical
//! serialization (`JSON.stringify` with keys in UTF-16 order), followed by
//! any envelope members this version does not know, in canonical order.
//!
//! `proj` is the *stored* form: [`pm_model::canonical::realize_project`] of
//! the canonical document, produced member by member from the shared tree
//! instead of materializing a second copy of the project as a `Value`. A
//! test checks the bytes equal `to_canonical_string(realize_project(..))`.

use pm_model::canonical::COMP_FIELDS;
use pm_model::json::write::{Sink, push_quoted, write_canonical};
use pm_model::json::ObjectMap;
use pm_model::{JsString, Map, Node, NodeKind, Value};

/// The envelope members around the project.
pub(crate) struct DocumentParts<'a> {
    pub root: Option<&'a Node>,
    pub had_comps: bool,
    pub history: &'a Value,
    pub workspace: Option<&'a Value>,
    pub extra: &'a Map,
}

fn key(sink: &mut impl Sink, k: &str) {
    let mut s = String::new();
    push_quoted(&JsString::from(k), &mut s);
    sink.push_str(&s);
}

fn js_key(sink: &mut impl Sink, k: &JsString) {
    let mut s = String::new();
    push_quoted(k, &mut s);
    sink.push_str(&s);
}

/// One member of the realized project.
enum Member<'a> {
    Node(&'a Node),
    /// `comps` without the open composition.
    Comps(&'a ObjectMap<Node>, &'a JsString),
}

fn object_of(node: &Node) -> Option<&ObjectMap<Node>> {
    match node.kind() {
        NodeKind::Object(map) => Some(map),
        _ => None,
    }
}

/// `realize_project` applies only with a non-empty string `compId` naming an
/// object in a `comps` object.
fn realizable(root: &Node) -> Option<(&ObjectMap<Node>, &JsString, &ObjectMap<Node>, &ObjectMap<Node>)> {
    let members = object_of(root)?;
    let id = match members.get("compId".as_bytes()).map(Node::kind) {
        Some(NodeKind::String(s)) if !s.as_bytes().is_empty() => s,
        _ => return None,
    };
    let comps = object_of(members.get("comps".as_bytes())?)?;
    let comp = object_of(comps.get(id.as_bytes())?)?;
    Some((members, id, comps, comp))
}

/// The project node whose `layers`, `w` and `h` will be written at the top
/// level (`assertProjectDocument` runs on the stored form).
pub(crate) fn stored_comp_fields(root: &Node) -> &Node {
    match realizable(root) {
        Some((_, id, comps, _)) => comps.get(id.as_bytes()).expect("realizable"),
        None => root,
    }
}

fn write_project(root: &Node, had_comps: bool, sink: &mut impl Sink) {
    let Some((members, id, comps, comp)) = realizable(root) else {
        write_canonical(root, sink);
        return;
    };
    let mut out: Vec<(JsString, Member)> = Vec::with_capacity(members.len() + comp.len());
    for (k, v) in members.iter() {
        if k.as_bytes() == b"comps" {
            let others = comps.len() - 1;
            if others == 0 && !had_comps {
                continue;
            }
            out.push((k.clone(), Member::Comps(comps, id)));
        } else {
            out.push((k.clone(), Member::Node(v)));
        }
    }
    for (k, v) in comp.iter() {
        let target = match k.as_str() {
            Some("name") => "compName",
            Some(f) if COMP_FIELDS.contains(&f) => f,
            _ => continue,
        };
        let target = JsString::from(target);
        // The composition's value wins over a stale top-level member.
        out.retain(|(existing, _)| *existing != target);
        out.push((target, Member::Node(v)));
    }
    out.sort_by(|a, b| a.0.cmp_utf16(&b.0));
    sink.push_str("{");
    for (i, (k, member)) in out.iter().enumerate() {
        if i > 0 {
            sink.push_str(",");
        }
        js_key(sink, k);
        sink.push_str(":");
        match member {
            Member::Node(node) => write_canonical(*node, sink),
            Member::Comps(comps, open) => {
                let mut entries: Vec<(&JsString, &Node)> =
                    comps.iter().filter(|(cid, _)| *cid != *open).collect();
                entries.sort_by(|a, b| a.0.cmp_utf16(b.0));
                sink.push_str("{");
                for (j, (cid, c)) in entries.iter().enumerate() {
                    if j > 0 {
                        sink.push_str(",");
                    }
                    js_key(sink, cid);
                    sink.push_str(":");
                    write_canonical(*c, sink);
                }
                sink.push_str("}");
            }
        }
    }
    sink.push_str("}");
}

/// Writes the whole document record.
pub(crate) fn write_document(parts: &DocumentParts<'_>, sink: &mut impl Sink) {
    sink.push_str("{");
    key(sink, "v");
    sink.push_str(":");
    write_canonical(&Value::from(super::ENVELOPE_VERSION), sink);
    sink.push_str(",");
    key(sink, "proj");
    sink.push_str(":");
    match parts.root {
        Some(root) => write_project(root, parts.had_comps, sink),
        None => sink.push_str("null"),
    }
    sink.push_str(",");
    key(sink, "history");
    sink.push_str(":");
    write_canonical(parts.history, sink);
    sink.push_str(",");
    key(sink, "ws");
    sink.push_str(":");
    match parts.workspace {
        Some(ws) => write_canonical(ws, sink),
        None => sink.push_str("null"),
    }
    let mut extra: Vec<(&JsString, &Value)> = parts
        .extra
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), Some("v" | "proj" | "history" | "ws")))
        .collect();
    extra.sort_by(|a, b| a.0.cmp_utf16(b.0));
    for (k, v) in extra {
        sink.push_str(",");
        js_key(sink, k);
        sink.push_str(":");
        write_canonical(v, sink);
    }
    sink.push_str("}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_model::canonical::{canonical_project, realize_project};
    use pm_model::json::{parse, to_canonical_string};

    fn realized(text: &str, had_comps: bool) -> (String, String) {
        let stored = parse(text).unwrap();
        let canonical = canonical_project(stored);
        let root = Node::from_value(canonical.clone(), 1);
        let mut streamed = String::new();
        write_project(&root, had_comps, &mut streamed);
        let mut expected = realize_project(canonical);
        if !had_comps {
            if let Value::Object(m) = &mut expected {
                if m.get("comps").and_then(Value::as_object).is_some_and(Map::is_empty) {
                    m.remove("comps");
                }
            }
        }
        (streamed, to_canonical_string(&expected))
    }

    #[test]
    fn streams_exactly_the_realized_project() {
        for (text, had) in [
            (r#"{"id":"P","compId":"C1","compName":"Main","w":4,"h":3,"layers":[{"id":"a"}],"comps":{"C2":{"id":"C2","layers":[]}},"zz":1,"assets":{}}"#, true),
            (r#"{"id":"P","compId":"C1","w":4,"h":3,"layers":[],"fps":30}"#, false),
            (r#"{"id":"P","compId":"C1","w":4,"h":3,"layers":[],"comps":{}}"#, true),
            (r#"{"id":"P","w":4,"h":3,"layers":[]}"#, true),
            (r#"{"id":"P","compId":"","w":4,"h":3,"layers":[]}"#, true),
            (r#"{"":1,"😀":2,"compId":"C1","w":4,"h":3,"layers":[],"comps":{"x":{},"😀y":{}}}"#, true),
        ] {
            let (streamed, expected) = realized(text, had);
            assert_eq!(streamed, expected, "{text}");
        }
    }
}
