//! Proxy/optimized relationships are separate from immutable original identity.
use crate::{
    storage::{invalid, regular},
    tasks::TaskId,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rational {
    pub numerator: u32,
    pub denominator: u32,
}
impl Rational {
    pub fn valid(self) -> bool {
        self.numerator != 0 && self.denominator != 0
    }
    pub fn equivalent(self, other: Self) -> bool {
        self.valid()
            && other.valid()
            && u64::from(self.numerator) * u64::from(other.denominator)
                == u64::from(other.numerator) * u64::from(self.denominator)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: Rational,
    pub time_base: Rational,
    pub start_tick: i64,
    pub duration_ticks: u64,
    pub alpha: bool,
    pub audio_channels: u16,
    pub audio_sample_rate: u32,
    /// Exact interpretation identity, e.g. primaries/transfer/matrix/range/ICC hash.
    pub colour: String,
}
impl MediaInfo {
    pub fn validate(&self) -> io::Result<()> {
        if self.width == 0
            || self.height == 0
            || !self.frame_rate.valid()
            || !self.time_base.valid()
            || self.duration_ticks == 0
            || self.colour.is_empty()
            || (self.audio_channels > 0 && self.audio_sample_rate == 0)
        {
            return Err(invalid("incomplete video metadata"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RenditionKind {
    Proxy,
    Optimized,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub width: u32,
    pub height: u32,
    pub codec: String,
    pub container: String,
}
impl Profile {
    pub fn validate(&self) -> io::Result<()> {
        if self.width == 0
            || self.height == 0
            || self.codec.is_empty()
            || self.container.is_empty()
            || !self.container.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(invalid("invalid rendition profile"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub path: PathBuf,
    pub fingerprint: [u8; 32],
    pub info: MediaInfo,
}
impl Artifact {
    /// Hash streamed bytes, never trust a filename as identity. Probe must be a
    /// native worker adapter; callers keep source stable under their media lease.
    pub fn inspect(path: &Path, probe: &dyn MediaProbe) -> io::Result<Self> {
        regular(path)?;
        let info = probe.probe(path)?;
        info.validate()?;
        let mut file = File::open(path)?;
        let mut hasher = blake3::Hasher::new();
        let mut block = [0u8; 65536];
        loop {
            let n = file.read(&mut block)?;
            if n == 0 {
                break;
            }
            hasher.update(&block[..n]);
        }
        Ok(Self {
            path: path.to_owned(),
            fingerprint: *hasher.finalize().as_bytes(),
            info,
        })
    }
}
pub trait MediaProbe {
    fn probe(&self, path: &Path) -> io::Result<MediaInfo>;
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rendition {
    pub source: [u8; 32],
    pub profile: Profile,
    pub artifact: Artifact,
    pub online: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: String,
    pub original_identity: [u8; 32],
    pub authored_info: MediaInfo,
    pub original: Option<Artifact>,
    pub original_online: bool,
    pub renditions: BTreeMap<RenditionKind, Rendition>,
    generation: u64,
    jobs: BTreeMap<RenditionKind, MediaJob>,
    failures: BTreeMap<RenditionKind, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaJob {
    pub task: TaskId,
    pub project: String,
    pub media: String,
    pub source: [u8; 32],
    pub kind: RenditionKind,
    pub profile: Profile,
    generation: u64,
    session: u64,
}
impl MediaJob {
    /// Content-derived, safe and collision-separated by source/profile/job epoch.
    pub fn file_name(&self) -> io::Result<String> {
        self.profile.validate()?;
        let key = blake3::hash(&serde_json::to_vec(self)?).to_hex();
        Ok(format!("{key}-{:?}.{}", self.kind, self.profile.container))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Original,
    ProxyOnly,
    OptimizedOnly,
    Offline,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaStatus {
    pub availability: Availability,
    pub proxy: Option<bool>,
    pub optimized: Option<bool>,
    pub jobs: Vec<MediaJob>,
    pub failures: BTreeMap<RenditionKind, String>,
    pub export_warning: bool,
}
pub struct MediaCatalog {
    project: String,
    session: u64,
    items: BTreeMap<String, MediaItem>,
}
impl MediaCatalog {
    pub fn new(project: String) -> io::Result<Self> {
        if project.is_empty() {
            return Err(invalid("empty project id"));
        }
        Ok(Self {
            project,
            session: crate::next_epoch(),
            items: BTreeMap::new(),
        })
    }
    /// Register offline/proxy-only records without inventing an original.
    pub fn register(&mut self, id: String, identity: [u8; 32], info: MediaInfo) -> io::Result<()> {
        info.validate()?;
        if id.is_empty() || self.items.contains_key(&id) {
            return Err(invalid("empty or duplicate media id"));
        }
        self.items.insert(
            id.clone(),
            MediaItem {
                id,
                original_identity: identity,
                authored_info: info,
                original: None,
                original_online: false,
                renditions: BTreeMap::new(),
                generation: 0,
                jobs: BTreeMap::new(),
                failures: BTreeMap::new(),
            },
        );
        Ok(())
    }
    pub fn get(&self, id: &str) -> Option<&MediaItem> {
        self.items.get(id)
    }
    fn item(&mut self, id: &str) -> io::Result<&mut MediaItem> {
        self.items
            .get_mut(id)
            .ok_or_else(|| invalid("unknown media id"))
    }
    pub fn relink_original(&mut self, id: &str, artifact: Artifact) -> io::Result<()> {
        let item = self.item(id)?;
        if artifact.fingerprint != item.original_identity || artifact.info != item.authored_info {
            return Err(invalid("original relink identity/interpretation mismatch"));
        }
        item.generation = item
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("generation overflow"))?;
        item.original = Some(artifact);
        item.original_online = true;
        item.jobs.clear();
        Ok(())
    }
    pub fn set_original_offline(&mut self, id: &str) -> io::Result<()> {
        let item = self.item(id)?;
        item.generation = item
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("generation overflow"))?;
        item.original_online = false;
        item.jobs.clear();
        Ok(())
    }
    pub fn begin(
        &mut self,
        id: &str,
        kind: RenditionKind,
        profile: Profile,
        task: TaskId,
    ) -> io::Result<MediaJob> {
        profile.validate()?;
        let project = self.project.clone();
        let session = self.session;
        let item = self.item(id)?;
        if !item.original_online || item.original.is_none() {
            return Err(invalid("original offline; do not promote proxy"));
        }
        if item.jobs.contains_key(&kind) {
            return Err(invalid("rendition job already active"));
        }
        if kind == RenditionKind::Optimized
            && (profile.width != item.authored_info.width
                || profile.height != item.authored_info.height)
        {
            return Err(invalid("optimized media retains original dimensions"));
        }
        item.generation = item
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("generation overflow"))?;
        let job = MediaJob {
            task,
            project,
            media: id.to_owned(),
            source: item.original_identity,
            kind,
            profile,
            generation: item.generation,
            session,
        };
        item.failures.remove(&kind);
        item.jobs.insert(kind, job.clone());
        Ok(job)
    }
    /// Only call after BackgroundTasks accepted completion for its live ticket.
    /// Native output is inspected before this pure model mutation. Failure/stale
    /// leaves bytes on disk for recovery, never replaces the original.
    pub fn complete(&mut self, job: &MediaJob, artifact: Artifact) -> io::Result<()> {
        if job.project != self.project {
            return Err(invalid("job belongs to another project"));
        }
        let item = self.item(&job.media)?;
        if item.jobs.get(&job.kind) != Some(job) {
            return Err(invalid("stale media job"));
        }
        validate_rendition(&item.authored_info, job.kind, &job.profile, &artifact.info)?;
        item.renditions.insert(
            job.kind,
            Rendition {
                source: job.source,
                profile: job.profile.clone(),
                artifact,
                online: true,
            },
        );
        item.jobs.remove(&job.kind);
        item.failures.remove(&job.kind);
        Ok(())
    }
    /// Preferred host route: validate output and task ownership before atomically
    /// accepting both task success and model attachment on the kernel actor.
    pub fn complete_task(
        &mut self,
        tasks: &mut crate::tasks::BackgroundTasks,
        ticket: crate::tasks::Ticket,
        job: &MediaJob,
        artifact: Artifact,
    ) -> io::Result<()> {
        let expected_kind = match job.kind {
            RenditionKind::Proxy => crate::tasks::TaskKind::Proxy,
            RenditionKind::Optimized => crate::tasks::TaskKind::Optimized,
        };
        let task = tasks
            .get(ticket.id)
            .ok_or_else(|| invalid("unknown task"))?;
        if ticket.id != job.task
            || task.project != self.project
            || task.kind != expected_kind
            || job.project != self.project
        {
            return Err(invalid("task/media ownership mismatch"));
        }
        let item = self
            .items
            .get(&job.media)
            .ok_or_else(|| invalid("unknown media"))?;
        if item.jobs.get(&job.kind) != Some(job) {
            return Err(invalid("stale media job"));
        }
        validate_rendition(&item.authored_info, job.kind, &job.profile, &artifact.info)?;
        tasks
            .finish(ticket, Ok(()))
            .map_err(|e| io::Error::other(format!("task completion refused: {e:?}")))?;
        self.complete(job, artifact)
    }
    pub fn stop(&mut self, job: &MediaJob, error: Option<String>) -> io::Result<()> {
        if job.project != self.project {
            return Err(invalid("foreign project"));
        }
        let item = self.item(&job.media)?;
        if item.jobs.get(&job.kind) != Some(job) {
            return Err(invalid("stale media job"));
        }
        item.jobs.remove(&job.kind);
        if let Some(e) = error {
            item.failures.insert(job.kind, e);
        }
        Ok(())
    }
    /// Migration/manual attach supplies original provenance even when offline.
    pub fn attach(
        &mut self,
        id: &str,
        kind: RenditionKind,
        rendition: Rendition,
    ) -> io::Result<Option<Rendition>> {
        let item = self.item(id)?;
        if rendition.source != item.original_identity {
            return Err(invalid("rendition source mismatch"));
        }
        validate_rendition(
            &item.authored_info,
            kind,
            &rendition.profile,
            &rendition.artifact.info,
        )?;
        item.jobs.remove(&kind);
        Ok(item.renditions.insert(kind, rendition))
    }
    pub fn detach(&mut self, id: &str, kind: RenditionKind) -> io::Result<Option<Rendition>> {
        let item = self.item(id)?;
        item.jobs.remove(&kind);
        Ok(item.renditions.remove(&kind))
    }
    pub fn rendition_offline(&mut self, id: &str, kind: RenditionKind) -> io::Result<()> {
        self.item(id)?
            .renditions
            .get_mut(&kind)
            .ok_or_else(|| invalid("no rendition"))?
            .online = false;
        Ok(())
    }
    pub fn relink_rendition(
        &mut self,
        id: &str,
        kind: RenditionKind,
        artifact: Artifact,
    ) -> io::Result<()> {
        let r = self
            .item(id)?
            .renditions
            .get_mut(&kind)
            .ok_or_else(|| invalid("no rendition"))?;
        if artifact.fingerprint != r.artifact.fingerprint || artifact.info != r.artifact.info {
            return Err(invalid("rendition relink mismatch"));
        }
        r.artifact = artifact;
        r.online = true;
        Ok(())
    }
    pub fn status(&self, id: &str) -> io::Result<MediaStatus> {
        let item = self.items.get(id).ok_or_else(|| invalid("unknown media"))?;
        let proxy = item.renditions.get(&RenditionKind::Proxy).map(|r| r.online);
        let optimized = item
            .renditions
            .get(&RenditionKind::Optimized)
            .map(|r| r.online);
        let availability = if item.original_online {
            Availability::Original
        } else if optimized == Some(true) {
            Availability::OptimizedOnly
        } else if proxy == Some(true) {
            Availability::ProxyOnly
        } else {
            Availability::Offline
        };
        Ok(MediaStatus {
            export_warning: !item.original_online,
            availability,
            proxy,
            optimized,
            jobs: item.jobs.values().cloned().collect(),
            failures: item.failures.clone(),
        })
    }
    /// Explicit preview selection; export defaults to the original and requires
    /// a separate user decision to use a derivative when the original is offline.
    pub fn resolve(&self, id: &str, preferred: Option<RenditionKind>) -> io::Result<&Artifact> {
        let item = self.items.get(id).ok_or_else(|| invalid("unknown media"))?;
        if let Some(kind) = preferred
            && let Some(r) = item.renditions.get(&kind).filter(|r| r.online)
        {
            return Ok(&r.artifact);
        }
        item.original
            .as_ref()
            .filter(|_| item.original_online)
            .ok_or_else(|| invalid("requested media offline"))
    }
}
fn validate_rendition(
    source: &MediaInfo,
    kind: RenditionKind,
    profile: &Profile,
    actual: &MediaInfo,
) -> io::Result<()> {
    profile.validate()?;
    actual.validate()?;
    if actual.width != profile.width
        || actual.height != profile.height
        || kind == RenditionKind::Optimized
            && (actual.width != source.width || actual.height != source.height)
        || !actual.frame_rate.equivalent(source.frame_rate)
        || !actual.time_base.equivalent(source.time_base)
        || actual.start_tick != source.start_tick
        || actual.duration_ticks != source.duration_ticks
        || actual.alpha != source.alpha
        || actual.audio_channels != source.audio_channels
        || actual.audio_sample_rate != source.audio_sample_rate
        || actual.colour != source.colour
    {
        return Err(invalid(
            "rendition changes authored timing, alpha, audio or colour",
        ));
    }
    Ok(())
}
