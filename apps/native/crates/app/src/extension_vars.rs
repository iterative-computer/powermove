//! The values a person entered for an extension's declared `vars` in the
//! shipped app, as first launch migrated them, for `api.vars`. As the shipped
//! app's main process: `extensions-provenance.json` names each extension
//! folder's env key, `env/<key>.env` holds its values (plain, or sealed with
//! `enc:`), and a sealed value is the Keychain item first launch wrote for it
//! (registered in the profile's secret locators). Read per activation, off the
//! main thread; a value this Mac cannot open is absent, as the shipped app left it.
use std::{collections::BTreeMap, path::Path};

/// Reads Keychain items: (service, account) to bytes.
pub(crate) type Items = dyn Fn(&str, &str) -> Option<Vec<u8>> + Send + Sync;

/// The Keychain service this build keeps migrated extension values under:
/// its own bundle's. A profile copied from another build (a lane or debug
/// run handed a real profile) carries that build's locators; they are never
/// opened here, so such a run reads no production item and nobody is asked
/// by macOS to let it.
pub(crate) fn own_service() -> String {
    service_of(&crate::platform::bundle_id().unwrap_or_default())
}
fn service_of(bundle: &str) -> String {
    format!("{bundle} migration secrets")
}

/// The declared `keys` of extension `id` that have a non-empty value.
/// `service`: the only Keychain service a sealed value may be read from
/// ([`own_service`]).
pub(crate) fn values(
    profile: &Path,
    id: &str,
    keys: &[String],
    service: &str,
    items: &Items,
) -> BTreeMap<String, String> {
    let Some(file) = env_key(profile, id).and_then(|key| env_file(&key)) else {
        return BTreeMap::new();
    };
    let Ok(text) = std::fs::read_to_string(profile.join("env").join(&file)) else {
        return BTreeMap::new();
    };
    let mut values = BTreeMap::new();
    for (key, stored) in lines(&text) {
        if !keys.contains(&key) {
            continue;
        }
        let value = match stored {
            Stored::Plain(value) => Some(value),
            Stored::Sealed(payload) => sealed(profile, &file, &key, &payload, service, items),
        };
        // A later line for the same key replaces an earlier one.
        match value.filter(|value| !value.is_empty()) {
            Some(value) => values.insert(key, value),
            None => values.remove(&key),
        };
    }
    values
}

/// The env key the provenance file gives extension folder `id`.
fn env_key(profile: &Path, id: &str) -> Option<String> {
    let text = std::fs::read_to_string(profile.join("extensions-provenance.json")).ok()?;
    let file: serde_json::Value = serde_json::from_str(&text).ok()?;
    let key = file.get(id)?.get("envKey")?.as_str()?;
    (!key.is_empty()).then(|| key.to_owned())
}

/// `local:<uuid>` is `local_<uuid>.env`; a key that could leave `env/` is refused.
fn env_file(key: &str) -> Option<String> {
    if key.len() > 200 {
        return None;
    }
    let name = key.replace([':', '/'], "_");
    let mut chars = name.chars();
    let first = chars.next()?;
    let valid = (first.is_ascii_alphanumeric() || matches!(first, '_' | '-'))
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    valid.then(|| format!("{name}.env"))
}

enum Stored {
    Plain(String),
    /// Base64 of the safeStorage ciphertext.
    Sealed(String),
}

/// `KEY=value` lines; comments, blanks and malformed keys are not values.
fn lines(text: &str) -> Vec<(String, Stored)> {
    text.lines()
        .filter_map(|raw| {
            let trimmed = raw.trim();
            let equals = raw.find('=')?;
            if trimmed.is_empty() || trimmed.starts_with('#') || equals == 0 {
                return None;
            }
            let key = raw[..equals].trim();
            let mut chars = key.chars();
            let first = chars.next()?;
            let valid = (first.is_ascii_alphabetic() || first == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
            valid.then(|| (key.to_owned(), decode(&raw[equals + 1..])))
        })
        .collect()
}

/// The right-hand side as the shipped app's `decodeStored` reads it.
fn decode(stored: &str) -> Stored {
    let value = stored.trim();
    if value.starts_with('"') {
        if let Ok(serde_json::Value::String(parsed)) = serde_json::from_str(value) {
            return Stored::Plain(parsed);
        }
        return Stored::Plain(value.to_owned());
    }
    if let Some(payload) = value.strip_prefix("enc:") {
        return Stored::Sealed(payload.to_owned());
    }
    // An unquoted value ends at an inline ` # comment`, as in dotenv.
    let comment = value
        .char_indices()
        .zip(value.chars().skip(1))
        .find(|((_, c), next)| c.is_whitespace() && *next == '#')
        .map(|((at, _), _)| at);
    Stored::Plain(match comment {
        Some(at) => value[..at].trim_end().to_owned(),
        None => value.to_owned(),
    })
}

/// The plaintext first launch stored for this sealed value: its locator is
/// `env/<file>/<KEY>` at the ciphertext's version, in this build's own
/// `service`. A locator that names another service is another build's item:
/// refused before the Keychain is asked for anything.
fn sealed(
    profile: &Path,
    file: &str,
    key: &str,
    payload: &str,
    service: &str,
    items: &Items,
) -> Option<String> {
    use base64::Engine;
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    let version = blake3::hash(&ciphertext).to_hex().to_string();
    let locators =
        pm_kernel::profile::registered_secrets(profile, &format!("env/{file}/{key}")).ok()?;
    let locator = locators
        .into_iter()
        .find(|l| l.version == version && l.service == service)?;
    String::from_utf8(items(&locator.service, &locator.account)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn plain_and_sealed_values_are_read_as_the_shipped_app_wrote_them() {
        let profile = tempfile::tempdir().unwrap();
        let root = profile.path();
        fs::write(
            root.join("extensions-provenance.json"),
            r#"{"needs-key":{"localId":"needs-key","envKey":"local:1234"},"other":{"envKey":"local:5678"}}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("env")).unwrap();
        let ciphertext = b"v10sealed-bytes";
        use base64::Engine;
        let sealed = base64::engine::general_purpose::STANDARD.encode(ciphertext);
        fs::write(
            root.join("env/local_1234.env"),
            format!(
                "# values\nMODEL=gpt-image-1 # default\nQUOTED=\" spaced # kept \"\nAPI_KEY=enc:{sealed}\nEMPTY=\nUNDECLARED=x\nMODEL=m-2\n"
            ),
        )
        .unwrap();
        fs::write(root.join("env/local_5678.env"), "API_KEY=other\n").unwrap();
        let version = blake3::hash(ciphertext).to_hex().to_string();
        fs::write(
            root.join("secret-locators.json"),
            serde_json::json!([{"id":"env/local_1234.env/API_KEY","version":version,"service":"svc","account":"acct"}])
                .to_string(),
        )
        .unwrap();
        let items = |service: &str, account: &str| {
            (service == "svc" && account == "acct").then(|| b"sk-synthetic".to_vec())
        };
        let keys: Vec<String> = ["API_KEY", "MODEL", "QUOTED", "EMPTY", "MISSING"]
            .map(str::to_owned)
            .to_vec();
        let got = values(root, "needs-key", &keys, "svc", &items);
        assert_eq!(
            got,
            BTreeMap::from([
                ("API_KEY".to_owned(), "sk-synthetic".to_owned()),
                ("MODEL".to_owned(), "m-2".to_owned()),
                ("QUOTED".to_owned(), " spaced # kept ".to_owned()),
            ])
        );
        // A sealed value this Mac can't open is absent; so is an unknown folder.
        let none = |_: &str, _: &str| None;
        assert!(!values(root, "needs-key", &keys, "svc", &none).contains_key("API_KEY"));
        assert!(values(root, "unknown", &keys, "svc", &items).is_empty());
        assert!(env_file("../escape").is_none());
    }

    /// A lane or debug build handed a profile copied from the shipped app's
    /// (its provenance, env files and secret locators) opens none of that
    /// build's Keychain items: a locator naming another build's service is
    /// refused before the Keychain is asked, so nothing is read and macOS
    /// shows no prompt. The same profile in the build it belongs to reads it.
    #[test]
    fn a_copied_profiles_keychain_locators_are_never_opened_by_another_build() {
        let profile = tempfile::tempdir().unwrap();
        let root = profile.path();
        fs::write(
            root.join("extensions-provenance.json"),
            r#"{"needs-key":{"envKey":"local:1234"}}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("env")).unwrap();
        let ciphertext = b"v10sealed-bytes";
        use base64::Engine;
        let sealed = base64::engine::general_purpose::STANDARD.encode(ciphertext);
        fs::write(
            root.join("env/local_1234.env"),
            format!("API_KEY=enc:{sealed}\nMODEL=plain\n"),
        )
        .unwrap();
        let shipped = service_of("com.example.shipped");
        fs::write(
            root.join("secret-locators.json"),
            serde_json::json!([{"id":"env/local_1234.env/API_KEY",
                "version":blake3::hash(ciphertext).to_hex().to_string(),
                "service":shipped,"account":"acct"}])
            .to_string(),
        )
        .unwrap();
        static ASKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let asked = &ASKED;
        let items = |_: &str, _: &str| {
            ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(b"sk-production".to_vec())
        };
        let keys: Vec<String> = ["API_KEY", "MODEL"].map(str::to_owned).to_vec();
        for lane in [service_of("com.example.dev.lane"), service_of("")] {
            let got = values(root, "needs-key", &keys, &lane, &items);
            assert_eq!(
                got,
                BTreeMap::from([("MODEL".to_owned(), "plain".to_owned())]),
                "only the plain value: {lane:?}"
            );
        }
        assert_eq!(
            asked.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "the Keychain was asked for another build's item"
        );
        let own = values(root, "needs-key", &keys, &shipped, &items);
        assert_eq!(
            own.get("API_KEY").map(String::as_str),
            Some("sk-production")
        );
        assert_eq!(asked.load(std::sync::atomic::Ordering::Relaxed), 1);
    }
}
