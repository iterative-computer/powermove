//! Store install/permission CAS on the existing kernel writer and durable state
//! transaction. No second profile writer, renderer state, or home discovery.
use pm_kernel::{
    Kernel, KernelHandle,
    state::{ExtensionEntry, StateOp, StateSnapshot, StateTransaction},
};
use pm_store_host::{
    Error, Result,
    identity::ExtensionId,
    install::{ExtensionState, StoreKernel},
};

use std::sync::{Arc, Mutex};

const SLOT: &str = "storeHostV1";

pub struct StoreArchive<'a> {
    pub profile: &'a str,
    pub release: &'a pm_store_host::artifact::SignedRelease,
    pub archive: &'a [u8],
    pub trust: &'a pm_store_host::artifact::TrustStore,
    pub expected: Option<u64>,
}

/// What Store code needs from the single writer. The app hands in its actor
/// handle; every call runs on a worker, so waiting on the actor here is allowed.
pub trait StoreAuthority: Send + Sync + 'static {
    fn state(&self) -> Arc<StateSnapshot>;
    fn transact(&self, tx: StateTransaction) -> std::result::Result<(), String>;
    /// `(generation, realized project)` of an open project.
    fn project(&self, project: &str) -> Option<(u64, pm_model::Value)>;
}
impl StoreAuthority for KernelHandle {
    fn state(&self) -> Arc<StateSnapshot> {
        KernelHandle::state(self)
    }
    fn transact(&self, tx: StateTransaction) -> std::result::Result<(), String> {
        self.state_transaction(tx)
            .wait()
            .ok_or("kernel stopped")?
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn project(&self, project: &str) -> Option<(u64, pm_model::Value)> {
        let published = self.published(project)?;
        let document = published.snapshot.document.to_value()?;
        Some((
            published.revision() + 1,
            pm_model::canonical::realize_project(document),
        ))
    }
}
impl StoreAuthority for Mutex<Kernel> {
    fn state(&self) -> Arc<StateSnapshot> {
        Arc::new(self.lock().unwrap().state().clone())
    }
    fn transact(&self, tx: StateTransaction) -> std::result::Result<(), String> {
        self.lock()
            .unwrap()
            .state_transaction(tx)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn project(&self, project: &str) -> Option<(u64, pm_model::Value)> {
        let kernel = self.lock().unwrap();
        let project = kernel.project(project)?;
        let snapshot = project.store().snapshot();
        Some((
            project.store().revision() + 1,
            pm_model::canonical::realize_project(snapshot.document.to_value()?),
        ))
    }
}

pub struct KernelStoreAdapter<'a> {
    kernel: &'a dyn StoreAuthority,
}
impl<'a> KernelStoreAdapter<'a> {
    pub fn new(kernel: &'a dyn StoreAuthority) -> Self {
        Self { kernel }
    }
    /// Worker-only release admission. Pins come from the native Store registry;
    /// catalog checksums and extension input JSON cannot provision trust. Return
    /// the exact verified virtual files for the compiler after durable install.
    /// Permission prompts/grants remain the separate reviewed native flow.
    pub fn install_archive(
        &mut self,
        artifacts: &mut impl pm_store_host::artifact::ArtifactStore,
        input: StoreArchive<'_>,
    ) -> Result<(
        pm_store_host::install::InstallOutcome,
        pm_store_host::artifact::VerifiedRelease,
    )> {
        let verified = pm_store_host::artifact::verify(
            input.release,
            input.archive,
            input.trust,
            pm_store_host::artifact::Limits::default(),
        )?;
        let outcome = pm_store_host::install::install(
            self,
            artifacts,
            input.profile,
            &verified,
            input.expected,
        )?;
        Ok((outcome, verified))
    }
}
impl StoreKernel for KernelStoreAdapter<'_> {
    fn load(&self, id: &ExtensionId) -> Result<Option<ExtensionState>> {
        let state = self.kernel.state();
        let Some(entry) = state.extensions.get(id.as_str()) else {
            return Ok(None);
        };
        let Some(value) = entry.source.get(SLOT) else {
            // A Store install cannot overwrite a local extension with this id.
            return if entry.installed {
                Err(Error::Conflict)
            } else {
                Ok(None)
            };
        };
        let state: ExtensionState =
            serde_json::from_str(&pm_model::json::to_canonical_string(value))
                .map_err(|_| Error::Invalid("kernel Store state"))?;
        if state.data_store.extension != *id {
            return Err(Error::Invalid("kernel Store identity"));
        }
        Ok(Some(state))
    }
    fn compare_exchange(
        &mut self,
        id: &ExtensionId,
        expected: Option<u64>,
        next: &ExtensionState,
    ) -> Result<()> {
        let current = self.load(id)?;
        if current.as_ref().map(|state| state.revision) != expected {
            return Err(Error::Conflict);
        }
        if next.data_store.extension != *id
            || next.revision
                != expected
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or(Error::Invalid("Store revision"))?
            || current
                .as_ref()
                .is_some_and(|state| state.data_store != next.data_store)
        {
            return Err(Error::Invalid("Store state identity/revision"));
        }
        let snapshot = self.kernel.state();
        let previous = snapshot.extensions.get(id.as_str());
        let mut source = previous
            .and_then(|entry| entry.source.as_object())
            .cloned()
            .unwrap_or_default();
        let value = serde_json::to_string(next).map_err(|_| Error::Invalid("Store state"))?;
        source.insert(
            SLOT,
            pm_model::json::parse(&value).map_err(|_| Error::Invalid("Store state"))?,
        );
        let entry = ExtensionEntry {
            id: id.as_str().into(),
            installed: next.active.is_some(),
            enabled: previous.is_none_or(|entry| entry.enabled),
            version: next
                .active
                .as_ref()
                .map(|release| release.manifest.version.clone())
                .unwrap_or_default(),
            source: pm_model::Value::Object(source),
        };
        self.kernel
            .transact(StateTransaction {
                expected_revision: Some(snapshot.revision),
                ops: vec![StateOp::Extension(entry)],
            })
            .map_err(Error::Io)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::{
        KernelConfig,
        persistence::DiskFormat,
        session::{ShellCall, ShellReply},
    };
    use pm_store_host::{identity::DataStoreIdentity, install::DataStoreStatus};
    use std::collections::BTreeMap;

    fn state(id: &ExtensionId, namespace: &str) -> ExtensionState {
        ExtensionState {
            revision: 1,
            data_store: DataStoreIdentity::new(namespace, id.clone()).unwrap(),
            data_status: DataStoreStatus::Archived,
            active: None,
            archives: vec![],
            permissions: BTreeMap::new(),
        }
    }
    fn kernel(profile: &std::path::Path, stage: &str) -> Kernel {
        let mut kernel = Kernel::new(KernelConfig::default());
        let profile = profile.canonicalize().unwrap();
        let reply = kernel
            .shell(ShellCall::AttachLibrary {
                profile: profile.clone(),
                format: DiskFormat::V1_2,
                electron_profile: None,
            })
            .unwrap();
        assert!(
            matches!(
                reply,
                ShellReply::Library(pm_kernel::locks::OpenDecision::Writable)
            ),
            "synthetic library {stage} admission refused: {reply:?}; lock={:?}",
            std::fs::read_to_string(profile.join("store/library.lock"))
        );
        kernel
    }
    #[test]
    fn store_state_is_durable_through_the_kernel_writer_and_cas_refuses_stale_or_foreign_identity()
    {
        let profile = tempfile::tempdir().unwrap();
        let id = ExtensionId::new("synthetic-store").unwrap();
        let initial = state(&id, "synthetic-profile");
        {
            let kernel = Mutex::new(kernel(profile.path(), "initial"));
            let mut store = KernelStoreAdapter::new(&kernel);
            store.compare_exchange(&id, None, &initial).unwrap();
            assert_eq!(
                store.compare_exchange(&id, None, &initial),
                Err(Error::Conflict)
            );
            let mut foreign = state(&id, "synthetic-other-profile");
            foreign.revision = 2;
            assert_eq!(
                store.compare_exchange(&id, Some(1), &foreign),
                Err(Error::Invalid("Store state identity/revision"))
            );
        }
        let kernel = Mutex::new(kernel(profile.path(), "reopen"));
        let mut store = KernelStoreAdapter::new(&kernel);
        assert_eq!(store.load(&id).unwrap(), Some(initial.clone()));
        let mut next = initial;
        next.revision = 2;
        store.compare_exchange(&id, Some(1), &next).unwrap();
        assert_eq!(store.load(&id).unwrap(), Some(next));
    }
    #[test]
    fn store_install_does_not_replace_local_extension_metadata() {
        let mut kernel = Kernel::new(KernelConfig::default());
        let id = ExtensionId::new("synthetic-local").unwrap();
        kernel
            .state_transaction(StateTransaction {
                expected_revision: None,
                ops: vec![StateOp::Extension(ExtensionEntry {
                    id: id.as_str().into(),
                    installed: true,
                    enabled: true,
                    version: "1.0.0".into(),
                    source: pm_model::Value::Null,
                })],
            })
            .unwrap();
        let kernel = Mutex::new(kernel);
        let mut store = KernelStoreAdapter::new(&kernel);
        assert_eq!(
            store.compare_exchange(&id, None, &state(&id, "synthetic-profile")),
            Err(Error::Conflict)
        );
    }
    #[test]
    fn signed_archive_installs_through_native_kernel_and_tampering_never_publishes() {
        use ed25519_dalek::{Signer, SigningKey};
        use pm_store_host::adapter::StoreSession;
        use pm_store_host::artifact::{
            FileArtifacts, FileSeal, ReleaseStatement, SignedRelease, TrustStore,
        };
        use sha2::{Digest, Sha256};
        fn hash(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
        let id = ExtensionId::new("synthetic-signed-store").unwrap();
        let entries=[("manifest.json",serde_json::to_vec(&serde_json::json!({"id":id,"name":"Synthetic signed fixture","version":"1.0.0","apiVersion":3,"permissions":[]})).unwrap()),("index.js",b"export default () => {}".to_vec())];
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut files = BTreeMap::new();
        for (path, bytes) in &entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, bytes.as_slice())
                .unwrap();
            files.insert(
                path.to_string(),
                FileSeal {
                    sha256: hash(bytes),
                    bytes: bytes.len() as u64,
                },
            );
        }
        let archive = tar.into_inner().unwrap().finish().unwrap();
        // Public synthetic seed; never a publisher credential.
        let key = SigningKey::from_bytes(&[7; 32]);
        let statement = ReleaseStatement {
            format: 1,
            key_id: "synthetic-key".into(),
            publisher: "synthetic-publisher".into(),
            extension: id.clone(),
            release_id: "synthetic-release".into(),
            archive_sha256: hash(&archive),
            files,
        };
        let mut release = SignedRelease {
            signature: key
                .sign(&statement.signing_bytes().unwrap())
                .to_bytes()
                .to_vec(),
            statement,
        };
        let mut trust = TrustStore::default();
        trust
            .trust(
                "synthetic-publisher".into(),
                "synthetic-key".into(),
                key.verifying_key(),
            )
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut artifacts = FileArtifacts::new(root.path().canonicalize().unwrap()).unwrap();
        let kernel = Mutex::new(Kernel::new(KernelConfig::default()));
        let mut store = KernelStoreAdapter::new(&kernel);
        release.signature[0] ^= 1;
        assert!(
            store
                .install_archive(
                    &mut artifacts,
                    StoreArchive {
                        profile: "synthetic-profile",
                        release: &release,
                        archive: &archive,
                        trust: &trust,
                        expected: None
                    }
                )
                .is_err()
        );
        assert!(store.load(&id).unwrap().is_none());
        release.signature[0] ^= 1;
        let (outcome, verified) = store
            .install_archive(
                &mut artifacts,
                StoreArchive {
                    profile: "synthetic-profile",
                    release: &release,
                    archive: &archive,
                    trust: &trust,
                    expected: None,
                },
            )
            .unwrap();
        assert_eq!(
            outcome,
            pm_store_host::install::InstallOutcome::Activated { revision: 1 }
        );
        assert_eq!(verified.file("index.js").unwrap(), entries[1].1);
        assert!(StoreSession::admit(&store, &id).is_ok());
        assert!(
            store
                .install_archive(
                    &mut artifacts,
                    StoreArchive {
                        profile: "synthetic-profile",
                        release: &release,
                        archive: &archive,
                        trust: &trust,
                        expected: None
                    }
                )
                .is_err()
        );
        assert_eq!(store.load(&id).unwrap().unwrap().revision, 1);
    }
}
