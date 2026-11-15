//! Local per-profile library; Electron's schemaVersion 1 object is the file format.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
#[derive(Default)]
pub(crate) struct Library {
    pub folder: Option<PathBuf>,
    deleted: Vec<Value>,
}
impl Library {
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("Invalid preset ID".into());
        }
        Ok(self
            .folder
            .as_ref()
            .ok_or("Preset folder unavailable")?
            .join(format!("{id}.json")))
    }
    pub fn list(&self) -> Vec<Value> {
        let Some(folder) = &self.folder else {
            return vec![];
        };
        let mut list: Vec<_> = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|s| s == "json"))
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
            .filter(|p| {
                p["schemaVersion"] == 1
                    && p["id"].as_str().is_some_and(|id| self.path(id).is_ok())
                    && p["id"].is_string()
                    && p["name"].is_string()
                    && p["effects"].is_array()
                    && p["properties"].is_array()
            })
            .collect();
        list.sort_by_key(|p| p["name"].as_str().unwrap_or("").to_lowercase());
        list
    }
    pub fn save(&self, preset: &Value) -> Result<(), String> {
        let name = preset["name"].as_str().ok_or("Name the preset")?;
        if name.trim().is_empty() || name.chars().count() > 80 || preset["schemaVersion"] != 1 {
            return Err("Invalid preset".into());
        }
        let path = self.path(preset["id"].as_str().ok_or("Preset ID")?)?;
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        use std::io::Write;
        file.write_all(
            serde_json::to_string_pretty(preset)
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn action(&mut self, input: &Value) -> Result<Value, String> {
        let id = input["id"].as_str().unwrap_or("");
        match input["action"].as_str() {
            Some("rename") => {
                let mut p = self
                    .list()
                    .into_iter()
                    .find(|p| p["id"] == id)
                    .ok_or("Preset not found")?;
                p["name"] = input["name"].clone();
                self.save(&p)?;
            }
            Some("delete") => {
                let p = self
                    .list()
                    .into_iter()
                    .find(|p| p["id"] == id)
                    .ok_or("Preset not found")?;
                std::fs::remove_file(self.path(id)?).map_err(|e| e.to_string())?;
                self.deleted.push(p);
            }
            Some("undo") => {
                if let Some(p) = self.deleted.last() {
                    if self.path(p["id"].as_str().unwrap())?.exists() {
                        return Err("Preset ID already exists".into());
                    }
                    self.save(p)?;
                    self.deleted.pop();
                }
            }
            _ => return Err("Unknown preset action".into()),
        };
        Ok(json!(self.list()))
    }
}
impl crate::Builtins {
    pub fn set_presets_folder(&self, profile: &Path) {
        self.broker.presets.shared().folder = Some(profile.join("presets"));
    }
}
use crate::LockShared;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn save_reload_rename_delete_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = Library {
            folder: Some(dir.path().join("presets")),
            ..Default::default()
        };
        let p = json!({"schemaVersion":1,"id":"p1","name":"Soft","effects":[],"properties":[]});
        l.save(&p).unwrap();
        assert_eq!(l.list(), vec![p.clone()]);
        l.action(&json!({"action":"rename","id":"p1","name":"Renamed"}))
            .unwrap();
        assert_eq!(l.list()[0]["name"], "Renamed");
        l.action(&json!({"action":"delete","id":"p1"})).unwrap();
        assert!(l.list().is_empty());
        l.action(&json!({"action":"undo"})).unwrap();
        assert_eq!(l.list()[0]["name"], "Renamed");
        assert!(l.path("../escape").is_err());
    }
}
#[cfg(test)]
mod wiring_tests {
    use super::*;
    use crate::tests::{actor, finish};
    #[test]
    fn animation_preset_sheet_and_library_use_live_v8_surface() {
        let actor = actor();
        let mut panels = crate::Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
        let dir = tempfile::tempdir().unwrap();
        panels.set_presets_folder(dir.path());
        panels.broker.presets.shared().save(&json!({"schemaVersion":1,"id":"soft","name":"Soft Glow","effects":[],"properties":[]})).unwrap();
        finish(panels.mount("fxbrowser").unwrap());
        let publication = panels.take_publications().pop().unwrap();
        assert!(
            publication
                .surface
                .nodes()
                .values()
                .any(|n| n.text == "My Presets")
        );
        assert!(
            publication
                .surface
                .nodes()
                .values()
                .any(|n| n.text == "Soft Glow")
        );
        finish(panels.preset_sheet(json!({"layer":"a"})).unwrap());
        let publication = panels.take_publications().pop().unwrap();
        assert!(
            publication
                .surface
                .nodes()
                .values()
                .any(|n| n.text == "Save Animation Preset")
        );
        assert!(panels.take_errors().is_empty());
    }
}
