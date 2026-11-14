//! The decode service: pooled sessions reused across seeks, rule 7
//! selection, read-ahead order by direction, speed, and GOP, the frame cache
//! under a lease, and hardware session arbitration.

mod common;

use common::*;
use pm_media::decode::Backend;
use pm_media::decode::arbiter::{HardwareArbiter, HwCodec, LocalArbiter};
use pm_media::decode::service::{
    DecodeService, Direction, MediaLease, MediaSource, ReadAhead, ServiceConfig, read_ahead_order,
};
use pm_media::vt::DecodePurpose;
use std::sync::Arc;

#[test]
fn sessions_are_pooled_per_source_and_instance() {
    let src = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 0,
        ..Default::default()
    });
    {
        let mut s = svc.session(&src, 7, DecodePurpose::Display).unwrap();
        s.frame(10).unwrap();
        assert_eq!(s.stats().seeks, 1);
    }
    assert_eq!(svc.usage().sessions, 1);
    {
        // The same instance gets the same session back: its counters continue.
        let mut s = svc.session(&src, 7, DecodePurpose::Display).unwrap();
        assert_eq!(s.stats().requests, 1);
        s.frame(45).unwrap();
        // 45 is in another GOP: a seek within the reused session.
        assert_eq!(s.stats().seeks, 2);
        s.frame(46).unwrap();
        assert_eq!(s.stats().seeks, 2, "the next frame continues the run");
        // Another instance gets its own session.
        let t = svc.session(&src, 8, DecodePurpose::Display).unwrap();
        assert_eq!(t.stats().requests, 0);
    }
    assert_eq!(svc.usage().sessions, 2);
    svc.release(Some(7), None);
    assert_eq!(svc.usage().sessions, 1);
    svc.release(None, Some(src.id));
    assert_eq!(svc.usage().sessions, 0);
}

#[test]
fn rule_7_selection_through_the_service() {
    let src = MediaSource::open(&fixtures_dir().join("vp9-opus.webm")).unwrap();
    let svc = DecodeService::new(ServiceConfig::default());
    let mut s = svc.session(&src, 1, DecodePurpose::Display).unwrap();
    assert!(
        s.frame_at(-src.time_of(1)).unwrap().is_none(),
        "before the first frame"
    );
    // At each frame's own time, rule 7 selects that frame (1 ms WebM timebase).
    for n in [0, 1, 17, 59] {
        let f = s.frame_at(src.time_of(n)).unwrap().unwrap();
        assert_eq!(f.pts, src.index.samples[src.index.frames[n].sample].cts);
    }
}

/// Frame-threaded VP9 must feed future packets before returning the requested
/// picture. Those extra buffers caused a viewer's small lease to discard its
/// decoder and seek from the GOP start on almost every frame. Display uses
/// intra-picture threads; every pixel must still equal the export decoder.
#[test]
fn vp9_display_has_no_threading_delay_and_matches_export() {
    for (path, backend) in [
        (fixtures_dir().join("vp9-opus.webm"), Backend::Ffmpeg),
        (
            fixtures_dir().join("vp9-alpha-threading.webm"),
            Backend::FfmpegAlphaPair,
        ),
    ] {
        let src = MediaSource::open(&path).unwrap();
        let display = DecodeService::new(ServiceConfig {
            cache_bytes: 0,
            ..Default::default()
        });
        let export = DecodeService::new(ServiceConfig {
            cache_bytes: 0,
            ..Default::default()
        });
        let mut live = display
            .session_with(&src, 1, DecodePurpose::Display, Some(backend))
            .unwrap();
        let mut reference = export
            .session_with(&src, 1, DecodePurpose::Export, Some(backend))
            .unwrap();
        for n in 0..src.pts.len() {
            let frame = live.frame(n).unwrap();
            assert_eq!(
                live.stats().samples_fed as usize,
                src.index.frames[n].sample + 1,
                "{} frame {n}: display must not feed later pictures",
                path.display()
            );
            assert_eq!(
                frame.md5_hex().unwrap(),
                reference.frame(n).unwrap().md5_hex().unwrap(),
                "{} frame {n}",
                path.display()
            );
        }
        // A backwards seek and a new forward run remain exact too.
        for n in [src.pts.len() / 2, 0, 1, src.pts.len() - 1] {
            assert_eq!(
                live.frame(n).unwrap().md5_hex().unwrap(),
                reference.frame(n).unwrap().md5_hex().unwrap()
            );
        }
    }
}

#[test]
fn a_returned_decoder_releases_free_buffers_before_its_decode_state() {
    let src = MediaSource::open(&fixtures_dir().join("vp9-opus.webm")).unwrap();
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 0,
        pool_free_bytes: 8 << 20,
        ..Default::default()
    });
    let mut session = svc
        .session_with(&src, 1, DecodePurpose::Display, Some(Backend::Ffmpeg))
        .unwrap();
    drop(session.frame(0).unwrap());
    let free_bytes = 8 * pm_media::pool::PAGE;
    let budget = svc.usage().decoders + free_bytes;
    svc.apply_lease(MediaLease {
        frame_cache: 0,
        decoders: budget,
    });
    // Simulate a larger buffer retired by another frame. It is entirely
    // reclaimable; retaining it must not cost the running decoder its state.
    drop(svc.pool().acquire(free_bytes).unwrap());
    // Live storage can grow after the free-pool allowance was calculated
    // (another decode worker), leaving yesterday's free allowance too large.
    let pinned = svc.pool().acquire(pm_media::pool::PAGE).unwrap();
    assert!(svc.usage().decoders > budget);
    assert!(svc.usage().pool.free_bytes > 0);
    drop(session);
    let usage = svc.usage();
    assert_eq!(
        usage.sessions, 1,
        "free buffers evicted a warm decoder: {usage:?}"
    );
    assert!(usage.decoders <= budget, "{usage:?}");
    let session = svc
        .session_with(&src, 1, DecodePurpose::Display, Some(Backend::Ffmpeg))
        .unwrap();
    assert_eq!(session.stats().requests, 1, "the same decoder is reused");
    drop(session);
    drop(pinned);
    let usage = svc.apply_lease(MediaLease {
        frame_cache: 0,
        decoders: 0,
    });
    assert_eq!(usage.sessions, 0, "a zero lease still closes it");
}

#[test]
fn frame_cache_obeys_its_lease() {
    let src = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    // Probe one frame's size, then allow four.
    let probe = DecodeService::new(ServiceConfig::default());
    let one = probe
        .session(&src, 1, DecodePurpose::Display)
        .unwrap()
        .frame(0)
        .unwrap()
        .bytes();
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 4 * one,
        ..Default::default()
    });
    let mut s = svc.session(&src, 1, DecodePurpose::Display).unwrap();
    for n in 0..20 {
        s.frame(n).unwrap();
    }
    let u = svc.usage();
    assert!(u.frame_cache <= 4 * one && u.cache.frames <= 4, "{u:?}");
    assert!(u.cache.evictions > 0);
    // A recent frame is a hit; an evicted one decodes again.
    let hits = u.cache.hits;
    s.frame(19).unwrap();
    assert_eq!(svc.usage().cache.hits, hits + 1);
    // A shrink to zero empties the cache and trims free pool memory.
    let after = svc.apply_lease(MediaLease {
        frame_cache: 0,
        decoders: 0,
    });
    assert_eq!(
        (after.frame_cache, after.cache.frames, after.pool.free_bytes),
        (0, 0, 0)
    );
}

#[test]
fn read_ahead_follows_direction_speed_and_gop() {
    let src = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    let fwd = read_ahead_order(
        &src,
        10,
        ReadAhead {
            direction: Direction::Forward,
            speed: 1.0,
            frames: 5,
        },
    );
    assert_eq!(fwd, vec![11, 12, 13, 14, 15]);
    let fast = read_ahead_order(
        &src,
        10,
        ReadAhead {
            direction: Direction::Forward,
            speed: 2.0,
            frames: 4,
        },
    );
    assert_eq!(fast, vec![12, 14, 16, 18]);
    // Reverse across a GOP boundary: the newer GOP first, each GOP forward.
    let syncs: Vec<usize> = (0..src.index.frames.len())
        .filter(|&n| src.index.samples[src.index.frames[n].sample].key)
        .collect();
    assert!(syncs.len() >= 2);
    let b = syncs[1];
    let rev = read_ahead_order(
        &src,
        b + 2,
        ReadAhead {
            direction: Direction::Reverse,
            speed: 1.0,
            frames: 5,
        },
    );
    assert_eq!(rev, vec![b, b + 1, b - 3, b - 2, b - 1]);
    // Read-ahead decodes into the cache: the frames are then hits.
    let svc = DecodeService::new(ServiceConfig::default());
    let mut s = svc.session(&src, 1, DecodePurpose::Display).unwrap();
    s.frame(10).unwrap();
    let decoded = s
        .read_ahead(
            10,
            ReadAhead {
                direction: Direction::Forward,
                speed: 1.0,
                frames: 5,
            },
        )
        .unwrap();
    assert!(decoded <= 5);
    let hits = svc.usage().cache.hits;
    for n in 11..=15 {
        s.frame(n).unwrap();
    }
    assert_eq!(svc.usage().cache.hits, hits + 5);
}

#[test]
fn hardware_limits_fall_back_or_use_the_software_decoder() {
    let none: Arc<dyn HardwareArbiter> = Arc::new(LocalArbiter::new(0, &[]));
    let svc = DecodeService::with_arbiter(ServiceConfig::default(), none.clone());
    // H.264 without a hardware session: FFmpeg.
    let h264 = MediaSource::open(&fixtures_dir().join("h264-aac.mov")).unwrap();
    let s = svc.session(&h264, 1, DecodePurpose::Display).unwrap();
    assert_eq!(s.info().backend, Backend::Ffmpeg);
    drop(s);
    // ProRes stays in VideoToolbox (decision 12), in its software decoder.
    let prores = MediaSource::open(&fixtures_dir().join("prores422.mov")).unwrap();
    let mut s = svc.session(&prores, 1, DecodePurpose::Display).unwrap();
    let info = s.info();
    assert_eq!(
        (info.backend, info.hardware),
        (Backend::VideoToolbox, false)
    );
    s.frame(3).unwrap();
    // With a permit, hardware is used and the permit is held while the session lives.
    let one = Arc::new(LocalArbiter::new(1, &[]));
    let svc = DecodeService::with_arbiter(
        ServiceConfig {
            idle_sessions: 0,
            ..Default::default()
        },
        one.clone(),
    );
    let s = svc.session(&prores, 1, DecodePurpose::Display).unwrap();
    assert!(s.info().hardware);
    assert_eq!(one.in_use().get(&HwCodec::ProRes), Some(&1));
    drop(s);
    assert_eq!(
        one.in_use().get(&HwCodec::ProRes),
        None,
        "the permit returns with the session"
    );
}

/// The lease report counts CoreVideo frames in the cache, pool frames in the
/// cache, and pool memory outside it (a frame a renderer still holds after
/// eviction stays counted) without double counting.
#[test]
fn lease_report_counts_every_frame_once() {
    let src = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 256 << 20,
        ..Default::default()
    });
    {
        let mut vt = svc
            .session_with(&src, 1, DecodePurpose::Display, Some(Backend::VideoToolbox))
            .unwrap();
        assert_eq!(vt.info().backend, Backend::VideoToolbox);
        for n in 0..5 {
            vt.frame(n).unwrap();
        }
    }
    let u = svc.usage();
    assert!(u.frame_cache > 0 && u.cache.pool_bytes == 0, "{u:?}");
    // The idle session's window shares its frames with the cache.
    assert_eq!(u.pool.external_bytes, u.frame_cache, "{u:?}");
    assert_eq!(u.decoders, u.pool.bytes);
    let held = {
        let mut ff = svc
            .session_with(&src, 2, DecodePurpose::Display, Some(Backend::Ffmpeg))
            .unwrap();
        assert_eq!(ff.info().backend, Backend::Ffmpeg);
        // Another instance: the FFmpeg session decodes its own frames.
        for n in 5..10 {
            ff.frame(n).unwrap();
        }
        ff.frame(9).unwrap()
    };
    let u = svc.usage();
    assert!(
        u.cache.pool_bytes > 0 && u.cache.pool_bytes < u.frame_cache,
        "{u:?}"
    );
    assert_eq!(u.decoders, u.pool.bytes - u.cache.pool_bytes);
    let c = u.lease_categories().unwrap();
    assert_eq!(
        c.total().unwrap().get(),
        (u.frame_cache + u.decoders) as u64
    );
    // Shrink to zero while a renderer holds a pool frame: the cache empties,
    // idle sessions close with their windows, free slots go, and only the
    // held frame remains, in the decoder category.
    let after = svc.apply_lease(MediaLease {
        frame_cache: 0,
        decoders: 0,
    });
    assert_eq!((after.frame_cache, after.cache.pool_bytes), (0, 0));
    assert_eq!(after.sessions, 0, "{after:?}");
    assert_eq!(after.pool.external_bytes, 0, "{after:?}");
    assert_eq!(after.pool.free_bytes, 0, "{after:?}");
    assert_eq!(after.decoders, held.bytes(), "{after:?}");
    drop(held);
    assert_eq!(svc.usage().decoders, 0);
}

/// Frames a session keeps in its window after the cache let them go are
/// counted (VideoToolbox's CoreVideo buffers included), and the lease's
/// decoder share takes them back: from idle sessions at once, from a session
/// in use at its next request, and by closing sessions when that is not
/// enough.
#[test]
fn session_windows_count_and_shrink_under_the_lease() {
    let src = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 0,
        ..Default::default()
    });
    let one = {
        let mut vt = svc
            .session_with(&src, 1, DecodePurpose::Display, Some(Backend::VideoToolbox))
            .unwrap();
        assert_eq!(vt.info().backend, Backend::VideoToolbox);
        for n in 0..5 {
            vt.frame(n).unwrap();
        }
        vt.frame(0).unwrap().bytes()
    };
    // The cache holds nothing; the idle session's window holds the frames.
    let u = svc.usage();
    assert_eq!((u.frame_cache, u.sessions), (0, 1), "{u:?}");
    assert!(u.pool.external_bytes >= 5 * one, "{u:?}");
    assert_eq!(u.decoders, u.pool.bytes + u.pool.external_bytes, "{u:?}");
    // A share of two frames: the idle window empties, the session stays.
    let share = 2 * one;
    let after = svc.apply_lease(MediaLease {
        frame_cache: 0,
        decoders: share,
    });
    assert_eq!((after.decoders, after.sessions), (0, 1), "{after:?}");
    // In use under that share, the window gives frames back as it decodes:
    // only the frame the caller holds counts.
    {
        let mut vt = svc
            .session_with(&src, 1, DecodePurpose::Display, Some(Backend::VideoToolbox))
            .unwrap();
        for n in 5..15 {
            let f = vt.frame(n).unwrap();
            let u = svc.usage();
            assert!(u.decoders <= share, "frame {n}: {u:?}");
            drop(f);
        }
        // A zero share while the session is in use: it closes on return.
        svc.apply_lease(MediaLease {
            frame_cache: 0,
            decoders: 0,
        });
        vt.frame(20).unwrap();
    }
    let u = svc.usage();
    assert_eq!(
        (u.decoders, u.sessions, u.pool.external_buffers),
        (0, 0, 0),
        "{u:?}"
    );
    // Growth restores normal behavior: windows keep frames again.
    svc.apply_lease(MediaLease::UNLIMITED);
    {
        let mut vt = svc
            .session_with(&src, 1, DecodePurpose::Display, Some(Backend::VideoToolbox))
            .unwrap();
        vt.frame(3).unwrap();
    }
    let u = svc.usage();
    assert_eq!(u.sessions, 1, "{u:?}");
    assert!(u.frame_cache > 0 && u.pool.external_bytes > 0, "{u:?}");
}

/// A decoder reference and a cached frame are both pool storage. Keeping a
/// live decoder may borrow unused frame-cache capacity, within the same
/// combined lease; a zero decoder request still closes every session.
#[test]
fn warm_decoder_borrows_only_unused_frame_capacity() {
    let src = MediaSource::open(&fixtures_dir().join("vp9-opus.webm")).unwrap();
    let svc = DecodeService::new(ServiceConfig {
        pool_free_bytes: 0,
        ..Default::default()
    });
    let mut session = svc
        .session_with(&src, 7, DecodePurpose::Display, Some(Backend::Ffmpeg))
        .unwrap();
    let one = session.frame(0).unwrap().bytes();
    // A pinned pool slot models reference storage outside the frame cache.
    let pinned = svc.pool().acquire(8 * pm_media::pool::PAGE).unwrap();
    let used = svc.usage();
    let total = used.frame_cache + used.decoders + 16 * one;
    let usage = svc.apply_lease(MediaLease {
        frame_cache: total - 1,
        decoders: 1,
    });
    assert!(usage.decoders > 1, "{usage:?}");
    drop(session);
    assert_eq!(
        svc.usage().sessions,
        1,
        "unused frame capacity should retain the warm decoder"
    );
    for n in 1..12 {
        let mut session = svc
            .session_with(&src, 7, DecodePurpose::Display, Some(Backend::Ffmpeg))
            .unwrap();
        assert_eq!(
            session.stats().requests,
            n,
            "a decoder was reopened at frame {n}"
        );
        session.frame(n).unwrap();
        drop(session);
        let u = svc.usage();
        assert!(u.frame_cache + u.decoders <= total, "{u:?}");
        assert!(u.frame_cache <= total - 1, "{u:?}");
    }
    drop(pinned);
    let u = svc.apply_lease(MediaLease {
        frame_cache: total,
        decoders: 0,
    });
    assert_eq!(
        u.sessions, 0,
        "zero is a close request even with unused frame capacity"
    );
    let u = svc.apply_lease(MediaLease {
        frame_cache: 0,
        decoders: 0,
    });
    assert_eq!(
        u.frame_cache + u.decoders,
        0,
        "all cache and pool storage returns on a zero lease"
    );
}

#[test]
fn a_displayed_decoder_displaces_cold_idle_state_before_reopening() {
    let src = MediaSource::open(&fixtures_dir().join("vp9-opus.webm")).unwrap();
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 0,
        pool_free_bytes: 0,
        ..Default::default()
    });
    {
        let mut cold = svc
            .session_with(&src, 7, DecodePurpose::Display, Some(Backend::Ffmpeg))
            .unwrap();
        cold.frame(10).unwrap();
    }
    let budget = svc.usage().decoders;
    assert!(budget > 0);
    svc.apply_lease(MediaLease {
        frame_cache: 0,
        decoders: budget,
    });
    svc.note_display(src.id, 8);
    {
        let mut current = svc
            .session_with(&src, 8, DecodePurpose::Display, Some(Backend::Ffmpeg))
            .unwrap();
        current.frame(10).unwrap();
    }
    assert_eq!(svc.usage().sessions, 1);
    assert!(svc.usage().decoders <= budget);
    let mut current = svc
        .session_with(&src, 8, DecodePurpose::Display, Some(Backend::Ffmpeg))
        .unwrap();
    assert_eq!(
        current.stats().requests,
        1,
        "the cold idle decoder evicted the displayed one"
    );
    current.frame(11).unwrap();
    assert_eq!(current.stats().seeks, 1, "the displayed run should continue");
    drop(current);
    assert!(svc.usage().decoders <= budget);
}
