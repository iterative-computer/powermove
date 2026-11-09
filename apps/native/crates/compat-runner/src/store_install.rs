//! Installing a release from the Store. The native installer accepts a release
//! only with a host-pinned publisher signature over its exact file inventory;
//! see docs/native/store-host.md for what the server has to publish.
use crate::{KernelStoreAdapter, StoreArchive, StoreAuthority, net, store_host::STORE_MARKER};
use pm_runtime_v8::{Error, Result};
use pm_store_host::artifact::{FileArtifacts, SignedRelease, TrustStore};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub const ORIGIN: &str = "https://cloud.trypowermove.com";
const MAX_ENVELOPE: usize = 8 * 1024 * 1024;
const MAX_ARCHIVE: usize = 64 * 1024 * 1024;

/// `(publisher, key id, Ed25519 public key)`. Empty until the Store signs
/// releases; extension or catalog input can never add to this list.
const PINNED_KEYS: &[(&str, &str, [u8; 32])] = &[];

/// The Store this build reads. Development builds started with
/// `--dev-store-origin http://127.0.0.1:<port>/` read a fixture Store on this
/// Mac instead; release builds always read [`ORIGIN`].
pub fn origin() -> &'static str {
    dev().map_or(ORIGIN, |dev| dev.origin)
}
/// Whether this development build reads a fixture Store (`--dev-store-origin`).
pub fn is_dev() -> bool {
    dev().is_some()
}
/// A development build's fixture Store: its loopback origin and the one test
/// key (`--dev-store-key publisher:key-id:<64 hex>`) its releases are signed
/// with. Release builds compile this away; no catalog or extension input can
/// reach it.
struct DevStore {
    origin: &'static str,
    key: Option<(String, String, [u8; 32])>,
}
fn dev() -> Option<&'static DevStore> {
    #[cfg(debug_assertions)]
    {
        static DEV: std::sync::OnceLock<Option<DevStore>> = std::sync::OnceLock::new();
        DEV.get_or_init(|| {
            let origin = std::env::var("PM_DEV_STORE_ORIGIN").ok()?;
            let url = url::Url::parse(&origin).ok()?;
            if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") || url.path() != "/" {
                return None;
            }
            let key = std::env::var("PM_DEV_STORE_KEY").ok().and_then(|raw| {
                let mut parts = raw.splitn(3, ':');
                let (publisher, key, hex) = (parts.next()?, parts.next()?, parts.next()?);
                let mut bytes = [0u8; 32];
                (hex.len() == 64).then_some(())?;
                for (i, byte) in bytes.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
                }
                Some((publisher.to_owned(), key.to_owned(), bytes))
            });
            Some(DevStore {
                origin: Box::leak(origin.into_boxed_str()),
                key,
            })
        })
        .as_ref()
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// Whether a new install can succeed. Without a pinned key every one is refused
/// before download, so the app offers none.
pub fn available() -> bool {
    !PINNED_KEYS.is_empty() || dev().is_some_and(|dev| dev.key.is_some())
}

/// Why a release was not installed, in words a person can act on.
fn refusal(error: pm_store_host::Error) -> Error {
    use pm_store_host::Error as Store;
    Error::Host(match error {
        Store::Integrity(_) => {
            "This release's signature doesn't match its files, so it wasn't installed.".into()
        }
        Store::Invalid(_) => "This isn't a valid Store release, so it wasn't installed.".into(),
        Store::Conflict | Store::Stale => {
            "Another change to this extension happened first. Try installing again.".into()
        }
        Store::Io(message) | Store::Platform(message) => {
            format!("The release couldn't be saved: {message}")
        }
        other => format!("The release wasn't installed ({other})."),
    })
}

pub fn pinned() -> Result<TrustStore> {
    let mut trust = TrustStore::default();
    for (publisher, key, bytes) in PINNED_KEYS {
        let key_bytes = ed25519_dalek::VerifyingKey::from_bytes(bytes)
            .map_err(|_| Error::Invalid("pinned Store key"))?;
        trust
            .trust((*publisher).into(), (*key).into(), key_bytes)
            .map_err(|e| Error::Host(format!("{e:?}")))?;
    }
    if let Some((publisher, key, bytes)) = dev().and_then(|dev| dev.key.as_ref()) {
        let key_bytes = ed25519_dalek::VerifyingKey::from_bytes(bytes)
            .map_err(|_| Error::Invalid("development Store key"))?;
        trust
            .trust(publisher.clone(), key.clone(), key_bytes)
            .map_err(|e| Error::Host(format!("{e:?}")))?;
    }
    Ok(trust)
}

/// Commits the verified release through the kernel, then writes exactly its
/// sealed files, so discovery finds it as it finds any installed extension.
pub fn install_verified(
    kernel: &dyn StoreAuthority,
    profile: &Path,
    namespace: &str,
    release: &SignedRelease,
    archive: &[u8],
    trust: &TrustStore,
    repo: &str,
) -> Result<String> {
    let mut artifacts = FileArtifacts::new(profile.join("store-artifacts")).map_err(refusal)?;
    // An update replaces the release this app installed before, at its revision.
    let expected = {
        use pm_store_host::install::StoreKernel;
        KernelStoreAdapter::new(kernel)
            .load(&release.statement.extension)
            .map_err(refusal)?
            .map(|state| state.revision)
    };
    let (_, verified) = KernelStoreAdapter::new(kernel)
        .install_archive(
            &mut artifacts,
            StoreArchive {
                profile: namespace,
                release,
                archive,
                trust,
                expected,
            },
        )
        .map_err(refusal)?;
    let id = release.statement.extension.as_str().to_owned();
    let root = profile.join("extensions");
    let staging = root.join(format!(".{id}.install"));
    let write = || -> std::io::Result<()> {
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging)?;
        for path in release.statement.files.keys() {
            let target = staging.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(target, verified.file(path).unwrap_or_default())?;
        }
        fs::write(
            staging.join(STORE_MARKER),
            json!({"repoId":repo,"releaseId":release.statement.release_id}).to_string(),
        )?;
        // An update keeps the folder it replaces (with any edits made on
        // this Mac) in the archive; nothing is deleted.
        let target = root.join(&id);
        let archived = archive_folder(&root, &id)?;
        if let Err(error) = fs::rename(&staging, &target) {
            if let Some(archived) = archived {
                let _ = fs::rename(archived, &target);
            }
            return Err(error);
        }
        Ok(())
    };
    write().map_err(|e| Error::Host(e.to_string()))?;
    Ok(id)
}

/// Moves `<root>/<id>` into `<root>/.archive/<id>-<seconds>` (dot folders are
/// never discovered), returning where it went; `None` when there was none.
fn archive_folder(root: &Path, id: &str) -> std::io::Result<Option<std::path::PathBuf>> {
    let target = root.join(id);
    if fs::symlink_metadata(&target).is_err() {
        return Ok(None);
    }
    let archive = root.join(".archive");
    fs::create_dir_all(&archive)?;
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut place = archive.join(format!("{id}-{seconds}"));
    let mut n = 1;
    while fs::symlink_metadata(&place).is_ok() {
        place = archive.join(format!("{id}-{seconds}-{n}"));
        n += 1;
    }
    fs::rename(&target, &place)?;
    Ok(Some(place))
}

/// Uninstalls a Store extension: its folder moves to the archive (with its
/// files and any edits), and a release this app installed is archived in the
/// kernel's Store record too, keeping its identity, decisions and values.
/// Only Store installs (folders with the marker) can be removed this way.
pub fn uninstall(kernel: &dyn StoreAuthority, profile: &Path, id: &str) -> Result<()> {
    let root = profile.join("extensions");
    if !crate::manifest::extension_id(id) || crate::store_host::marker(&root.join(id)).is_none() {
        return Err(Error::Host(
            "Only extensions installed from the Store can be uninstalled here.".into(),
        ));
    }
    let extension = pm_store_host::identity::ExtensionId::new(id)
        .map_err(|_| Error::Invalid("extension id"))?;
    let mut adapter = KernelStoreAdapter::new(kernel);
    use pm_store_host::install::StoreKernel;
    if let Some(state) = adapter.load(&extension).ok().flatten()
        && state.active.is_some()
    {
        pm_store_host::install::uninstall(&mut adapter, &extension, state.revision)
            .map_err(refusal)?;
    }
    archive_folder(&root, id).map_err(|e| Error::Host(e.to_string()))?;
    Ok(())
}

/// Name, release and repository of each listing, for the one page a menu shows.
pub fn catalog() -> Result<Vec<Value>> {
    let page = net::get_within(
        origin(),
        &format!(
            "{}/v1/store/extensions?sort=name&limit=50",
            origin().trim_end_matches('/')
        ),
        MAX_ENVELOPE,
        30,
    )?;
    if page.status != 200 {
        return Err(Error::Host("The Store is unavailable.".into()));
    }
    let page: Value =
        serde_json::from_slice(&page.body).map_err(|_| Error::Invalid("Store catalog"))?;
    Ok(listings(&page))
}
fn listings(page: &Value) -> Vec<Value> {
    page["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let release = item["latest"]["id"].as_str()?;
            let repo = item["repoId"].as_str()?;
            let label = item["name"].as_str().unwrap_or(repo);
            Some(json!({"label":label,"release":release,"repo":repo}))
        })
        .collect()
}

/// Where releases come from and the keys that may sign them.
pub struct Source {
    pub origin: &'static str,
    pub trust: TrustStore,
}
/// This build's Store and pins; `None` while no key is pinned.
pub fn source() -> Result<Option<Source>> {
    if !available() {
        return Ok(None);
    }
    Ok(Some(Source {
        origin: origin(),
        trust: pinned()?,
    }))
}

/// Downloads a release and its signed envelope, verifies, installs.
pub fn install(
    kernel: &dyn StoreAuthority,
    profile: &Path,
    namespace: &str,
    release_id: &str,
    repo: &str,
) -> Result<String> {
    let Some(source) = source()? else {
        return Err(Error::Host(
            "Store installs need a signed release, and the Store doesn't sign releases yet.".into(),
        ));
    };
    install_from(&source, kernel, profile, namespace, release_id, repo)
}
/// [`install`] from a given Store and pins.
pub fn install_from(
    source: &Source,
    kernel: &dyn StoreAuthority,
    profile: &Path,
    namespace: &str,
    release_id: &str,
    repo: &str,
) -> Result<String> {
    // Release ids are UUIDs; nothing else is ever put in a Store path.
    let uuid = release_id.len() == 36
        && release_id.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
    if !uuid {
        return Err(Error::Invalid("Store release id"));
    }
    let (origin, trust) = (source.origin, &source.trust);
    let base = origin.trim_end_matches('/');
    let envelope = net::get_within(
        origin,
        &format!("{base}/v1/store/releases/{release_id}/signature"),
        MAX_ENVELOPE,
        30,
    )?;
    if envelope.status != 200 {
        return Err(Error::Host(
            "This release has no signed envelope, so it can't be verified.".into(),
        ));
    }
    let signed: SignedRelease = serde_json::from_slice(&envelope.body)
        .map_err(|_| Error::Invalid("Store signed envelope"))?;
    let archive = net::get_within(
        origin,
        &format!("{base}/v1/store/releases/{release_id}/tar"),
        MAX_ARCHIVE,
        120,
    )?;
    if archive.status != 200 {
        return Err(Error::Host("The release archive is unavailable.".into()));
    }
    install_verified(
        kernel,
        profile,
        namespace,
        &signed,
        &archive.body,
        trust,
        repo,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn listings_need_a_release_and_a_build_without_keys_refuses_before_any_download() {
        let rows = listings(&json!({"items":[
            {"repoId":"r","name":"First","latest":{"id":"one"}},
            {"repoId":"unreleased","name":"Second"}]}));
        assert_eq!(rows, [json!({"label":"First","release":"one","repo":"r"})]);
        let kernel = Mutex::new(Kernel::new(KernelConfig::default()));
        let error = install(&kernel, Path::new("/nonexistent"), "native", "one", "r").unwrap_err();
        assert!(error.to_string().contains("signed release"));
    }
    use ed25519_dalek::{Signer, SigningKey};
    use pm_kernel::{Kernel, KernelConfig};
    use pm_store_host::{
        artifact::{FileSeal, ReleaseStatement},
        identity::ExtensionId,
    };
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, sync::Mutex};
    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        for (name, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, *bytes).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }
    #[test]
    fn a_verified_release_is_committed_and_found_like_any_install_but_unpinned_keys_fail() {
        let manifest = json!({"id":"synthetic-store","name":"Synthetic","version":"1.0.0","apiVersion":3,"permissions":[]}).to_string();
        let source = b"export default function(){}";
        let files: [(&str, &[u8]); 2] =
            [("manifest.json", manifest.as_bytes()), ("index.ts", source)];
        let bytes = archive(&files);
        let key = SigningKey::from_bytes(&[7; 32]);
        let statement = ReleaseStatement {
            format: 1,
            key_id: "synthetic-key".into(),
            publisher: "synthetic-publisher".into(),
            extension: ExtensionId::new("synthetic-store").unwrap(),
            release_id: "synthetic-release".into(),
            archive_sha256: format!("{:x}", Sha256::digest(&bytes)),
            files: files
                .iter()
                .map(|(name, data)| {
                    (
                        (*name).to_owned(),
                        FileSeal {
                            sha256: format!("{:x}", Sha256::digest(data)),
                            bytes: data.len() as u64,
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        };
        let signature = key.sign(&statement.signing_bytes().unwrap()).to_vec();
        let release = SignedRelease {
            statement,
            signature,
        };
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().canonicalize().unwrap();
        let kernel = Mutex::new(Kernel::new(KernelConfig::default()));
        let unpinned = install_verified(
            &kernel,
            &profile,
            "synthetic",
            &release,
            &bytes,
            &TrustStore::default(),
            "repo",
        );
        assert!(unpinned.is_err());
        assert!(!profile.join("extensions/synthetic-store").exists());
        let mut trust = TrustStore::default();
        trust
            .trust(
                "synthetic-publisher".into(),
                "synthetic-key".into(),
                key.verifying_key(),
            )
            .unwrap();
        let id = install_verified(
            &kernel,
            &profile,
            "synthetic",
            &release,
            &bytes,
            &trust,
            "repo",
        )
        .unwrap();
        assert_eq!(id, "synthetic-store");
        let found = crate::discovery::scan(&[profile.join("extensions")]);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].store.as_ref().unwrap()["releaseId"],
            "synthetic-release"
        );
    }
    fn signed(version: &str, release: &str, source: &[u8]) -> (SignedRelease, Vec<u8>) {
        let manifest = json!({"id":"synthetic-store","name":"Synthetic","version":version,"apiVersion":3,"permissions":[]}).to_string();
        let files: [(&str, &[u8]); 2] =
            [("manifest.json", manifest.as_bytes()), ("index.ts", source)];
        let bytes = archive(&files);
        let statement = ReleaseStatement {
            format: 1,
            key_id: "synthetic-key".into(),
            publisher: "synthetic-publisher".into(),
            extension: ExtensionId::new("synthetic-store").unwrap(),
            release_id: release.into(),
            archive_sha256: format!("{:x}", Sha256::digest(&bytes)),
            files: files
                .iter()
                .map(|(name, data)| {
                    let seal = FileSeal {
                        sha256: format!("{:x}", Sha256::digest(data)),
                        bytes: data.len() as u64,
                    };
                    ((*name).to_owned(), seal)
                })
                .collect(),
        };
        let signature = SigningKey::from_bytes(&[7; 32])
            .sign(&statement.signing_bytes().unwrap())
            .to_vec();
        (
            SignedRelease {
                statement,
                signature,
            },
            bytes,
        )
    }
    #[test]
    fn an_update_archives_the_folder_it_replaces_and_uninstall_archives_without_deleting() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().canonicalize().unwrap();
        let kernel = Mutex::new(Kernel::new(KernelConfig::default()));
        let mut trust = TrustStore::default();
        let key = SigningKey::from_bytes(&[7; 32]).verifying_key();
        trust
            .trust("synthetic-publisher".into(), "synthetic-key".into(), key)
            .unwrap();
        let (first, bytes) = signed("1.0.0", "release-one", b"export default 1");
        install_verified(
            &kernel,
            &profile,
            "synthetic",
            &first,
            &bytes,
            &trust,
            "repo",
        )
        .unwrap();
        let folder = profile.join("extensions/synthetic-store");
        // A person's edit on this Mac must survive the update, in the archive.
        fs::write(folder.join("notes.txt"), "mine").unwrap();
        let (second, bytes) = signed("1.1.0", "release-two", b"export default 2");
        install_verified(
            &kernel,
            &profile,
            "synthetic",
            &second,
            &bytes,
            &trust,
            "repo",
        )
        .unwrap();
        assert_eq!(
            fs::read(folder.join("index.ts")).unwrap(),
            b"export default 2"
        );
        assert!(!folder.join("notes.txt").exists());
        let archived: Vec<_> = fs::read_dir(profile.join("extensions/.archive"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        assert_eq!(archived.len(), 1);
        assert_eq!(fs::read(archived[0].join("notes.txt")).unwrap(), b"mine");
        let state = kernel.lock().unwrap().state().clone();
        assert_eq!(state.extensions["synthetic-store"].version, "1.1.0");
        // Uninstall: only Store installs, and nothing is deleted.
        fs::create_dir_all(profile.join("extensions/local-one")).unwrap();
        assert!(uninstall(&kernel, &profile, "local-one").is_err());
        assert!(uninstall(&kernel, &profile, "../escape").is_err());
        uninstall(&kernel, &profile, "synthetic-store").unwrap();
        assert!(!folder.exists());
        assert_eq!(
            fs::read_dir(profile.join("extensions/.archive"))
                .unwrap()
                .count(),
            2
        );
        assert!(crate::discovery::scan(&[profile.join("extensions")]).is_empty());
        let state = kernel.lock().unwrap().state().clone();
        assert!(!state.extensions["synthetic-store"].installed);
        // Reinstalling after an uninstall reactivates the same identity.
        install_verified(
            &kernel,
            &profile,
            "synthetic",
            &second,
            &bytes,
            &trust,
            "repo",
        )
        .unwrap();
        assert!(folder.join("index.ts").exists());
    }
    #[test]
    fn installs_refuse_a_release_id_that_is_not_a_uuid() {
        let kernel = Mutex::new(Kernel::new(KernelConfig::default()));
        if available() {
            let error =
                install(&kernel, Path::new("/nonexistent"), "native", "../x", "r").unwrap_err();
            assert!(error.to_string().contains("release id"));
        }
    }
}
