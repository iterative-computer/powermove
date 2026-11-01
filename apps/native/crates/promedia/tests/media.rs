use pm_promedia::{media::*, tasks::TaskId};
use std::{io, path::Path};
fn info() -> MediaInfo {
    MediaInfo {
        width: 1920,
        height: 1080,
        frame_rate: Rational {
            numerator: 30000,
            denominator: 1001,
        },
        time_base: Rational {
            numerator: 1,
            denominator: 30000,
        },
        start_tick: -10,
        duration_ticks: 30030,
        alpha: true,
        audio_channels: 2,
        audio_sample_rate: 48000,
        colour: "bt709/srgb/full".into(),
    }
}
fn profile(kind: RenditionKind) -> Profile {
    Profile {
        width: if kind == RenditionKind::Proxy {
            960
        } else {
            1920
        },
        height: if kind == RenditionKind::Proxy {
            540
        } else {
            1080
        },
        codec: "intra-alpha".into(),
        container: "mov".into(),
    }
}
fn original() -> Artifact {
    Artifact {
        path: "/synthetic/original.mov".into(),
        fingerprint: [1; 32],
        info: info(),
    }
}
fn output(kind: RenditionKind) -> Artifact {
    let p = profile(kind);
    let mut i = info();
    i.width = p.width;
    i.height = p.height;
    Artifact {
        path: "/synthetic/derived.mov".into(),
        fingerprint: [2; 32],
        info: i,
    }
}
fn catalog(online: bool) -> MediaCatalog {
    let mut c = MediaCatalog::new("project".into()).unwrap();
    c.register("m".into(), [1; 32], info()).unwrap();
    if online {
        c.relink_original("m", original()).unwrap();
    }
    c
}
fn rendition(kind: RenditionKind) -> Rendition {
    Rendition {
        source: [1; 32],
        profile: profile(kind),
        artifact: output(kind),
        online: true,
    }
}
#[test]
fn proxy_job_preserves_original_and_names_are_stable() {
    let mut c = catalog(true);
    let job = c
        .begin(
            "m",
            RenditionKind::Proxy,
            profile(RenditionKind::Proxy),
            TaskId(1),
        )
        .unwrap();
    let name = job.file_name().unwrap();
    assert!(name.ends_with("-Proxy.mov"));
    assert_eq!(name, job.file_name().unwrap());
    let mut unsafe_job = job.clone();
    unsafe_job.profile.container = "../../escape".into();
    assert!(unsafe_job.file_name().is_err());
    c.complete(&job, output(RenditionKind::Proxy)).unwrap();
    assert_eq!(c.resolve("m", None).unwrap(), &original());
    assert_eq!(
        c.resolve("m", Some(RenditionKind::Proxy)).unwrap(),
        &output(RenditionKind::Proxy)
    );
    assert!(c.status("m").unwrap().jobs.is_empty());
}
#[test]
fn optimized_job_and_status() {
    let mut c = catalog(true);
    let job = c
        .begin(
            "m",
            RenditionKind::Optimized,
            profile(RenditionKind::Optimized),
            TaskId(1),
        )
        .unwrap();
    c.complete(&job, output(RenditionKind::Optimized)).unwrap();
    assert_eq!(c.status("m").unwrap().optimized, Some(true));
}
#[test]
fn offline_proxy_only_never_becomes_original() {
    let mut c = catalog(false);
    c.attach("m", RenditionKind::Proxy, rendition(RenditionKind::Proxy))
        .unwrap();
    let s = c.status("m").unwrap();
    assert_eq!(s.availability, Availability::ProxyOnly);
    assert!(s.export_warning);
    assert!(c.resolve("m", None).is_err());
    assert!(c.resolve("m", Some(RenditionKind::Proxy)).is_ok());
    assert!(
        c.begin(
            "m",
            RenditionKind::Optimized,
            profile(RenditionKind::Optimized),
            TaskId(1)
        )
        .is_err()
    );
}
#[test]
fn attach_detach_and_relink_do_not_delete_payload() {
    let mut c = catalog(true);
    c.attach("m", RenditionKind::Proxy, rendition(RenditionKind::Proxy))
        .unwrap();
    c.rendition_offline("m", RenditionKind::Proxy).unwrap();
    assert_eq!(c.status("m").unwrap().proxy, Some(false));
    let mut wrong = output(RenditionKind::Proxy);
    wrong.fingerprint = [3; 32];
    assert!(
        c.relink_rendition("m", RenditionKind::Proxy, wrong)
            .is_err()
    );
    c.relink_rendition("m", RenditionKind::Proxy, output(RenditionKind::Proxy))
        .unwrap();
    assert!(c.detach("m", RenditionKind::Proxy).unwrap().is_some());
    assert_eq!(c.status("m").unwrap().proxy, None);
}
#[test]
fn relink_original_requires_identity_and_exact_interpretation() {
    let mut c = catalog(false);
    let mut wrong = original();
    wrong.fingerprint = [9; 32];
    assert!(c.relink_original("m", wrong).is_err());
    let mut wrong = original();
    wrong.info.colour = "changed".into();
    assert!(c.relink_original("m", wrong).is_err());
    assert_eq!(c.status("m").unwrap().availability, Availability::Offline);
    c.relink_original("m", original()).unwrap();
    assert!(!c.status("m").unwrap().export_warning);
}
#[test]
fn original_relink_fences_inflight_results() {
    let mut c = catalog(true);
    let job = c
        .begin(
            "m",
            RenditionKind::Proxy,
            profile(RenditionKind::Proxy),
            TaskId(1),
        )
        .unwrap();
    c.relink_original("m", original()).unwrap();
    assert!(c.complete(&job, output(RenditionKind::Proxy)).is_err());
    assert!(c.get("m").unwrap().renditions.is_empty());
}
#[test]
fn detach_and_cancel_fence_old_completion() {
    let mut c = catalog(true);
    let a = c
        .begin(
            "m",
            RenditionKind::Proxy,
            profile(RenditionKind::Proxy),
            TaskId(1),
        )
        .unwrap();
    c.detach("m", RenditionKind::Proxy).unwrap();
    let b = c
        .begin(
            "m",
            RenditionKind::Proxy,
            profile(RenditionKind::Proxy),
            TaskId(2),
        )
        .unwrap();
    assert!(c.complete(&a, output(RenditionKind::Proxy)).is_err());
    c.stop(&b, Some("codec failed".into())).unwrap();
    assert!(c.complete(&b, output(RenditionKind::Proxy)).is_err());
    assert_eq!(
        c.status("m").unwrap().failures[&RenditionKind::Proxy],
        "codec failed"
    );
}
#[test]
fn foreign_project_and_wrong_source_refuse_atomically() {
    let mut a = catalog(true);
    let mut b = MediaCatalog::new("other".into()).unwrap();
    b.register("m".into(), [1; 32], info()).unwrap();
    b.relink_original("m", original()).unwrap();
    let job = a
        .begin(
            "m",
            RenditionKind::Proxy,
            profile(RenditionKind::Proxy),
            TaskId(1),
        )
        .unwrap();
    assert!(b.complete(&job, output(RenditionKind::Proxy)).is_err());
    let mut r = rendition(RenditionKind::Proxy);
    r.source = [4; 32];
    assert!(a.attach("m", RenditionKind::Proxy, r).is_err());
    assert!(a.get("m").unwrap().renditions.is_empty());
}
#[test]
fn every_authored_property_is_validated_before_attachment() {
    let mut cases = Vec::new();
    for n in 0..9 {
        let mut i = output(RenditionKind::Proxy).info;
        match n {
            0 => i.alpha = false,
            1 => i.start_tick += 1,
            2 => i.duration_ticks += 1,
            3 => i.frame_rate.numerator += 1,
            4 => i.time_base.denominator += 1,
            5 => i.audio_channels = 1,
            6 => i.audio_sample_rate = 44100,
            7 => i.colour = "wrong".into(),
            _ => i.width += 1,
        };
        cases.push(i);
    }
    for i in cases {
        let mut c = catalog(true);
        let job = c
            .begin(
                "m",
                RenditionKind::Proxy,
                profile(RenditionKind::Proxy),
                TaskId(1),
            )
            .unwrap();
        let mut out = output(RenditionKind::Proxy);
        out.info = i;
        assert!(c.complete(&job, out).is_err());
        assert_eq!(c.status("m").unwrap().jobs.len(), 1);
        assert!(c.get("m").unwrap().renditions.is_empty());
    }
}
#[test]
fn rational_equivalence_and_no_dimension_loss_for_optimized() {
    assert!(
        Rational {
            numerator: 30,
            denominator: 1
        }
        .equivalent(Rational {
            numerator: 60,
            denominator: 2
        })
    );
    let mut c = catalog(true);
    assert!(
        c.begin(
            "m",
            RenditionKind::Optimized,
            profile(RenditionKind::Proxy),
            TaskId(1)
        )
        .is_err()
    );
}
#[test]
fn multiple_job_kinds_are_independent_but_one_per_kind() {
    let mut c = catalog(true);
    let a = c
        .begin(
            "m",
            RenditionKind::Proxy,
            profile(RenditionKind::Proxy),
            TaskId(1),
        )
        .unwrap();
    let b = c
        .begin(
            "m",
            RenditionKind::Optimized,
            profile(RenditionKind::Optimized),
            TaskId(2),
        )
        .unwrap();
    assert_ne!(a.file_name().unwrap(), b.file_name().unwrap());
    assert!(
        c.begin(
            "m",
            RenditionKind::Proxy,
            profile(RenditionKind::Proxy),
            TaskId(3)
        )
        .is_err()
    );
    c.complete(&a, output(RenditionKind::Proxy)).unwrap();
    c.complete(&b, output(RenditionKind::Optimized)).unwrap();
}
#[test]
fn invalid_ids_profiles_and_metadata_are_rejected() {
    assert!(MediaCatalog::new("".into()).is_err());
    let mut c = catalog(false);
    assert!(c.register("m".into(), [1; 32], info()).is_err());
    let mut p = profile(RenditionKind::Proxy);
    p.container = "../mov".into();
    assert!(p.validate().is_err());
    let mut i = info();
    i.frame_rate.denominator = 0;
    assert!(c.register("bad".into(), [1; 32], i).is_err());
}
struct Probe;
impl MediaProbe for Probe {
    fn probe(&self, _: &Path) -> io::Result<MediaInfo> {
        Ok(info())
    }
}
#[test]
fn artifact_inspection_streams_hash_and_probe() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("media");
    std::fs::write(&path, b"synthetic video bytes").unwrap();
    let a = Artifact::inspect(&path, &Probe).unwrap();
    assert_eq!(
        a.fingerprint,
        *blake3::hash(b"synthetic video bytes").as_bytes()
    );
    assert_eq!(a.info, info());
    assert!(Artifact::inspect(d.path(), &Probe).is_err());
}
#[test]
fn rendition_record_roundtrips_without_losing_provenance() {
    let r = rendition(RenditionKind::Proxy);
    assert_eq!(
        serde_json::from_slice::<Rendition>(&serde_json::to_vec(&r).unwrap()).unwrap(),
        r
    );
}

#[test]
fn reopening_same_project_fences_old_catalog_jobs() {
    let mut a = catalog(true);
    let old = a
        .begin(
            "m",
            RenditionKind::Proxy,
            profile(RenditionKind::Proxy),
            TaskId(1),
        )
        .unwrap();
    let mut b = catalog(true);
    let fresh = b
        .begin(
            "m",
            RenditionKind::Proxy,
            profile(RenditionKind::Proxy),
            TaskId(1),
        )
        .unwrap();
    assert_ne!(old, fresh);
    assert!(b.complete(&old, output(RenditionKind::Proxy)).is_err());
    b.complete(&fresh, output(RenditionKind::Proxy)).unwrap();
}
#[test]
fn coordinated_task_attachment_refuses_cancelled_or_invalid_outputs_atomically() {
    use pm_promedia::tasks::{BackgroundTasks, ProgressUnit, TaskKind, TaskSpec, TaskState};
    for cancel in [false, true] {
        let mut c = catalog(true);
        let mut q = BackgroundTasks::default();
        let id = q
            .enqueue(
                "w".into(),
                "project".into(),
                TaskSpec {
                    title: "Clip".into(),
                    kind: TaskKind::Proxy,
                    unit: ProgressUnit::Frames,
                    priority: 0,
                    total: 10,
                },
            )
            .unwrap();
        let ticket = q.start_next().unwrap();
        let job = c
            .begin("m", RenditionKind::Proxy, profile(RenditionKind::Proxy), id)
            .unwrap();
        if cancel {
            q.cancel(id).unwrap();
        } else {
            let mut bad = output(RenditionKind::Proxy);
            bad.info.alpha = false;
            assert!(c.complete_task(&mut q, ticket, &job, bad).is_err());
            assert_eq!(q.get(id).unwrap().state, TaskState::Running);
        }
        let result = c.complete_task(&mut q, ticket, &job, output(RenditionKind::Proxy));
        if cancel {
            assert!(result.is_err());
            assert!(c.get("m").unwrap().renditions.is_empty());
            assert_eq!(q.get(id).unwrap().state, TaskState::CancelRequested);
        } else {
            result.unwrap();
            assert_eq!(q.get(id).unwrap().state, TaskState::Completed);
            assert_eq!(c.status("m").unwrap().proxy, Some(true));
        }
    }
}
#[test]
fn coordinated_completion_checks_task_kind_and_project() {
    use pm_promedia::tasks::{BackgroundTasks, ProgressUnit, TaskKind, TaskSpec, TaskState};
    for (project, kind) in [("other", TaskKind::Proxy), ("project", TaskKind::Render)] {
        let mut c = catalog(true);
        let mut q = BackgroundTasks::default();
        let id = q
            .enqueue(
                "w".into(),
                project.into(),
                TaskSpec {
                    title: "Clip".into(),
                    kind,
                    unit: ProgressUnit::Frames,
                    priority: 0,
                    total: 10,
                },
            )
            .unwrap();
        let ticket = q.start_next().unwrap();
        let job = c
            .begin("m", RenditionKind::Proxy, profile(RenditionKind::Proxy), id)
            .unwrap();
        assert!(
            c.complete_task(&mut q, ticket, &job, output(RenditionKind::Proxy))
                .is_err()
        );
        assert_eq!(q.get(id).unwrap().state, TaskState::Running);
        assert!(c.get("m").unwrap().renditions.is_empty());
    }
}

#[test]
fn source_offline_retains_optimized_playability_and_fences_pending_jobs() {
    let mut c = catalog(true);
    c.attach(
        "m",
        RenditionKind::Optimized,
        rendition(RenditionKind::Optimized),
    )
    .unwrap();
    let job = c
        .begin(
            "m",
            RenditionKind::Proxy,
            profile(RenditionKind::Proxy),
            TaskId(1),
        )
        .unwrap();
    c.set_original_offline("m").unwrap();
    assert_eq!(
        c.status("m").unwrap().availability,
        Availability::OptimizedOnly
    );
    assert!(c.status("m").unwrap().export_warning);
    assert!(c.get("m").unwrap().original.is_some());
    assert!(c.resolve("m", None).is_err());
    assert!(c.resolve("m", Some(RenditionKind::Optimized)).is_ok());
    assert!(c.complete(&job, output(RenditionKind::Proxy)).is_err());
}
