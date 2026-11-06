//! Asynchronous Sparkle 2.10.1 policy binding. No extraction/replacement in Rust.
//! The app must keep a migration/import lease and route EVERY termination
//! through `allow_termination` while a session is armed (including normal Quit).
use crate::update_distribution::*;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Checking,
    Downloading,
    Extracting,
    Ready,
    InstallRequested,
    Aborted,
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Found,
    Downloading,
    Extracting,
    Ready,
    Installed,
    Aborted,
    Cancelled,
}
/// Main-run-loop capability. No synchronous wait, invented archive-adoption API,
/// ready ticket, or persisted-state success fallback.
pub trait Binding {
    fn check(&mut self, release: &Release) -> Result<()>;
    fn download(&mut self) -> Result<()>;
    fn install(&mut self) -> Result<()>;
    fn cancel(&mut self) -> Result<()>;
}
/// An application capability that installs a termination veto BEFORE Sparkle
/// downloads/extracts. The guard owns fresh import/migration leases; keeping only
/// Sparkle's ready callback is insufficient. Keep this value alive for the cycle.
pub trait TerminationGuard {
    /// Verify the candidate bound to these exact archive bytes: full signed
    /// bundle id/version/architecture, Team/DR continuity and notarization.
    /// Sparkle's EdDSA key supports identity rotation; it does NOT by itself
    /// enforce this application's stricter same-DR policy. Missing proof is false.
    fn verify_candidate(&mut self, archive: &std::path::Path, release: &Release) -> Result<bool>;
    fn protect(&mut self) -> Result<()>;
    fn protected(&self) -> bool;
}
/// Archive key pinned in the signed bundle's SUPublicEDKey, independent of the
/// JSON policy key. Verify the retained SHA-256 archive against this exact EdDSA
/// signature BEFORE Sparkle downloads again. Two independently valid envelopes
/// cannot accidentally bind policy bytes A to Sparkle bytes B.
pub struct ArchiveKey(ed25519_dalek::VerifyingKey);
impl ArchiveKey {
    pub fn from_base64(key: &str) -> Result<Self> {
        use base64::Engine;
        let bytes: [u8; 32] = base64::engine::general_purpose::STANDARD
            .decode(key)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid Sparkle public key"))?;
        let key = ed25519_dalek::VerifyingKey::from_bytes(&bytes)?;
        ensure!(!key.is_weak(), "weak Sparkle public key");
        Ok(Self(key))
    }
    fn verify(&self, path: &std::path::Path, release: &Release) -> Result<()> {
        use base64::Engine;
        let signature = ed25519_dalek::Signature::from_slice(
            &base64::engine::general_purpose::STANDARD.decode(&release.sparkle_signature)?,
        )?;
        let file = std::fs::File::open(path)?;
        ensure!(
            file.metadata()?.len() == release.size,
            "archive length changed"
        );
        // The process-exclusive StagingStore owns this private immutable file.
        // Mapping avoids allocating an archive-sized heap buffer (up to 16 GiB).
        let bytes = unsafe { memmap2::MmapOptions::new().map(&file)? };
        self.0.verify_strict(&bytes, &signature)?;
        Ok(())
    }
}
pub struct ReferenceArchive<'a> {
    pub file: String,
    pub key: &'a ArchiveKey,
}

pub struct Session<'a, G> {
    store: &'a StagingStore,
    offer: &'a Offer,
    file: String,
    phase: Phase,
    guard: G,
}
impl<'a, G: TerminationGuard> Session<'a, G> {
    /// Only an independently SHA-256 verified download may authorize a cycle.
    pub fn begin(
        store: &'a StagingStore,
        offer: &'a Offer,
        archive: ReferenceArchive<'_>,
        mut guard: G,
        readiness: &mut impl Readiness,
        verifier: &impl SignatureVerifier,
        binding: &mut impl Binding,
    ) -> Result<Self> {
        ensure!(readiness.current()?.allows_native(), "update gate closed");
        store.validate_offer(offer, verifier, readiness.now_unix()?)?;
        let file = archive.file;
        let path = store.verify_archive(offer, &file)?;
        archive.key.verify(&path, offer.release())?;
        ensure!(
            guard.verify_candidate(&path, offer.release())?,
            "exact candidate identity/DR proof unavailable"
        );
        ensure!(
            matches!(store.status()?.map(|(_, stage)| stage), Some(Stage::Downloaded { file: current }) if current == file),
            "recover existing Sparkle cycle first"
        );
        guard.protect()?;
        ensure!(
            guard.protected(),
            "application termination guard unavailable"
        );
        ensure!(
            readiness.current()?.allows_native(),
            "migration changed while arming updater"
        );
        store.validate_offer(offer, verifier, readiness.now_unix()?)?;
        let session = Self {
            store,
            offer,
            file,
            phase: Phase::Checking,
            guard,
        };
        session.record()?; // durable before Sparkle can contact the appcast
        binding.check(offer.release())?;
        Ok(session)
    }
    fn record(&self) -> Result<()> {
        self.store.append(
            self.offer,
            Stage::Sparkle {
                file: self.file.clone(),
                phase: self.phase,
            },
        )
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn event(
        &mut self,
        event: Event,
        readiness: &mut impl Readiness,
        verifier: &impl SignatureVerifier,
        binding: &mut impl Binding,
    ) -> Result<()> {
        if event == Event::Cancelled {
            ensure!(
                self.phase != Phase::InstallRequested,
                "installer cancellation needs recovery proof"
            );
            self.phase = Phase::Cancelled;
            return self.record();
        }
        if self.phase == Phase::Aborted {
            binding.cancel()?;
            return Ok(());
        }
        if event == Event::Aborted {
            self.phase = Phase::Aborted;
            return self.record();
        }
        // Installed callbacks are advisory. Only signed-identity recovery below
        // can publish Completed; a relaunch notification is not signed proof.
        if event == Event::Installed {
            return Ok(());
        }
        if !self.guard.protected()
            || !readiness.current()?.allows_native()
            || self
                .store
                .validate_offer(self.offer, verifier, readiness.now_unix()?)
                .is_err()
        {
            binding.cancel()?;
            self.phase = Phase::Aborted;
            self.record()?;
            anyhow::bail!("update eligibility changed");
        }
        let next = match (self.phase, event) {
            (Phase::Checking, Event::Found) => Phase::Downloading,
            (Phase::Downloading, Event::Downloading) => Phase::Downloading,
            (Phase::Downloading, Event::Extracting) => Phase::Extracting,
            (Phase::Extracting, Event::Ready) => Phase::Ready,
            _ => anyhow::bail!("unexpected Sparkle callback"),
        };
        self.store.verify_archive(self.offer, &self.file)?;
        self.phase = next;
        self.record()?;
        if event == Event::Found {
            binding.download()?;
        }
        Ok(())
    }
    /// Cancel a manual/background cycle without force-quitting. The guard stays
    /// held until the native binding confirms cancellation; extraction may need
    /// to reach its ready callback before Sparkle accepts Skip.
    pub fn cancel(&mut self, binding: &mut impl Binding) -> Result<()> {
        ensure!(
            self.phase != Phase::InstallRequested,
            "installation already requested; inspect recovery"
        );
        self.phase = Phase::Aborted;
        self.record()?;
        binding.cancel()
    }
    /// Call for EVERY termination, not just updater-triggered quits. A decline
    /// retains the staged update and guard; retry needs another ordinary barrier.
    pub fn allow_termination(
        &mut self,
        readiness: &mut impl Readiness,
        verifier: &impl SignatureVerifier,
        quit: &mut impl QuitBarrier,
        binding: &mut impl Binding,
    ) -> Result<bool> {
        if self.phase == Phase::Cancelled {
            return quit.ready();
        }
        if !matches!(self.phase, Phase::Ready | Phase::InstallRequested)
            || !self.guard.protected()
            || !quit.ready()?
        {
            return Ok(false);
        }
        ensure!(
            self.guard.protected() && readiness.current()?.allows_native(),
            "migration changed during quit"
        );
        self.store
            .validate_offer(self.offer, verifier, readiness.now_unix()?)?;
        self.store.verify_archive(self.offer, &self.file)?;
        if self.phase == Phase::Ready {
            self.phase = Phase::InstallRequested;
            self.record()?;
            binding.install()?;
        }
        Ok(true)
    }
}
/// Must inspect the installed bundle's full version, architecture, exact id,
/// designated requirement and valid signature. Never infer this from a receipt.
pub trait InstalledProof {
    fn matches(&mut self, release: &Release) -> Result<bool>;
}
pub fn recover(
    store: &StagingStore,
    offer: &Offer,
    verifier: &impl SignatureVerifier,
    proof: &mut impl InstalledProof,
) -> Result<Recovery> {
    // Recovery needs authentic policy but remains possible after expiry.
    let Some((id, state)) = store.status()? else {
        return Ok(Recovery::Pending);
    };
    ensure!(id == offer.id(), "wrong recovery offer");
    store.validate_offer(offer, verifier, 0)?;
    if matches!(
        state,
        Stage::Sparkle {
            phase: Phase::InstallRequested,
            ..
        } | Stage::Completed
    ) && proof.matches(offer.release())?
    {
        store.append(offer, Stage::Completed)?;
        return Ok(Recovery::Installed);
    }
    // No public Sparkle API certifies a cached extracted ticket after restart.
    // Keep all receipts/caches. The app may explicitly restart an eligible cycle.
    Ok(Recovery::Pending)
}
#[cfg(all(target_os = "macos", feature = "sparkle-macos"))]
pub mod macos;
