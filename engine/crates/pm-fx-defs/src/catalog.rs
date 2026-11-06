//! Exact authored effect revisions. Transport generations are separate from
//! saved implementation versions. A missing revision is an error, never latest.
use crate::{Definition, Parameter};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, fmt, sync::OnceLock};

pub const MAIN_COMMIT: &str = "c7f46a05641b23944d3692c9d80f9677c7e1ef1d";
pub const MAX_SAFE_VERSION: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogError(pub String);
impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CatalogError {}

/// Missing and JSON null instances mean revision 1 (`effectVersion` in main).
/// Strings, booleans, fractions, zero, and unsafe integers never coerce.
pub fn effect_version(value: Option<&Value>) -> Result<u64, CatalogError> {
    match value {
        None | Some(Value::Null) => Ok(1),
        Some(Value::Number(n)) => {
            let v = n.as_f64().unwrap_or(f64::NAN);
            if v >= 1. && v <= MAX_SAFE_VERSION as f64 && v.fract() == 0. {
                Ok(v as u64)
            } else {
                Err(CatalogError("invalid effect version".into()))
            }
        }
        _ => Err(CatalogError("invalid effect version".into())),
    }
}

/// Last-registration-wins is the host registry's responsibility. This catalog
/// resolves one immutable winning set, retaining every full historical shader,
/// parameter, flag and unknown field. It does not mix history from old providers.
#[derive(Clone, Debug)]
pub struct EffectCatalog {
    current: BTreeMap<String, u64>,
    revisions: BTreeMap<(String, u64), Definition>,
}
impl EffectCatalog {
    pub fn new(definitions: Vec<Definition>) -> Result<Self, CatalogError> {
        let mut result = Self {
            current: BTreeMap::new(),
            revisions: BTreeMap::new(),
        };
        for definition in definitions {
            let id = definition.id.clone();
            let version = effect_version(definition.metadata.get("version"))?;
            if result.current.insert(id.clone(), version).is_some() {
                return Err(CatalogError(format!("duplicate effect {id}")));
            }
            if let Some(history) = definition.metadata.get("legacy") {
                let history = history
                    .as_array()
                    .ok_or_else(|| CatalogError(format!("{id}: legacy must be an array")))?;
                if history.len() > 32 {
                    return Err(CatalogError(format!("{id}: too many historical revisions")));
                }
                for raw in history {
                    let old: Definition = serde_json::from_value(raw.clone()).map_err(|e| {
                        CatalogError(format!("{id}: invalid historical definition: {e}"))
                    })?;
                    let old_version = effect_version(old.metadata.get("version"))?;
                    if old.id != id
                        || old.metadata.get("version").is_none_or(Value::is_null)
                        || old_version >= version
                        || old.metadata.contains_key("legacy")
                    {
                        return Err(CatalogError(format!(
                            "{id}: invalid historical revision {old_version}"
                        )));
                    }
                    if result
                        .revisions
                        .insert((id.clone(), old_version), old)
                        .is_some()
                    {
                        return Err(CatalogError(format!(
                            "{id}: duplicate historical revision {old_version}"
                        )));
                    }
                }
            }
            result.revisions.insert((id, version), definition);
        }
        Ok(result)
    }

    /// API `effects.get(id)` means current; saved instances must use `instance`.
    pub fn current(&self, id: &str) -> Option<&Definition> {
        self.revisions.get(&(id.to_owned(), *self.current.get(id)?))
    }
    pub fn get(&self, id: &str, version: u64) -> Option<&Definition> {
        self.revisions.get(&(id.to_owned(), version))
    }
    pub fn instance(&self, id: &str, saved: Option<&Value>) -> Result<&Definition, CatalogError> {
        let version = effect_version(saved)?;
        self.get(id, version)
            .ok_or_else(|| CatalogError(format!("missing effect {id} version {version}")))
    }
    pub fn definitions(&self) -> impl Iterator<Item = (&str, u64, &Definition)> {
        self.revisions
            .iter()
            .map(|((id, version), def)| (id.as_str(), *version, def))
    }
    pub fn current_definitions(&self) -> impl Iterator<Item = &Definition> {
        self.current
            .iter()
            .map(|(id, v)| &self.revisions[&(id.clone(), *v)])
    }
}

#[derive(Debug, Deserialize)]
pub struct MainEffects {
    pub commit: String,
    pub blobs: BTreeMap<String, String>,
    pub effects: Vec<Definition>,
    /// Fully assembled shaders, generated by the pinned kernel's toLegacyFx.
    pub shaders: Vec<EffectShader>,
}
#[derive(Debug, Deserialize)]
pub struct EffectShader {
    pub id: String,
    pub version: u64,
    pub source: String,
}
pub fn main_effects() -> &'static MainEffects {
    static MAIN: OnceLock<MainEffects> = OnceLock::new();
    MAIN.get_or_init(|| {
        let mut main: MainEffects = serde_json::from_str(include_str!("main-effects.json"))
            .expect("pinned main effect definitions");
        add_lut_revision(&mut main);
        main
    })
}

/// The Apply Color LUT program, including its stated input and output spaces.
const LUT_SOURCE: &str = include_str!("apply-color-lut.glsl");
const LUT_SPACES: [&str; 9] = [
    "Working",
    "Rec.709 Linear",
    "Apple Log (Rec.2020)",
    "S-Log3 (S-Gamut3.Cine)",
    "S-Log3 (S-Gamut3)",
    "V-Log (V-Gamut)",
    "LogC3 (AWG3)",
    "LogC4 (AWG4)",
    "PQ (Rec.2020)",
];

/// Revision 2 of Apply Color LUT, added here and not in main: tetrahedral
/// cubes, 1D shapers and stated colour spaces. Revision 1 stays exact for files
/// that saved it; new effects take revision 2.
fn add_lut_revision(main: &mut MainEffects) {
    let revision = main
        .effects
        .iter_mut()
        .find(|e| e.id == "applyColorLut")
        .expect("pinned Apply Color LUT");
    let mut history = revision.clone();
    history.metadata.insert("version".into(), Value::from(1));
    let space = |k: &str, label: &str| -> Parameter {
        serde_json::from_value(serde_json::json!({
            "k": k,
            "label": label,
            "def": 0,
            "min": 0,
            "max": LUT_SPACES.len() - 1,
            "step": 1,
            "advanced": false,
            "options": LUT_SPACES
                .iter()
                .enumerate()
                .map(|(v, label)| serde_json::json!({ "v": v, "label": label }))
                .collect::<Vec<_>>(),
        }))
        .expect("a LUT space parameter")
    };
    revision.params.push(space("inputSpace", "Input Space"));
    revision.params.push(space("outputSpace", "Output Space"));
    revision.metadata.insert("version".into(), Value::from(2));
    revision.metadata.insert(
        "legacy".into(),
        Value::Array(vec![
            serde_json::to_value(history).expect("a serializable definition"),
        ]),
    );
    let (_, body) = LUT_SOURCE
        .split_once("void main(){\n")
        .expect("the LUT program's main");
    revision.frag = body.trim_end().trim_end_matches('}').trim_end().to_owned();
    main.shaders.push(EffectShader {
        id: revision.id.clone(),
        version: 2,
        source: LUT_SOURCE.to_owned(),
    });
}
pub fn main_catalog() -> &'static EffectCatalog {
    static CATALOG: OnceLock<EffectCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        EffectCatalog::new(main_effects().effects.clone()).expect("valid pinned effect revisions")
    })
}

/// Creation/Reset uses source dimensions only when declared and valid. Bounds
/// do not clamp an authored default (`effectParameterDefault` in main).
pub fn parameter_default(param: &Parameter, width: f64, height: f64) -> Value {
    if param.kind.as_deref().is_none_or(|kind| kind == "number")
        && let Some(context) = param.presentation.get("defaultFrom")
        && let Some(factor) = context.get("factor").and_then(Value::as_f64)
    {
        let dimension = if context.get("dimension").and_then(Value::as_str) == Some("width") {
            width
        } else {
            height
        };
        let value = dimension * factor;
        if dimension.is_finite() && dimension > 0. && value.is_finite() {
            return Value::from(value);
        }
    }
    param.def.clone()
}

/// Project provenance is needed where main reused an older version number for
/// different rendering semantics. Never guess from the currently active UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceGeneration {
    V120,
    Main,
}

pub fn v120_catalog() -> &'static EffectCatalog {
    static CATALOG: OnceLock<EffectCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        EffectCatalog::new(crate::builtins().effects.clone()).expect("valid tagged definitions")
    })
}

/// This bridge deliberately diagnoses main's changed historical Gradient v1
/// when provenance is unavailable. Both originals stay available; neither set
/// is rewritten to make an equality assertion pass.
pub fn saved_definition(
    id: &str,
    saved: Option<&Value>,
    provenance: Option<ReferenceGeneration>,
) -> Result<&'static Definition, CatalogError> {
    let version = effect_version(saved)?;
    match provenance {
        Some(ReferenceGeneration::V120) => v120_catalog().instance(id, saved),
        Some(ReferenceGeneration::Main) => main_catalog().instance(id, saved),
        None => {
            let current = main_catalog().instance(id, saved)?;
            if let Some(old) = v120_catalog().get(id, version)
                && (old.frag != current.frag || old.defaults() != current.defaults())
            {
                return Err(CatalogError(format!(
                    "ambiguous effect {id} version {version}: source generation required"
                )));
            }
            Ok(current)
        }
    }
}

/// Owner policy: missing/unknown provenance preserves the tagged v1.2.0
/// generation. No field of the saved effect or file is rewritten.
pub fn saving_generation(version: Option<&str>) -> ReferenceGeneration {
    let Some(version) = version else {
        return ReferenceGeneration::V120;
    };
    let version = version.strip_prefix('v').unwrap_or(version);
    let core = version.split(['-', '+']).next().unwrap_or("");
    let values = core
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>();
    match values {
        Ok(parts) if parts.len() == 3 && (parts[0], parts[1], parts[2]) > (1, 2, 0) => {
            ReferenceGeneration::Main
        }
        _ => ReferenceGeneration::V120,
    }
}

pub fn definition_for_file(
    id: &str,
    saved: Option<&Value>,
    app_version: Option<&str>,
) -> Result<&'static Definition, CatalogError> {
    let generation = saving_generation(app_version);
    // Versions and effects added after the tag cannot come from the tag, even
    // with unknown provenance. Version one retains its exact tagged definition.
    let version = effect_version(saved)?;
    if version > 1 || v120_catalog().get(id, version).is_none() {
        return main_catalog().instance(id, saved);
    }
    saved_definition(id, saved, Some(generation))
}
