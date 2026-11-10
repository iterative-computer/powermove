//! Installing a release from the Store. The native installer accepts a release
//! only with a host-pinned publisher signature over its exact file inventory,
//! naming the extension, release and version the person chose, and an update
//! only to a newer version than the one installed; see
//! docs/native/store-host.md for what the server has to publish.
use crate::{KernelStoreAdapter, StoreAuthority, net, store_host::STORE_MARKER};
use pm_runtime_v8::{Error, Result};
use pm_store_host::artifact::{FileArtifacts, SignedRelease, TrustStore};
use serde_json::json;
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

/// What a person chose on a Store page: the release and its repository, and
/// the extension and version that page names for it.
pub struct Request<'a> {
    pub release: &'a str,
    pub repo: &'a str,
    pub extension: &'a str,
    pub version: &'a str,
}
/// Whether `next` is a newer version than `installed` (both semver).
fn newer(next: &str, installed: &str) -> bool {
    match (
        semver::Version::parse(next),
        semver::Version::parse(installed),
    ) {
        (Ok(next), Ok(installed)) => next > installed,
        _ => false,
    }
}
/// Verifies the release against the pins and the request, commits it
/// through the kernel, then writes exactly its sealed files, so discovery
/// finds it as it finds any installed extension. A valid envelope for
/// another extension, release or version, or one not newer than the release
/// installed now, is refused before anything changes.
pub fn install_verified(
    kernel: &dyn StoreAuthority,
    profile: &Path,
    namespace: &str,
    request: &Request,
    release: &SignedRelease,
    archive: &[u8],
    trust: &TrustStore,
) -> Result<String> {
    use pm_store_host::install::StoreKernel;
    let verified = pm_store_host::artifact::verify(
        release,
        archive,
        trust,
        pm_store_host::artifact::Limits::default(),
    )
    .map_err(refusal)?;
    let statement = verified.statement();
    if statement.release_id != request.release
        || statement.extension.as_str() != request.extension
        || verified.manifest().version != request.version
    {
        return Err(Error::Host(
            "This release isn't the one the Store page offered, so it wasn't installed.".into(),
        ));
    }
    let mut adapter = KernelStoreAdapter::new(kernel);
    let current = adapter.load(&statement.extension).map_err(refusal)?;
    if let Some(installed) = current
        .as_ref()
        .and_then(|state| state.active.as_ref())
        .map(|active| active.manifest.version.as_str())
        && !newer(request.version, installed)
    {
        return Err(Error::Host(format!(
            "{installed} is installed, and this release ({}) isn't newer, so it wasn't installed.",
            request.version
        )));
    }
    // An update replaces the release this app installed before, at its revision.
    let expected = current.map(|state| state.revision);
    let mut artifacts = FileArtifacts::new(profile.join("store-artifacts")).map_err(refusal)?;
    pm_store_host::install::install(&mut adapter, &mut artifacts, namespace, &verified, expected)
        .map_err(refusal)?;
    let repo = request.repo;
    let id = statement.extension.as_str().to_owned();
    let root = profile.join("extensions");
    let staging = root.join(format!(".{id}.install"));
    let write = || -> std::io::Result<()> {
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging)?;
        for path in statement.files.keys() {
            let target = staging.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(target, verified.file(path).unwrap_or_default())?;
        }
        fs::write(
            staging.join(STORE_MARKER),
            json!({"repoId":repo,"releaseId":statement.release_id}).to_string(),
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

/// Settings › Extensions › Delete…: a Store install is uninstalled as the
/// Store does; any other extension folder moves to the archive with its files
/// (manifest, source, fork provenance). Saved values stay in the profile.
pub fn archive(kernel: &dyn StoreAuthority, profile: &Path, id: &str) -> Result<()> {
    let root = profile.join("extensions");
    if !crate::manifest::extension_id(id) {
        return Err(Error::Invalid("extension id"));
    }
    if crate::store_host::marker(&root.join(id)).is_some() {
        return uninstall(kernel, profile, id);
    }
    match archive_folder(&root, id) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(Error::Host("That extension isn’t on this Mac.".into())),
        Err(e) => Err(Error::Host(e.to_string())),
    }
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

/// Downloads the chosen release and its signed envelope from `source`,
/// verifies them against its pins and the request, and installs.
pub fn install_from(
    source: &Source,
    kernel: &dyn StoreAuthority,
    profile: &Path,
    namespace: &str,
    request: &Request,
) -> Result<String> {
    let release_id = request.release;
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
        request,
        &signed,
        &archive.body,
        trust,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_build_without_keys_has_no_store_to_install_from() {
        assert!(!available());
        assert!(source().unwrap().is_none());
    }
    fn request<'a>(release: &'a str, version: &'a str) -> Request<'a> {
        Request {
            release,
            repo: "repo",
            extension: "synthetic-store",
            version,
        }
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
            &request("synthetic-release", "1.0.0"),
            &release,
            &bytes,
            &TrustStore::default(),
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
            &request("synthetic-release", "1.0.0"),
            &release,
            &bytes,
            &trust,
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
            &request("release-one", "1.0.0"),
            &first,
            &bytes,
            &trust,
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
            &request("release-two", "1.1.0"),
            &second,
            &bytes,
            &trust,
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
            &request("release-two", "1.1.0"),
            &second,
            &bytes,
            &trust,
        )
        .unwrap();
        assert!(folder.join("index.ts").exists());
    }
    #[test]
    fn installs_refuse_a_release_id_that_is_not_a_uuid() {
        let kernel = Mutex::new(Kernel::new(KernelConfig::default()));
        let source = Source {
            origin: "http://127.0.0.1:9/",
            trust: TrustStore::default(),
        };
        let error = install_from(
            &source,
            &kernel,
            Path::new("/nonexistent"),
            "native",
            &request("../x", "1.0.0"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("release id"));
    }
    #[test]
    fn a_valid_envelope_for_another_choice_or_an_older_version_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().canonicalize().unwrap();
        let kernel = Mutex::new(Kernel::new(KernelConfig::default()));
        let mut trust = TrustStore::default();
        let key = SigningKey::from_bytes(&[7; 32]).verifying_key();
        trust
            .trust("synthetic-publisher".into(), "synthetic-key".into(), key)
            .unwrap();
        let install = |request: &Request, release: &(SignedRelease, Vec<u8>)| {
            install_verified(
                &kernel,
                &profile,
                "synthetic",
                request,
                &release.0,
                &release.1,
                &trust,
            )
            .map_err(|e| match e {
                Error::Host(message) => message,
                other => panic!("{other:?}"),
            })
        };
        let old = signed("1.0.0", "release-one", b"export default 1");
        let new = signed("1.1.0", "release-two", b"export default 2");
        let offered = "This release isn't the one the Store page offered, so it wasn't installed.";
        // The page offered release two; the Store answered with release one's
        // envelope, or one for another extension, or another version.
        for request in [
            request("release-two", "1.1.0"),
            Request {
                extension: "other-extension",
                ..request("release-one", "1.0.0")
            },
            request("release-one", "1.1.0"),
        ] {
            assert_eq!(install(&request, &old).unwrap_err(), offered);
        }
        assert!(!profile.join("extensions/synthetic-store").exists());
        install(&request("release-two", "1.1.0"), &new).unwrap();
        // Replaying the older (validly signed) release can't downgrade, and the
        // same version isn't an update.
        let older = install(&request("release-one", "1.0.0"), &old).unwrap_err();
        assert_eq!(
            older,
            "1.1.0 is installed, and this release (1.0.0) isn't newer, so it wasn't installed."
        );
        assert!(install(&request("release-two", "1.1.0"), &new).is_err());
        let folder = profile.join("extensions/synthetic-store");
        assert_eq!(
            fs::read(folder.join("index.ts")).unwrap(),
            b"export default 2"
        );
        assert!(!profile.join("extensions/.archive").exists());
    }
    #[test]
    fn deleting_a_custom_extension_archives_its_folder_and_refuses_the_rest() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().canonicalize().unwrap();
        let kernel = Mutex::new(Kernel::new(KernelConfig::default()));
        let folder = profile.join("extensions/my-glow");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("manifest.json"), "{}").unwrap();
        super::archive(&kernel, &profile, "my-glow").unwrap();
        assert!(!folder.exists());
        let archived: Vec<_> = fs::read_dir(profile.join("extensions/.archive"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(archived.len(), 1);
        assert!(archived[0].path().join("manifest.json").is_file());
        assert!(super::archive(&kernel, &profile, "my-glow").is_err());
        assert!(super::archive(&kernel, &profile, "../escape").is_err());
    }
}
