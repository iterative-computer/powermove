//! The frozen extension manifest contract (`@powermove/registry/manifest`).
use serde_json::{Map, Value, json};

/// The frozen parser's ceiling is 3; 4 is the native surface API.
pub const API_VERSION: u64 = 4;
pub const SOURCE_FILES: usize = 400;
pub const SOURCE_BYTES: usize = 8 * 1024 * 1024;
pub const BUNDLE_BYTES: usize = 16 * 1024 * 1024;
pub const ERROR_CHARS: usize = 4_000;
const NAME_CHARS: usize = 80;
const DESCRIPTION_CHARS: usize = 400;
const LIST_ITEMS: usize = 32;
const FORKED_FROM_CHARS: usize = 160;
const LINKS: usize = 5;
const LINK_CHARS: usize = 200;
const KINDS: [&str; 13] = [
    "panels",
    "inspector",
    "media",
    "commands",
    "keybindings",
    "effects",
    "transitions",
    "layers",
    "themes",
    "palette",
    "menus",
    "status",
    "hooks",
];
const PERMISSIONS: [&str; 5] = ["network", "clipboard", "assets", "project:write", "full-access"];

pub fn extension_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (2..=64).contains(&bytes.len())
        && matches!(bytes[0], b'a'..=b'z' | b'0'..=b'9')
        && bytes[1..]
            .iter()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}
fn extension_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| (1..=6).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
}
fn var_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    (2..=64).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes[1..]
            .iter()
            .all(|b| matches!(b, b'A'..=b'Z' | b'0'..=b'9' | b'_'))
}
pub fn safe_entry(entry: &str) -> bool {
    !entry.contains('\\')
        && !entry.starts_with('/')
        && !entry.contains("..")
        && !entry.contains('\0')
        && [".ts", ".tsx", ".js", ".mjs"]
            .iter()
            .any(|s| entry.ends_with(s))
}
fn link_origin(value: &Value) -> bool {
    let Some(text) = value.as_str().filter(|t| t.len() <= LINK_CHARS) else {
        return false;
    };
    let Ok(url) = url::Url::parse(text) else {
        return false;
    };
    if url.scheme() != "https" || url.origin().ascii_serialization() != text {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    let labels: Vec<_> = host.split('.').collect();
    let label = |l: &&str| {
        let b = l.as_bytes();
        (1..=63).contains(&b.len())
            && b[0].is_ascii_alphanumeric()
            && b[b.len() - 1].is_ascii_alphanumeric()
            && b.iter().all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'-'))
    };
    let last = labels[labels.len() - 1];
    labels.len() > 1
        && labels.iter().all(label)
        && !last.bytes().all(|b| b.is_ascii_digit())
        && !matches!(last, "localhost" | "local")
}
fn utf16(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The normalized manifest, or the frozen parser's error text.
pub fn parse(raw: &Value) -> Result<Value, String> {
    let m = raw.as_object().ok_or("manifest must be an object")?;
    let text = |key: &str, max: usize, required: bool| -> Result<Option<String>, ()> {
        match m.get(key) {
            None | Some(Value::Null) if !required => Ok(None),
            Some(Value::String(v)) if !v.is_empty() && utf16(v) <= max => Ok(Some(v.clone())),
            _ => Err(()),
        }
    };
    let list = |key: &str| -> Result<Option<Vec<String>>, ()> {
        match m.get(key) {
            None => Ok(None),
            Some(Value::Array(v)) if v.len() <= LIST_ITEMS => v
                .iter()
                .map(|x| {
                    x.as_str()
                        .filter(|x| extension_id(x))
                        .map(str::to_owned)
                        .ok_or(())
                })
                .collect::<Result<_, _>>()
                .map(Some),
            _ => Err(()),
        }
    };
    let id = text("id", 64, true)
        .ok()
        .flatten()
        .filter(|id| extension_id(id))
        .ok_or("invalid \"id\" (a-z, 0-9, -; 2–64 chars)")?;
    let name = text("name", NAME_CHARS, true)
        .ok()
        .flatten()
        .ok_or("invalid \"name\"")?;
    let version = text("version", 20, true)
        .ok()
        .flatten()
        .filter(|v| extension_version(v))
        .ok_or("invalid \"version\" (x.y.z)")?;
    let api = m
        .get("apiVersion")
        .and_then(Value::as_f64)
        .filter(|v| v.fract() == 0. && *v >= 1.)
        .ok_or("invalid \"apiVersion\"")? as u64;
    if api > API_VERSION {
        return Err(format!(
            "apiVersion {api} is newer than this app ({API_VERSION})"
        ));
    }
    let description =
        text("description", DESCRIPTION_CHARS, false).map_err(|_| "invalid \"description\"")?;
    let entry = text("entry", 200, false).map_err(|_| "invalid \"entry\"")?;
    if entry.as_deref().is_some_and(|e| !safe_entry(e)) {
        return Err("invalid \"entry\" path".into());
    }
    if let Some(contributes) = m.get("contributes") {
        let valid = contributes.as_array().is_some_and(|v| {
            v.iter()
                .all(|c| c.as_str().is_some_and(|c| KINDS.contains(&c)))
        });
        if !valid {
            return Err("invalid \"contributes\"".into());
        }
    }
    let replaces = list("replaces").map_err(|_| "invalid \"replaces\"")?;
    let depends = list("dependsOn").map_err(|_| "invalid \"dependsOn\"")?;
    if replaces.iter().chain(&depends).flatten().any(|x| *x == id) {
        return Err("extension cannot reference itself".into());
    }
    let features = list("features").map_err(|_| "invalid \"features\"")?;
    let integrates = list("integrates").map_err(|_| "invalid \"integrates\"")?;
    let forked =
        text("forkedFrom", FORKED_FROM_CHARS, false).map_err(|_| "invalid \"forkedFrom\"")?;
    if let Some(vars) = m.get("vars") {
        if api < 2 {
            return Err("\"vars\" requires apiVersion 2".into());
        }
        let valid = vars.as_array().is_some_and(|v| {
            v.len() <= LIST_ITEMS
                && v.iter().all(|v| {
                    v.as_object().is_some_and(|v| {
                        v.get("key").and_then(Value::as_str).is_some_and(var_key)
                            && v.get("label")
                                .and_then(Value::as_str)
                                .is_some_and(|l| (1..=80).contains(&utf16(l)))
                            && v.get("hint")
                                .is_none_or(|h| h.as_str().is_some_and(|h| utf16(h) <= 200))
                            && v.get("secret").is_none_or(Value::is_boolean)
                            && v.get("required").is_none_or(Value::is_boolean)
                    })
                })
        });
        if !valid {
            return Err("invalid \"vars\"".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for var in vars.as_array().into_iter().flatten() {
            let key = var["key"].as_str().unwrap_or_default();
            if !seen.insert(key) {
                return Err(format!("duplicate var key \"{key}\""));
            }
        }
    }
    if let Some(permissions) = m.get("permissions") {
        if api < 3 {
            return Err("\"permissions\" requires apiVersion 3".into());
        }
        let items = permissions.as_array();
        let valid = items.is_some_and(|v| {
            let unique: std::collections::BTreeSet<_> = v.iter().filter_map(Value::as_str).collect();
            v.len() <= 8
                && unique.len() == v.len()
                && unique.iter().all(|p| PERMISSIONS.contains(p))
        });
        if !valid {
            return Err("invalid \"permissions\"".into());
        }
    }
    if let Some(links) = m.get("links") {
        if api < 3 {
            return Err("\"links\" requires apiVersion 3".into());
        }
        let valid = links.as_array().is_some_and(|v| {
            let unique: std::collections::BTreeSet<_> = v.iter().filter_map(Value::as_str).collect();
            v.len() <= LINKS && unique.len() == v.len() && v.iter().all(link_origin)
        });
        if !valid {
            return Err("invalid \"links\" (up to 5 origins like \"https://example.com\")".into());
        }
    }
    if let Some(author) = m.get("author")
        && !matches!(author.as_str(), Some("powermove" | "user" | "agent"))
    {
        return Err("invalid \"author\"".into());
    }
    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    out.insert("name".into(), json!(name));
    out.insert("version".into(), json!(version));
    out.insert("apiVersion".into(), json!(api));
    if let Some(v) = description {
        out.insert("description".into(), json!(v));
    }
    if let Some(v) = entry {
        out.insert("entry".into(), json!(v));
    }
    for key in ["contributes", "vars", "permissions", "links", "author"] {
        if let Some(v) = m.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    if let Some(v) = replaces {
        out.insert("replaces".into(), json!(v));
    }
    if let Some(v) = depends {
        out.insert("dependsOn".into(), json!(v));
    }
    if let Some(v) = forked {
        out.insert("forkedFrom".into(), json!(v));
    }
    for (key, v) in [("features", features), ("integrates", integrates)] {
        if let Some(v) = v {
            let mut unique: Vec<String> = vec![];
            for item in v {
                if !unique.contains(&item) {
                    unique.push(item);
                }
            }
            out.insert(key.into(), json!(unique));
        }
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> Value {
        json!({"id":"synthetic","name":"Synthetic","version":"1.0.0","apiVersion":1})
    }
    #[test]
    fn defaults_and_refusals_match_the_frozen_parser() {
        assert_eq!(parse(&base()).unwrap(), base());
        for (key, value, error) in [
            ("id", json!("Bad"), "invalid \"id\" (a-z, 0-9, -; 2–64 chars)"),
            ("version", json!("1.0"), "invalid \"version\" (x.y.z)"),
            ("apiVersion", json!(9), "apiVersion 9 is newer than this app (4)"),
            ("entry", json!("../x.ts"), "invalid \"entry\" path"),
            ("contributes", json!(["nope"]), "invalid \"contributes\""),
            ("replaces", json!(["synthetic"]), "extension cannot reference itself"),
            ("vars", json!([]), "\"vars\" requires apiVersion 2"),
            ("permissions", json!([]), "\"permissions\" requires apiVersion 3"),
        ] {
            let mut raw = base();
            raw[key] = value;
            assert_eq!(parse(&raw).unwrap_err(), error);
        }
    }
    #[test]
    fn link_origins_are_named_https_origins_only() {
        for (origin, ok) in [
            ("https://example.com", true),
            ("https://example.com/", false),
            ("http://example.com", false),
            ("https://localhost", false),
            ("https://1.2.3.4", false),
        ] {
            assert_eq!(link_origin(&json!(origin)), ok, "{origin}");
        }
    }
}
