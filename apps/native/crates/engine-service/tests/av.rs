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
