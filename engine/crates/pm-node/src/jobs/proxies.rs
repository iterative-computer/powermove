//! Proxy generation on its own utility lane. All media and store paths come
//! from host capabilities; the job request contains asset ids, not paths.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pm_protocol::common::{AssetId, JobId, SessionId};
use pm_protocol::envelope::Request;
use pm_protocol::error::{EngineError, ErrorCode};
use pm_protocol::host::{
    CacheKind, CapabilityKind, HandleAccess, MintHandle, PortId, ResolveHandle,
};
use pm_protocol::priority::{JobState, PriorityClass, ProxyFormat, ProxySize};

use super::{Shared, job_error, status_event, status_of};

pub(super) fn run(
    shared: &Arc<Shared>,
    job: &JobId,
    host: PortId,
    asset: &AssetId,
    format: ProxyFormat,
    size: ProxySize,
    cancel: &Arc<AtomicBool>,
) {
    shared.status(host, job, |r| {
        r.state = JobState::Running;
        r.progress = 0.0;
    });
    let result = run_inner(shared, job, host, asset, format, size, cancel);
    let (state, path, error) = match result {
        Ok(path) => (JobState::Done, Some(path), None),
        Err(_) if cancel.load(Ordering::Relaxed) => (JobState::Cancelled, None, None),
        Err(error) => (JobState::Failed, None, Some(error)),
    };
    let mut st = shared.lock();
    match state {
        JobState::Done => st.stats.proxies_done += 1,
        JobState::Cancelled => st.stats.proxies_cancelled += 1,
        _ => st.stats.proxies_failed += 1,
    }
    if let Some(record) = st.jobs.get_mut(job) {
        record.state = state;
        if state == JobState::Done {
            record.progress = 1.0;
        }
        let status = status_of(job, record, path, error);
        st.outbox.messages.push((host, status_event(status)));
        st.retire(job.clone());
    }
    drop(st);
    (shared.wake)();
}

fn run_inner(
    shared: &Arc<Shared>,
    job: &JobId,
    host: PortId,
    asset: &AssetId,
    format: ProxyFormat,
    size: ProxySize,
    cancel: &Arc<AtomicBool>,
) -> Result<String, EngineError> {
    let (source, hash) = media_source(shared, job, asset, cancel)?;
    let store_path = cache_path(shared, job)?;
    let store = pm_proxy::ProxyStore::open(store_path)
        .map_err(|e| job_error(job, ErrorCode::ExportFailed, e.to_string()))?;
    let (ffmpeg, ffprobe) =
        codec_binaries().map_err(|message| job_error(job, ErrorCode::ExportFailed, message))?;
    let should_pause = || {
        if shared.gate.is_playing() {
            return true;
        }
        shared
            .lock()
            .jobs
            .values()
            .any(|r| r.class != PriorityClass::Generation && r.state == JobState::Running)
    };
    let on_pause = |reason: Option<pm_proxy::PauseReason>| match reason {
        Some(reason) => shared.status_with(
            host,
            job,
            None,
            Some(job_error(
                job,
                ErrorCode::BudgetExceeded,
                match reason {
                    pm_proxy::PauseReason::LowDisk => "Proxies paused · Low disk space",
                    pm_proxy::PauseReason::InteractiveWork => {
                        "Proxies paused · Playback or export in progress"
                    }
                },
            )),
            |r| r.state = JobState::Paused,
        ),
        _ => {
            if shared
                .lock()
                .jobs
                .get(job)
                .is_some_and(|r| r.state == JobState::Paused)
            {
                shared.status(host, job, |r| r.state = JobState::Running);
            }
        }
    };
    let result = pm_proxy::generate(pm_proxy::GenerationRequest {
        source: &source,
        source_hash: &hash,
        format,
        size,
        ffmpeg: &ffmpeg,
        ffprobe: &ffprobe,
        store: &store,
        control: pm_proxy::GenerationControl {
            cancel,
            pause: &should_pause,
            on_pause: &on_pause,
        },
    });
    match result {
        Ok(generated) => Ok(generated.stored.path.to_string_lossy().into_owned()),
        Err(pm_proxy::ProxyError::Cancelled) => {
            cancel.store(true, Ordering::Relaxed);
            Err(job_error(
                job,
                ErrorCode::Cancelled,
                "Proxy generation cancelled.",
            ))
        }
        Err(e) => Err(job_error(
            job,
            ErrorCode::DecodeFailed,
            format!("Could not create proxy: {e}"),
        )),
    }
}

fn media_source(
    shared: &Shared,
    job: &JobId,
    asset: &AssetId,
    cancel: &AtomicBool,
) -> Result<(PathBuf, String), EngineError> {
    let session = SessionId::ZERO;
    let grant = shared.call(
        Request::MintHandle(MintHandle {
            kind: CapabilityKind::MediaRead {
                asset: asset.clone(),
            },
            session,
        }),
        Some(session),
    )?;
    let pm_protocol::envelope::Response::Handle(grant) = grant else {
        return Err(job_error(
            job,
            ErrorCode::CapabilityRejected,
            "The media handle was not granted.",
        ));
    };
    let resolved = shared.call(
        Request::ResolveHandle(ResolveHandle {
            handle: grant.handle,
            session,
        }),
        Some(session),
    )?;
    let pm_protocol::envelope::Response::HandleResolved(resolved) = resolved else {
        return Err(job_error(
            job,
            ErrorCode::CapabilityRejected,
            "The media handle did not resolve.",
        ));
    };
    if resolved.access != HandleAccess::Read
        || resolved.kind
            != (CapabilityKind::MediaRead {
                asset: asset.clone(),
            })
    {
        return Err(job_error(
            job,
            ErrorCode::CapabilityRejected,
            "The handle names different media.",
        ));
    }
    let path = PathBuf::from(resolved.path);
    let hex = match grant.content_hash {
        Some(hash) => hash
            .0
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        None => pm_media::hash::hash_file(
            &path,
            pm_media::hash::Settle {
                quiet: std::time::Duration::ZERO,
                poll: std::time::Duration::from_millis(100),
            },
            cancel,
            &|_| {},
        )
        .map_err(|e| {
            job_error(
                job,
                ErrorCode::MediaOffline,
                format!("Could not hash original: {e}"),
            )
        })?
        .hex(),
    };
    Ok((path, hex))
}

fn cache_path(shared: &Shared, job: &JobId) -> Result<PathBuf, EngineError> {
    let session = SessionId::ZERO;
    let grant = shared.call(
        Request::MintHandle(MintHandle {
            kind: CapabilityKind::Cache {
                cache: CacheKind::GeneratedMedia,
            },
            session,
        }),
        Some(session),
    )?;
    let pm_protocol::envelope::Response::Handle(grant) = grant else {
        return Err(job_error(
            job,
            ErrorCode::CapabilityRejected,
            "The generated-media store was not granted.",
        ));
    };
    let resolved = shared.call(
        Request::ResolveHandle(ResolveHandle {
            handle: grant.handle,
            session,
        }),
        Some(session),
    )?;
    let pm_protocol::envelope::Response::HandleResolved(resolved) = resolved else {
        return Err(job_error(
            job,
            ErrorCode::CapabilityRejected,
            "The generated-media store did not resolve.",
        ));
    };
    if resolved.access != HandleAccess::ReadWrite
        || resolved.kind
            != (CapabilityKind::Cache {
                cache: CacheKind::GeneratedMedia,
            })
    {
        return Err(job_error(
            job,
            ErrorCode::CapabilityRejected,
            "The handle is not a generated-media store.",
        ));
    }
    Ok(PathBuf::from(resolved.path))
}

fn codec_binaries() -> Result<(PathBuf, PathBuf), String> {
    if let Some(dir) = std::env::var_os("PM_FFMPEG_DIR") {
        let dir = PathBuf::from(dir).join("bin");
        if dir.join("ffmpeg").is_file() && dir.join("ffprobe").is_file() {
            return Ok((dir.join("ffmpeg"), dir.join("ffprobe")));
        }
    }
    if let Some(binary) = std::env::var_os("POWERMOVE_FFMPEG") {
        let binary = PathBuf::from(binary);
        if let Some(dir) = binary.parent() {
            let probe = dir.join("ffprobe");
            if binary.is_file() && probe.is_file() {
                return Ok((binary, probe));
            }
        }
    }
    Err("The packaged LGPL FFmpeg/ffprobe pair is unavailable.".into())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::process::Command;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    use pm_engine::jobs::JobService;
    use pm_protocol::common::{ContentHash, HostMicros, PlaybackRate};
    use pm_protocol::envelope::{Event, Message, Response};
    use pm_protocol::host::{CapabilityHandle, HandleGrant, HandleResolved};
    use pm_protocol::priority::{JobSpec, SubmitJob};
    use pm_protocol::slicing::DeadlineSignal;
    use tempfile::tempdir;

    use super::*;
    use crate::jobs::JobEngineService;

    #[test]
    fn service_resolves_capabilities_and_resumes_generation_after_playback() {
        let (ffmpeg, _) = codec_binaries().expect("source engine/env.sh");
        let temp = tempdir().unwrap();
        let source = temp.path().join("source.mov");
        let store_root = temp.path().join("generated");
        let built = Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=128x72:rate=24:duration=1.5",
                "-c:v",
                "prores_videotoolbox",
                "-profile:v",
                "proxy",
                "-pix_fmt",
                "p216le",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let hash = pm_media::hash::hash_file(
            &source,
            pm_media::hash::Settle {
                quiet: Duration::ZERO,
                poll: Duration::from_millis(1),
            },
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        let asset = AssetId::new("source-asset");
        let job = JobId::new("proxy-job");
        let host = PortId(7);
        let mut service = JobEngineService::start(Arc::new(|| {})).unwrap();
        service.signal(&DeadlineSignal::PlaybackStarted {
            at: HostMicros::ZERO,
            rate: PlaybackRate::NORMAL,
            frame_budget_us: 16_666,
        });
        let accepted = service
            .submit(
                host,
                SubmitJob {
                    job: job.clone(),
                    class: PriorityClass::Generation,
                    run: None,
                    spec: JobSpec::GenerateProxy {
                        asset: asset.clone(),
                        format: ProxyFormat::ProResProxy,
                        size: ProxySize::Half,
                    },
                    output: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(accepted.state, JobState::Queued);

        let source_handle = CapabilityHandle::from_random_bytes([1; 16]);
        let store_handle = CapabilityHandle::from_random_bytes([2; 16]);
        let mut statuses: Vec<pm_protocol::priority::JobStatus> = Vec::new();
        let mut minted = Vec::new();
        let mut resolved = Vec::new();
        let pump = |service: &mut JobEngineService,
                    statuses: &mut Vec<pm_protocol::priority::JobStatus>,
                    minted: &mut Vec<CapabilityKind>,
                    resolved: &mut Vec<CapabilityKind>| {
            let drained = service.drain();
            for (port, envelope) in drained.messages {
                assert_eq!(port, host);
                if let Message::Event {
                    body: Event::JobStatus(status),
                } = envelope.message
                {
                    statuses.push(status);
                }
            }
            for request in drained.host_requests {
                assert_eq!(request.session, Some(SessionId::ZERO));
                let answer = match request.request {
                    Request::MintHandle(mint) => {
                        assert_eq!(mint.session, SessionId::ZERO);
                        let handle = match &mint.kind {
                            CapabilityKind::MediaRead { asset: named } if named == &asset => {
                                source_handle.clone()
                            }
                            CapabilityKind::Cache {
                                cache: CacheKind::GeneratedMedia,
                            } => store_handle.clone(),
                            kind => panic!("unexpected capability kind: {kind:?}"),
                        };
                        minted.push(mint.kind.clone());
                        Response::Handle(HandleGrant {
                            handle,
                            kind: mint.kind.clone(),
                            content_hash: matches!(mint.kind, CapabilityKind::MediaRead { .. })
                                .then_some(ContentHash(hash.blake3)),
                        })
                    }
                    Request::ResolveHandle(resolve) => {
                        assert_eq!(resolve.session, SessionId::ZERO);
                        let (kind, access, path) = if resolve.handle == source_handle {
                            (
                                CapabilityKind::MediaRead {
                                    asset: asset.clone(),
                                },
                                HandleAccess::Read,
                                source.clone(),
                            )
                        } else if resolve.handle == store_handle {
                            (
                                CapabilityKind::Cache {
                                    cache: CacheKind::GeneratedMedia,
                                },
                                HandleAccess::ReadWrite,
                                store_root.clone(),
                            )
                        } else {
                            panic!("unexpected handle")
                        };
                        resolved.push(kind.clone());
                        Response::HandleResolved(HandleResolved {
                            handle: resolve.handle,
                            kind,
                            access,
                            path: path.to_string_lossy().into_owned(),
                        })
                    }
                    other => panic!("unexpected host call: {other:?}"),
                };
                service.host_answer(request.token, Ok(answer));
            }
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while !statuses.iter().any(|s| s.state == JobState::Paused) && Instant::now() < deadline {
            pump(&mut service, &mut statuses, &mut minted, &mut resolved);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            statuses.iter().any(|s| s.state == JobState::Paused),
            "{statuses:?}"
        );
        assert!(
            statuses
                .iter()
                .find(|s| s.state == JobState::Paused)
                .unwrap()
                .error
                .is_some()
        );
        service.signal(&DeadlineSignal::PlaybackStopped {
            at: HostMicros::ZERO,
        });
        while !statuses.iter().any(|s| s.state.is_terminal()) && Instant::now() < deadline {
            pump(&mut service, &mut statuses, &mut minted, &mut resolved);
            std::thread::sleep(Duration::from_millis(10));
        }
        let final_status = statuses
            .iter()
            .find(|s| s.state.is_terminal())
            .expect("job finished");
        assert_eq!(final_status.state, JobState::Done, "{final_status:?}");
        assert!(statuses.iter().any(|s| s.state == JobState::Running));
        assert_eq!(minted, resolved);
        assert_eq!(minted.len(), 2);
        assert_eq!(service.stats().proxies_done, 1);
        let stored = pm_proxy::ProxyStore::open(&store_root)
            .unwrap()
            .find_for_source(
                &hash.hex(),
                Some(ProxyFormat::ProResProxy),
                Some(ProxySize::Half),
            )
            .unwrap()
            .expect("generated proxy persisted");
        assert_eq!(
            final_status.path.as_deref(),
            Some(stored.path.to_str().unwrap())
        );
    }
}
