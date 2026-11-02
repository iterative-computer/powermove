//! Admission for platform/runtime adapters. Grants never come from page JSON.
use crate::{
    Error, Result,
    identity::{DataStoreIdentity, ExtensionId},
    install::{DataStoreStatus, StoreKernel},
    permission::{self, Admission, MethodPolicy, Permission},
};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct StoreSession {
    pub data_store: DataStoreIdentity,
    pub release: String,
    pub revision: u64,
    pub grants: BTreeSet<Permission>,
}
impl StoreSession {
    pub fn admit(kernel: &impl StoreKernel, id: &ExtensionId) -> Result<Self> {
        let state = kernel.load(id)?.ok_or(Error::NotInstalled)?;
        let release = state.active.as_ref().ok_or(Error::NotInstalled)?;
        if state.data_status != DataStoreStatus::Active
            || state.data_store.extension != *id
            || release.manifest.id != *id
        {
            return Err(Error::Denied);
        }
        state.data_store.webkit_uuid()?;
        let mut grants = BTreeSet::new();
        for permission in &release.manifest.permissions {
            if *permission == Permission::FullAccess {
                return Err(Error::Invalid("full access requires trusted realm"));
            }
            match permission::request(kernel, id, *permission)? {
                Admission::Granted => {
                    grants.insert(*permission);
                }
                Admission::RequiresTrustedRealm => {
                    return Err(Error::Invalid("full access requires trusted realm"));
                }
                Admission::Denied => (),
                Admission::Prompt(_) => return Err(Error::Denied),
            }
        }
        Ok(Self {
            data_store: state.data_store,
            release: release.release_id.clone(),
            revision: state.revision,
            grants,
        })
    }
    /// Re-check live installation, publisher decisions and release revision on
    /// every call. Cached grants are presentation metadata, never authority.
    pub fn authorize(
        &self,
        kernel: &impl StoreKernel,
        policy: &MethodPolicy,
        method: &str,
    ) -> Result<()> {
        let state = kernel
            .load(&self.data_store.extension)?
            .ok_or(Error::NotInstalled)?;
        if state.revision != self.revision
            || state.data_store != self.data_store
            || state.active.as_ref().map(|r| &r.release_id) != Some(&self.release)
        {
            return Err(Error::Stale);
        }
        policy.authorize(kernel, &self.data_store.extension, method)
    }
}
