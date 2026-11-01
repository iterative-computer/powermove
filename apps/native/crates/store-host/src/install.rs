use crate::{
    Error, Result,
    artifact::{ArtifactStore, SignedRelease, VerifiedRelease},
    identity::{DataStoreIdentity, ExtensionId},
    permission::{Manifest, Permission, PermissionDecision},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledRelease {
    pub manifest: Manifest,
    pub publisher: String,
    pub release_id: String,
    pub bundle: String,
    pub proof: SignedRelease,
    /// Kernel-owned reference to preserved user-authored source changes. An
    /// update cannot overwrite this; conflict candidates stay alongside it.
    pub local_overlay: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveReason {
    Updated,
    Uninstalled,
    RolledBack,
    ConflictCandidate,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchivedRelease {
    pub id: u64,
    pub reason: ArchiveReason,
    pub release: InstalledRelease,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataStoreStatus {
    Active,
    Archived,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionState {
    pub revision: u64,
    pub data_store: DataStoreIdentity,
    pub data_status: DataStoreStatus,
    pub active: Option<InstalledRelease>,
    pub archives: Vec<ArchivedRelease>,
    pub permissions: BTreeMap<Permission, PermissionDecision>,
}
/// Implement on the single kernel writer. `compare_exchange` durably commits
/// the ENTIRE state or nothing, checks expected revision, and rejects states
/// whose identity differs from the existing one. Failure must not publish.
/// Worker APIs only: UI uses the actor's nonblocking ticket/command adapter.
pub trait StoreKernel {
    fn load(&self, extension: &ExtensionId) -> Result<Option<ExtensionState>>;
    fn compare_exchange(
        &mut self,
        extension: &ExtensionId,
        expected: Option<u64>,
        next: &ExtensionState,
    ) -> Result<()>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    Activated { revision: u64 },
    ConflictPreserved { revision: u64, archive: u64 },
}

fn advance(state: &mut ExtensionState) -> Result<()> {
    state.revision = state
        .revision
        .checked_add(1)
        .ok_or(Error::Invalid("revision exhausted"))?;
    Ok(())
}
fn archive_active(state: &mut ExtensionState, reason: ArchiveReason) {
    if let Some(release) = state.active.take() {
        state.archives.push(ArchivedRelease {
            id: state.revision,
            reason,
            release,
        });
    }
}
fn check_expected(state: &Option<ExtensionState>, expected: Option<u64>) -> Result<()> {
    if state.as_ref().map(|s| s.revision) != expected {
        return Err(Error::Conflict);
    }
    Ok(())
}

/// Verify/download before invoking. Durable immutable bytes precede atomic
/// publication. A failed write/commit leaves old code and data fully reachable;
/// interrupted candidates remain archived on disk. Retry reads kernel state.
pub fn install<K: StoreKernel, A: ArtifactStore>(
    kernel: &mut K,
    artifacts: &mut A,
    profile: &str,
    verified: &VerifiedRelease,
    expected: Option<u64>,
) -> Result<InstallOutcome> {
    let id = &verified.manifest.id;
    let previous = kernel.load(id)?;
    check_expected(&previous, expected)?;
    let data_store = DataStoreIdentity::new(profile, id.clone())?;
    if let Some(old) = &previous {
        if old.data_store != data_store {
            return Err(Error::Conflict);
        }
        // Publisher transfers require a separately authorized migration, never a
        // release's self-declared publisher replacing an installed identity.
        if let Some(pinned) = old
            .active
            .as_ref()
            .or_else(|| old.archives.last().map(|a| &a.release))
            && pinned.publisher != verified.statement.publisher
        {
            return Err(Error::Denied);
        }
    }
    artifacts.preserve(verified)?;
    let mut state = previous.unwrap_or(ExtensionState {
        revision: 0,
        data_store,
        data_status: DataStoreStatus::Archived,
        active: None,
        archives: Vec::new(),
        permissions: BTreeMap::new(),
    });
    advance(&mut state)?;
    let candidate = InstalledRelease {
        manifest: verified.manifest.clone(),
        publisher: verified.statement.publisher.clone(),
        release_id: verified.statement.release_id.clone(),
        bundle: verified.statement.archive_sha256.clone(),
        proof: verified.proof.clone(),
        local_overlay: None,
    };
    let outcome = if state
        .active
        .as_ref()
        .is_some_and(|a| a.local_overlay.is_some())
    {
        state.archives.push(ArchivedRelease {
            id: state.revision,
            reason: ArchiveReason::ConflictCandidate,
            release: candidate,
        });
        InstallOutcome::ConflictPreserved {
            revision: state.revision,
            archive: state.revision,
        }
    } else {
        archive_active(&mut state, ArchiveReason::Updated);
        state.active = Some(candidate);
        state.data_status = DataStoreStatus::Active;
        InstallOutcome::Activated {
            revision: state.revision,
        }
    };
    kernel.compare_exchange(id, expected, &state)?;
    Ok(outcome)
}

pub fn uninstall<K: StoreKernel>(kernel: &mut K, id: &ExtensionId, expected: u64) -> Result<u64> {
    let mut state = kernel.load(id)?.ok_or(Error::NotInstalled)?;
    if state.revision != expected {
        return Err(Error::Conflict);
    }
    if state.active.is_none() {
        return Err(Error::NotInstalled);
    }
    advance(&mut state)?;
    archive_active(&mut state, ArchiveReason::Uninstalled);
    state.data_status = DataStoreStatus::Archived;
    // Identity, decisions, user overlays and all kernel ext.<id> data survive.
    kernel.compare_exchange(id, Some(expected), &state)?;
    Ok(state.revision)
}

/// Select an explicit archived version, including local overrides. Never removes
/// the chosen archive or rewinds JSON/user data to the old code's snapshot.
/// The kernel adapter must revoke the old view/realm generation when publishing
/// an install/uninstall/rollback, before issuing a replacement capability.
pub fn rollback<K: StoreKernel>(
    kernel: &mut K,
    id: &ExtensionId,
    expected: u64,
    archive: u64,
) -> Result<u64> {
    let mut state = kernel.load(id)?.ok_or(Error::NotInstalled)?;
    if state.revision != expected {
        return Err(Error::Conflict);
    }
    let target = state
        .archives
        .iter()
        .find(|a| a.id == archive)
        .ok_or(Error::Invalid("archive id"))?
        .release
        .clone();
    advance(&mut state)?;
    archive_active(&mut state, ArchiveReason::RolledBack);
    state.active = Some(target);
    state.data_status = DataStoreStatus::Active;
    kernel.compare_exchange(id, Some(expected), &state)?;
    Ok(state.revision)
}

/// Linux/reference adapter, deliberately explicit about being in-memory. Real
/// permission/install durability belongs to pm-kernel, not a second store writer.
#[derive(Debug, Default)]
pub struct MemoryKernel {
    states: BTreeMap<ExtensionId, ExtensionState>,
    pub fail_next_commit: bool,
}
impl StoreKernel for MemoryKernel {
    fn load(&self, extension: &ExtensionId) -> Result<Option<ExtensionState>> {
        Ok(self.states.get(extension).cloned())
    }
    fn compare_exchange(
        &mut self,
        id: &ExtensionId,
        expected: Option<u64>,
        next: &ExtensionState,
    ) -> Result<()> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(Error::Io("injected commit failure".into()));
        }
        let current = self.states.get(id);
        if current.map(|s| s.revision) != expected {
            return Err(Error::Conflict);
        }
        if next.data_store.extension != *id
            || next.revision
                != expected
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or(Error::Invalid("revision"))?
            || current.is_some_and(|s| s.data_store != next.data_store)
        {
            return Err(Error::Invalid("state identity/revision"));
        }
        self.states.insert(id.clone(), next.clone());
        Ok(())
    }
}
