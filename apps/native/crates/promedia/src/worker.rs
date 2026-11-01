//! Frozen worker requests and fenced kernel-actor acceptance for real encodes.
use crate::{
    media::{Artifact, MediaCatalog, MediaInfo, MediaJob, MediaProbe, Rational, RenditionKind},
    proxy::{Control, EncodeOutcome, EncodeRequest, Probe, RenditionEncoder},
    storage::{invalid, no_links},
    tasks::{
        BackgroundTasks, ProgressUnit, TaskFailure, TaskFailureCode, TaskKind, TaskState, Ticket,
    },
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

fn info(probe: &Probe) -> MediaInfo {
    MediaInfo {
        width: probe.width,
        height: probe.height,
        frame_rate: Rational {
            numerator: probe.rate.numerator,
            denominator: probe.rate.denominator,
        },
        time_base: Rational {
            numerator: probe.time_base.numerator,
            denominator: probe.time_base.denominator,
        },
        start_tick: probe.video.start,
        duration_ticks: probe.video.duration,
        alpha: probe.alpha,
        audio_channels: probe.audio.as_ref().map_or(0, |a| a.channels),
        audio_sample_rate: probe.audio.as_ref().map_or(0, |a| a.sample_rate),
        colour: probe.colour.clone(),
    }
}
pub struct EngineProbe<'a>(pub &'a dyn RenditionEncoder);
impl MediaProbe for EngineProbe<'_> {
    fn probe(&self, path: &Path) -> io::Result<MediaInfo> {
        Ok(info(&self.0.probe(path)?))
    }
}
#[derive(Clone, Debug)]
pub struct RenditionWork {
    job: MediaJob,
    ticket: Ticket,
    source: Artifact,
    progress_floor: u64,
}
#[derive(Debug)]
pub enum WorkOutcome {
    Ready(Artifact),
    Stopped(EncodeOutcome),
    Failed(TaskFailure),
}
#[derive(Debug)]
pub struct WorkReport {
    job: MediaJob,
    ticket: Ticket,
    /// Retained on failure/cancel too. The host archives these bytes; no worker
    /// deletes files, attaches a rendition or acknowledges a control early.
    pub staging: Option<PathBuf>,
    pub outcome: WorkOutcome,
}
impl RenditionWork {
    /// Kernel actor: freeze source identity and validate the live worker ticket.
    /// Host dispatches the result to a worker; no filesystem/codec work here.
    pub fn prepare(
        catalog: &MediaCatalog,
        tasks: &mut BackgroundTasks,
        ticket: Ticket,
        job: &MediaJob,
    ) -> io::Result<Self> {
        if tasks
            .worker_state(ticket)
            .map_err(|e| invalid(&format!("worker ticket: {e:?}")))?
            != TaskState::Running
        {
            return Err(invalid("worker is not running"));
        }
        let task = tasks
            .get(ticket.id)
            .ok_or_else(|| invalid("missing task"))?;
        let expected = match job.kind {
            RenditionKind::Proxy => TaskKind::Proxy,
            RenditionKind::Optimized => TaskKind::Optimized,
        };
        if ticket.id != job.task
            || task.kind != expected
            || task.project != job.project
            || task.unit != Some(ProgressUnit::Frames)
        {
            return Err(invalid("worker task/media mismatch"));
        }
        if !catalog.status(&job.media)?.jobs.contains(job) {
            return Err(invalid("stale media job"));
        }
        let source = catalog.resolve(&job.media, None)?.clone();
        if source.fingerprint != job.source {
            return Err(invalid("source identity mismatch"));
        }
        Ok(Self {
            job: job.clone(),
            ticket,
            source,
            progress_floor: task.completed,
        })
    }
    /// Worker: hold the host's source lease throughout this call. Observer sends
    /// frame progress/control over the host's actor channel; it must not borrow
    /// the UI or synchronously call JS. Restart paused work into a fresh directory.
    pub fn run(
        self,
        encoder: &dyn RenditionEncoder,
        staging_root: &Path,
        observe: impl FnMut(u64) -> Control,
    ) -> WorkReport {
        let mut completed = self.progress_floor;
        let mut observer = observe;
        let mut observe = |frames: u64| {
            completed = completed.max(frames);
            observer(completed)
        };
        let mut staging = None;
        let result = (|| -> io::Result<WorkOutcome> {
            // Refuse control before expensive probing or creating any output.
            match observe(0) {
                Control::Pause => return Ok(WorkOutcome::Stopped(EncodeOutcome::Paused)),
                Control::Cancel => return Ok(WorkOutcome::Stopped(EncodeOutcome::Cancelled)),
                _ => {}
            }
            no_links(staging_root)?;
            if !staging_root.is_dir() {
                return Err(invalid("staging root must be a directory"));
            }
            let source_probe = encoder.probe(&self.source.path)?;
            let actual = Artifact::inspect(&self.source.path, &EngineProbe(encoder))?;
            if actual != self.source {
                return Err(invalid("source changed before encode"));
            }
            let dir =
                staging_root.join(format!("{}-{}", self.job.file_name()?, crate::next_epoch()));
            fs::create_dir(&dir)?;
            staging = Some(dir.clone());
            let output = dir.join(format!("encoded.{}", self.job.profile.container));
            let request = EncodeRequest {
                source: self.source.path.clone(),
                output: output.clone(),
                width: self.job.profile.width,
                height: self.job.profile.height,
                codec: self.job.profile.codec.clone(),
                container: self.job.profile.container.clone(),
            };
            let encoded = encoder.encode(&request, &mut observe)?;
            if encoded != EncodeOutcome::Complete {
                return Ok(WorkOutcome::Stopped(encoded));
            }
            match observe(source_probe.video.samples) {
                Control::Pause => return Ok(WorkOutcome::Stopped(EncodeOutcome::Paused)),
                Control::Cancel => return Ok(WorkOutcome::Stopped(EncodeOutcome::Cancelled)),
                _ => {}
            }
            let output_probe = encoder.probe(&output)?;
            source_probe.verify_rendition(&output_probe, request.width, request.height)?;
            // Lease is required; this second identity check additionally catches
            // accidental host misuse. Output remains private if anything differs.
            if Artifact::inspect(&self.source.path, &EngineProbe(encoder))? != self.source {
                return Err(invalid("source changed during encode"));
            }
            let artifact = Artifact::inspect(&output, &EngineProbe(encoder))?;
            Ok(WorkOutcome::Ready(artifact))
        })();
        let outcome = result.unwrap_or_else(|e| {
            WorkOutcome::Failed(TaskFailure {
                code: match e.kind() {
                    io::ErrorKind::NotFound => TaskFailureCode::SourceOffline,
                    io::ErrorKind::StorageFull => TaskFailureCode::DiskFull,
                    io::ErrorKind::Unsupported => TaskFailureCode::CodecUnsupported,
                    _ => TaskFailureCode::Unknown,
                },
                diagnostic: e.to_string(),
            })
        });
        WorkReport {
            job: self.job,
            ticket: self.ticket,
            staging,
            outcome,
        }
    }
}
impl WorkReport {
    /// Kernel actor only. Refusal leaves this report and its private bytes intact
    /// for archival/recovery. Pause/cancel acknowledgement happens only after run
    /// returned and the codec released its resources.
    pub fn accept(
        &self,
        catalog: &mut MediaCatalog,
        tasks: &mut BackgroundTasks,
    ) -> io::Result<()> {
        if !catalog.status(&self.job.media)?.jobs.contains(&self.job) {
            return Err(invalid("stale report"));
        }
        let state = tasks
            .worker_state(self.ticket)
            .map_err(|e| invalid(&format!("worker ticket: {e:?}")))?;
        // A control arriving during final probing wins over success or failure.
        if matches!(
            state,
            TaskState::PauseRequested | TaskState::CancelRequested
        ) {
            tasks
                .settle_control(self.ticket)
                .map_err(|e| invalid(&format!("settle: {e:?}")))?;
            catalog.stop(&self.job, None)?;
            return Ok(());
        }
        match &self.outcome {
            WorkOutcome::Ready(artifact) => {
                catalog.complete_task(tasks, self.ticket, &self.job, artifact.clone())
            }
            WorkOutcome::Failed(failure) => {
                tasks
                    .finish(self.ticket, Err(failure.clone()))
                    .map_err(|e| invalid(&format!("finish: {e:?}")))?;
                catalog.stop(&self.job, Some(failure.diagnostic.clone()))
            }
            WorkOutcome::Stopped(_) => Err(invalid(
                "worker stopped without a matching actor control request",
            )),
        }
    }
}
