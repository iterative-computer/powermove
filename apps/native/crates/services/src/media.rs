//! Inspectable import state machine and exact, content-addressed CPU artifact cache.
use crate::storage::{ServiceStorage, component, digest, generations, read_capped};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provenance {
    Original,
    ProxyOnly,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRequest {
    pub project: String,
    pub asset: String,
    pub source: PathBuf,
    pub provenance: Provenance,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probe {
    pub content_sha256: String,
    pub bytes: u64,
    pub duration_ticks: u64,
    pub ticks_per_second: u64,
    pub width: u32,
    pub height: u32,
    pub audio_channels: u16,
}
impl Probe {
    pub fn validate(&self) -> Result<()> {
        validate_hash(&self.content_sha256)?;
        ensure!(self.ticks_per_second > 0, "invalid media timebase");
        ensure!(
            (self.width > 0 && self.height > 0)
                || (self.width == 0 && self.height == 0 && self.audio_channels > 0),
            "invalid media dimensions"
        );
        Ok(())
    }
}
pub trait MediaHook {
    /// Worker-only CPU probe; errors never become successful zero-duration imports.
    fn probe(&mut self, source: &std::path::Path) -> Result<Probe>;
    /// The kernel admits the imported asset. Use job id as the idempotency key.
    fn admit(&mut self, job: &str, request: &ImportRequest, probe: &Probe) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportState {
    Queued,
    Running,
    Completed(Probe),
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportJob {
    pub id: String,
    pub request: ImportRequest,
    pub attempt: u64,
    pub state: ImportState,
    pub order: u64,
}
/// Generation fences prevent a cancelled/retried worker from committing stale work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportTicket {
    pub id: String,
    pub attempt: u64,
    pub request: ImportRequest,
}

#[derive(Default, Serialize, Deserialize)]
pub struct ImportQueue {
    jobs: BTreeMap<String, ImportJob>,
    sequence: u64,
}
impl ImportQueue {
    pub fn jobs(&self) -> impl Iterator<Item = &ImportJob> {
        self.jobs.values()
    }
    pub fn get(&self, id: &str) -> Option<&ImportJob> {
        self.jobs.get(id)
    }
    pub fn enqueue(&mut self, id: &str, request: ImportRequest) -> Result<()> {
        component(id)?;
        component(&request.project)?;
        component(&request.asset)?;
        ensure!(
            request.source.is_absolute(),
            "media source must be explicitly absolute"
        );
        if let Some(job) = self.jobs.get(id) {
            ensure!(job.request == request, "job id reused for different media");
            return Ok(());
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .context("queue sequence exhausted")?;
        self.jobs.insert(
            id.into(),
            ImportJob {
                id: id.into(),
                request,
                attempt: 0,
                state: ImportState::Queued,
                order: self.sequence,
            },
        );
        Ok(())
    }
    pub fn start_next(&mut self) -> Result<Option<ImportTicket>> {
        let Some(job) = self
            .jobs
            .values_mut()
            .filter(|job| job.state == ImportState::Queued)
            .min_by_key(|job| job.order)
        else {
            return Ok(None);
        };
        job.attempt = job.attempt.checked_add(1).context("attempt exhausted")?;
        job.state = ImportState::Running;
        Ok(Some(ImportTicket {
            id: job.id.clone(),
            attempt: job.attempt,
            request: job.request.clone(),
        }))
    }
    pub fn cancel(&mut self, id: &str) -> Result<()> {
        let job = self.jobs.get_mut(id).context("unknown import job")?;
        ensure!(
            !matches!(job.state, ImportState::Completed(_)),
            "completed imports require kernel Undo"
        );
        job.state = ImportState::Cancelled;
        Ok(())
    }
    pub fn retry(&mut self, id: &str) -> Result<()> {
        let job = self.jobs.get_mut(id).context("unknown import job")?;
        ensure!(
            matches!(job.state, ImportState::Failed | ImportState::Cancelled),
            "job is not retryable"
        );
        job.state = ImportState::Queued;
        Ok(())
    }
    /// Feed a worker result back to the queue owner, then admit through the kernel hook.
    /// Hooks must deduplicate job IDs: a crash after admit and before persist replays safely.
    pub fn finish(
        &mut self,
        ticket: ImportTicket,
        result: Result<Probe>,
        hook: &mut impl MediaHook,
    ) -> Result<()> {
        let job = self
            .jobs
            .get_mut(&ticket.id)
            .context("unknown import ticket")?;
        ensure!(
            job.state == ImportState::Running
                && job.attempt == ticket.attempt
                && job.request == ticket.request,
            "stale import completion"
        );
        job.state = ImportState::Failed;
        let probe = result?;
        probe.validate()?;
        hook.admit(&job.id, &job.request, &probe)?;
        job.state = ImportState::Completed(probe);
        Ok(())
    }
    pub fn persist(&self, storage: &mut ServiceStorage) -> Result<()> {
        storage.set("services.importQueue", serde_json::to_value(self)?)?;
        Ok(())
    }
    pub fn restore(storage: &ServiceStorage) -> Result<Self> {
        let Some(value) = storage.setting("services.importQueue")? else {
            return Ok(Self::default());
        };
        let mut queue: Self = serde_json::from_value(value)?;
        for (id, job) in &mut queue.jobs {
            component(id)?;
            component(&job.request.asset)?;
            component(&job.request.project)?;
            ensure!(
                *id == job.id && job.order <= queue.sequence && job.request.source.is_absolute(),
                "invalid saved queue"
            );
            if let ImportState::Completed(probe) = &job.state {
                probe.validate()?;
            }
            if job.state == ImportState::Running {
                job.state = ImportState::Queued;
            }
        }
        Ok(queue)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactKind {
    Thumbnail,
    Waveform,
}
/// No approximate cache hits: decoder, original/proxy identity, time, dimensions,
/// colour pipeline, channel count and sample rate all participate in the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactKey {
    pub kind: ArtifactKind,
    pub source_sha256: String,
    pub provenance: Provenance,
    pub decoder_version: String,
    pub colour_pipeline: String,
    pub time_ticks: i64,
    pub ticks_per_second: u64,
    pub width: u32,
    pub height: u32,
    pub channels: u16,
    pub sample_rate: u32,
}
#[derive(Serialize, Deserialize)]
struct Artifact {
    key: ArtifactKey,
    sha256: String,
    bytes: Vec<u8>,
}
pub struct ArtifactCache {
    max_bytes: usize,
}
impl ArtifactCache {
    pub fn new(max_bytes: usize) -> Result<Self> {
        ensure!(
            max_bytes > 0 && max_bytes <= 64 * 1024 * 1024,
            "artifact cap must be 1..64 MiB"
        );
        Ok(Self { max_bytes })
    }
    fn key(&self, key: &ArtifactKey) -> Result<String> {
        validate_hash(&key.source_sha256)?;
        ensure!(
            key.ticks_per_second > 0
                && !key.decoder_version.is_empty()
                && !key.colour_pipeline.is_empty(),
            "incomplete artifact identity"
        );
        ensure!(
            key.decoder_version.len() <= 128 && key.colour_pipeline.len() <= 128,
            "artifact identity too large"
        );
        Ok(digest(&serde_json::to_vec(key)?))
    }
    pub fn put(
        &self,
        storage: &mut ServiceStorage,
        key: ArtifactKey,
        bytes: Vec<u8>,
    ) -> Result<()> {
        ensure!(bytes.len() <= self.max_bytes, "artifact too large");
        let dir = storage.directory("artifacts", &self.key(&key)?)?;
        let record = Artifact {
            key,
            sha256: digest(&bytes),
            bytes,
        };
        storage.publish(&dir, serde_json::to_vec(&record)?.as_slice())?;
        Ok(())
    }
    pub fn get(&self, storage: &ServiceStorage, key: &ArtifactKey) -> Result<Option<Vec<u8>>> {
        let dir = storage.directory("artifacts", &self.key(key)?)?;
        let Some((_, path)) = generations(&dir)?.pop() else {
            return Ok(None);
        };
        let record: Artifact =
            serde_json::from_slice(&read_capped(&path, self.max_bytes * 4 + 4096)?)?;
        ensure!(
            record.key == *key
                && record.bytes.len() <= self.max_bytes
                && digest(&record.bytes) == record.sha256,
            "artifact integrity failure"
        );
        Ok(Some(record.bytes))
    }
}
pub(crate) fn validate_hash(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "invalid SHA-256"
    );
    Ok(())
}
