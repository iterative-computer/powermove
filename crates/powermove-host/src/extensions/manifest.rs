//! The extension manifest and record contract (`packages/registry/src/manifest.ts`).
//!
//! Lengths are counted in UTF-16 code units, as JavaScript's `string.length` does.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use crate::ids::is_extension_id;

pub const EXTENSION_API_VERSION: u64 = 3;
const EXTENSION_API_VERSION_VARS: u64 = 2;
const EXTENSION_PERMISSIONS: [&str; 5] = ["network", "clipboard", "assets", "project:write", "full-access"];
const CONTRIBUTION_KINDS: [&str; 13] =
    ["panels", "inspector", "media", "commands", "keybindings", "effects", "transitions", "layers", "themes", "palette", "menus", "status", "hooks"];

/// `MANIFEST_LIMITS`.
pub mod limits {
    pub const NAME_CHARS: usize = 80;
    pub const DESCRIPTION_CHARS: usize = 400;
    pub const LIST_ITEMS: usize = 32;
    pub const FORKED_FROM_CHARS: usize = 160;
    pub const LINKS: usize = 5;
    pub const LINK_CHARS: usize = 200;
    pub const SOURCE_FILES: usize = 400;
    pub const SOURCE_BYTES: u64 = 8 * 1024 * 1024;
    pub const BUNDLE_BYTES: usize = 16 * 1024 * 1024;
    pub const ERROR_CHARS: usize = 4_000;
}

/// A validated manifest. Field order matches `parseManifest`'s output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
    /// Kept as written: `parseManifest` passes var objects through unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vars: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrates: Option<Vec<String>>,
}

impl Manifest {
    pub fn entry(&self) -> &str {
        self.entry.as_deref().unwrap_or("index.ts")
    }
}

fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `EXTENSION_VERSION = /^\d{1,6}\.\d{1,6}\.\d{1,6}$/`
pub fn is_extension_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3 && parts.iter().all(|part| (1..=6).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// `EXTENSION_VAR_KEY = /^[A-Z][A-Z0-9_]{1,63}$/`
fn is_var_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    (2..=64).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes.iter().all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
}

/// A relative `.ts`, `.js` or `.mjs` path that cannot climb out of its folder.
pub fn is_safe_entry(entry: &str) -> bool {
    if entry.contains('\\') || entry.starts_with('/') || entry.contains("..") || entry.contains('\0') {
        return false;
    }
    [".ts", ".js", ".mjs"].iter().any(|suffix| entry.ends_with(suffix))
}

/// An https origin exactly as the URL standard serializes it (see `isLinkOrigin`).
pub fn is_link_origin(value: &str) -> bool {
    if js_len(value) > limits::LINK_CHARS {
        return false;
    }
    let Some(authority) = value.strip_prefix("https://") else { return false };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    if let Some(port) = port {
        // A serialized origin drops default and empty ports and leading zeros.
        let valid = !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) && !port.starts_with('0') && port != "443";
        if !valid || port.parse::<u32>().map_or(true, |number| number > 65_535) {
            return false;
        }
    }
    let labels: Vec<&str> = host.split('.').collect();
    let label_ok = |label: &&str| {
        let bytes = label.as_bytes();
        let edge = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
        (1..=63).contains(&bytes.len())
            && edge(bytes[0])
            && edge(bytes[bytes.len() - 1])
            && bytes.iter().all(|byte| edge(*byte) || *byte == b'-')
    };
    let last = labels[labels.len() - 1];
    labels.len() > 1 && labels.iter().all(label_ok) && !last.bytes().all(|byte| byte.is_ascii_digit()) && last != "localhost" && last != "local"
}

/// Validate an unknown value as a manifest (`parseManifest`).
pub fn parse_manifest(raw: &Value) -> Result<Manifest, String> {
    let Value::Object(map) = raw else { return Err("manifest must be an object".into()) };
    let fail = |message: &str| Err(message.to_owned());

    let id = match text(map, "id", 64, true) {
        Ok(Some(id)) if is_extension_id(&id) => id,
        _ => return fail("invalid \"id\" (a-z, 0-9, -; 2–64 chars)"),
    };
    let Ok(Some(name)) = text(map, "name", limits::NAME_CHARS, true) else { return fail("invalid \"name\"") };
    let version = match text(map, "version", 20, true) {
        Ok(Some(version)) if is_extension_version(&version) => version,
        _ => return fail("invalid \"version\" (x.y.z)"),
    };
    let api_version = match map.get("apiVersion").and_then(Value::as_f64) {
        Some(number) if number.fract() == 0.0 && number >= 1.0 && number.is_finite() => number,
        _ => return fail("invalid \"apiVersion\""),
    };
    if api_version > EXTENSION_API_VERSION as f64 {
        return Err(format!("apiVersion {api_version} is newer than this app ({EXTENSION_API_VERSION})"));
    }
    let api_version = api_version as u64;
    let Ok(description) = text(map, "description", limits::DESCRIPTION_CHARS, false) else { return fail("invalid \"description\"") };
    let Ok(entry) = text(map, "entry", 200, false) else { return fail("invalid \"entry\"") };
    if entry.as_deref().is_some_and(|entry| !is_safe_entry(entry)) {
        return fail("invalid \"entry\" path");
    }
    let contributes = match map.get("contributes") {
        None => None,
        Some(Value::Array(items)) if items.iter().all(|item| item.as_str().is_some_and(|kind| CONTRIBUTION_KINDS.contains(&kind))) => {
            Some(items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect::<Vec<_>>())
        }
        Some(_) => return fail("invalid \"contributes\""),
    };
    let Ok(replaces) = id_list(map, "replaces") else { return fail("invalid \"replaces\"") };
    let Ok(depends_on) = id_list(map, "dependsOn") else { return fail("invalid \"dependsOn\"") };
    let references_self = |list: &Option<Vec<String>>| list.as_ref().is_some_and(|items| items.contains(&id));
    if references_self(&replaces) || references_self(&depends_on) {
        return fail("extension cannot reference itself");
    }
    let Ok(features) = id_list(map, "features") else { return fail("invalid \"features\"") };
    let Ok(integrates) = id_list(map, "integrates") else { return fail("invalid \"integrates\"") };
    let Ok(forked_from) = text(map, "forkedFrom", limits::FORKED_FROM_CHARS, false) else { return fail("invalid \"forkedFrom\"") };

    let vars = match map.get("vars") {
        None => None,
        Some(vars) => {
            if api_version < EXTENSION_API_VERSION_VARS {
                return fail("\"vars\" requires apiVersion 2");
            }
            let Value::Array(items) = vars else { return fail("invalid \"vars\"") };
            if items.len() > limits::LIST_ITEMS || !items.iter().all(valid_var) {
                return fail("invalid \"vars\"");
            }
            let mut keys = std::collections::HashSet::new();
            for item in items {
                let key = item["key"].as_str().unwrap_or_default();
                if !keys.insert(key) {
                    return Err(format!("duplicate var key \"{key}\""));
                }
            }
            Some(items.clone())
        }
    };
    let permissions = match map.get("permissions") {
        None => None,
        Some(permissions) => {
            if api_version < 3 {
                return fail("\"permissions\" requires apiVersion 3");
            }
            let list = string_array(permissions).filter(|items| {
                items.len() <= 8 && items.iter().all(|item| EXTENSION_PERMISSIONS.contains(&item.as_str())) && unique(items)
            });
            match list {
                Some(items) => Some(items),
                None => return fail("invalid \"permissions\""),
            }
        }
    };
    let links = match map.get("links") {
        None => None,
        Some(links) => {
            if api_version < 3 {
                return fail("\"links\" requires apiVersion 3");
            }
            let list = string_array(links).filter(|items| items.len() <= limits::LINKS && items.iter().all(|item| is_link_origin(item)) && unique(items));
            match list {
                Some(items) => Some(items),
                None => return fail("invalid \"links\" (up to 5 origins like \"https://example.com\")"),
            }
        }
    };
    let author = match map.get("author") {
        None => None,
        Some(Value::String(author)) if ["powermove", "user", "agent"].contains(&author.as_str()) => Some(author.clone()),
        Some(_) => return fail("invalid \"author\""),
    };

    Ok(Manifest {
        id,
        name,
        version,
        api_version,
        description: description.filter(|value| !value.is_empty()),
        entry,
        contributes,
        replaces,
        depends_on,
        forked_from,
        vars,
        permissions,
        links,
        author,
        features: features.map(dedupe),
        integrates: integrates.map(dedupe),
    })
}

/// `str(k, max, required)`: `Ok(None)` when absent and optional, `Err` when invalid.
fn text(map: &Map<String, Value>, key: &str, max: usize, required: bool) -> Result<Option<String>, ()> {
    match map.get(key) {
        None | Some(Value::Null) => if required { Err(()) } else { Ok(None) },
        Some(Value::String(value)) if !value.is_empty() && js_len(value) <= max => Ok(Some(value.clone())),
        Some(_) => Err(()),
    }
}

/// `list(k)`: an array of up to 32 extension ids.
fn id_list(map: &Map<String, Value>, key: &str) -> Result<Option<Vec<String>>, ()> {
    match map.get(key) {
        None => Ok(None),
        Some(value) => match string_array(value) {
            Some(items) if items.len() <= limits::LIST_ITEMS && items.iter().all(|item| is_extension_id(item)) => Ok(Some(items)),
            _ => Err(()),
        },
    }
}

fn string_array(value: &Value) -> Option<Vec<String>> {
    value.as_array()?.iter().map(|item| item.as_str().map(str::to_owned)).collect()
}

fn unique(items: &[String]) -> bool {
    let set: std::collections::HashSet<&String> = items.iter().collect();
    set.len() == items.len()
}

fn dedupe(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items.into_iter().filter(|item| seen.insert(item.clone())).collect()
}

fn valid_var(value: &Value) -> bool {
    let Value::Object(var) = value else { return false };
    let optional_bool = |key: &str| var.get(key).is_none_or(Value::is_boolean);
    var.get("key").and_then(Value::as_str).is_some_and(is_var_key)
        && var.get("label").and_then(Value::as_str).is_some_and(|label| (1..=80).contains(&js_len(label)))
        && var.get("hint").is_none_or(|hint| hint.as_str().is_some_and(|hint| js_len(hint) <= 200))
        && optional_bool("secret")
        && optional_bool("required")
}

/// `<id>@<version>` of a built-in fork, as `staleForkUpdate` reads it.
pub fn builtin_fork_origin(forked_from: &str) -> Option<(&str, &str)> {
    let (id, version) = forked_from.split_once('@')?;
    (!id.is_empty() && !version.contains('@') && is_extension_id(id) && is_extension_version(version)).then_some((id, version))
}

pub fn truncate_error(error: &str) -> String {
    // JavaScript slices UTF-16 units; never split a character here.
    let mut units = 0;
    let mut end = error.len();
    for (index, character) in error.char_indices() {
        units += character.len_utf16();
        if units > limits::ERROR_CHARS {
            end = index;
            break;
        }
    }
    error[..end].to_owned()
}

/* ── runtime records (host → renderer) ───────────────────── */

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Health {
    Ok,
    Disabled,
    BuildError { error: String },
    ManifestError { error: String },
    ActivationError { error: String },
    RuntimeError { error: String },
    NeedsTrust,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Builtin,
    User,
    Project,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ForkUpdate {
    #[serde(rename = "forkedFrom")]
    pub forked_from: String,
    pub base: String,
    pub current: String,
}

/// `ExtensionRecord`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub trust: &'static str,
    pub id: String,
    pub scope: Scope,
    pub manifest: Option<Manifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<ForkUpdate>,
    pub dir: String,
    pub enabled: bool,
    pub bundle_url: Option<String>,
    pub bundle_hash: Option<String>,
    pub health: Health,
    pub updated_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> Value {
        json!({ "id": "my-mod", "name": "My mod", "version": "1.2.3", "apiVersion": 3 })
    }

    fn with(key: &str, value: Value) -> Value {
        let mut manifest = base();
        manifest[key] = value;
        manifest
    }

    #[test]
    fn accepts_a_minimal_manifest_and_normalizes_lists() {
        let manifest = parse_manifest(&with("features", json!(["ab", "ab", "cd"]))).unwrap();
        assert_eq!(manifest.entry(), "index.ts");
        assert_eq!(manifest.features, Some(vec!["ab".into(), "cd".into()]));
        assert_eq!(serde_json::to_value(&manifest).unwrap(), json!({ "id": "my-mod", "name": "My mod", "version": "1.2.3", "apiVersion": 3, "features": ["ab", "cd"] }));
        assert_eq!(parse_manifest(&with("apiVersion", json!(2.0))).unwrap().api_version, 2);
    }

    #[test]
    fn rejects_invalid_fields_with_the_same_messages() {
        let cases = [
            (json!([]), "manifest must be an object"),
            (with("id", json!("Bad")), "invalid \"id\" (a-z, 0-9, -; 2–64 chars)"),
            (with("name", json!("")), "invalid \"name\""),
            (with("name", json!("x".repeat(81))), "invalid \"name\""),
            (with("version", json!("1.2")), "invalid \"version\" (x.y.z)"),
            (with("apiVersion", json!(1.5)), "invalid \"apiVersion\""),
            (with("apiVersion", json!(4)), "apiVersion 4 is newer than this app (3)"),
            (with("entry", json!("../x.ts")), "invalid \"entry\" path"),
            (with("entry", json!("x.svelte")), "invalid \"entry\" path"),
            (with("entry", json!("/abs.ts")), "invalid \"entry\" path"),
            (with("contributes", json!(["nope"])), "invalid \"contributes\""),
            (with("replaces", json!(["my-mod"])), "extension cannot reference itself"),
            (with("dependsOn", json!(["UP"])), "invalid \"dependsOn\""),
            (with("forkedFrom", json!(7)), "invalid \"forkedFrom\""),
            (with("vars", json!([{ "key": "API_KEY", "label": "Key" }, { "key": "API_KEY", "label": "Again" }])), "duplicate var key \"API_KEY\""),
            (with("vars", json!([{ "key": "lower", "label": "Key" }])), "invalid \"vars\""),
            (with("permissions", json!(["network", "network"])), "invalid \"permissions\""),
            (with("permissions", json!(["root"])), "invalid \"permissions\""),
            (with("links", json!(["http://example.com"])), "invalid \"links\" (up to 5 origins like \"https://example.com\")"),
            (with("author", json!("someone")), "invalid \"author\""),
        ];
        for (raw, expected) in cases {
            assert_eq!(parse_manifest(&raw).unwrap_err(), expected, "{raw}");
        }
        let mut old = with("vars", json!([{ "key": "API_KEY", "label": "Key" }]));
        old["apiVersion"] = json!(1);
        assert_eq!(parse_manifest(&old).unwrap_err(), "\"vars\" requires apiVersion 2");
    }

    #[test]
    fn checks_link_origins_like_the_url_standard() {
        for good in ["https://example.com", "https://xn--bcher-kva.example", "https://api.example.com:8443"] {
            assert!(is_link_origin(good), "{good}");
        }
        for bad in ["https://example.com/", "https://Example.com", "https://localhost", "https://printer.local", "https://1.2.3.4", "https://example.com:443", "https://example.com:", "https://com", "https://-a.com"] {
            assert!(!is_link_origin(bad), "{bad}");
        }
    }

    #[test]
    fn serializes_records_like_the_renderer_expects() {
        let record = Record {
            trust: "local",
            id: "my-mod".into(),
            scope: Scope::User,
            manifest: None,
            update: None,
            dir: "C:\\mods\\my-mod".into(),
            enabled: true,
            bundle_url: None,
            bundle_hash: None,
            health: Health::BuildError { error: "boom".into() },
            updated_at: 5,
        };
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            json!({ "trust": "local", "id": "my-mod", "scope": "user", "manifest": null, "dir": "C:\\mods\\my-mod", "enabled": true, "bundleUrl": null, "bundleHash": null, "health": { "state": "build-error", "error": "boom" }, "updatedAt": 5 })
        );
        assert_eq!(serde_json::to_value(Health::NeedsTrust).unwrap(), json!({ "state": "needs-trust" }));
    }
}
