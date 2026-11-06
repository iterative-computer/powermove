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

pub fn pinned() -> Result<TrustStore> {
    let mut trust = TrustStore::default();
    for (publisher, key, bytes) in PINNED_KEYS {
        let key_bytes = ed25519_dalek::VerifyingKey::from_bytes(bytes)
            .map_err(|_| Error::Invalid("pinned Store key"))?;
        trust
            .trust((*publisher).into(), (*key).into(), key_bytes)
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
    let mut artifacts = FileArtifacts::new(profile.join("store-artifacts"))
        .map_err(|e| Error::Host(format!("{e:?}")))?;
    let (_, verified) = KernelStoreAdapter::new(kernel)
        .install_archive(
            &mut artifacts,
            StoreArchive {
                profile: namespace,
                release,
                archive,
                trust,
                expected: None,
            },
        )
        .map_err(|e| Error::Host(format!("{e:?}")))?;
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
        let target = root.join(&id);
        let _ = fs::remove_dir_all(&target);
        fs::rename(&staging, target)
    };
    write().map_err(|e| Error::Host(e.to_string()))?;
    Ok(id)
}

/// Name, release and repository of each listing, for the one page a menu shows.
pub fn catalog() -> Result<Vec<Value>> {
    let page = net::get(
        &format!("{ORIGIN}/v1/store/extensions?sort=name&limit=50"),
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

/// Downloads a release and its signed envelope, verifies, installs.
pub fn install(
    kernel: &dyn StoreAuthority,
    profile: &Path,
    namespace: &str,
    release_id: &str,
    repo: &str,
) -> Result<String> {
    if PINNED_KEYS.is_empty() {
        return Err(Error::Host(
            "Store installs need a signed release, and the Store doesn't sign releases yet.".into(),
        ));
    }
    let trust = pinned()?;
    let envelope = net::get(
        &format!("{ORIGIN}/v1/store/releases/{release_id}/signature"),
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
    let archive = net::get(
        &format!("{ORIGIN}/v1/store/releases/{release_id}/tar"),
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
        &trust,
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
        let files: [(&str, &[u8]); 2] = [("manifest.json", manifest.as_bytes()), ("index.ts", source)];
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
        assert_eq!(found[0].store.as_ref().unwrap()["releaseId"], "synthetic-release");
    }
}
