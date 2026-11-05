//! Pinned full definitions, with distinct program identities for every saved
//! revision and the tag/main Gradient collision. Never mutate saved instances.
use super::Effects;
use pm_fx_defs::{Definition, catalog::*};
use pm_protocol::{common::DefinitionId, definitions::*};

fn alias(id: &str, version: u64, tagged: bool) -> String {
    format!(
        "pm.history.{}.{id}.{version}",
        if tagged { "v120" } else { "main" }
    )
}
fn snapshot(def: &Definition, source: &str, id: &str) -> DefinitionSnapshot {
    let flag = |k| {
        def.metadata
            .get(k)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    DefinitionSnapshot {
        id: DefinitionId::new(id),
        kind: DefinitionKind::Effect,
        version: effect_version(def.metadata.get("version")).unwrap() as u32,
        origin: DefinitionOrigin::BuiltIn,
        renderer: None,
        glsl: Some(WrappedGlsl {
            source: source.into(),
            body_line_offset: 0,
            body_utf16_len: source.encode_utf16().count() as u32,
            body_utf8_len: source.len() as u32,
        }),
        params: def
            .params
            .iter()
            .map(|p| match p.kind.as_deref() {
                Some("color") => ParamSpec::Color {
                    key: p.k.clone(),
                    default: p.def.as_str().unwrap_or("#000000").into(),
                },
                Some("toggle") => ParamSpec::Toggle {
                    key: p.k.clone(),
                    default: p.def.as_bool().unwrap_or(false),
                },
                _ => ParamSpec::Number {
                    key: p.k.clone(),
                    default: p.def.as_f64().unwrap_or(0.),
                    min: p.min.as_ref().and_then(|n| n.as_f64()).unwrap_or(-1e12),
                    max: p.max.as_ref().and_then(|n| n.as_f64()).unwrap_or(1e12),
                    step: None,
                },
            })
            .collect(),
        flags: DefinitionFlags {
            passes: def
                .metadata
                .get("passes")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(1) as u8,
            keep_orig: flag("keepOrig"),
            backdrop: flag("backdrop"),
            raw_shader: flag("rawShader"),
            viewport_safe: flag("viewportSafe"),
            viewport_padding: def
                .metadata
                .get("viewportPadding")
                .and_then(serde_json::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            clamp: flag("clamp"),
        },
    }
}
pub fn builtin_snapshot() -> DefinitionSetSnapshot {
    let mut set = crate::all_builtin_definitions();
    let tagged: Vec<_> = set
        .definitions
        .iter()
        .filter(|d| d.kind == DefinitionKind::Effect)
        .cloned()
        .collect();
    set.definitions.retain(|d| d.kind != DefinitionKind::Effect);
    for mut old in tagged {
        old.id = DefinitionId::new(alias(old.id.as_str(), 1, true));
        set.definitions.push(old);
    }
    for (id, version, def) in main_catalog().definitions() {
        let source = &main_effects()
            .shaders
            .iter()
            .find(|s| s.id == id && s.version == version)
            .unwrap()
            .source;
        set.definitions
            .push(snapshot(def, source, &alias(id, version, false)));
        if std::ptr::eq(main_catalog().current(id).unwrap(), def) {
            set.definitions.push(snapshot(def, source, id));
        }
    }
    set
}
impl Effects {
    pub(crate) fn saved_key(
        &self,
        id: &str,
        version: Option<&pm_model::Node>,
    ) -> Result<String, String> {
        if !self.versioned_builtins {
            return Ok(id.into());
        }
        let saved = version
            .map(|n| {
                serde_json::from_str::<serde_json::Value>(&pm_model::json::to_canonical_string(
                    &n.to_value(),
                ))
                .map_err(|e| e.to_string())
            })
            .transpose()?;
        let revision = effect_version(saved.as_ref()).map_err(|e| e.to_string())?;
        let current = self
            .get(DefinitionKind::Effect, id)
            .ok_or_else(|| format!("missing effect provider {id}"))?;
        let registered = main_catalog()
            .current(id)
            .ok_or_else(|| format!("unversioned provider history unavailable for {id}"))?;
        let registered_source = &main_effects()
            .shaders
            .iter()
            .find(|s| {
                s.id == id
                    && s.version == effect_version(registered.metadata.get("version")).unwrap()
            })
            .unwrap()
            .source;
        // History always belongs to the winning registration, never a built-in
        // underneath a fork/local/Store registration with the same ID.
        if current.origin != DefinitionOrigin::BuiltIn
            || current
                .glsl
                .as_ref()
                .is_none_or(|g| g.source != *registered_source)
        {
            return Err(format!(
                "saved {id} version {revision}: winning provider history unavailable"
            ));
        }
        let def = definition_for_file(id, saved.as_ref(), self.file_app_version.as_deref())
            .map_err(|e| e.to_string())?;
        // These programs need producer rasters/leases or special spatial passes.
        // Keep their failure visible until those bindings are installed.
        if def.params.iter().any(|p| p.is_reference())
            || def.metadata.contains_key("temporal")
            || def.metadata.contains_key("sampling")
            || def.metadata.contains_key("grid")
            || matches!(
                id,
                "echo" | "blur" | "pixelate" | "unsharpMask" | "findEdges" | "shadow"
            ) && revision > 1
        {
            return Err(format!(
                "saved {id} version {revision}: native resource bindings unavailable"
            ));
        }
        let tagged = v120_catalog()
            .get(id, revision)
            .is_some_and(|tag| std::ptr::eq(tag, def));
        let key = alias(id, revision, tagged);
        self.get(DefinitionKind::Effect, &key)
            .ok_or_else(|| format!("missing saved {id} version {revision}"))?;
        Ok(key)
    }
}

pub fn for_document(document: &pm_model::Document) -> DefinitionSetSnapshot {
    let mut ids = std::collections::HashSet::new();
    fn walk(node: &pm_model::Node, ids: &mut std::collections::HashSet<String>) {
        match node.kind() {
            pm_model::NodeKind::Object(map) => {
                if let Some(fx) = map.get(b"fx" as &[u8]) {
                    for effect in pm_eval::node::array(Some(fx)).unwrap_or(&[]) {
                        if let Some(id) =
                            pm_eval::node::string(pm_eval::node::member(Some(effect), "type"))
                        {
                            ids.insert(id.to_string_lossy());
                        }
                    }
                }
                for child in map.values() {
                    walk(child, ids);
                }
            }
            pm_model::NodeKind::Array(array) => {
                for child in array.iter() {
                    walk(child, ids);
                }
            }
            _ => {}
        }
    }
    if let Some(root) = document.root() {
        walk(root, &mut ids);
    }
    let mut set = builtin_snapshot();
    set.definitions.retain(|d| {
        d.kind != DefinitionKind::Effect
            || ids.iter().any(|id| {
                d.id.as_str() == id
                    || d.id.as_str().starts_with(&format!("pm.history.main.{id}."))
                    || d.id.as_str().starts_with(&format!("pm.history.v120.{id}."))
            })
    });
    set
}
