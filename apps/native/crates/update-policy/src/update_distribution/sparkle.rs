//! Asynchronous public-API Sparkle path. Never feeds an external archive to
//! SPUUpdater. The host holds migration/import leases and a normal-termination
//! guard for the whole session; callbacks are queued to this Rust worker.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enclosure {
    pub version: String,
    pub url: String,
    pub bytes: u64,
    pub ed_signature: String,
}
impl Enclosure {
    pub fn matches(&self, release: &Release) -> bool {
        self.version == release.version
            && self.url == release.archive_url
            && self.bytes == release.size
            && self.ed_signature == release.sparkle_signature
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Found(Enclosure),
    Extracting,
    Ready,
    /// Informational only. Signed installed identity must be independently read.
    Installed,
    Aborted,
    RecoveryRequired,
}
/// Enqueue to the main run loop. Never wait for AppKit or invent a stage ticket.
/// The selected full enclosure must match before download, including a resumed
/// installation. Automatic checks/downloads and delta selection stay disabled.
pub trait Session {
    fn check(&mut self, id: &str, release: &Release) -> Result<()>;
    fn download(&mut self, id: &str) -> Result<()>;
    fn install(&mut self, id: &str) -> Result<()>;
    fn cancel(&mut self, id: &str) -> Result<()>;
}
impl StagingStore {
    fn fresh_sparkle(
        &self,
        offer: &Offer,
        readiness: &mut impl Readiness,
        verifier: &impl SignatureVerifier,
    ) -> Result<()> {
        ensure!(
            readiness.current()?.allows_native(),
            "Sparkle update gate closed"
        );
        self.validate_offer(offer, verifier, readiness.now_unix()?)
    }
    /// The application must install its ordinary-termination guard before this
    /// call. Once extraction starts Sparkle can install on *any* normal quit.
    pub fn begin_sparkle(
        &self,
        offer: &Offer,
        readiness: &mut impl Readiness,
        verifier: &impl SignatureVerifier,
        session: &mut impl Session,
    ) -> Result<()> {
        self.fresh_sparkle(offer, readiness, verifier)?;
        ensure!(
            self.status()?.is_none(),
            "recover existing update before a new Sparkle session"
        );
        self.append(offer, Stage::SparkleChecking)?;
        session.check(offer.id(), offer.release())
    }
    /// Unexpected/out-of-order callbacks remain errors, never readiness claims.
    /// A retained receipt after a process death requires native recovery evidence.
    pub fn sparkle_event(
        &self,
        id: &str,
        event: Event,
        readiness: &mut impl Readiness,
        verifier: &impl SignatureVerifier,
        session: &mut impl Session,
    ) -> Result<Stage> {
        let receipt = self.journal()?.pop().context("no Sparkle transaction")?;
        let offer = &receipt.offer;
        ensure!(offer.id() == id, "stale Sparkle callback identity");
        let verified = verify_policy(&offer.policy, verifier)?;
        ensure!(
            verified.id == offer.id && verified.release == offer.release,
            "Sparkle policy changed"
        );
        let next = match (&receipt.stage, event) {
            (Stage::SparkleChecking, Event::Found(item)) => {
                if !item.matches(offer.release()) {
                    session.cancel(offer.id())?;
                    bail!("Sparkle enclosure differs from signed policy");
                }
                if let Err(error) = self.fresh_sparkle(offer, readiness, verifier) {
                    session.cancel(offer.id())?;
                    return Err(error);
                }
                self.append(offer, Stage::SparkleDownloading)?;
                session.download(offer.id())?;
                return Ok(Stage::SparkleDownloading);
            }
            (Stage::SparkleDownloading, Event::Extracting) => Stage::SparkleExtracting,
            (Stage::SparkleExtracting, Event::Ready) => Stage::SparkleReady,
            (_, Event::Installed) => {
                bail!("verify installed signed identity through recovery evidence")
            }
            (_, Event::Aborted) => {
                bail!("Sparkle aborted; receipts and native installer state retained")
            }
            (_, Event::RecoveryRequired) => {
                bail!("Sparkle has persisted installer state; signed recovery evidence required")
            }
            _ => bail!("unexpected Sparkle callback"),
        };
        self.append(offer, next.clone())?;
        Ok(next)
    }
    pub fn install_sparkle(
        &self,
        readiness: &mut impl Readiness,
        verifier: &impl SignatureVerifier,
        barrier: &mut impl QuitBarrier,
        session: &mut impl Session,
    ) -> Result<bool> {
        let receipt = self.journal()?.pop().context("no Sparkle transaction")?;
        ensure!(
            receipt.stage == Stage::SparkleReady,
            "Sparkle is not ready to install"
        );
        self.fresh_sparkle(&receipt.offer, readiness, verifier)?;
        if !barrier.ready()? {
            return Ok(false);
        }
        self.fresh_sparkle(&receipt.offer, readiness, verifier)?;
        self.append(&receipt.offer, Stage::SparkleInstallRequested)?;
        session.install(receipt.offer.id())?;
        Ok(true)
    }
}
