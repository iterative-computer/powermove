//! A hardware decode session the system invalidates (a GPU reset, sleep
//! and wake) does not take the clip down with it: the next frame decodes,
//! through a new session, and a stream with a software decoder falls back
//! to it when hardware stays unavailable.

mod common;

use common::*;
use pm_media::decode::Backend;
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::vt::DecodePurpose;

#[test]
fn an_invalidated_hardware_session_is_replaced() {
    for file in ["h264-aac.mp4", "hevc.mp4", "prores422.mov"] {
        let source = MediaSource::open(&fixtures_dir().join(file)).unwrap();
        let frames = source.index.frames.len();
        for purpose in [DecodePurpose::Display, DecodePurpose::Export] {
            let service = DecodeService::new(ServiceConfig {
                // No frame cache: every request must reach the decoder.
                cache_bytes: 0,
                session_window: 1,
                ..ServiceConfig::default()
            });
            let reference: Vec<_> = {
                let mut session = service.session(&source, 1, purpose).unwrap();
                [frames / 2, frames - 1]
                    .map(|n| norm_hash(&session.frame(n).unwrap()))
                    .to_vec()
            };
            let mut session = service.session(&source, 2, purpose).unwrap();
            assert_eq!(session.info().backend, Backend::VideoToolbox, "{file}");
            session.frame(0).unwrap();
            assert!(session.invalidate_hardware());
            // The very next frame, and the session after it goes back to
            // the pool and comes out again.
            assert_eq!(
                norm_hash(&session.frame(frames / 2).unwrap()),
                reference[0],
                "{file} {purpose:?}"
            );
            drop(session);
            let mut session = service.session(&source, 2, purpose).unwrap();
            assert_eq!(
                norm_hash(&session.frame(frames - 1).unwrap()),
                reference[1],
                "{file} {purpose:?}"
            );
            // One session stands in for the failed one, not one more.
            drop(session);
            assert_eq!(service.usage().sessions, 2, "{file}");
        }
    }
}

/// The failed session gives back its hardware before the replacement asks
/// for it: with one H.264 session allowed, the replacement is hardware
/// again, not a software decoder the failed session forced.
#[test]
fn a_replacement_takes_the_failed_sessions_hardware() {
    let source = MediaSource::open(&fixtures_dir().join("h264-aac.mp4")).unwrap();
    let frames = source.index.frames.len();
    let service = DecodeService::with_arbiter(
        ServiceConfig {
            cache_bytes: 0,
            session_window: 1,
            ..ServiceConfig::default()
        },
        std::sync::Arc::new(pm_media::decode::arbiter::LocalArbiter::new(1, &[])),
    );
    let mut session = service.session(&source, 1, DecodePurpose::Display).unwrap();
    assert_eq!(session.info().backend, Backend::VideoToolbox);
    session.frame(0).unwrap();
    assert!(session.invalidate_hardware());
    session.frame(frames / 2).unwrap();
    assert_eq!(session.info().backend, Backend::VideoToolbox);
}

/// A recovery that cannot open any decoder is not tried again on every
/// later request: the failed session stays, marked, and its errors stand.
#[test]
fn a_failed_recovery_is_not_repeated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clip.mp4");
    std::fs::copy(fixtures_dir().join("h264-aac.mp4"), &path).unwrap();
    let source = MediaSource::open(&path).unwrap();
    let frames = source.index.frames.len();
    let service = DecodeService::new(ServiceConfig {
        cache_bytes: 0,
        session_window: 1,
        ..ServiceConfig::default()
    });
    let mut session = service.session(&source, 1, DecodePurpose::Display).unwrap();
    session.frame(0).unwrap();
    // No decoder can open the file any more.
    std::fs::remove_file(&path).unwrap();
    assert!(session.invalidate_hardware());
    assert!(session.frame(frames / 2).is_err());
    let tried = service.recovery_opens();
    assert!(tried > 0);
    for n in [frames / 2 + 1, frames - 1, 1] {
        assert!(session.frame(n).is_err());
    }
    assert_eq!(service.recovery_opens(), tried);
}
