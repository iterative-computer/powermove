//! S099–S101. Full workspace definitions are authored data, not just dock layout.
//! Tag: legacy/core/workspace.ts, WorkspacesPanel, library/after-effects-imports.
use super::{RecordHost, Records, id, invalid, replace, stale};
use crate::Result;
use serde_json::Value;

pub trait WorkspaceHost: RecordHost {
    /// Same validator as visual/agent edits, preserving unknown metadata. Must
    /// enforce required Composition access and panel registration, not just JSON syntax.
    fn validate_definition(&mut self, definition: &Value) -> Result<()>;
    /// Activate the published full record, registering custom panels and applying
    /// features, but retain the user's explicit preview-resolution preference.
    fn activate(&mut self, workspace_id: &str, revision: u64) -> Result<()>;
}
#[derive(Clone, Debug, PartialEq)]
pub struct DefinitionDraft {
    pub source_id: String,
    pub source_revision: u64,
    pub json: String,
}
impl DefinitionDraft {
    pub fn open(records: &Records, workspace_id: &str) -> Result<Self> {
        records.validate()?;
        Ok(Self {
            source_id: workspace_id.into(),
            source_revision: records.revision,
            json: serde_json::to_string_pretty(records.find(workspace_id)?)?,
        })
    }
    pub fn apply(&self, host: &mut impl WorkspaceHost, records: &Records) -> Result<u64> {
        if records.revision != self.source_revision {
            return Err(stale());
        }
        if records.find(&self.source_id)?["builtin"] == true {
            return Err(invalid("built-in requires Save as New"));
        }
        let mut value: Value = serde_json::from_str(&self.json)?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| invalid("workspace definition must be an object"))?;
        // Tagged editor pins identity; raw JSON cannot replace another workspace.
        object.insert("id".into(), Value::String(self.source_id.clone()));
        host.validate_definition(&value)?;
        let after: Vec<_> = records
            .entries
            .iter()
            .map(|old| {
                if old["id"] == self.source_id {
                    value.clone()
                } else {
                    old.clone()
                }
            })
            .collect();
        let revision = replace(host, "workspaces", records, &after)?;
        // Activation failure remains visible; definition has already published.
        host.activate(&self.source_id, revision)?;
        Ok(revision)
    }
}
pub fn save_as(
    host: &mut impl WorkspaceHost,
    records: &Records,
    source: &Value,
    new_id: &str,
    name: &str,
) -> Result<u64> {
    records.validate()?;
    if new_id.is_empty()
        || records.entries.iter().any(|v| v["id"] == new_id)
        || name.trim().is_empty()
    {
        return Err(invalid("invalid workspace identity or name"));
    }
    if records
        .entries
        .iter()
        .any(|v| v["name"].as_str() == Some(name.trim()))
    {
        return Err(invalid(
            "workspace name already exists; use explicit Replace or a new name",
        ));
    }
    let mut new = source.clone();
    let object = new
        .as_object_mut()
        .ok_or_else(|| invalid("workspace must be an object"))?;
    object.insert("id".into(), new_id.into());
    object.insert("name".into(), name.trim().into());
    object.insert("builtin".into(), false.into());
    host.validate_definition(&new)?;
    let mut after = records.entries.clone();
    after.push(new);
    let revision = replace(host, "workspaces", records, &after)?;
    host.activate(new_id, revision)?;
    Ok(revision)
}
pub fn archive_workspace(
    host: &mut impl WorkspaceHost,
    records: &Records,
    workspace_id: &str,
    current_id: &str,
    fallback_id: &str,
) -> Result<u64> {
    if records.find(workspace_id)?["builtin"] == true {
        return Err(invalid("cannot archive built-in workspace"));
    }
    if current_id == workspace_id {
        if fallback_id == workspace_id {
            return Err(invalid("archive needs a surviving workspace"));
        }
        records.find(fallback_id)?;
    }
    let after: Vec<_> = records
        .entries
        .iter()
        .filter(|v| v["id"] != workspace_id)
        .cloned()
        .collect();
    let revision = replace(host, "workspaces", records, &after)?;
    if current_id == workspace_id {
        host.activate(fallback_id, revision)?;
    }
    Ok(revision)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shelf {
    Workspaces,
    Panels,
    AfterEffects,
}
/// Source-app and panel metadata survives preview/editor handoff verbatim.
pub fn shelf_entries(records: &Records, shelf: Shelf) -> Result<Vec<Value>> {
    records.validate()?;
    Ok(records
        .entries
        .iter()
        .filter(|v| match shelf {
            Shelf::AfterEffects => v["sourceApp"] == "after-effects",
            Shelf::Workspaces => v["sourceApp"] != "after-effects" && v["kind"] != "panel",
            Shelf::Panels => v["sourceApp"] != "after-effects" && v["kind"] == "panel",
        })
        .cloned()
        .collect())
}
#[derive(Clone, Debug, PartialEq)]
pub struct PanelPreview {
    pub panel_id: String,
    pub generation: super::Ticket,
    pub definition: Value,
}
impl PanelPreview {
    pub fn new(definition: Value) -> Result<Self> {
        Ok(Self {
            panel_id: id(&definition)?.into(),
            generation: super::ticket(),
            definition,
        })
    }
}
pub trait PanelHost {
    /// V1–3 logic stays in V8; DOM goes through the remote-DOM bridge. No
    /// source evaluation, Node process or WebKit logic realm in these models.
    fn preview(&mut self, preview: &PanelPreview) -> Result<()>;
    fn edit(&mut self, panel_id: &str, revision: u64) -> Result<()>;
    fn archive(&mut self, panel_id: &str, revision: u64) -> Result<()>;
}
