//! Numerical device-callback harness; not a physical speaker/scanout gate.
use pm_engine_service::transport::{AvMeasurements, Transport};
use pm_protocol::{
    clock::TransportCommand,
    common::{HostMicros, PlaybackRate, TICKS_PER_SECOND},
};
use std::time::{Duration, Instant};
#[test]
fn callback_audio_clock_and_presenter_share_one_line() {
    let cache = pm_audio::SourceCache::new(1024 * 1024, 0);
    let (mut playback, mut side) = pm_audio::playback::Playback::start_headless(
        pm_audio::MixFormat::stereo(48_000),
        cache.clone(),
        Default::default(),
        48_000,
        2,
        160_000,
    )
    .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    let mut presenter = Transport::new(TICKS_PER_SECOND / 30);
    let mut measurements = AvMeasurements::default();
    let origin = pm_audio::clock::host_now_us() + 200_000;
    let mut out = [0f32; 1024];
    for i in 0..120u64 {
        let deadline = Instant::now() + Duration::from_secs(10);
        while side.queued_frames() < 512 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let host = origin + i * 512 * 1_000_000 / 48_000;
        side.callback(&mut out, 2, host);
        let sample = playback.clock().expect("callback timestamp");
        presenter.clock.follow(&sample);
        // Independent matching: find when this exact audio media time reaches
        // the display line. Output latency is already in sample.host_time.
        let video_host = presenter
            .clock
            .host_at(sample.media_time.get())
            .unwrap()
            .round() as u64;
        measurements.record(video_host, sample.host_time.get());
        assert_eq!(
            presenter.clock.media_at((host + 1000) as i64),
            sample
                .media_time_at(HostMicros::new(host + 1000).unwrap())
                .get()
        );
    }
    let report = measurements.report().unwrap();
    println!("simulated callback A/V matched-time offset: {report:?}");
    assert!(report.mean_abs_ms <= 0.001 && report.p95_jitter_ms <= 0.001);
    assert!(playback.shutdown());
    cache.shutdown();
}
#[test]
fn empty_av_samples_stay_unmeasured_and_bias_is_not_hidden() {
    let mut m = AvMeasurements::default();
    assert!(m.report().is_none());
    for i in 0..100 {
        m.record(i * 1000 + 3000, i * 1000);
    }
    let r = m.report().unwrap();
    assert_eq!(r.mean_abs_ms, 3.);
    assert_eq!(r.p95_jitter_ms, 0.);
}

/// Decodes an actual PCM clip and crosses multiple wraps and seek epochs.
/// Callback host times are the independent oracle (includes output latency).
#[test]
fn real_clip_loop_and_seek_keep_matched_device_epoch() {
    use pm_audio::offline::{PcmFormat, WavWriter};
    use pm_protocol::{
        clock::SeekMode,
        common::{TickRange, Ticks},
    };
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clip.wav");
    let mut writer = WavWriter::new(
        std::fs::File::create(&path).unwrap(),
        PcmFormat::F32,
        2,
        48_000,
    )
    .unwrap();
    let pcm: Vec<_> = (0..48_000)
        .flat_map(|i| {
            let sample = (i as f32 * std::f32::consts::TAU * 440. / 48_000.).sin() * 0.25;
            [sample, sample]
        })
        .collect();
    writer.write(0, &pcm).unwrap();
    writer.finish().unwrap();
    let doc = pm_model::Document::new(
        Some(
            pm_model::json::parse(
                r#"{
        "id":"P","compId":"C","w":320,"h":180,"fps":60,"dur":1,
        "layers":[{"id":"a","type":"audio","name":"clip","from":0,"dur":1,"on":true,"solo":false,
        "p":{},"fx":[],"masks":[],"d":{"asset":"clip","gain":1,"trim":0,"fadeIn":0,"fadeOut":0}}],
        "comps":{},"assets":{},"params":{}}
    "#,
            )
            .unwrap(),
        ),
        1,
    );
    let cache = pm_audio::SourceCache::new(8 << 20, 2);
    let (mut playback, mut side) = pm_audio::playback::Playback::start_headless(
        pm_audio::MixFormat::stereo(48_000),
        cache.clone(),
        Default::default(),
        48_000,
        2,
        160_000,
    )
    .unwrap();
    let problems = playback
        .set_project(Arc::new(doc), "C", &move |_: &str| Some(path.clone()))
        .unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    let range = TickRange {
        start: Ticks::ZERO,
        end: Ticks::new(TICKS_PER_SECOND / 5).unwrap(),
    };
    playback
        .command(TransportCommand::SetLoop { range: Some(range) })
        .unwrap();
    playback
        .command(TransportCommand::Play {
            rate: PlaybackRate::NORMAL,
        })
        .unwrap();
    let mut presenter = Transport::new(TICKS_PER_SECOND / 60);
    presenter.command(TransportCommand::SetLoop { range: Some(range) }, 0);
    let origin = pm_audio::clock::host_now_us() + 200_000;
    let mut measurements = AvMeasurements::default();
    let mut audible = 0;
    let mut wraps = 0;
    let mut prior = None;
    let mut out = [0.; 960];
    let mut old_epoch = None;
    for i in 0..160u64 {
        if i == 80 {
            old_epoch = playback.clock().map(|s| s.epoch);
            playback
                .command(TransportCommand::Seek {
                    time: Ticks::new(TICKS_PER_SECOND / 10).unwrap(),
                    mode: SeekMode::Exact,
                })
                .unwrap();
            presenter
                .clock
                .hold(TICKS_PER_SECOND / 10, (origin + i * 10_000) as i64);
            // The device must retire old-epoch ring slots after a seek.
            // Waiting for current-epoch PCM before driving it can deadlock
            // the producer behind those slots in a loaded test process.
            side.callback(&mut out, 2, origin + i * 10_000);
            continue;
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while side.queued_frames() < 480 {
            assert!(
                Instant::now() < deadline,
                "clip preparation timed out at callback {i}: {:?}",
                playback.take_errors()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let host = origin + i * 10_000;
        let wall = pm_audio::clock::host_now_us();
        if host > wall {
            std::thread::sleep(Duration::from_micros(host - wall));
        }
        side.callback(&mut out, 2, host);
        audible += out.iter().filter(|v| v.abs() > 0.01).count();
        let sample = playback.clock().unwrap();
        if old_epoch == Some(sample.epoch) {
            continue;
        }
        old_epoch = None;
        if prior.is_some_and(|t| sample.media_time < t) {
            wraps += 1;
        }
        prior = Some(sample.media_time);
        presenter.clock.follow(&sample);
        // Deliberately use the service mapping, never an inverse joined to
        // an unrelated clock sample or a nearest timestamp from another epoch.
        let matched = presenter
            .audio_host_for(sample.media_time, sample.host_time.get() as i64)
            .unwrap();
        measurements.record(matched.round() as u64, sample.host_time.get());
    }
    assert!(
        audible > 1000,
        "must exercise decoded audio, not a silent clock"
    );
    assert!(wraps >= 4, "must cross repeated loop boundaries");
    let report = measurements.report().unwrap();
    println!("real PCM loop/seek callback mapping: {report:?}");
    assert!(playback.shutdown());
    cache.shutdown();
    assert!(
        report.mean_abs_ms <= 1. && report.p95_jitter_ms <= 2.,
        "{report:?}"
    );
}
