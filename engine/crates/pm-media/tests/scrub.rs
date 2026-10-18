//! The viewer's decode worker (docs/engine/scrub.md): exact requests equal
//! the sequential decode; scrub requests answer at once with the nearest
//! decoded frame and land the exact one; superseded targets are dropped;
//! look-ahead fills the cache in the direction of motion; long-GOP seeks
//! that leave out unreferenced pictures stay exact.

mod common;

use common::*;
use pm_media::decode::scrub::{ViewerDecode, Want};
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::vt::DecodePurpose;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn service() -> Arc<DecodeService> {
    Arc::new(DecodeService::new(ServiceConfig {
        cache_bytes: 256 << 20,
        ..Default::default()
    }))
}

/// MD5 of every presented frame, decoded in order by a plain session.
fn sequential(src: &Arc<MediaSource>) -> Vec<String> {
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 0,
        ..Default::default()
    });
    let mut s = svc.session(src, 99, DecodePurpose::Display).unwrap();
    s.set_skip_disposable(false);
    (0..src.pts.len())
        .map(|n| s.frame(n).unwrap().md5_hex().unwrap())
        .collect()
}

/// Waits until `f` holds (the worker runs on its own thread).
fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(20), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn exact_requests_equal_the_sequential_decode(path: &Path) {
    let src = MediaSource::open(path).unwrap();
    let want = sequential(&src);
    let viewer = ViewerDecode::new(service());
    let n = src.pts.len();
    for k in [n / 2, 0, n - 1, n / 3, n / 3 + 1, 1, n / 2 - 1] {
        let f = viewer.fetch(&src, 1, k, Want::Exact).unwrap();
        assert!(f.exact);
        assert_eq!(f.shown, k);
        assert_eq!(f.frame.md5_hex().unwrap(), want[k], "frame {k}");
    }
}

#[test]
fn exact_requests_wait_for_their_frame() {
    exact_requests_equal_the_sequential_decode(&fixtures_dir().join("h264-aac.mp4"));
}

#[test]
fn scrub_requests_show_the_nearest_decoded_frame_then_land_the_exact_one() {
    let src = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    let want = sequential(&src);
    let viewer = ViewerDecode::new(service());
    // Nothing decoded yet: the first scrub request waits for its frame.
    let first = viewer.fetch(&src, 1, 5, Want::Scrub).unwrap();
    assert!(first.exact);
    assert_eq!(first.frame.md5_hex().unwrap(), want[5]);
    // Far away, in the other GOP: a stand-in now (the nearest decoded frame,
    // which the worker's look-ahead may have moved on from frame 5).
    let landed = viewer.landed();
    let far = 50;
    let got = viewer.fetch(&src, 1, far, Want::Scrub).unwrap();
    if !got.exact {
        assert!(
            got.shown < 20,
            "a frame near 5 stands in, got {}",
            got.shown
        );
        assert_eq!(got.frame.md5_hex().unwrap(), want[got.shown]);
        eventually("the scrub target lands", || viewer.landed() != landed);
    }
    let exact = viewer.fetch(&src, 1, far, Want::Scrub).unwrap();
    assert!(exact.exact);
    assert_eq!(exact.frame.md5_hex().unwrap(), want[far]);
    let stats = viewer.stats();
    assert_eq!(stats.errors, 0);
    assert!(stats.decoded >= 2, "{stats:?}");
}

#[test]
fn superseded_scrub_targets_are_dropped_and_the_newest_lands() {
    let src = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    let want = sequential(&src);
    let viewer = ViewerDecode::new(service());
    viewer.fetch(&src, 1, 0, Want::Exact).unwrap();
    // A fast drag backwards and forwards across both GOPs.
    let targets = [55, 12, 48, 3, 41, 27, 58];
    for &k in &targets {
        let f = viewer.fetch(&src, 1, k, Want::Scrub).unwrap();
        assert_eq!(
            f.frame.md5_hex().unwrap(),
            want[f.shown],
            "frame shown for {k}"
        );
    }
    let last = *targets.last().unwrap();
    eventually("the newest target lands", || {
        viewer.fetch(&src, 1, last, Want::Scrub).unwrap().exact
    });
    let f = viewer.fetch(&src, 1, last, Want::Scrub).unwrap();
    assert_eq!(f.frame.md5_hex().unwrap(), want[last]);
    let stats = viewer.stats();
    assert!(
        stats.decoded < targets.len() as u64 + 1,
        "superseded targets were not all decoded: {stats:?}"
    );
    assert_eq!(stats.errors, 0);
}

#[test]
fn look_ahead_follows_the_direction_of_motion() {
    let src = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    let svc = service();
    let viewer = ViewerDecode::new(svc.clone());
    let key = |n: usize| pm_media::cache::FrameKey {
        source: src.id,
        sample: src.index.frames[n].sample,
        purpose: DecodePurpose::Display,
    };
    viewer.fetch(&src, 1, 10, Want::Exact).unwrap();
    viewer.fetch(&src, 1, 11, Want::Exact).unwrap();
    // Moving forward: frames after 11 decode ahead into the cache.
    eventually("frames ahead are cached", || {
        (12..16).all(|n| svc.cache().contains(&key(n)))
    });
    assert!(viewer.stats().ahead > 0);
    // A forward drag through them is served from the cache.
    let before = viewer.stats().cached;
    for n in 12..16 {
        assert!(viewer.fetch(&src, 1, n, Want::Scrub).unwrap().exact);
    }
    assert_eq!(viewer.stats().cached, before + 4);
}

#[test]
fn released_sources_stop_their_workers() {
    let src = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    let viewer = ViewerDecode::new(service());
    viewer.fetch(&src, 1, 3, Want::Exact).unwrap();
    viewer.fetch(&src, 2, 3, Want::Exact).unwrap();
    assert_eq!(viewer.stats().workers, 2);
    // Two running workers share the frame cache's look-ahead room.
    assert_eq!(viewer.live_workers(), 2);
    viewer.release(Some(src.id));
    assert_eq!(viewer.live_workers(), 0);
    // A new request starts a new worker.
    viewer.fetch(&src, 1, 4, Want::Exact).unwrap();
    assert_eq!(viewer.stats().workers, 3);
    assert_eq!(viewer.live_workers(), 1);
    assert!(!viewer.pending());
}

/// Workers that exit idle while requests arrive: every request is
/// answered (an exact one with its frame, a scrub one lands), never with
/// "the decode worker stopped" or a target nobody decodes.
#[test]
fn requests_racing_an_idle_exit_are_answered() {
    let src = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    let want = sequential(&src);
    let viewer = ViewerDecode::new(service()).with_idle_exit(Duration::ZERO);
    let n = src.pts.len();
    for i in 0..300usize {
        // Pauses around the idle timeout, so requests meet workers that
        // are exiting.
        std::thread::sleep(Duration::from_micros(((i * 37) % 200) as u64));
        let k = (i * 7) % n;
        if i % 3 == 0 {
            let landed = viewer.landed();
            let f = viewer.fetch(&src, 1, k, Want::Scrub).unwrap();
            if !f.exact {
                eventually("the scrub target landed", || viewer.landed() != landed);
            }
        } else {
            let f = viewer.fetch(&src, 1, k, Want::Exact).unwrap();
            assert!(f.exact);
            assert_eq!(f.frame.md5_hex().unwrap(), want[k], "frame {k}");
        }
    }
    assert!(viewer.stats().workers > 1, "{:?}", viewer.stats());
    assert_eq!(viewer.stats().errors, 0);
}

/// Long GOP with a B pyramid (the generated corpus): exact and scrub
/// requests in a drag stay equal to the sequential decode with
/// unreferenced pictures left out of every seek.
#[test]
#[ignore]
fn long_gop_corpus() {
    for name in ["h264-bpyramid.mp4", "hevc-opengop.mp4", "h264-opengop.mov"] {
        let path = corpus_dir().join(name);
        exact_requests_equal_the_sequential_decode(&path);
        let src = MediaSource::open(&path).unwrap();
        let want = sequential(&src);
        let viewer = ViewerDecode::new(service());
        let n = src.pts.len();
        viewer.fetch(&src, 1, n - 1, Want::Exact).unwrap();
        // A reverse drag, then a forward one, two frames per request.
        let mut shown = 0;
        let mut asked = 0;
        for k in (0..n).rev().step_by(2).chain((0..n).step_by(2)) {
            let f = viewer.fetch(&src, 1, k, Want::Scrub).unwrap();
            assert_eq!(
                f.frame.md5_hex().unwrap(),
                want[f.shown],
                "{name} frame {}",
                f.shown
            );
            shown += usize::from(f.exact);
            asked += 1;
            std::thread::sleep(Duration::from_millis(4));
        }
        let stats = viewer.stats();
        eprintln!("{name}: {shown} exact of {asked} requests; {stats:?}");
        assert_eq!(stats.errors, 0);
    }
}
