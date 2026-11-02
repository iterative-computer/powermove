//! Store install/permission CAS on the existing kernel writer and durable state
//! transaction. No second profile writer, renderer state, or home discovery.
use pm_kernel::{
    Kernel,
    state::{ExtensionEntry, StateOp, StateTransaction},
};
use pm_store_host::{
    Error, Result,
    identity::ExtensionId,
    install::{ExtensionState, StoreKernel},
};

const SLOT: &str = "storeHostV1";

pub struct KernelStoreAdapter<'a> {
    kernel: &'a mut Kernel,
}
impl<'a> KernelStoreAdapter<'a> {
    pub fn new(kernel: &'a mut Kernel) -> Self {
        Self { kernel }
    }
}
impl StoreKernel for KernelStoreAdapter<'_> {
    fn load(&self, id: &ExtensionId) -> Result<Option<ExtensionState>> {
        let Some(entry) = self.kernel.state().extensions.get(id.as_str()) else {
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
            .state_transaction(StateTransaction {
                expected_revision: Some(snapshot.revision),
                ops: vec![StateOp::Extension(entry)],
            })
            .map_err(|error| Error::Io(error.to_string()))?;
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
    fn kernel(profile: &std::path::Path) -> Kernel {
        let mut kernel = Kernel::new(KernelConfig::default());
        assert!(matches!(
            kernel
                .shell(ShellCall::AttachLibrary {
                    profile: profile.into(),
                    format: DiskFormat::V1_2,
                    electron_profile: None,
                })
                .unwrap(),
            ShellReply::Library(pm_kernel::locks::OpenDecision::Writable)
        ));
        kernel
    }
    #[test]
    fn store_state_is_durable_through_the_kernel_writer_and_cas_refuses_stale_or_foreign_identity()
    {
        let profile = tempfile::tempdir().unwrap();
        let id = ExtensionId::new("synthetic-store").unwrap();
        let initial = state(&id, "synthetic-profile");
        {
            let mut kernel = kernel(profile.path());
            let mut store = KernelStoreAdapter::new(&mut kernel);
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
        let mut kernel = kernel(profile.path());
        let mut store = KernelStoreAdapter::new(&mut kernel);
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
        let mut store = KernelStoreAdapter::new(&mut kernel);
        assert_eq!(
            store.compare_exchange(&id, None, &state(&id, "synthetic-profile")),
            Err(Error::Conflict)
        );
    }
}
