//! Winning full definitions and contextual defaults for application actions.
use crate::{EffectDefinition, Inspector, InspectorError, ParamDefinition};
use pm_fx_defs::{Definition, catalog::EffectCatalog};
use pm_kernel::QueryError;
use pm_model::Value;

pub(crate) fn typed(definition: &Definition) -> EffectDefinition {
    EffectDefinition {
        effect_type: definition.id.clone(),
        version: pm_fx_defs::catalog::effect_version(definition.metadata.get("version"))
            .expect("validated catalog version") as u32,
        label: definition.label.clone(),
        params: definition
            .params
            .iter()
            .map(|p| ParamDefinition {
                key: p.k.clone(),
                kind: p.kind.clone().unwrap_or_else(|| "number".into()),
                default: pm_model::json::parse(&p.def.to_string()).expect("JSON parameter default"),
                min: p.min.as_ref().and_then(|n| n.as_f64()),
                max: p.max.as_ref().and_then(|n| n.as_f64()),
                reference: p.is_reference() || p.kind.as_deref() == Some("texture"),
            })
            .collect(),
        source_processing: definition
            .metadata
            .get("sourceProcessing")
            .is_some_and(|v| !v.is_null() && v.as_bool() != Some(false)),
        backdrop: definition
            .metadata
            .get("backdrop")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}
impl Inspector {
    /// Install the reconciled display registry's full version metadata.
    /// The host owns current display-only descriptors separately; this catalog
    /// never supplies renderer availability. Identical publications retain caches.
    pub fn install_effect_catalog(&mut self, catalog: EffectCatalog) -> Result<(), InspectorError> {
        if catalog
            .definitions()
            .any(|(_, version, _)| version > u64::from(u32::MAX))
        {
            return Err(InspectorError::InvalidInput);
        }
        let changed = !self
            .effect_catalog
            .as_ref()
            .is_some_and(|old| old.definitions().eq(catalog.definitions()));
        for d in catalog.current_definitions() {
            self.definitions.insert(d.id.clone(), typed(d));
        }
        if !changed {
            return Ok(());
        }
        self.effect_catalog = Some(catalog);
        self.definition_generation = self.definition_generation.wrapping_add(1);
        Ok(())
    }
    pub(crate) fn saved_effect_definition(&self, effect: &Value) -> Option<EffectDefinition> {
        let id = effect.get("type")?.as_str()?;
        let Some(catalog) = &self.effect_catalog else {
            return self.definitions.get(id).cloned();
        };
        let version = saved_version(effect)?;
        if catalog.current(id).is_none() {
            return self
                .definitions
                .get(id)
                .filter(|d| u64::from(d.version) == version)
                .cloned();
        }
        catalog.get(id, version).map(typed)
    }
    pub(crate) fn contextual_effect_definition(
        &mut self,
        layer: &str,
        definition: EffectDefinition,
        time: f64,
    ) -> Result<EffectDefinition, InspectorError> {
        let contexts = self
            .effect_catalog
            .as_ref()
            .and_then(|c| c.get(&definition.effect_type, u64::from(definition.version)))
            .map(|d| {
                d.params
                    .iter()
                    .filter(|p| p.kind.as_deref().is_none_or(|k| k == "number"))
                    .filter_map(|p| {
                        let c = p.presentation.get("defaultFrom")?;
                        Some((
                            p.k.clone(),
                            c.get("dimension")?.as_str() == Some("width"),
                            c.get("factor")?.as_f64()?,
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if contexts.is_empty() {
            return Ok(definition);
        }
        let value = self.layer(layer)?;
        let location = self
            .published
            .snapshot
            .root()
            .and_then(|r| pm_kernel::model::find_layer(r, layer))
            .ok_or_else(|| InspectorError::MissingLayer(layer.into()))?;
        // The mirror has no fonts or models, so those layers have no bounds
        // here and take the same fallback as an empty layer.
        let bounds = match self
            .mirror
            .queries()
            .bounds(location.comp_id.as_deref(), layer, time)
        {
            Err(
                QueryError::NeedsFonts
                | QueryError::NeedsInk
                | QueryError::NeedsModels(_)
                | QueryError::NeedsHost
                | QueryError::Unsupported(_),
            ) => None,
            bounds => bounds?,
        };
        let positive = |a: Option<f64>, field: &str| {
            let data = value.get("d").and_then(|d| d.get(field));
            let data = data.and_then(|v| v.get("v").unwrap_or(v).as_f64());
            [a, data, self.composition_field(field).as_f64(), Some(1.)]
                .into_iter()
                .flatten()
                .find(|v| v.is_finite() && *v > 0.)
                .expect("positive fallback")
        };
        let width = positive(bounds.map(|b| b.w), "w");
        let height = positive(bounds.map(|b| b.h), "h");
        let mut definition = definition;
        for (key, is_width, factor) in contexts {
            let v = if is_width { width } else { height } * factor;
            if v.is_finite()
                && let Some(p) = definition.params.iter_mut().find(|p| p.key == key)
            {
                p.default = Value::Number(v);
            }
        }
        Ok(definition)
    }
}

/// A saved effect's revision: none is 1, anything else must be a safe integer.
fn saved_version(effect: &Value) -> Option<u64> {
    match effect.get("version") {
        None | Some(Value::Null) => Some(1),
        Some(v) => {
            let n = v.as_f64()?;
            (n.is_finite()
                && n >= 1.
                && n.fract() == 0.
                && n <= pm_fx_defs::catalog::MAX_SAFE_VERSION as f64)
                .then_some(n as u64)
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ColorAction {
    pub index: usize,
    pub label: String,
    pub keys: [String; 2],
}
impl Inspector {
    fn raw_saved_effect_definition(&self, effect: &Value) -> Option<&Definition> {
        let id = effect.get("type")?.as_str()?;
        self.effect_catalog
            .as_ref()?
            .get(id, saved_version(effect)?)
    }
    /// The effect's definition says `grade: true`: a grading tool the Colour
    /// panel lists (built-in or an extension's).
    pub(crate) fn is_grade(&self, effect: &Value) -> bool {
        self.raw_saved_effect_definition(effect)
            .and_then(|d| d.metadata.get("grade"))
            .and_then(|v| v.as_bool())
            == Some(true)
    }
    pub(crate) fn color_actions(&self, effect: &Value) -> Result<Vec<ColorAction>, InspectorError> {
        let Some(def) = self.raw_saved_effect_definition(effect) else {
            return Ok(vec![]);
        };
        let Some(raw) = def.metadata.get("actions") else {
            return Ok(vec![]);
        };
        let list = raw
            .as_array()
            .filter(|v| v.len() <= 16)
            .ok_or(InspectorError::InvalidInput)?;
        list.iter()
            .enumerate()
            .map(|(index, a)| {
                let keys = a
                    .get("keys")
                    .and_then(|v| v.as_array())
                    .filter(|v| v.len() == 2)
                    .ok_or(InspectorError::InvalidInput)?;
                let keys = [
                    keys[0]
                        .as_str()
                        .ok_or(InspectorError::InvalidInput)?
                        .to_owned(),
                    keys[1]
                        .as_str()
                        .ok_or(InspectorError::InvalidInput)?
                        .to_owned(),
                ];
                let label = a
                    .get("label")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .ok_or(InspectorError::InvalidInput)?;
                if a.get("kind").and_then(|v| v.as_str()) != Some("swap")
                    || keys[0] == keys[1]
                    || keys.iter().any(|key| {
                        !def.params
                            .iter()
                            .any(|p| &p.k == key && p.kind.as_deref() == Some("color"))
                    })
                {
                    return Err(InspectorError::InvalidInput);
                }
                Ok(ColorAction {
                    index,
                    label: label.into(),
                    keys,
                })
            })
            .collect()
    }
    pub(crate) fn saved_parameter_presentation(
        &self,
        effect: &Value,
        key: &str,
    ) -> Option<crate::v4_effects::ParameterPresentation> {
        let p = self
            .raw_saved_effect_definition(effect)?
            .params
            .iter()
            .find(|p| p.k == key)?;
        let options = p
            .presentation
            .get("options")
            .and_then(|v| v.as_array())
            .map(|v| {
                v.iter()
                    .map(|v| pm_model::json::parse(&v.to_string()).expect("JSON option"))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let kind = p.kind.as_deref().unwrap_or("number");
        Some(crate::v4_effects::ParameterPresentation {
            label: p.label.clone(),
            control: if !options.is_empty() {
                "choice"
            } else {
                match kind {
                    "number" => "number",
                    "color" => "color",
                    "toggle" => "toggle",
                    "text" => "text",
                    "select" => "choice",
                    _ => "opaque",
                }
            }
            .into(),
            step: p.presentation.get("step").and_then(|v| v.as_f64()),
            unit: p
                .presentation
                .get("unit")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            precision: p
                .presentation
                .get("precision")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize),
            options,
        })
    }
}
