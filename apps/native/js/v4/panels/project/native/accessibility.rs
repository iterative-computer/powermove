//! Panel-owned AX enrichment; the shared UI kit remains unchanged.
use crate::bridge::Projection;
use accesskit::{Action, ActionData, ActionRequest, CustomAction, Role, TreeId, TreeUpdate};
use pm_panel_media::{
    Availability, Download, Input, ItemId, MenuCommand, Modifiers, PointerPhase, Row, RowData,
    Stamp,
};
use pm_surface::{
    Surface,
    kit::{Layout, accessibility_tree},
};
use std::collections::BTreeMap;

pub struct ProjectAccessibility {
    pub tree: TreeUpdate,
    revision: u64,
    owner: Stamp,
    actions: BTreeMap<u64, BTreeMap<i32, Input>>,
}
fn commands(row: &Row, composition_count: usize) -> Vec<(&'static str, Input)> {
    let invoke = |command| Input::Invoke(command);
    match &row.data {
        RowData::Folder(f) => vec![
            ("Open", Input::OpenFolder(Some(f.id.clone()))),
            ("Rename", Input::BeginRename(f.id.clone())),
            ("Delete", invoke(MenuCommand::DeleteFolder(f.id.clone()))),
        ],
        RowData::Composition(c) => {
            let mut result = vec![];
            if !c.open {
                result.push((
                    "Open in Timeline",
                    invoke(MenuCommand::OpenComposition(c.id.clone())),
                ));
            }
            if c.can_nest {
                result.push((
                    "Add to Timeline",
                    invoke(MenuCommand::NestComposition(c.id.clone())),
                ));
            }
            if composition_count > 1 {
                result.push((
                    "Delete",
                    invoke(MenuCommand::DeleteComposition(c.id.clone())),
                ));
            }
            result
        }
        RowData::Asset(a, runtime) => {
            let mut result = vec![];
            match &runtime.availability {
                Availability::Ready => {
                    result.push((
                        "Add to Timeline",
                        invoke(MenuCommand::AddAsset(a.id.clone())),
                    ));
                    if a.kind != "model" {
                        result.push((
                            "Preview",
                            Input::Pointer {
                                id: ItemId::Asset(a.id.clone()),
                                phase: PointerPhase::DoubleClick,
                                button: 0,
                                nested_control: false,
                                modifiers: Modifiers::default(),
                            },
                        ));
                    }
                    if a.source_path.is_some() {
                        result.push(("Show in Finder", invoke(MenuCommand::Reveal(a.id.clone()))));
                    }
                }
                Availability::Offline | Availability::Failed(_) => {
                    result.push(("Locate", invoke(MenuCommand::Replace(a.id.clone()))))
                }
                Availability::Cloud { download, .. }
                    if !matches!(download, Download::Downloading) =>
                {
                    result.push(("Download", invoke(MenuCommand::Download(a.id.clone()))))
                }
                _ => {}
            }
            result.push(("Delete", Input::DeleteAsset(a.id.clone())));
            result
        }
    }
}
// Semantic IDs never depend on the current enabled-action order. Runtime-only
// readiness changes may leave the surface/document revision unchanged (N3/N7).
// Append new IDs; never recycle an ID for another intent.
fn action_id(input: &Input) -> i32 {
    match input {
        Input::OpenFolder(_) => 1,
        Input::BeginRename(_) => 2,
        Input::DeleteAsset(_) => 3,
        Input::Pointer { .. } => 4,
        Input::Invoke(command) => match command {
            MenuCommand::AddAsset(_) => 5,
            MenuCommand::Reveal(_) => 6,
            MenuCommand::Replace(_) => 7,
            MenuCommand::Download(_) => 8,
            MenuCommand::DeleteFolder(_) => 9,
            MenuCommand::OpenComposition(_) => 10,
            MenuCommand::NestComposition(_) => 11,
            MenuCommand::DeleteComposition(_) => 12,
            _ => unreachable!("custom action needs a stable semantic ID"),
        },
        _ => unreachable!("custom action needs a stable semantic ID"),
    }
}
impl ProjectAccessibility {
    pub fn prepare(
        surface: &Surface,
        view: &Projection,
        layout: &Layout,
        focus: Option<u64>,
    ) -> Self {
        let mut tree = accessibility_tree(surface, layout, focus);
        let mut actions = BTreeMap::new();
        let mut active = None;
        for (node_id, node) in &mut tree.nodes {
            let Some(source) = surface.node(node_id.0) else {
                continue;
            };
            if source.props.get("role").and_then(serde_json::Value::as_str) == Some("toolbar") {
                node.set_role(Role::Toolbar);
            }
            if source
                .props
                .get("label")
                .and_then(serde_json::Value::as_str)
                == Some("Search media")
            {
                node.set_role(Role::SearchInput);
            }
            let Some(id) = source
                .props
                .get("attr")
                .and_then(|a| a.get("item"))
                .and_then(|v| serde_json::from_value::<ItemId>(v.clone()).ok())
            else {
                continue;
            };
            let Some((index, row)) = view
                .page
                .rows
                .iter()
                .enumerate()
                .find(|(_, row)| row.id == id)
            else {
                continue;
            };
            node.set_role(Role::Cell);
            node.set_row_index(view.page.offset + index);
            node.set_selected(row.selected);
            if row.focused {
                active = Some(*node_id);
            }
            let choices = commands(row, view.composition_count);
            node.set_custom_actions(
                choices
                    .iter()
                    .map(|(label, input)| CustomAction {
                        id: action_id(input),
                        description: (*label).into(),
                    })
                    .collect::<Vec<_>>(),
            );
            if !choices.is_empty() {
                node.add_action(Action::CustomAction);
            }
            actions.insert(
                node_id.0,
                choices
                    .into_iter()
                    .map(|(_, input)| (action_id(&input), input))
                    .collect(),
            );
        }
        for (node_id, node) in &mut tree.nodes {
            if surface
                .node(node_id.0)
                .and_then(|s| s.props.get("label"))
                .and_then(serde_json::Value::as_str)
                == Some("Project media")
            {
                node.set_row_count(view.page.total);
                node.set_multiselectable();
                if let Some(active) = active {
                    node.set_active_descendant(active);
                }
            }
        }
        Self {
            tree,
            revision: surface.revision(),
            owner: view.page.stamp.clone(),
            actions,
        }
    }
    /// Validate on the native worker, then enqueue the returned Input. Never
    /// dispatch into JS synchronously from an accessibility callback.
    pub fn input(
        &self,
        revision: u64,
        owner: &Stamp,
        request: &ActionRequest,
    ) -> Result<Input, &'static str> {
        if revision != self.revision || owner != &self.owner || request.target_tree != TreeId::ROOT
        {
            return Err("stale project accessibility tree");
        }
        if request.action != Action::CustomAction {
            return Err("unsupported project accessibility action");
        }
        let Some(ActionData::CustomAction(id)) = request.data else {
            return Err("missing custom action");
        };
        self.actions
            .get(&request.target_node.0)
            .and_then(|a| a.get(&id))
            .cloned()
            .ok_or("unavailable project accessibility action")
    }
}
