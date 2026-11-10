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
    /// The saved revision of `id` when this build has the effect but cannot
    /// resolve that revision (a newer app saved it): bypassed in the viewer,
    /// refused by exports, the saved data untouched.
    pub(crate) fn unavailable_revision(
        &self,
        id: &str,
        version: Option<&pm_model::Node>,
    ) -> Option<String> {
        self.saved_key(id, version).err().map(|_| match version {
            Some(v) if !pm_eval::node::is_nullish(Some(v)) => {
                pm_eval::node::to_js_string(Some(v)).to_string_lossy()
            }
            _ => "1".to_owned(),
        })
    }

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
        let Some(registered) = main_catalog().current(id) else {
            // An extension's own effect (no built-in has its id): the
            // registration the kernel chose is its only version.
            return if current.origin == DefinitionOrigin::BuiltIn {
                Err(format!("unversioned provider history unavailable for {id}"))
            } else {
                Ok(id.into())
            };
        };
        // A fork or override that wins a built-in id is the registration the
        // kernel chose, as for an extension's own effect. Contributed effects
        // carry no history, so it is its own version 1 (absent saved versions
        // mean 1); any other saved revision has no provider and stays refused.
        if current.origin != DefinitionOrigin::BuiltIn {
            return if revision == 1 {
                Ok(id.into())
            } else {
                Err(format!(
                    "saved {id} version {revision}: winning provider history unavailable"
                ))
            };
        }
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
        if current
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
        let tagged = v120_catalog()
            .get(id, revision)
            .is_some_and(|tag| std::ptr::eq(tag, def));
        let key = alias(id, revision, tagged);
        self.get(DefinitionKind::Effect, &key)
            .ok_or_else(|| format!("missing saved {id} version {revision}"))?;
        Ok(key)
    }
}

/// The built-in definitions the document's effects use. `memo` keeps each
/// unchanged subtree's effect types from the last call.
pub(crate) fn for_document(
    document: &pm_model::Document,
    memo: &mut crate::doc_walk::SubtreeMemo<String>,
) -> DefinitionSetSnapshot {
    /// One node's effect types: the `type` of each entry of its `fx`.
    fn visit(node: &pm_model::Node, ids: &mut Vec<String>) {
        if let pm_model::NodeKind::Object(map) = node.kind()
            && let Some(fx) = map.get(b"fx" as &[u8])
        {
            for effect in pm_eval::node::array(Some(fx)).unwrap_or(&[]) {
                if let Some(id) = pm_eval::node::string(pm_eval::node::member(Some(effect), "type"))
                {
                    ids.push(id.to_string_lossy());
                }
            }
        }
    }
    let ids: std::collections::HashSet<String> = document
        .root()
        .map(|root| memo.walk(root, &mut visit).into_iter().collect())
        .unwrap_or_default();
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

/// Whether this build can draw an effect, given its type and saved version:
/// `Some(None)` no provider, `Some(Some(revision))` no such revision.
pub type Unavailable<'a> = dyn Fn(&str, Option<&pm_model::Node>) -> Option<Option<String>> + 'a;

/// An effect type the document uses that no definition in its set provides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingEffect {
    pub effect: String,
    /// Set when this build has the effect but not the saved revision (a
    /// newer Powermove saved it); `None` when no provider is installed.
    pub revision: Option<String>,
    /// Layers that use it.
    pub layers: usize,
}

/// The enabled effects the layers drawn by `composition` (and the
/// compositions it nests) use that this build cannot draw, commonest first:
/// `unavailable(type, saved version)` is `Some(None)` when no provider is
/// installed, `Some(Some(revision))` when this build lacks the saved
/// revision, `None` when it draws. Effects
/// switched off, on layers switched off, in hidden groups or in compositions
/// it never reaches don't count: they never draw.
pub fn missing_effects(
    document: &pm_model::Document,
    composition: &str,
    unavailable: &Unavailable<'_>,
) -> Vec<MissingEffect> {
    use pm_eval::node;
    use pm_model::{Node, NodeKind};
    let Some(root) = document.root() else {
        return vec![];
    };
    let open = crate::capability::open_composition_id(root);
    let comp_node = |id: &str| -> Option<&Node> {
        if open.as_deref() == Some(id) && node::member(Some(root), "layers").is_some() {
            return Some(root);
        }
        node::member(node::member(Some(root), "comps"), id)
    };
    // `on` as authored: a plain false, or a property held at false, is off.
    let off = |n: Option<&Node>| match n.map(Node::kind) {
        Some(NodeKind::Bool(false)) => true,
        _ if node::is_property(n) => {
            !node::has_kf_array(n)
                && matches!(
                    node::member(n, "v").map(Node::kind),
                    Some(NodeKind::Bool(false))
                )
        }
        _ => false,
    };
    let mut counts = std::collections::BTreeMap::<(String, Option<String>), usize>::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec![composition.to_owned()];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(comp) = comp_node(&id) else { continue };
        let layers = node::array(node::member(Some(comp), "layers")).unwrap_or(&[]);
        let layer_id = |l: &Node| {
            node::js_value_opt(node::member(Some(l), "id"))
                .to_js_string()
                .to_string_lossy()
        };
        // A matte source draws even when it is switched off.
        let matte = |l: &Node| {
            let id = layer_id(l);
            layers.iter().any(|o| {
                node::js_value_opt(node::member(Some(o), "matteSource"))
                    .to_js_string()
                    .to_string_lossy()
                    == id
            })
        };
        let hidden_group = |l: &Node| {
            let mut at = node::member(Some(l), "group");
            for _ in 0..64 {
                let Some(g) = at.filter(|g| node::truthy(Some(g))) else {
                    return false;
                };
                let g = node::js_value_opt(Some(g)).to_js_string().to_string_lossy();
                let Some(group) = layers.iter().find(|o| layer_id(o) == g) else {
                    return false;
                };
                if off(node::member(Some(group), "on")) {
                    return true;
                }
                at = node::member(Some(group), "group");
            }
            false
        };
        for layer in layers {
            if (off(node::member(Some(layer), "on")) || hidden_group(layer)) && !matte(layer) {
                continue;
            }
            if crate::js::layer_type(layer) == "precomp"
                && let Some(c) = node::member(node::member(Some(layer), "d"), "comp")
                    .filter(|c| node::truthy(Some(c)))
            {
                stack.push(node::js_value_opt(Some(c)).to_js_string().to_string_lossy());
            }
            let mut types = std::collections::BTreeSet::new();
            for fx in node::array(node::member(Some(layer), "fx")).unwrap_or(&[]) {
                if off(node::member(Some(fx), "on")) {
                    continue;
                }
                if let Some(ty) = node::string(node::member(Some(fx), "type")) {
                    let ty = ty.to_string_lossy();
                    if let Some(revision) = unavailable(&ty, node::member(Some(fx), "version")) {
                        types.insert((ty, revision));
                    }
                }
            }
            for ty in types {
                *counts.entry(ty).or_default() += 1;
            }
        }
    }
    let mut out: Vec<_> = counts
        .into_iter()
        .map(|((effect, revision), layers)| MissingEffect {
            effect,
            revision,
            layers,
        })
        .collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.layers));
    out
}

pub(crate) fn main_definition(key: &str) -> Option<&'static Definition> {
    let key = key.strip_prefix("pm.history.main.")?;
    let (id, version) = key.rsplit_once('.')?;
    main_catalog().get(id, version.parse().ok()?)
}
