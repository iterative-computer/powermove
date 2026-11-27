//! The project's saved sections (`library.sections`).
//!
//! Ports `createSection` and `updateSection` of `legacy/core/editing.ts`.

use pm_model::{Map, Value};

use super::{Cx, values};
use crate::error::CommandError;

/// Library commands.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LibraryCommand {
    /// `create_section`: the manifest goes first; at most 24 are kept
    /// (`Section id already exists`). Label "Save Section".
    CreateSection { section: Value },
    /// `update_section`: new layers (required, non-empty), thumbnail and
    /// time; a version is appended (12 kept); tags follow the layer types
    /// (`Section not found`). Label "Update Section".
    UpdateSection {
        section_id: String,
        layers: Vec<Value>,
        thumb: Option<Value>,
        /// Milliseconds since the epoch (`at`).
        at: f64,
        version: Option<Value>,
    },
}

impl LibraryCommand {
    pub fn label(&self) -> String {
        match self {
            LibraryCommand::CreateSection { .. } => "Save Section",
            LibraryCommand::UpdateSection { .. } => "Update Section",
        }
        .to_owned()
    }

    pub(super) fn plan(&self, cx: &mut Cx<'_>) -> Result<(), CommandError> {
        let path: Vec<pm_model::PathPart> = vec!["library".into()];
        let mut library = cx
            .value_at(&path)
            .filter(|v| v.as_object().is_some())
            .unwrap_or_else(|| {
                let mut m = Map::new();
                m.insert("sections", Value::Array(Vec::new()));
                m.insert("looks", Value::Array(Vec::new()));
                Value::Object(m)
            });
        let lib = values::obj(&mut library);
        if lib.get("sections").and_then(Value::as_array).is_none() {
            lib.insert("sections", Value::Array(Vec::new()));
        }
        let sections = lib
            .get_mut("sections")
            .and_then(Value::as_array_mut)
            .expect("made");
        match self {
            LibraryCommand::CreateSection { section } => {
                if section.as_object().is_none() {
                    return Err(CommandError::Invalid("Section manifest is required".into()));
                }
                let id = section.get("id");
                if sections.iter().any(|s| s.get("id") == id) {
                    return Err(CommandError::Invalid("Section id already exists".into()));
                }
                sections.insert(0, section.clone());
                sections.truncate(24);
            }
            LibraryCommand::UpdateSection {
                section_id,
                layers,
                thumb,
                at,
                version,
            } => {
                let section = sections
                    .iter_mut()
                    .find(|s| values::text(s, "id") == Some(section_id.as_str()))
                    .ok_or_else(|| CommandError::Invalid("Section not found".into()))?;
                if layers.is_empty() {
                    return Err(CommandError::Invalid("Section layers are required".into()));
                }
                let mut tags: Vec<Value> = Vec::new();
                for l in layers {
                    if let Some(t) = l.get("type").filter(|t| t.is_truthy())
                        && !tags.contains(t)
                    {
                        tags.push(t.clone());
                    }
                }
                let thumb = thumb
                    .clone()
                    .filter(Value::is_truthy)
                    .or_else(|| section.get("thumb").cloned().filter(Value::is_truthy))
                    .unwrap_or(Value::Null);
                let at = if *at != 0.0 && at.is_finite() {
                    *at
                } else {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as f64)
                        .unwrap_or_default()
                };
                let m = values::obj(section);
                m.insert("layers", Value::Array(layers.clone()));
                m.insert("thumb", thumb.clone());
                m.insert("at", Value::Number(at));
                m.insert("tags", Value::Array(tags));
                if m.get("versions").and_then(Value::as_array).is_none() {
                    m.insert("versions", Value::Array(Vec::new()));
                }
                let entry = version.clone().unwrap_or_else(|| {
                    let mut v = Map::new();
                    v.insert("id", values::s(&super::new_id("SV")));
                    v.insert("layers", Value::Array(layers.clone()));
                    v.insert("thumb", thumb);
                    v.insert("at", Value::Number(at));
                    Value::Object(v)
                });
                let versions = m
                    .get_mut("versions")
                    .and_then(Value::as_array_mut)
                    .expect("made");
                versions.push(entry);
                if versions.len() > 12 {
                    let extra = versions.len() - 12;
                    versions.drain(..extra);
                }
            }
        }
        cx.replace(path, library)
    }
}
