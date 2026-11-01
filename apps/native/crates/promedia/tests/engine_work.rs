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
        cmd.args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo"]);
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
    let work = RenditionWork::prepare(&catalog, &mut tasks, ticket, &job).unwrap();
    tasks.pause(ticket.id).unwrap();
    let report = work.run(&SoftwareEncoder::default(), dir.path(), |_| Control::Pause);
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
