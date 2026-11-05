use pm_promedia::{media::*, proxy::*, tasks::*, worker::*};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    process::{Command, Stdio},
};
use tempfile::TempDir;

fn fixture(root: &Path, audio: bool, offset: bool) -> std::path::PathBuf {
    let output = root.join("source.mkv");
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-v",
        "error",
        "-nostdin",
        "-f",
        "rawvideo",
        "-pixel_format",
        "rgba",
        "-video_size",
        "4x2",
        "-framerate",
        "25",
        "-i",
        "pipe:0",
    ]);
    if audio {
        cmd.args([
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.1*sin(2*PI*440*t)|0.2*sin(2*PI*660*t):s=48000",
        ]);
    }
    if offset {
        cmd.args(["-output_ts_offset", "5"]);
    }
    cmd.args([
        "-frames:v",
        "4",
        "-c:v",
        "ffv1",
        "-level",
        "3",
        "-pix_fmt",
        "bgra",
        "-threads",
        "1",
    ]);
    if audio {
        cmd.args(["-c:a", "pcm_f32le", "-t", "0.16"]);
    }
    cmd.args([
        "-color_range",
        "pc",
        "-colorspace",
        "rgb",
        "-avoid_negative_ts",
        "disabled",
    ]);
    let mut child = cmd.arg(&output).stdin(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for frame in 0..4 {
        for pixel in 0..8 {
            stdin
                .write_all(&[17 + frame, 60 + pixel, 190, 32 + pixel * 24])
                .unwrap();
        }
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
    output
}
fn profile() -> Profile {
    Profile {
        width: 2,
        height: 1,
        codec: "ffv1".into(),
        container: "mkv".into(),
    }
}
fn setup(
    path: &Path,
    kind: RenditionKind,
    profile: Profile,
) -> (MediaCatalog, BackgroundTasks, Ticket, MediaJob) {
    let original = Artifact::inspect(path, &EngineProbe(&SoftwareEncoder::default())).unwrap();
    let mut catalog = MediaCatalog::new("p".into()).unwrap();
    catalog
        .register("m".into(), original.fingerprint, original.info.clone())
        .unwrap();
    catalog.relink_original("m", original).unwrap();
    let mut tasks = BackgroundTasks::default();
    let task = tasks
        .enqueue(
            "w".into(),
            "p".into(),
            TaskSpec {
                title: "Synthetic media".into(),
                kind: if kind == RenditionKind::Proxy {
                    TaskKind::Proxy
                } else {
                    TaskKind::Optimized
                },
                unit: ProgressUnit::Frames,
                priority: 0,
                total: 4,
            },
        )
        .unwrap();
    let job = catalog.begin("m", kind, profile, task).unwrap();
    let ticket = tasks.start_next().unwrap();
    (catalog, tasks, ticket, job)
}
#[test]
fn real_proxy_preserves_alpha_audio_pts_and_original_identity() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), true, false);
    let original_bytes = fs::read(&source).unwrap();
    let backend = SoftwareEncoder::default();
    let (mut catalog, mut tasks, ticket, job) = setup(&source, RenditionKind::Proxy, profile());
    let original = catalog.resolve("m", None).unwrap().clone();
    let work = RenditionWork::prepare(&catalog, &mut tasks, ticket, &job).unwrap();
    let mut frames = Vec::new();
    let report = work.run(&backend, dir.path(), |n| {
        frames.push(n);
        Control::Continue
    });
    assert!(
        matches!(report.outcome, WorkOutcome::Ready(_)),
        "{:?}",
        report.outcome
    );
    report.accept(&mut catalog, &mut tasks).unwrap();
    assert_eq!(tasks.get(ticket.id).unwrap().state, TaskState::Completed);
    let output = catalog.resolve("m", Some(RenditionKind::Proxy)).unwrap();
    assert_eq!((output.info.width, output.info.height), (2, 1));
    assert!(output.info.alpha);
    assert_eq!(output.info.audio_channels, 2);
    backend
        .probe(&source)
        .unwrap()
        .verify_rendition(&backend.probe(&output.path).unwrap(), 2, 1)
        .unwrap();
    let pcm = |path: &Path| {
        let output = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-map", "0:a:0", "-f", "f32le", "pipe:1"])
            .output()
            .unwrap();
        assert!(output.status.success());
        output.stdout
    };
    let original_pcm = pcm(&source);
    assert!(original_pcm.iter().any(|v| *v != 0));
    assert_eq!(pcm(&output.path), original_pcm);
    assert_eq!(catalog.resolve("m", None).unwrap(), &original);
    assert_eq!(fs::read(source).unwrap(), original_bytes);
    assert!(frames.windows(2).all(|n| n[0] <= n[1]));
}
#[test]
fn real_optimized_retains_dimensions_nonzero_start_and_no_audio() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), false, true);
    let mut p = profile();
    p.width = 4;
    p.height = 2;
    let (mut catalog, mut tasks, ticket, job) = setup(&source, RenditionKind::Optimized, p);
    assert!(catalog.resolve("m", None).unwrap().info.start_tick > 0);
    let report = RenditionWork::prepare(&catalog, &mut tasks, ticket, &job)
        .unwrap()
        .run(&SoftwareEncoder::default(), dir.path(), |_| {
            Control::Continue
        });
    assert!(
        matches!(report.outcome, WorkOutcome::Ready(_)),
        "{:?}",
        report.outcome
    );
    report.accept(&mut catalog, &mut tasks).unwrap();
    let output = catalog
        .resolve("m", Some(RenditionKind::Optimized))
        .unwrap();
    assert_eq!((output.info.width, output.info.height), (4, 2));
    assert_eq!(output.info.audio_channels, 0);
}
#[test]
fn cancellation_after_encode_wins_and_retains_private_bytes() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), false, false);
    let (mut catalog, mut tasks, ticket, job) = setup(&source, RenditionKind::Proxy, profile());
    let report = RenditionWork::prepare(&catalog, &mut tasks, ticket, &job)
        .unwrap()
        .run(&SoftwareEncoder::default(), dir.path(), |_| {
            Control::Continue
        });
    assert!(
        matches!(report.outcome, WorkOutcome::Ready(_)),
        "{:?}",
        report.outcome
    );
    tasks.cancel(ticket.id).unwrap();
    report.accept(&mut catalog, &mut tasks).unwrap();
    assert_eq!(tasks.get(ticket.id).unwrap().state, TaskState::Cancelled);
    assert!(catalog.get("m").unwrap().renditions.is_empty());
    assert!(report.staging.unwrap().join("encoded.mkv").exists());
}
#[test]
fn pause_and_restart_have_fresh_jobs_and_private_outputs() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), false, false);
    let (mut catalog, mut tasks, ticket, job) = setup(&source, RenditionKind::Proxy, profile());
    tasks.progress(ticket, 2, 4).unwrap();
    let work = RenditionWork::prepare(&catalog, &mut tasks, ticket, &job).unwrap();
    tasks.pause(ticket.id).unwrap();
    let report = work.run(&SoftwareEncoder::default(), dir.path(), |n| {
        assert_eq!(n, 2);
        Control::Pause
    });
    assert!(report.staging.is_none());
    report.accept(&mut catalog, &mut tasks).unwrap();
    assert_eq!(tasks.get(ticket.id).unwrap().state, TaskState::Paused);
    tasks.resume(ticket.id).unwrap();
    let new_ticket = tasks.start_next().unwrap();
    let new_job = catalog
        .begin("m", RenditionKind::Proxy, profile(), new_ticket.id)
        .unwrap();
    assert_ne!(job.file_name().unwrap(), new_job.file_name().unwrap());
    assert!(RenditionWork::prepare(&catalog, &mut tasks, ticket, &job).is_err());
    let report = RenditionWork::prepare(&catalog, &mut tasks, new_ticket, &new_job)
        .unwrap()
        .run(&SoftwareEncoder::default(), dir.path(), |_| {
            Control::Continue
        });
    report.accept(&mut catalog, &mut tasks).unwrap();
}
#[test]
fn unsupported_profile_is_a_typed_failure_and_never_attaches() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), false, false);
    let mut p = profile();
    p.codec = "not-a-codec".into();
    let (mut catalog, mut tasks, ticket, job) = setup(&source, RenditionKind::Proxy, p);
    let report = RenditionWork::prepare(&catalog, &mut tasks, ticket, &job)
        .unwrap()
        .run(&SoftwareEncoder::default(), dir.path(), |_| {
            Control::Continue
        });
    report.accept(&mut catalog, &mut tasks).unwrap();
    assert_eq!(
        tasks.get(ticket.id).unwrap().presentation().error_code,
        Some(TaskFailureCode::CodecUnsupported)
    );
    assert!(catalog.get("m").unwrap().renditions.is_empty());
}
#[test]
fn changed_source_and_late_report_are_fenced() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), false, false);
    let (mut catalog, mut tasks, ticket, job) = setup(&source, RenditionKind::Proxy, profile());
    let work = RenditionWork::prepare(&catalog, &mut tasks, ticket, &job).unwrap();
    let mut bytes = fs::read(&source).unwrap();
    bytes.extend_from_slice(b"synthetic change");
    fs::write(&source, bytes).unwrap();
    let report = work.run(&SoftwareEncoder::default(), dir.path(), |_| {
        Control::Continue
    });
    assert!(matches!(report.outcome, WorkOutcome::Failed(_)));
    assert!(report.staging.is_none());
    catalog.set_original_offline("m").unwrap();
    assert!(report.accept(&mut catalog, &mut tasks).is_err());
    assert_eq!(tasks.get(ticket.id).unwrap().state, TaskState::Running);
}
#[test]
fn software_encode_never_clobbers_an_existing_output() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), false, false);
    let output = dir.path().join("existing.mkv");
    fs::write(&output, b"keep me").unwrap();
    let request = EncodeRequest {
        source,
        output: output.clone(),
        width: 2,
        height: 1,
        codec: "ffv1".into(),
        container: "mkv".into(),
    };
    assert!(
        SoftwareEncoder::default()
            .encode(&request, &mut |_| Control::Continue)
            .is_err()
    );
    assert_eq!(fs::read(output).unwrap(), b"keep me");
}
#[test]
fn decoded_engine_frame_has_real_straight_alpha_and_exact_ordinal() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), false, false);
    let backend = SoftwareEncoder::default();
    let pixels = backend.rgba_frame(&source, 2, 4, 2).unwrap();
    assert_eq!(pixels.len(), 8);
    assert!((pixels[0][0] - 19.0 / 255.0).abs() < 0.001);
    assert!((pixels[0][3] - 32.0 / 255.0).abs() < 0.001);
    assert!(backend.rgba_frame(&source, 10, 4, 2).is_err());
    assert!(backend.rgba_frame(&source, 0, u32::MAX, u32::MAX).is_err());
}
struct NativeDouble {
    software: SoftwareEncoder,
    changed: bool,
}
impl RenditionEncoder for NativeDouble {
    fn probe(&self, path: &Path) -> io::Result<Probe> {
        let mut p = self.software.probe(path)?;
        if self.changed && path.file_name().unwrap() == "encoded.mkv" {
            p.video.digest[0] ^= 1;
        }
        Ok(p)
    }
    fn encode(
        &self,
        req: &EncodeRequest,
        obs: &mut dyn FnMut(u64) -> Control,
    ) -> io::Result<EncodeOutcome> {
        self.software.encode(req, obs)
    }
}
#[test]
fn native_trait_output_with_altered_sample_timing_is_refused() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), false, false);
    let (mut catalog, mut tasks, ticket, job) = setup(&source, RenditionKind::Proxy, profile());
    let report = RenditionWork::prepare(&catalog, &mut tasks, ticket, &job)
        .unwrap()
        .run(
            &NativeDouble {
                software: SoftwareEncoder::default(),
                changed: true,
            },
            dir.path(),
            |_| Control::Continue,
        );
    assert!(matches!(report.outcome, WorkOutcome::Failed(_)));
    report.accept(&mut catalog, &mut tasks).unwrap();
    assert_eq!(tasks.get(ticket.id).unwrap().state, TaskState::Failed);
    assert!(catalog.get("m").unwrap().renditions.is_empty());
}

#[cfg(unix)]
fn blocking_tool(root: &Path, encode: bool) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("synthetic tool.py");
    let pid = root.join("pid");
    let script = format!(
        "#!/usr/bin/python3\nimport os,sys,time\nfrom pathlib import Path\nPath({:?}).write_text(str(os.getpid()))\n{}\nwhile True: time.sleep(0.05)\n",
        pid.to_str().unwrap(),
        if encode {
            "Path(sys.argv[-1]).write_bytes(b'private partial')\nprint('frame=1',flush=True)"
        } else {
            ""
        }
    );
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    (path, pid)
}
#[cfg(unix)]
fn assert_reaped(pid: &Path) {
    let pid = fs::read_to_string(pid).unwrap();
    assert!(
        !Command::new("kill")
            .args(["-0", &pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
}
#[test]
#[cfg(unix)]
fn blocked_probe_observes_control_and_reaps_its_process() {
    let dir = TempDir::new().unwrap();
    let (tool, pid) = blocking_tool(dir.path(), false);
    let backend = SoftwareEncoder {
        ffprobe: tool,
        ffmpeg: "unused".into(),
    };
    let start = std::time::Instant::now();
    let err = backend
        .probe_controlled(&dir.path().join("synthetic"), &mut || {
            assert!(start.elapsed().as_secs() < 5);
            if pid.exists() {
                Control::Pause
            } else {
                Control::Continue
            }
        })
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    assert_reaped(&pid);
}
#[test]
#[cfg(unix)]
fn running_encode_cancel_releases_process_and_retains_partial_output() {
    let dir = TempDir::new().unwrap();
    let (tool, pid) = blocking_tool(dir.path(), true);
    let backend = SoftwareEncoder {
        ffmpeg: tool,
        ffprobe: "unused".into(),
    };
    let output = dir.path().join("partial.mkv");
    let request = EncodeRequest {
        source: dir.path().join("source"),
        output: output.clone(),
        width: 2,
        height: 1,
        codec: "ffv1".into(),
        container: "mkv".into(),
    };
    let start = std::time::Instant::now();
    let result = backend
        .encode(&request, &mut |n| {
            assert!(start.elapsed().as_secs() < 5);
            if n >= 1 {
                Control::Cancel
            } else {
                Control::Continue
            }
        })
        .unwrap();
    assert_eq!(result, EncodeOutcome::Cancelled);
    assert_reaped(&pid);
    assert_eq!(fs::read(output).unwrap(), b"private partial");
}
// FFmpeg releases differ in whether Matroska's DefaultDuration is exposed on
// decoded frames. Exercise the absent-duration contract with explicit probe
// records, rather than relying on one release to omit the last duration.
#[cfg(unix)]
fn variable_timing_probe(root: &Path, last_duration: &str) -> SoftwareEncoder {
    use std::os::unix::fs::PermissionsExt;
    let tool = root.join("timing-probe.py");
    let streams = serde_json::json!({"streams": [{
        "codec_type": "video", "width": 4, "height": 2, "pix_fmt": "bgra",
        "r_frame_rate": "25/1", "time_base": "1/1000"
    }]});
    let frames = format!(
        "best_effort_timestamp=0|duration=40\nbest_effort_timestamp=40|duration=40\nbest_effort_timestamp=160|duration=40\nbest_effort_timestamp=360{last_duration}\n"
    );
    fs::write(&tool, format!(
        "#!/usr/bin/python3\nimport sys\nprint({:?} if '-show_streams' in sys.argv else {:?})\n",
        streams.to_string(), frames,
    )).unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
    SoftwareEncoder {
        ffprobe: tool,
        ffmpeg: "unused".into(),
    }
}

#[test]
#[cfg(unix)]
fn unverifiable_variable_duration_is_refused_without_inventing_timing() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), false, false);
    let bytes = fs::read(&source).unwrap();
    // Missing, unknown and zero durations must never fall back to 25 fps or
    // the previous PTS gap, including for the final sample.
    for last in ["", "|duration=N/A", "|duration=0"] {
        let error = variable_timing_probe(dir.path(), last)
            .probe(&source)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("exact sample duration"));
    }
    assert_eq!(fs::read(source).unwrap(), bytes);
}

#[test]
#[cfg(unix)]
fn reported_variable_timing_is_preserved_without_frame_rate_inference() {
    let dir = TempDir::new().unwrap();
    let probe = variable_timing_probe(dir.path(), "|duration=40")
        .probe(&dir.path().join("source"))
        .unwrap();
    let mut expected = blake3::Hasher::new();
    for pts in [0i64, 40, 160, 360] {
        expected.update(&pts.to_le_bytes());
        expected.update(&40i64.to_le_bytes());
        expected.update(&0i64.to_le_bytes());
    }
    assert_eq!(probe.video.samples, 4);
    assert_eq!(probe.video.start, 0);
    assert_eq!(probe.video.duration, 400);
    assert_eq!(probe.video.digest, *expected.finalize().as_bytes());
}

#[test]
fn control_during_probe_settles_without_failure_or_output() {
    let dir = TempDir::new().unwrap();
    let source = fixture(dir.path(), false, false);
    let (mut catalog, mut tasks, ticket, job) = setup(&source, RenditionKind::Proxy, profile());
    let work = RenditionWork::prepare(&catalog, &mut tasks, ticket, &job).unwrap();
    let mut polls = 0;
    let report = work.run(&SoftwareEncoder::default(), dir.path(), |_| {
        polls += 1;
        if polls == 4 {
            tasks.pause(ticket.id).unwrap();
        }
        if polls >= 4 {
            Control::Pause
        } else {
            Control::Continue
        }
    });
    assert!(
        matches!(report.outcome, WorkOutcome::Stopped(EncodeOutcome::Paused)),
        "{:?}",
        report.outcome
    );
    assert!(report.staging.is_none());
    report.accept(&mut catalog, &mut tasks).unwrap();
    assert_eq!(tasks.get(ticket.id).unwrap().state, TaskState::Paused);
    assert!(catalog.status("m").unwrap().failures.is_empty());
}

#[test]
fn real_source_frames_flow_through_lut_cache_and_scope_pipeline_at_exact_pts() {
    use pm_promedia::{cache::RenderCache, cache_frame::*, render::*, scopes::*, source_frames::*};
    let dir = TempDir::new().unwrap();
    let path = fixture(dir.path(), false, true);
    let backend = SoftwareEncoder::default();
    let artifact = Artifact::inspect(&path, &EngineProbe(&backend)).unwrap();
    let mut source =
        SoftwareSourceFrames::new(backend, artifact.clone(), "synthetic decoder-v1".into())
            .unwrap();
    let request = FrameRequest {
        identity: source.identity(),
        ticks: (artifact.info.start_tick + 80) * 705600,
        width: 4,
        height: 2,
    };
    let lut = LutTransform::parse(
        "LUT_1D_SIZE 2\n1 1 1\n0 0 0\n",
        request.identity.encoding.clone(),
    )
    .unwrap();
    let work = RenderWork::new(request.clone(), Some(lut)).unwrap();
    let cache = RenderCache::open(&dir.path().join("renders"), 4096).unwrap();
    let config = ScopeConfig {
        columns: 4,
        levels: 256,
        vector_bins: 16,
        min: 0.0,
        max: 1.0,
        matrix: Matrix::Bt709,
    };
    let scope = Some(ScopeRequest {
        stage: AnalysisStage::AfterLut,
        config,
    });
    let result = work.run(&mut source, &cache, scope).unwrap();
    assert!((result.frame.rgba[0][0] - (1.0 - 19.0 / 255.0)).abs() < 0.001);
    assert!((result.frame.rgba[0][3] - 32.0 / 255.0).abs() < 0.001);
    assert_eq!(result.scopes.as_ref().unwrap().pixels, 8);
    assert_eq!(
        result.scopes.as_ref().unwrap().histogram[0]
            .bins
            .iter()
            .sum::<u64>(),
        8
    );
    let again = work.run(&mut source, &cache, scope).unwrap();
    assert!(again.cache_hit);
    assert_eq!(result.frame, again.frame);
    assert_eq!(result.scopes, again.scopes);
    // The real decoder drives Render tasks too, with a cache hit and a fill.
    use pm_promedia::render_job::RenderFillWork;
    let mut next_request = request.clone();
    next_request.ticks += 40 * 705600;
    let next_lut = LutTransform::parse(
        "LUT_1D_SIZE 2\n1 1 1\n0 0 0\n",
        request.identity.encoding.clone(),
    )
    .unwrap();
    let mut tasks = BackgroundTasks::default();
    tasks
        .enqueue(
            "w".into(),
            "p".into(),
            TaskSpec {
                title: "Synthetic source".into(),
                kind: TaskKind::Render,
                unit: ProgressUnit::Frames,
                priority: 0,
                total: 2,
            },
        )
        .unwrap();
    let ticket = tasks.start_next().unwrap();
    let fill = RenderFillWork::prepare(
        &mut tasks,
        ticket,
        "p".into(),
        source.identity(),
        vec![
            work.clone(),
            RenderWork::new(next_request, Some(next_lut)).unwrap(),
        ],
    )
    .unwrap();
    let report = fill.run(&mut source, &cache, |_| Control::Continue);
    fill.accept(&report, &mut tasks, &source.identity())
        .unwrap();
    assert_eq!(tasks.get(ticket.id).unwrap().state, TaskState::Completed);
    assert_eq!(report.completed, 2);
    let mut missing = request.clone();
    missing.ticks += 705600;
    assert!(source.render(&missing).is_err()); // no nearest-frame fallback
    let mut fractional = request;
    fractional.ticks += 1;
    assert!(source.render(&fractional).is_err());
}
#[test]
fn software_readback_identity_tracks_source_interpretation_and_decoder_version() {
    use pm_promedia::{cache_frame::*, source_frames::*};
    let dir = TempDir::new().unwrap();
    let path = fixture(dir.path(), false, false);
    let backend = SoftwareEncoder::default();
    let original = Artifact::inspect(&path, &EngineProbe(&backend)).unwrap();
    let mut source =
        SoftwareSourceFrames::new(backend.clone(), original.clone(), "v1".into()).unwrap();
    let other = SoftwareSourceFrames::new(backend.clone(), original.clone(), "v2".into()).unwrap();
    assert_ne!(source.identity(), other.identity());
    assert!(SoftwareSourceFrames::new(backend.clone(), original.clone(), String::new()).is_err());
    let mut changed = original.clone();
    changed.info.colour.push_str(";different-stage");
    let changed = SoftwareSourceFrames::new(backend, changed, "v1".into()).unwrap();
    assert_ne!(source.identity(), changed.identity());
    let mut request = FrameRequest {
        identity: source.identity(),
        ticks: 0,
        width: 2,
        height: 1,
    };
    assert!(source.render(&request).is_err());
    request.width = 4;
    request.height = 2;
    request.identity.media[0] ^= 1;
    assert!(source.render(&request).is_err());
    request.identity = source.identity();
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend_from_slice(b"synthetic mutation");
    fs::write(path, bytes).unwrap();
    assert!(source.render(&request).is_err());
}

#[test]
fn generic_yuv_scope_readback_refuses_an_implicit_colour_conversion() {
    use pm_promedia::{cache_frame::*, source_frames::*};
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("yuv.mkv");
    assert!(
        Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=4x2:rate=25",
                "-frames:v",
                "4",
                "-c:v",
                "ffv1",
                "-threads:v",
                "1"
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let backend = SoftwareEncoder::default();
    let artifact = Artifact::inspect(&path, &EngineProbe(&backend)).unwrap();
    let mut source = SoftwareSourceFrames::new(backend, artifact, "v1".into()).unwrap();
    let request = FrameRequest {
        identity: source.identity(),
        ticks: 0,
        width: 4,
        height: 2,
    };
    assert_eq!(
        source.render(&request).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
}
