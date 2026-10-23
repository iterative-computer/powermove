#![cfg(target_os = "macos")]

use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::decode::{Backend, DecodeReading};
use pm_media::frame::{DecodedFrame, Pixels};
use pm_media::vt::DecodePurpose;
use std::path::PathBuf;

fn visible_plane(frame: &DecodedFrame, plane: usize) -> Vec<u8> {
    let Pixels::Pool(pool) = &frame.pixels else {
        panic!("expected direct FFmpeg pool frame");
    };
    let w = frame.format.width >> usize::from(plane != 0);
    let h = frame.format.height >> usize::from(plane != 0);
    let p = &pool.planes[plane];
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = unsafe { std::slice::from_raw_parts(p.ptr().add(y * p.stride), w) };
        out.extend_from_slice(row);
    }
    out
}

#[test]
fn chromium_full_range_h264_exposes_coded_padding_without_changing_visible_pixels() {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../pm-render/tests/fixtures/video-parity/h264-709-full.mp4");

    let source = MediaSource::open(&file).unwrap();
    assert_eq!(source.info.codec, "h264");
    assert_eq!(source.info.bit_depth, 8);
    assert!(source.info.full_range);
    let service = DecodeService::new(ServiceConfig::default());
    let mut native = service
        .session_with(&source, 1, DecodePurpose::Export, Some(Backend::Ffmpeg))
        .unwrap();
    let mut chromium = service
        .session_with_reading(&source, 2, DecodePurpose::Export, DecodeReading::Chromium)
        .unwrap();
    assert_eq!(native.info().backend, Backend::Ffmpeg);
    assert_eq!(chromium.info().backend, Backend::Ffmpeg);
    let native = native.frame(6).unwrap();
    let chromium = chromium.frame(6).unwrap();
    assert_eq!(native.pts, chromium.pts);
    assert_eq!((chromium.format.width, chromium.format.height), (256, 144));
    assert_eq!(chromium.format.label, "Chromium I420 coded");
    assert_eq!(
        chromium
            .format
            .planes
            .iter()
            .map(|p| (p.width, p.height))
            .collect::<Vec<_>>(),
        vec![(256, 162), (128, 81), (128, 81)]
    );
    for plane in 0..3 {
        assert_eq!(
            visible_plane(&native, plane),
            visible_plane(&chromium, plane)
        );
    }
    let Pixels::Pool(pool) = &chromium.pixels else {
        unreachable!();
    };
    assert!(!pool.copied);
}

#[test]
fn prepared_decoder_is_reused_without_decoding_a_frame() {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../pm-render/tests/fixtures/video-parity/h264-709-full.mp4");
    let source = MediaSource::open(&file).unwrap();
    let service = DecodeService::new(ServiceConfig::default());
    let backend = service
        .prewarm_session(&source, 17, DecodePurpose::Export, DecodeReading::Chromium)
        .unwrap();
    assert_eq!(backend.backend, Backend::Ffmpeg);
    assert_eq!(service.usage().sessions, 1);
    let session = service
        .session_with_reading(&source, 17, DecodePurpose::Export, DecodeReading::Chromium)
        .unwrap();
    assert_eq!(session.stats().requests, 0);
    assert_eq!(session.info().backend, Backend::Ffmpeg);
}
