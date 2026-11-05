//! Direct composition relationships used by the viewer flowchart. No rendering.
use crate::{Snapshot, model};
use pm_model::{Node, NodeKind};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composition {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flowchart {
    pub current: Composition,
    pub parents: Vec<Composition>,
    pub children: Vec<Composition>,
}
impl Flowchart {
    pub fn read(snapshot: &Snapshot) -> Option<Self> {
        let root = snapshot.document.root()?;
        let active = model::open_comp_id(root)?;
        let NodeKind::Object(comps) = root.get("comps")?.kind() else {
            return None;
        };
        let item = |id: &str| -> Option<Composition> {
            let node = model::comp_node(root, Some(id))?;
            Some(Composition {
                id: id.into(),
                name: node
                    .get("name")
                    .and_then(model::str_of)
                    .unwrap_or("Composition")
                    .into(),
            })
        };
        let mut children = vec![];
        for layer in model::layers_of(model::comp_node(root, Some(&active))?) {
            if let Some(id) = target(layer)
                && let Some(child) = item(id)
                && !children.contains(&child)
            {
                children.push(child);
            }
        }
        let parents = comps
            .iter()
            .filter_map(|(id, node)| {
                model::layers_of(node)
                    .any(|l| target(l) == Some(active.as_str()))
                    .then(|| id.as_str().and_then(item))
                    .flatten()
            })
            .collect();
        Some(Self {
            current: item(&active)?,
            parents,
            children,
        })
    }
}
fn target(layer: &Node) -> Option<&str> {
    (layer.get("type").and_then(model::str_of) == Some("precomp"))
        .then(|| layer.get("d")?.get("comp").and_then(model::str_of))
        .flatten()
}
