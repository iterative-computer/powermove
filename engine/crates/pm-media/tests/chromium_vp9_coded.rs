#![cfg(target_os = "macos")]

use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::decode::{Backend, DecodeReading};
use pm_media::frame::{DecodedFrame, Pixels};
use pm_media::vt::DecodePurpose;
use std::path::PathBuf;

fn visible_plane(frame: &DecodedFrame, plane: usize) -> Vec<u8> {
    let Pixels::Pool(pool) = &frame.pixels else {
        panic!("expected FFmpeg pool planes");
    };
    let chroma = plane == 1 || plane == 2;
    let w = frame.format.width >> usize::from(chroma);
    let h = frame.format.height >> usize::from(chroma);
    let p = &pool.planes[plane];
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = unsafe { std::slice::from_raw_parts(p.ptr().add(y * p.stride), w) };
        out.extend_from_slice(row);
    }
    out
}

#[test]
fn chromium_full_range_vp9_preserves_decoder_coded_rows_and_visible_picture() {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../pm-render/tests/fixtures/video-parity-round3/vp9-709-full-r3.webm");
    let source = MediaSource::open(&file).unwrap();
    assert_eq!(source.info.codec, "vp9");
    assert_eq!(source.info.bit_depth, 8);
    assert!(source.info.full_range);
    assert!(!source.info.alpha_mode);
    let service = DecodeService::new(ServiceConfig::default());
    let mut native = service
        .session_with(&source, 1, DecodePurpose::Export, Some(Backend::Ffmpeg))
        .unwrap();
    let mut chromium = service
        .session_with_reading(&source, 2, DecodePurpose::Export, DecodeReading::Chromium)
        .unwrap();
    assert_eq!(chromium.info().backend, Backend::Ffmpeg);
    assert_eq!(chromium.info().name, "libvpx-vp9");
    let native = native.frame(6).unwrap();
    let chromium = chromium.frame(6).unwrap();
    assert_eq!(native.pts, chromium.pts);
    assert_eq!((chromium.format.width, chromium.format.height), (272, 160));
    assert_eq!(chromium.format.label, "Chromium VP9 stride coded");
    assert_eq!(
        chromium
            .format
            .planes
            .iter()
            .map(|p| (p.width, p.height))
            .collect::<Vec<_>>(),
        vec![(352, 160), (176, 80), (176, 80)]
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
    assert!(pool.copied);
}

#[test]
fn chromium_alpha_vp9_preserves_all_decoder_coded_rows_and_visible_codes() {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../pm-render/tests/fixtures/video-parity-round3/vp9-alpha-709-r3.webm");
    let source = MediaSource::open(&file).unwrap();
    assert_eq!(source.info.codec, "vp9");
    assert!(source.info.alpha_mode);
    assert_eq!(source.info.bit_depth, 8);
    let service = DecodeService::new(ServiceConfig::default());
    let mut native = service.session(&source, 3, DecodePurpose::Export).unwrap();
    let mut chromium = service
        .session_with_reading(&source, 4, DecodePurpose::Export, DecodeReading::Chromium)
        .unwrap();
    assert_eq!(chromium.info().backend, Backend::Ffmpeg);
    assert_eq!(chromium.info().name, "libvpx-vp9");
    for n in [1, 7, 13] {
        let native_frame = native.frame(n).unwrap();
        let chromium_frame = chromium.frame(n).unwrap();
        assert_eq!(chromium_frame.pts, native_frame.pts);
        assert_eq!(
            (chromium_frame.format.width, chromium_frame.format.height),
            (256, 144)
        );
        assert_eq!(chromium_frame.format.alpha_bits, 8);
        assert_eq!(chromium_frame.format.label, "Chromium VP9 stride coded");
        assert_eq!(chromium_frame.format.planes.len(), 4);
        assert_eq!(chromium_frame.format.planes[0].width, 320);
        assert_eq!(chromium_frame.format.planes[3].width, 320);
        for plane in 0..4 {
            assert_eq!(
                visible_plane(&native_frame, plane),
                visible_plane(&chromium_frame, plane),
                "plane {plane}, frame {n}"
            );
        }
        let Pixels::Pool(pool) = &chromium_frame.pixels else {
            unreachable!();
        };
        assert!(pool.copied);
    }
}
