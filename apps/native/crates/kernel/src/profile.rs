//! Native-profile services. Constructed only under the library writer's lock.
//! Secrets stay in Keychain: this module stores locators, never secret values.
use crate::{
    Kernel, KernelFailure,
    persistence::{LibraryWriter, atomic_write},
};
use pm_model::json::{Map, Value, parse, to_canonical_string};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalStorageEntry {
    pub origin: String,
    pub key: String,
    pub value: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretLocator {
    pub id: String,
    pub version: String,
    pub service: String,
    pub account: String,
}
#[derive(Clone, Debug)]
pub struct ExtensionSources {
    pub shared: PathBuf,
    pub shared_read_only: bool,
    pub overlay: PathBuf,
}
#[derive(Clone, Debug)]
pub enum ProfileCall {
    LocalStorageGet {
        origin: String,
        key: String,
    },
    LocalStorageSet {
        origin: String,
        key: String,
        value: Option<String>,
    },
    SecretLocators {
        id: String,
    },
    RegisterSecret(SecretLocator),
    ExtensionSources {
        id: String,
    },
    ReadExtension {
        id: String,
        relative: PathBuf,
    },
    WriteExtension {
        id: String,
        relative: PathBuf,
        bytes: Vec<u8>,
    },
    /// Read an external .pmv without creating a lock beside the source.
    ExternalMedia {
        path: PathBuf,
    },
}
#[derive(Clone, Debug)]
pub enum ProfileReply {
    String(Option<String>),
    Secrets(Vec<SecretLocator>),
    Sources(ExtensionSources),
    Bytes(Vec<u8>),
    Media(crate::project_io::MediaTable),
    Changed,
}
fn err(e: impl std::fmt::Display) -> KernelFailure {
    KernelFailure::Storage(e.to_string())
}
fn object(fields: &[(&str, &str)]) -> Value {
    let mut m = Map::new();
    for (key, value) in fields {
        m.insert(*key, Value::from(*value));
    }
    Value::Object(m)
}
/// Borrows the lock owner, so migration and runtime cannot create a second writer.
pub struct ProfileStorage<'a> {
    writer: &'a LibraryWriter,
}
impl<'a> ProfileStorage<'a> {
    pub fn new(writer: &'a LibraryWriter) -> Self {
        Self { writer }
    }
    fn root(&self) -> &Path {
        self.writer.store().dir().parent().expect("profile/store")
    }
    fn read(&self, name: &str) -> Result<Vec<Value>, KernelFailure> {
        let path = safe_path(self.root(), Path::new(name), false)?;
        match fs::read_to_string(path) {
            Ok(text) => parse(&text)
                .map_err(err)?
                .as_array()
                .cloned()
                .ok_or_else(|| err("Invalid profile sidecar")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(err(e)),
        }
    }
    fn write(&self, name: &str, values: Vec<Value>) -> Result<(), KernelFailure> {
        let path = safe_path(self.root(), Path::new(name), false)?;
        atomic_write(&path, to_canonical_string(&Value::Array(values)).as_bytes()).map_err(err)
    }
    /// Initial migration only. Existing native values win, including empty strings.
    pub fn import_local_storage(&self, entries: &[LocalStorageEntry]) -> Result<(), KernelFailure> {
        let mut values = self.read("local-storage.json")?;
        for entry in entries {
            if !values.iter().any(|v| {
                v.get("origin").and_then(Value::as_str) == Some(&entry.origin)
                    && v.get("key").and_then(Value::as_str) == Some(&entry.key)
            }) {
                values.push(object(&[
                    ("origin", &entry.origin),
                    ("key", &entry.key),
                    ("value", &entry.value),
                ]));
            }
        }
        self.write("local-storage.json", values)
    }
    pub fn local_storage(&self) -> Result<Vec<LocalStorageEntry>, KernelFailure> {
        Ok(self
            .read("local-storage.json")?
            .iter()
            .filter_map(|v| {
                Some(LocalStorageEntry {
                    origin: v.get("origin")?.as_str()?.to_owned(),
                    key: v.get("key")?.as_str()?.to_owned(),
                    value: v.get("value")?.as_str()?.to_owned(),
                })
            })
            .collect())
    }
    /// Publishes a merged set. Migration deltas own the merge.
    pub fn replace_local_storage(
        &self,
        entries: &[LocalStorageEntry],
    ) -> Result<(), KernelFailure> {
        self.write(
            "local-storage.json",
            entries
                .iter()
                .map(|e| object(&[("origin", &e.origin), ("key", &e.key), ("value", &e.value)]))
                .collect(),
        )
    }
    pub fn register_secret(&self, locator: &SecretLocator) -> Result<(), KernelFailure> {
        if [
            &locator.id,
            &locator.version,
            &locator.service,
            &locator.account,
        ]
        .iter()
        .any(|s| s.is_empty())
        {
            return Err(err("Secret locator fields must be nonempty"));
        }
        let mut values = self.read("secret-locators.json")?;
        let value = object(&[
            ("id", &locator.id),
            ("version", &locator.version),
            ("service", &locator.service),
            ("account", &locator.account),
        ]);
        if let Some(old) = values.iter().find(|v| {
            v.get("id").and_then(Value::as_str) == Some(&locator.id)
                && v.get("version").and_then(Value::as_str) == Some(&locator.version)
        }) {
            if old != &value {
                return Err(err(
                    "Secret locator version conflict; existing locator retained",
                ));
            }
            return Ok(());
        }
        values.push(value);
        self.write("secret-locators.json", values)
    }
    fn sources(&self, id: &str) -> Result<ExtensionSources, KernelFailure> {
        if id.is_empty()
            || Path::new(id).components().count() != 1
            || !matches!(
                Path::new(id).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err(err("Invalid extension id"));
        }
        let shared = safe_path(self.root(), &Path::new("extensions-shared").join(id), false)?;
        let overlay = safe_path(
            self.root(),
            &Path::new("extensions-overlay").join(id),
            false,
        )?;
        Ok(ExtensionSources {
            shared,
            shared_read_only: true,
            overlay,
        })
    }
}
/// Reject traversal and linked ancestors in both read and write capabilities.
fn safe_path(root: &Path, relative: &Path, parents: bool) -> Result<PathBuf, KernelFailure> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(err("Use a relative path inside the native profile"));
    }
    if fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(err("Linked profile roots are read-only"));
    }
    let mut path = root.to_path_buf();
    for c in relative.components() {
        path.push(c);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(err("Linked profile paths are read-only"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err(e)),
        }
    }
    if parents {
        fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
    }
    Ok(path)
}
/// The registered locations of secret `id`, in registration order.
fn locators(values: &[Value], id: &str) -> Result<Vec<SecretLocator>, KernelFailure> {
    let mut out = vec![];
    for v in values
        .iter()
        .filter(|v| v.get("id").and_then(Value::as_str) == Some(id))
    {
        let get = |key| {
            v.get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| err("Invalid secret locator"))
        };
        out.push(SecretLocator {
            id: id.to_owned(),
            version: get("version")?,
            service: get("service")?,
            account: get("account")?,
        });
    }
    Ok(out)
}
/// Every location secret `id` was registered at in `profile`, read without
/// the library writer (registration replaces the sidecar atomically), so a
/// worker that holds no writer (the Cloud account) can find them.
pub fn registered_secrets(profile: &Path, id: &str) -> Result<Vec<SecretLocator>, KernelFailure> {
    let path = safe_path(profile, Path::new("secret-locators.json"), false)?;
    let values = match fs::read_to_string(path) {
        Ok(text) => parse(&text)
            .map_err(err)?
            .as_array()
            .cloned()
            .ok_or_else(|| err("Invalid profile sidecar"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => return Err(err(e)),
    };
    locators(&values, id)
}
impl Kernel {
    pub fn profile(&self, call: ProfileCall) -> Result<ProfileReply, KernelFailure> {
        let writer = self
            .library
            .as_ref()
            .ok_or_else(|| err("Attach a native library first"))?;
        let storage = ProfileStorage::new(writer);
        match call {
            ProfileCall::LocalStorageGet { origin, key } => {
                let values = storage.read("local-storage.json")?;
                let value = values.iter().find(|v| {
                    v.get("origin").and_then(Value::as_str) == Some(&origin)
                        && v.get("key").and_then(Value::as_str) == Some(&key)
                });
                Ok(ProfileReply::String(
                    value
                        .and_then(|v| v.get("value"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                ))
            }
            ProfileCall::LocalStorageSet { origin, key, value } => {
                let mut values = storage.read("local-storage.json")?;
                values.retain(|v| {
                    !(v.get("origin").and_then(Value::as_str) == Some(&origin)
                        && v.get("key").and_then(Value::as_str) == Some(&key))
                });
                if let Some(value) = value {
                    values.push(object(&[
                        ("origin", &origin),
                        ("key", &key),
                        ("value", &value),
                    ]));
                }
                storage.write("local-storage.json", values)?;
                Ok(ProfileReply::Changed)
            }
            ProfileCall::RegisterSecret(locator) => {
                storage.register_secret(&locator)?;
                Ok(ProfileReply::Changed)
            }
            ProfileCall::SecretLocators { id } => Ok(ProfileReply::Secrets(locators(
                &storage.read("secret-locators.json")?,
                &id,
            )?)),
            ProfileCall::ExtensionSources { id } => {
                Ok(ProfileReply::Sources(storage.sources(&id)?))
            }
            ProfileCall::ReadExtension { id, relative } => {
                let sources = storage.sources(&id)?;
                // Validate both even when the first exists: a linked shared root
                // cannot become a hidden writable fallback.
                let overlay = safe_path(&sources.overlay, &relative, false)?;
                let shared = safe_path(&sources.shared, &relative, false)?;
                let bytes = match fs::read(overlay) {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        fs::read(shared).map_err(err)?
                    }
                    Err(e) => return Err(err(e)),
                };
                Ok(ProfileReply::Bytes(bytes))
            }
            ProfileCall::WriteExtension {
                id,
                relative,
                bytes,
            } => {
                let sources = storage.sources(&id)?;
                let path = safe_path(&sources.overlay, &relative, true)?;
                atomic_write(&path, &bytes).map_err(err)?;
                Ok(ProfileReply::Changed)
            }
            ProfileCall::ExternalMedia { path } => Ok(ProfileReply::Media(
                crate::open_project(&path, crate::OpenOptions::default())
                    .map_err(err)?
                    .file
                    .media,
            )),
        }
    }
}
