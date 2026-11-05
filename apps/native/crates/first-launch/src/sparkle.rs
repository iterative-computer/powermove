//! Legacy synchronous external-archive policy bridge, retained for existing
//! injected-capability consumers/tests. Sparkle 2.10.1 has no public archive-
//! adoption API. The linked asynchronous production route now lives in
//! `pm_update_policy::sparkle`; do not implement this trait using a fabricated
//! stage selector or a ticket returned before authentication/extraction.
use anyhow::{Result, ensure};
use pm_update_policy::update_distribution::{Installer, Recovery, Release, Stage};
use sha2::{Digest, Sha256};
use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path};

/// Exact enclosure selected by the custom user driver, not "latest appcast".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enclosure {
    pub version: String,
    pub archive_url: String,
    pub size: u64,
    pub signature: String,
}
impl Enclosure {
    fn matches(&self, release: &Release) -> bool {
        self.version == release.version
            && self.archive_url == release.archive_url
            && self.size == release.size
            && self.signature == release.sparkle_signature
    }
}
/// Legacy externally staged installer capability; not Sparkle's public API.
/// A ticket is returned only after Sparkle's authenticated extraction/DR/Team,
/// architecture, OS, space and notarization checks complete. No success fallback.
/// Operations are idempotent by the signed policy hash `id`, including recovery
/// after process death. `recover` must inspect Sparkle's persisted state, never an
/// in-memory cache. Errors retain archives, helpers, checkpoints and owned apps.
pub trait Sparkle {
    fn selected_enclosure(&mut self, id: &str) -> Result<Enclosure>;
    fn stage_verified(&mut self, id: &str, archive: &Path, release: &Release) -> Result<String>;
    fn recover(&mut self, id: &str, last: &Stage) -> Result<Recovery>;
    fn install_on_normal_quit(&mut self, id: &str, ticket: &str) -> Result<()>;
}
pub struct Driver<S> {
    pub sparkle: S,
}
impl<S: Sparkle> Installer for Driver<S> {
    fn available_space(&mut self, root: &Path) -> Result<u64> {
        Ok(fs2::available_space(root)?)
    }
    fn stage(&mut self, id: &str, archive: &Path, release: &Release) -> Result<String> {
        ensure!(
            id.len() == 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "invalid policy id"
        );
        ensure!(
            self.sparkle.selected_enclosure(id)?.matches(release),
            "Sparkle enclosure differs from signed policy"
        );
        // Do not trust the appcast's name or an earlier download verification.
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(archive)?;
        ensure!(input.metadata()?.is_file(), "archive is not a file");
        let mut digest = Sha256::new();
        let mut size = 0u64;
        let mut buf = [0u8; 65536];
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            size = size
                .checked_add(n as u64)
                .ok_or_else(|| anyhow::anyhow!("archive size overflow"))?;
            ensure!(size <= release.size, "archive exceeds signed size");
            digest.update(&buf[..n]);
        }
        ensure!(
            size == release.size && format!("{:x}", digest.finalize()) == release.sha256,
            "archive differs from signed policy"
        );
        let ticket = self.sparkle.stage_verified(id, archive, release)?;
        ensure!(
            !ticket.is_empty() && ticket.len() <= 4096,
            "invalid Sparkle ticket"
        );
        Ok(ticket)
    }
    fn recover(&mut self, id: &str, last: &Stage) -> Result<Recovery> {
        self.sparkle.recover(id, last)
    }
    fn request_install(&mut self, id: &str, ticket: &str) -> Result<()> {
        // StagingStore has already repeated the real save/quit + freshness gate.
        self.sparkle.install_on_normal_quit(id, ticket)
    }
}
