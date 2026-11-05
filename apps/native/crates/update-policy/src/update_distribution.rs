//! Signed policy and durable staging for the S6 Sparkle adapter. All calls run
//! on a worker. This module never extracts, replaces an app, quits, or publishes.
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use url::Url;

pub mod sparkle;

const MAX_POLICY: u64 = 128 * 1024;
const MAX_RECEIPT: u64 = 256 * 1024;
const RESERVE: u64 = 4 * 1024 * 1024 * 1024;
pub const PRODUCTION_ID: &str = "com.zellzoi.powermove";
pub const BETA_ID: &str = "com.zellzoi.powermove.native-beta";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Channel {
    Cutover,
    Beta,
}
impl Channel {
    pub fn bundle_id(self) -> &'static str {
        match self {
            Self::Cutover => PRODUCTION_ID,
            Self::Beta => BETA_ID,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Architecture {
    Arm64,
    X86_64,
}

/// Bundled configuration. No default endpoint or runtime env override. The
/// detached policy key and Sparkle key are independent pins in the signed app.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdateConfig {
    pub version: u32,
    pub enabled: bool,
    #[serde(rename = "feedURL")]
    pub feed_url: Option<String>,
    pub public_key: Option<String>,
    pub channel: Channel,
}
impl UpdateConfig {
    pub fn validate(&self) -> Result<()> {
        self.endpoint().map(|_| ())
    }
    fn endpoint(&self) -> Result<Option<Url>> {
        ensure!(self.version == 1, "unsupported update config");
        if !self.enabled {
            return Ok(None);
        }
        ensure!(
            self.public_key.as_ref().is_some_and(|k| !k.is_empty()),
            "missing pinned update key"
        );
        let url = Url::parse(self.feed_url.as_deref().context("missing update feed")?)?;
        validate_https(&url)?;
        ensure!(
            !url.path().ends_with("latest-mac.yml") && !url.path().ends_with("beta-mac.yml"),
            "legacy feed is not a cutover policy"
        );
        Ok(Some(url))
    }
}

/// Construct only from trusted host verification, not IPC, feed fields, or TR's
/// advisory `cutoverReady`. Re-capture after any new import, edit or secret change.
#[derive(Debug, Clone, Copy, Default)]
pub struct Eligibility {
    pub packaged_macos: bool,
    pub macos_major: u32,
    pub installed_identity_verified: bool,
    pub transition_or_native_receipt_verified: bool,
    pub owner_approved: bool,
    pub source_generation: u64,
    pub checkpoint_generation: u64,
    pub final_delta_complete: bool,
    pub all_six_owner_domains: bool,
    pub state_and_secrets_complete: bool,
    pub restore_verified: bool,
}
impl Eligibility {
    pub fn allows_native(self) -> bool {
        let route = crate::updater::TransitionGate {
            packaged_macos: self.packaged_macos,
            macos_major: self.macos_major,
            migration_complete: self.final_delta_complete
                && self.all_six_owner_domains
                && self.state_and_secrets_complete
                && self.source_generation == self.checkpoint_generation,
            transition_release_verified: self.transition_or_native_receipt_verified,
        }
        .route();
        route == Some(crate::updater::FeedRoute::NativeCutover)
            && self.installed_identity_verified
            && self.owner_approved
            && self.restore_verified
    }
}

/// Exact UTF-8 payload bytes are signed with a domain separator. Do not parse or
/// reserialize before verification; this is a policy sidecar, not a Sparkle appcast.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedPolicy {
    pub payload: String,
    pub signature: String,
}
pub const SIGNING_DOMAIN: &[u8] = b"Powermove Native update policy v1\0";
pub trait SignatureVerifier {
    fn verify(&self, message: &[u8], signature: &str) -> Result<()>;
}
pub trait Transport {
    /// Implementations must validate TLS, reject redirects and non-200 responses,
    /// bound time, and avoid logging URLs with sensitive values.
    fn open(&mut self, url: &Url) -> Result<Box<dyn Read>>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Release {
    pub schema: u32,
    pub channel: Channel,
    pub version: String,
    pub architecture: Architecture,
    pub bundle_id: String,
    pub minimum_macos: u32,
    pub archive_url: String,
    pub size: u64,
    pub sha256: String,
    /// Copied from the signed policy to the Sparkle adapter, which must verify
    /// the archive EdDSA signature AND installed app designated requirement.
    pub sparkle_signature: String,
    pub expires_unix: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    release: Release,
    id: String,
    policy: Vec<u8>,
}
impl Offer {
    pub fn release(&self) -> &Release {
        &self.release
    }
    pub fn id(&self) -> &str {
        &self.id
    }
}

pub fn validate_https(url: &Url) -> Result<()> {
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "invalid distribution URL"
    );
    Ok(())
}
fn read_bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "distribution metadata too large"
    );
    Ok(bytes)
}
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
fn verify_policy(bytes: &[u8], verifier: &impl SignatureVerifier) -> Result<Offer> {
    ensure!(bytes.len() as u64 <= MAX_POLICY, "policy too large");
    let signed: SignedPolicy = serde_json::from_slice(bytes)?;
    let mut message = SIGNING_DOMAIN.to_vec();
    message.extend_from_slice(signed.payload.as_bytes());
    verifier.verify(&message, &signed.signature)?;
    let release: Release = serde_json::from_str(&signed.payload)?;
    ensure!(
        release.schema == 1 && release.size > 0 && release.size <= 16 * 1024 * 1024 * 1024,
        "invalid release schema or size"
    );
    ensure!(
        release.minimum_macos >= 14 && release.bundle_id == release.channel.bundle_id(),
        "invalid release identity"
    );
    ensure!(
        release.sha256.len() == 64
            && release
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid archive digest"
    );
    ensure!(
        release.version.len() <= 128 && release.archive_url.len() <= 4096,
        "release field too large"
    );
    ensure!(
        release.sparkle_signature.len() == 88
            && release.sparkle_signature.ends_with("==")
            && release.sparkle_signature[..86]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/'),
        "invalid Sparkle Ed25519 signature encoding"
    );
    validate_https(&Url::parse(&release.archive_url)?)?;
    Version::parse(&release.version)?;
    Ok(Offer {
        release,
        id: hex(Sha256::digest(bytes)),
        policy: bytes.to_vec(),
    })
}

#[derive(Debug)]
pub enum Check {
    Gated,
    Current,
    Available(Offer),
}
/// The verifier must be constructed from config.public_key by the host. `now`
/// is the trusted host clock; expired policies never authorize a new install.
pub fn check(
    config: &UpdateConfig,
    eligibility: Eligibility,
    current: &str,
    architecture: Architecture,
    now: u64,
    transport: &mut impl Transport,
    verifier: &impl SignatureVerifier,
) -> Result<Check> {
    let Some(endpoint) = config.endpoint()? else {
        return Ok(Check::Gated);
    };
    if !eligibility.allows_native() {
        return Ok(Check::Gated);
    }
    let current = Version::parse(current)?;
    let bytes = read_bounded(transport.open(&endpoint)?, MAX_POLICY)?;
    let offer = verify_policy(&bytes, verifier)?;
    let release = &offer.release;
    ensure!(
        release.channel == config.channel && release.architecture == architecture,
        "feed target mismatch"
    );
    ensure!(release.expires_unix > now, "expired update policy");
    let offered = Version::parse(&release.version)?;
    if offered <= current
        || release.minimum_macos > eligibility.macos_major
        || (config.channel == Channel::Cutover && !offered.pre.is_empty())
    {
        return Ok(Check::Current);
    }
    Ok(Check::Available(offer))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Stage {
    Downloading,
    Downloaded {
        file: String,
    },
    Staging {
        file: String,
    },
    Staged {
        file: String,
        ticket: String,
    },
    InstallRequested {
        file: String,
        ticket: String,
    },
    Completed,
    /// Sparkle owns its download; these states never imply external ZIP adoption.
    SparkleChecking,
    SparkleDownloading,
    SparkleExtracting,
    SparkleReady,
    SparkleInstallRequested,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    offer: Offer,
    stage: Stage,
}

/// Capability supplied by the macOS Sparkle 2 adapter. No permissive production
/// implementation exists on Linux. The adapter owns helper/extraction/DR checks,
/// atomic replacement and recovery; opaque tickets identify idempotent operations.
pub trait Installer {
    fn available_space(&mut self, staging_root: &Path) -> Result<u64>;
    fn stage(&mut self, id: &str, archive: &Path, release: &Release) -> Result<String>;
    /// Idempotent recovery of *this exact id*, never a new install. Keep old app,
    /// installer caches and migration checkpoints, including when this errors.
    fn recover(&mut self, id: &str, last: &Stage) -> Result<Recovery>;
    fn request_install(&mut self, id: &str, ticket: &str) -> Result<()>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    Staged { ticket: String },
    Installed,
    Pending,
}

/// Host barrier issued only after kernel journals/settings/secrets are durable,
/// all dirty windows approve normal quit, and migration freshness is rechecked.
pub trait QuitBarrier {
    fn ready(&mut self) -> Result<bool>;
}
/// Host re-reads generations and keeps migration/import leases while returning
/// readiness. Never cache this across a network operation or the quit barrier.
pub trait Readiness {
    fn current(&mut self) -> Result<Eligibility>;
    /// Trusted worker clock, re-read after network and the quit barrier.
    fn now_unix(&mut self) -> Result<u64>;
}

/// Process-exclusive, private staging root; append-only receipts retain every
/// attempt. A private root is a host capability, not a renderer-supplied path.
pub struct StagingStore {
    root: PathBuf,
    _lock: File,
}
impl StagingStore {
    pub fn open(root: &Path) -> Result<Self> {
        ensure!(root.is_absolute(), "staging root must be absolute");
        reject_links(root)?;
        fs::create_dir_all(root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        }
        let lock_path = root.join("update.lock");
        match fs::symlink_metadata(&lock_path) {
            Ok(meta) => ensure!(meta.file_type().is_file(), "invalid update lock"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.try_lock_exclusive()
            .context("another update worker owns staging")?;
        Ok(Self {
            root: root.to_path_buf(),
            _lock: lock,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn journal(&self) -> Result<Vec<Receipt>> {
        let mut files = fs::read_dir(&self.root)?.collect::<std::io::Result<Vec<_>>>()?;
        files.sort_by_key(|entry| entry.file_name());
        let mut receipts: Vec<Receipt> = Vec::new();
        for entry in files {
            let name = entry.file_name();
            let name = name.to_str().context("invalid staging filename")?;
            if !name.starts_with("receipt-") || !name.ends_with(".json") {
                continue;
            }
            ensure!(
                name == format!("receipt-{:08}.json", receipts.len()),
                "receipt sequence gap"
            );
            ensure!(entry.file_type()?.is_file(), "invalid receipt file");
            let receipt: Receipt =
                serde_json::from_slice(&read_bounded(File::open(entry.path())?, MAX_RECEIPT)?)?;
            if let Some(previous) = receipts.last() {
                ensure!(
                    previous.offer.id == receipt.offer.id,
                    "mixed update transactions"
                );
            }
            receipts.push(receipt);
        }
        Ok(receipts)
    }
    fn append(&self, offer: &Offer, stage: Stage) -> Result<()> {
        let sequence = self.journal()?.len();
        // A torn pending publication is retained, not reused or deleted.
        let attempt = self.root.join(format!(
            "receipt-{sequence:08}-{}.pending",
            next_attempt(&self.root)?
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&attempt)?;
        let bytes = serde_json::to_vec(&Receipt {
            offer: offer.clone(),
            stage,
        })?;
        ensure!(
            bytes.len() as u64 <= MAX_RECEIPT,
            "update receipt too large"
        );
        file.write_all(&bytes)?;
        file.sync_all()?;
        let target = self.root.join(format!("receipt-{sequence:08}.json"));
        ensure!(!target.exists(), "unexpected receipt publication");
        fs::rename(attempt, target)?;
        sync_dir(&self.root)
    }
    pub fn status(&self) -> Result<Option<(String, Stage)>> {
        Ok(self.journal()?.pop().map(|r| (r.offer.id, r.stage)))
    }
    fn validate_offer(
        &self,
        offer: &Offer,
        verifier: &impl SignatureVerifier,
        now: u64,
    ) -> Result<()> {
        let verified = verify_policy(&offer.policy, verifier)?;
        ensure!(
            verified.id == offer.id && verified.release == offer.release,
            "persisted offer mismatch"
        );
        ensure!(offer.release.expires_unix > now, "expired update policy");
        if let Some(receipt) = self.journal()?.last() {
            ensure!(
                receipt.offer.id == offer.id,
                "use a separate staging root per update id"
            );
        }
        Ok(())
    }
    /// Streams into a new retained attempt; partial downloads are never executed.
    /// Rechecks the gate before network and again before installer staging.
    pub fn download_and_stage(
        &self,
        offer: &Offer,
        readiness: &mut impl Readiness,
        now: u64,
        transport: &mut impl Transport,
        verifier: &impl SignatureVerifier,
        installer: &mut impl Installer,
    ) -> Result<()> {
        ensure!(readiness.current()?.allows_native(), "update gate closed");
        self.validate_offer(offer, verifier, now)?;
        match self.status()?.map(|(_, stage)| stage) {
            Some(Stage::Staged { .. } | Stage::InstallRequested { .. } | Stage::Completed) => {
                return Ok(());
            }
            Some(Stage::Staging { .. }) => bail!("recover interrupted installer before staging"),
            Some(
                Stage::SparkleChecking
                | Stage::SparkleDownloading
                | Stage::SparkleExtracting
                | Stage::SparkleReady
                | Stage::SparkleInstallRequested,
            ) => {
                bail!("Sparkle owns this transaction; external download is forbidden");
            }
            _ => {}
        }
        let required = offer
            .release
            .size
            .checked_mul(2)
            .and_then(|n| n.checked_add(RESERVE))
            .context("update space overflow")?;
        ensure!(
            installer.available_space(&self.root)? >= required,
            "insufficient update space"
        );
        self.append(offer, Stage::Downloading)?;
        let name = format!("archive-{:08}.part", next_attempt(&self.root)?);
        let path = self.root.join(&name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let mut source = transport.open(&Url::parse(&offer.release.archive_url)?)?;
        let mut hash = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size = size
                .checked_add(count as u64)
                .context("download size overflow")?;
            ensure!(size <= offer.release.size, "oversized update archive");
            file.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
        }
        file.sync_all()?;
        ensure!(
            size == offer.release.size && hex(hash.finalize()) == offer.release.sha256,
            "archive integrity rejected"
        );
        self.append(offer, Stage::Downloaded { file: name.clone() })?;
        ensure!(
            readiness.current()?.allows_native(),
            "migration changed during download"
        );
        self.validate_offer(offer, verifier, readiness.now_unix()?)?;
        self.stage_verified(offer, &name, installer)
    }
    fn stage_verified(
        &self,
        offer: &Offer,
        name: &str,
        installer: &mut impl Installer,
    ) -> Result<()> {
        let path = self.verify_archive(offer, name)?;
        self.append(
            offer,
            Stage::Staging {
                file: name.to_owned(),
            },
        )?;
        let ticket = installer.stage(&offer.id, &path, &offer.release)?;
        valid_ticket(&ticket)?;
        self.append(
            offer,
            Stage::Staged {
                file: name.to_owned(),
                ticket,
            },
        )
    }
    fn verify_archive(&self, offer: &Offer, name: &str) -> Result<PathBuf> {
        ensure!(
            Path::new(name).components().count() == 1
                && name.starts_with("archive-")
                && name.ends_with(".part"),
            "unsafe archive path"
        );
        let path = self.root.join(name);
        ensure!(
            fs::symlink_metadata(&path)?.file_type().is_file(),
            "invalid archive file"
        );
        let mut file = File::open(&path)?;
        ensure!(
            file.metadata()?.len() == offer.release.size,
            "archive size changed"
        );
        let mut hash = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        ensure!(
            hex(hash.finalize()) == offer.release.sha256,
            "staged archive changed"
        );
        Ok(path)
    }
    /// Re-verifies persisted policy and bytes. Does not contact the feed, quit or
    /// delete anything. Expiry may block a new stage; already-requested recovery
    /// remains available so an expired policy cannot strand a moved old bundle.
    pub fn recover(
        &self,
        verifier: &impl SignatureVerifier,
        installer: &mut impl Installer,
        readiness: &mut impl Readiness,
        now: u64,
    ) -> Result<Option<Stage>> {
        let Some(receipt) = self.journal()?.pop() else {
            return Ok(None);
        };
        let offer = &receipt.offer;
        let verified = verify_policy(&offer.policy, verifier)?;
        ensure!(
            verified.id == offer.id && verified.release == offer.release,
            "receipt policy mismatch"
        );
        match &receipt.stage {
            Stage::Downloading => {} // caller may explicitly retry; bytes retained
            Stage::Downloaded { file } => {
                ensure!(
                    readiness.current()?.allows_native(),
                    "migration gate closed during recovery"
                );
                self.validate_offer(offer, verifier, now)?;
                self.stage_verified(offer, file, installer)?;
            }
            Stage::Staging { file } | Stage::Staged { file, .. } => {
                self.verify_archive(offer, file)?;
                match installer.recover(&offer.id, &receipt.stage)? {
                    Recovery::Staged { ticket } => {
                        valid_ticket(&ticket)?;
                        self.append(
                            offer,
                            Stage::Staged {
                                file: file.clone(),
                                ticket,
                            },
                        )?;
                    }
                    Recovery::Installed => bail!("unexpected installation before quit barrier"),
                    Recovery::Pending => {}
                }
            }
            Stage::InstallRequested { file, .. } => {
                match installer.recover(&offer.id, &receipt.stage)? {
                    Recovery::Installed => self.append(offer, Stage::Completed)?,
                    Recovery::Staged { ticket } => {
                        self.verify_archive(offer, file)?;
                        valid_ticket(&ticket)?;
                        self.append(
                            offer,
                            Stage::Staged {
                                file: file.clone(),
                                ticket,
                            },
                        )?;
                    }
                    Recovery::Pending => {}
                }
            }
            Stage::Completed => {}
            Stage::SparkleChecking
            | Stage::SparkleDownloading
            | Stage::SparkleExtracting
            | Stage::SparkleReady
            | Stage::SparkleInstallRequested => {
                bail!("use Sparkle session recovery; never restart an unknown installer");
            }
        }
        Ok(self.status()?.map(|(_, stage)| stage))
    }
    pub fn request_install(
        &self,
        readiness: &mut impl Readiness,
        now: u64,
        verifier: &impl SignatureVerifier,
        barrier: &mut impl QuitBarrier,
        installer: &mut impl Installer,
    ) -> Result<bool> {
        ensure!(readiness.current()?.allows_native(), "stale migration gate");
        let receipt = self.journal()?.pop().context("no staged update")?;
        self.validate_offer(&receipt.offer, verifier, now)?;
        let Stage::Staged { file, ticket } = receipt.stage else {
            bail!("update is not staged");
        };
        self.verify_archive(&receipt.offer, &file)?;
        if !barrier.ready()? {
            return Ok(false);
        }
        ensure!(
            readiness.current()?.allows_native(),
            "migration changed at quit barrier"
        );
        self.validate_offer(&receipt.offer, verifier, readiness.now_unix()?)?;
        self.append(
            &receipt.offer,
            Stage::InstallRequested {
                file,
                ticket: ticket.clone(),
            },
        )?;
        installer.request_install(&receipt.offer.id, &ticket)?;
        Ok(true)
    }
}
fn valid_ticket(ticket: &str) -> Result<()> {
    ensure!(
        !ticket.is_empty() && ticket.len() <= 512 && !ticket.chars().any(char::is_control),
        "invalid installer ticket"
    );
    Ok(())
}
fn next_attempt(root: &Path) -> Result<usize> {
    Ok(fs::read_dir(root)?.count())
}
fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}
fn reject_links(path: &Path) -> Result<()> {
    let mut cursor = PathBuf::new();
    for part in path.components() {
        ensure!(
            !matches!(part, Component::ParentDir | Component::CurDir),
            "unclean staging path"
        );
        cursor.push(part);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) => ensure!(
                meta.file_type().is_dir(),
                "staging path contains a link or non-directory"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
