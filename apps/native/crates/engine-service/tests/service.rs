use pm_engine_service::*;
use pm_protocol::{
    clock::{SeekMode, TransportCommand as C},
    common::{PlaybackRate, Size, TICKS_PER_SECOND, TickRange, Ticks},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use transport::{ShuttleKey::*, shuttle};

fn snapshot() -> Snapshot {
    let project = pm_model::json::parse(include_str!("fixtures/shapes.json")).unwrap();
    #[cfg(feature = "kernel")]
    {
        pm_kernel::document::DocumentStore::new_untitled(project).snapshot()
    }
    #[cfg(not(feature = "kernel"))]
    {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Snapshot {
            doc_id: ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            revision: 1,
            document: Arc::new(pm_model::Document::new(
                Some(pm_model::canonical::canonical_project(project)),
                1,
            )),
        }
    }
}
fn scene(s: &Snapshot) -> Scene {
    Scene {
        doc: s.document.clone(),
        comp: s.open_comp().unwrap(),
        media: Default::default(),
        content: s.doc_id,
    }
}
fn config(s: Snapshot) -> ViewerConfig {
    ViewerConfig {
        composition: s.open_comp().unwrap(),
        snapshot: s,
        media: Default::default(),
        output: Output {
            width: 320,
            height: 180,
            display: 0,
            headroom: 1.,
            crop: None,
        },
        layer: None,
        rate: pm_time::Rate::FPS_30,
        end: Some(3 * TICKS_PER_SECOND),
        audio: false,
    }
}
fn wait_frame(v: &Viewer, stamp: Stamp) -> FrameReport {
    let start = Instant::now();
    loop {
        let status = v.status();
        assert!(status.error.is_none(), "{:?}", status.error);
        if let Some(f) = status.frame
            && f.stamp == stamp
        {
            assert!(f.gpu_ok);
            return f;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "frame timed out: {stamp:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn transport_jkl_pause_seek_step_and_loop() {
    let mut t = transport::Transport::new(TICKS_PER_SECOND / 30);
    assert!(shuttle(K, PlaybackRate::STOPPED, true).is_none());
    assert!(shuttle(J, PlaybackRate::STOPPED, false).is_none());
    for (key, expected) in [(L, 1), (L, 2), (L, 2), (J, -1), (J, -2), (J, -2), (L, 1)] {
        t.command(shuttle(key, t.rate(), true).unwrap(), 1_000_000);
        assert_eq!(t.rate().num, expected);
    }
    t.command(C::Pause, 1_500_000);
    assert_eq!(t.rate(), PlaybackRate::STOPPED);
    t.command(
        C::Seek {
            time: Ticks::from_whole_seconds(1),
            mode: SeekMode::Exact,
        },
        2_000_000,
    );
    t.command(C::Step { frames: -1 }, 2_000_000);
    assert_eq!(
        t.poll(2_000_000).media_time.get(),
        TICKS_PER_SECOND - TICKS_PER_SECOND / 30
    );
    t.command(
        C::SetLoop {
            range: Some(TickRange {
                start: Ticks::ZERO,
                end: Ticks::from_whole_seconds(2),
            }),
        },
        2_000_000,
    );
    t.command(
        C::Play {
            rate: PlaybackRate::NORMAL,
        },
        2_000_000,
    );
    assert!(t.poll(5_000_000).media_time < Ticks::from_whole_seconds(2));
}
#[test]
fn cache_key_includes_every_content_coordinate() {
    let k = policy::CacheKey {
        content: 1,
        revision: 3,
        time: 7,
        width: 100,
        height: 100,
        crop: None,
        moment: policy::Moment::Paused,
    };
    for other in [
        policy::CacheKey { content: 2, ..k },
        policy::CacheKey { revision: 4, ..k },
        policy::CacheKey { width: 101, ..k },
        policy::CacheKey { height: 101, ..k },
        policy::CacheKey {
            crop: Some(policy::Crop {
                x: 1,
                y: 0,
                canvas_width: 200,
                canvas_height: 200,
            }),
            ..k
        },
    ] {
        assert!(!k.same_content(other));
    }
    assert_ne!(k, policy::CacheKey { time: 8, ..k });
    // A playing frame is never the exact paused picture of its time.
    assert_ne!(
        k,
        policy::CacheKey {
            moment: policy::Moment::Playing,
            ..k
        }
    );
}
#[test]
fn export_resolver_never_silently_uses_proxies() {
    let c = MediaCatalog {
        originals: [("a".into(), "original.mov".into())].into(),
        proxies: [
            ("a".into(), "proxy.mov".into()),
            ("b".into(), "proxy-only.mov".into()),
        ]
        .into(),
    };
    assert_eq!(c.export_paths(false).paths.len(), 1);
    assert_eq!(
        c.export_paths(false).paths["a"].to_str(),
        Some("original.mov")
    );
    assert_eq!(c.export_paths(true).paths["a"].to_str(), Some("proxy.mov"));
}
#[test]
fn headless_pixels_equal_export_reference_and_overlays_share_pass() {
    let s = snapshot();
    let scene = scene(&s);
    let size = Size::new(320, 180);
    let mut h = Headless::new().unwrap();
    let time = Ticks::from_whole_seconds(1);
    let actual = h.frame(&scene, time, size, &Default::default()).unwrap();
    let reference = h.export_reference(&scene, time, size).unwrap();
    let mut errors: Vec<_> = actual
        .iter()
        .zip(reference.chunks_exact(4))
        .map(|(a, b)| color::delta(*a, color::p3(b)))
        .collect();
    errors.sort_by(f64::total_cmp);
    let p99 = errors[errors.len() * 99 / 100];
    let max = *errors.last().unwrap();
    println!(
        "headless/export ΔE2000: pixels={} p99={p99:.6} max={max:.6}",
        errors.len()
    );
    assert!(p99 <= 1. && max <= 2.);
    assert!(actual.iter().any(|p| p[0] > 0.1)); // Not an all-black false match.
    let overlay = OverlayGeometry::new(vec![OverlayLine {
        ends: [0., 0.5, 1., 0.5],
        color: [0., 1., 0., 1.],
    }])
    .unwrap();
    let drawn = h.frame(&scene, time, size, &overlay).unwrap();
    assert_eq!(drawn[90 * 320 + 160], [0., 1., 0.]);
    assert_eq!(drawn[0], actual[0]);
}
#[test]
fn worker_exact_cache_and_revision_resize_retirement() {
    let s = snapshot();
    let service = EngineService::start(pm_gpu::Gpu::new("service-test").unwrap()).unwrap();
    let v = service.viewer(config(s.clone())).unwrap();
    wait_frame(&v, v.stamp().unwrap());
    v.command(C::Seek {
        time: Ticks::from_whole_seconds(1),
        mode: SeekMode::Scrub,
    })
    .unwrap();
    let first = wait_frame(&v, v.stamp().unwrap());
    assert!(!first.cache_hit);
    v.command(C::Seek {
        time: Ticks::ZERO,
        mode: SeekMode::Scrub,
    })
    .unwrap();
    wait_frame(&v, v.stamp().unwrap());
    v.command(C::Seek {
        time: Ticks::from_whole_seconds(1),
        mode: SeekMode::Scrub,
    })
    .unwrap();
    assert!(wait_frame(&v, v.stamp().unwrap()).cache_hit);
    let old = v.stamp().unwrap();
    let next = Snapshot {
        revision: s.revision + 1,
        document: Arc::new(pm_model::Document::new(
            s.document.to_value(),
            s.revision + 1,
        )),
        ..s.clone()
    };
    v.snapshot(next).unwrap();
    assert!(!wait_frame(&v, v.stamp().unwrap()).cache_hit);
    assert!(v.snapshot(s).is_err());
    assert_ne!(old, v.stamp().unwrap());
    let before = v.stamp().unwrap();
    v.configure(Output {
        width: 160,
        height: 90,
        display: 2,
        headroom: 2.,
        crop: None,
    })
    .unwrap();
    assert_ne!(before.generation, v.stamp().unwrap().generation);
    wait_frame(&v, v.stamp().unwrap());
    // Separate content/session at the same revision cannot collide.
    let b = service.viewer(config(snapshot())).unwrap();
    assert!(!wait_frame(&b, b.stamp().unwrap()).cache_hit);
    v.retire();
    b.retire();
}

/// A main viewer and a popout share one engine. Drawing one must not drop
/// the other's cached frames or reinstall definitions: neither pins resources.
#[test]
fn two_viewers_keep_each_others_cached_frames() {
    let service = EngineService::start(pm_gpu::Gpu::new("two-viewers-test").unwrap()).unwrap();
    let a = service.viewer(config(snapshot())).unwrap();
    let b = service.viewer(config(snapshot())).unwrap();
    wait_frame(&a, a.stamp().unwrap());
    wait_frame(&b, b.stamp().unwrap());
    let seek = |v: &pm_engine_service::Viewer, seconds| {
        v.command(C::Seek {
            time: Ticks::from_whole_seconds(seconds),
            mode: SeekMode::Scrub,
        })
        .unwrap();
        wait_frame(v, v.stamp().unwrap())
    };
    for v in [&a, &b] {
        assert!(!seek(v, 1).cache_hit);
        seek(v, 0);
    }
    for v in [&a, &b] {
        assert!(
            seek(v, 1).cache_hit,
            "the other viewer's frame cleared this one's cache"
        );
    }
    a.retire();
    b.retire();
}
#[test]
fn worker_evaluates_locked_geometry_at_the_rendered_time() {
    let service = EngineService::start(pm_gpu::Gpu::new("locked-overlay-test").unwrap()).unwrap();
    let v = service.viewer(config(snapshot())).unwrap();
    v.locked_overlays(Arc::new(|_doc, _comp, time| {
        let y = if time.get() == 0 { 0.25 } else { 0.75 };
        OverlayGeometry::new(vec![OverlayLine {
            ends: [0., y, 1., y],
            color: [0., 1., 0., 1.],
        }])
    }))
    .unwrap();
    v.capture_next().unwrap();
    let first = wait_frame(&v, v.stamp().unwrap());
    assert_eq!(first.pixels.unwrap()[45 * 320 + 160], [0., 1., 0.]);
    v.scrub(Ticks::from_whole_seconds(1)).unwrap();
    v.capture_next().unwrap();
    let next = wait_frame(&v, v.stamp().unwrap());
    assert_eq!(next.pixels.unwrap()[135 * 320 + 160], [0., 1., 0.]);
}

#[test]
fn silent_composition_with_audio_enabled_seeks_on_the_host_clock() {
    let s = snapshot();
    let mut t = transport::Transport::new(TICKS_PER_SECOND / 30);
    t.enable_audio(
        &s.document,
        s.open_comp().unwrap(),
        Default::default(),
        Box::new(|_| {}),
    )
    .unwrap();
    t.command(
        C::Play {
            rate: PlaybackRate::NORMAL,
        },
        1_000_000,
    );
    assert_eq!(t.poll(1_100_000).rate, PlaybackRate::NORMAL);
    t.command(
        C::Seek {
            time: Ticks::from_whole_seconds(1),
            mode: SeekMode::Exact,
        },
        1_200_000,
    );
    assert_eq!(t.poll(1_200_000).rate, PlaybackRate::NORMAL);
    assert!(
        !t.fallback,
        "silent seek must not wait for an audio-device timeout"
    );
}

#[test]
fn viewer_shuttle_and_scrub_bursts_keep_the_latest_intent() {
    let service = EngineService::start(pm_gpu::Gpu::new("input-burst-test").unwrap()).unwrap();
    let v = service.viewer(config(snapshot())).unwrap();
    assert!(v.shuttle(L, true).unwrap());
    assert!(v.shuttle(L, true).unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while v.status().clock.is_none_or(|s| s.rate.num != 2) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    for i in 0..100 {
        v.scrub(Ticks::new(i * TICKS_PER_SECOND / 30).unwrap())
            .unwrap();
    }
    let f = wait_frame(&v, v.stamp().unwrap());
    assert_eq!(f.time.get(), 99 * TICKS_PER_SECOND / 30);
    assert!(!v.shuttle(K, false).unwrap());
    assert!(!v.shuttle(L, false).unwrap());
}

#[test]
fn paused_notifications_are_coalesced_and_resource_pins_retire_cache() {
    let service = EngineService::start(pm_gpu::Gpu::new("status-resource-test").unwrap()).unwrap();
    let rx = service.notifications();
    let v = service.viewer(config(snapshot())).unwrap();
    wait_frame(&v, v.stamp().unwrap());
    while rx.try_recv().is_ok() {}
    std::thread::sleep(Duration::from_millis(30));
    assert!(
        rx.try_recv().is_err(),
        "paused service generated a periodic wake"
    );
    let old = v.stamp().unwrap();
    v.resources(Arc::new(resources::ResourceSnapshot {
        generation: 2,
        ..Default::default()
    }))
    .unwrap();
    let frame = wait_frame(&v, v.stamp().unwrap());
    assert!(!frame.cache_hit);
    assert!(frame.stamp.generation > old.generation);
    assert!(rx.try_recv().is_ok());
    assert!(v.resources(Arc::default()).is_err());
    for i in 0..100 {
        v.scrub(Ticks::new(i * 1000).unwrap()).unwrap();
    }
    wait_frame(&v, v.stamp().unwrap());
    assert_eq!(rx.len(), 1, "notifications must stay bounded");
}

#[test]
fn pending_edit_origin_belongs_to_the_new_revision() {
    let s = snapshot();
    let engine = EngineService::start(pm_gpu::Gpu::new("edit-origin-test").unwrap()).unwrap();
    let v = engine.viewer(config(s.clone())).unwrap();
    wait_frame(&v, v.stamp().unwrap());
    let input = pm_engine_service::trace::now();
    v.input_event(input);
    // An old-revision frame may complete while the kernel processes the edit.
    v.capture_next().unwrap();
    assert!(
        wait_frame(&v, v.stamp().unwrap())
            .input_host_seconds
            .is_none()
    );
    v.snapshot(Snapshot {
        revision: s.revision + 1,
        document: Arc::new(pm_model::Document::new(
            s.document.to_value(),
            s.revision + 1,
        )),
        ..s
    })
    .unwrap();
    assert_eq!(
        wait_frame(&v, v.stamp().unwrap()).input_host_seconds,
        Some(input)
    );
}

#[test]
fn telemetry_headroom_does_not_invalidate_identical_pixels() {
    let engine =
        EngineService::start(pm_gpu::Gpu::new("headroom-telemetry-test").unwrap()).unwrap();
    let c = config(snapshot());
    let output = c.output;
    let v = engine.viewer(c).unwrap();
    let stamp = v.stamp().unwrap();
    v.configure(Output {
        headroom: 2.,
        crop: None,
        ..output
    })
    .unwrap();
    assert_eq!(v.stamp().unwrap(), stamp);
    v.configure(Output {
        display: output.display + 1,
        ..output
    })
    .unwrap();
    assert_ne!(v.stamp().unwrap(), stamp);
}

#[test]
fn playback_holds_identical_frames_instead_of_re_presenting_them() {
    // 30 fps content under 120 Hz ticks: each content frame is submitted once
    // and held for the remaining ticks; no two consecutive submissions match.
    let service = EngineService::start(pm_gpu::Gpu::new("held-frame-test").unwrap()).unwrap();
    let v = service.viewer(config(snapshot())).unwrap();
    v.command(C::Play {
        rate: PlaybackRate::NORMAL,
    })
    .unwrap();
    let start = Instant::now();
    let mut times = Vec::<Ticks>::new();
    let mut last = None;
    while start.elapsed() < Duration::from_millis(900) {
        v.display_tick(trace::now() + 1. / 120., 1. / 120.).unwrap();
        let status = v.status();
        assert!(status.error.is_none(), "{:?}", status.error);
        if let Some(f) = status.frame
            && f.completion_host_seconds != last
        {
            last = f.completion_host_seconds;
            times.push(f.time);
        }
        std::thread::sleep(Duration::from_micros(8_333));
    }
    let status = v.status();
    assert!(status.wants_display);
    assert!(status.held_frames > 0, "identical ticks must be held");
    assert!(
        times.len() >= 10,
        "content frames keep advancing: {times:?}"
    );
    for pair in times.windows(2) {
        assert_ne!(pair[0], pair[1], "identical frame re-presented: {times:?}");
    }
    v.command(C::Pause).unwrap();
    let start = Instant::now();
    while v.status().wants_display {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn safe_areas_persist_across_revisions_and_display_moves() {
    let s = snapshot();
    let service = EngineService::start(pm_gpu::Gpu::new("safe-area-test").unwrap()).unwrap();
    let v = service.viewer(config(s.clone())).unwrap();
    v.safe_areas(
        OverlayGeometry::new(vec![OverlayLine {
            ends: [0., 0.5, 1., 0.5],
            color: [0., 1., 0., 1.],
        }])
        .unwrap(),
    )
    .unwrap();
    let line = |v: &Viewer, row: usize, width: usize| {
        v.capture_next().unwrap();
        wait_frame(v, v.stamp().unwrap()).pixels.unwrap()[row * width + width / 2]
    };
    assert_eq!(line(&v, 90, 320), [0., 1., 0.]);
    // An edit publishes a new revision: time-invariant overlays stay drawn.
    v.snapshot(Snapshot {
        revision: s.revision + 1,
        document: Arc::new(pm_model::Document::new(
            s.document.to_value(),
            s.revision + 1,
        )),
        ..s.clone()
    })
    .unwrap();
    assert_eq!(line(&v, 90, 320), [0., 1., 0.]);
    // A move to another display/size retires old output; overlays follow the
    // new drawable. A headroom-only change keeps the generation (telemetry).
    let before = v.stamp().unwrap().generation;
    v.configure(Output {
        width: 160,
        height: 90,
        display: 7,
        headroom: 1.,
        crop: None,
    })
    .unwrap();
    assert_ne!(v.stamp().unwrap().generation, before);
    assert_eq!(line(&v, 45, 160), [0., 1., 0.]);
    let moved = v.stamp().unwrap().generation;
    v.configure(Output {
        width: 160,
        height: 90,
        display: 7,
        headroom: 4.,
        crop: None,
    })
    .unwrap();
    assert_eq!(v.stamp().unwrap().generation, moved);
    v.retire();
}

#[test]
fn viewer_output_switch_modes_encode_distinct_finite_signals() {
    // The app's SDR / PQ / HLG switch maps to these engine modes. `Viewer::
    // mastering` also relabels the layer and needs the main thread; the
    // shared final-pass transform is checked through the headless path.
    let s = snapshot();
    let size = Size::new(64, 36);
    let time = Ticks::from_whole_seconds(1);
    let mut frames = vec![];
    for mode in [
        color::Mastering::LinearP3,
        color::Mastering::Pq {
            source_peak: 1000.,
            target_peak: 1000.,
        },
        color::Mastering::Hlg {
            source_peak: 1000.,
            target_peak: 1000.,
        },
    ] {
        let mut h = Headless::new().unwrap();
        h.mastering(mode).unwrap();
        let px = h
            .frame(&scene(&s), time, size, &Default::default())
            .unwrap();
        assert!(px.iter().all(|p| p.iter().all(|c| c.is_finite())));
        frames.push(px);
    }
    assert_ne!(frames[0], frames[1], "PQ must encode, not relabel");
    assert_ne!(frames[1], frames[2], "HLG and PQ are different signals");
    assert!(
        color::Mastering::Pq {
            source_peak: 1000.,
            target_peak: 100.
        }
        .params()
        .is_err(),
        "peaks below reference white are refused"
    );
}
