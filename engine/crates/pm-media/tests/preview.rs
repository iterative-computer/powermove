//! Engine consumer API: posters, ordered streaming filmstrips, original
//! animation timing, source replacement, cancellation and named failures.

use pm_media::preview::PreviewReader;
use pm_media::{MediaError, image};
use std::path::{Path, PathBuf};

fn fixtures(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures")
        .join(name)
}

fn pixel(frame: &pm_media::preview::PreviewFrame, x: usize, y: usize) -> Vec<u8> {
    frame
        .decoded()
        .with_planes(|planes| {
            let p = &planes[0];
            let size = p.desc.format.bytes();
            unsafe { std::slice::from_raw_parts(p.ptr.add(y * p.stride + x * size), size) }.to_vec()
        })
        .unwrap()
}

#[test]
fn original_animation_posters_and_nonmonotonic_filmstrips() {
    for name in ["animated.gif", "animated-apng.png", "animated.webp"] {
        let mut reader = PreviewReader::open(&fixtures(name)).unwrap();
        let timing = image::animation::read_timing(&fixtures(name))
            .unwrap()
            .unwrap();
        assert_eq!((timing.fps, timing.frames), (10.0, 3), "{name}");
        assert!((reader.duration().unwrap() - 0.3).abs() < 1e-9);
        let first = reader.frame_at(0.0).unwrap();
        assert_eq!(
            pixel(&reader.poster().unwrap(), 40, 45),
            pixel(&first, 40, 45)
        );
        let mut seen = Vec::new();
        reader
            .filmstrip(&[0.2, 0.0, 0.1, 0.2], |i, time, f| {
                assert_eq!(i, seen.len());
                let rgba = pixel(&f, 40, 45);
                let channel = (time * 10.0).round() as usize;
                assert!(rgba[channel] >= 200, "{name} {time} {rgba:?}");
                assert_eq!(rgba[3], 255);
                assert_eq!(pixel(&f, 120, 45)[3], 0);
                seen.push(channel);
                Ok(())
            })
            .unwrap();
        assert_eq!(seen, [2, 0, 1, 2]);
        let mut called = 0;
        let result = reader.filmstrip(&[0.0, 0.1, 0.2], |_, _, _| {
            called += 1;
            Err(MediaError::Cancelled)
        });
        assert!(matches!(result, Err(MediaError::Cancelled)));
        assert_eq!(called, 1);
    }
}

#[test]
fn source_replacement_offline_corrupt_and_invalid_requests() {
    let dir = tempfile::Builder::new()
        .prefix("pm-preview-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("original"); // host-store objects need no extension
    std::fs::copy(fixtures("still-red.png"), &path).unwrap();
    let mut reader = PreviewReader::open(&path).unwrap();
    assert_eq!(reader.duration().unwrap(), 0.0);
    let red = pixel(&reader.poster().unwrap(), 1, 1);
    std::fs::copy(fixtures("animated-apng.png"), &path).unwrap();
    assert!((reader.duration().unwrap() - 0.3).abs() < 1e-9);
    assert_ne!(pixel(&reader.frame_at(0.1).unwrap(), 1, 1), red);
    for times in [vec![], vec![0.0; 65], vec![f64::NAN], vec![-1.0]] {
        assert!(
            reader
                .filmstrip(&times, |_, _, _| panic!("invalid request must not decode"))
                .is_err()
        );
    }
    std::fs::write(&path, b"corrupt").unwrap();
    assert!(reader.poster().is_err());
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(reader.poster(), Err(MediaError::Io(_))));
}

#[test]
fn video_poster_and_filmstrip_use_exact_native_decode() {
    // Use a committed video fixture: no browser, subprocess or external corpus.
    let path = fixtures("h264-aac.mp4");
    let mut reader = PreviewReader::open(&path).unwrap();
    let duration = reader.duration().unwrap();
    assert!(duration > 0.0);
    let time = (duration * 0.1).min(0.5);
    let poster = reader.poster().unwrap();
    let exact = reader.frame_at(time).unwrap();
    assert_eq!(poster.decoded().pts, exact.decoded().pts);
    assert_eq!(poster.decoded().format, exact.decoded().format);
    let mut count = 0;
    reader
        .filmstrip(&[time, 0.0, time], |_, _, _| {
            count += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(count, 3);
}

#[test]
fn selected_sequence_skips_gaps_preserves_alpha_and_reads_replaced_members() {
    let dir = tempfile::Builder::new()
        .prefix("pm-preview-sequence-")
        .tempdir()
        .unwrap();
    let paths: Vec<_> = ["frame_12.png", "frame_009.png", "frame_10.PNG"]
        .map(|n| dir.path().join(n))
        .into();
    for path in &paths {
        std::fs::copy(fixtures("still-red.png"), path).unwrap();
    }
    std::fs::copy(fixtures("still-alpha.png"), &paths[0]).unwrap();
    let sequence = image::sequence::ImageSequence::from_paths(&paths).unwrap();
    assert_eq!(sequence.gaps, [(11, 11)]);
    assert_eq!(
        sequence.frames.iter().map(|f| f.0).collect::<Vec<_>>(),
        [9, 10, 12]
    );
    let mut reader = PreviewReader::open_sequence(&paths, 120.0).unwrap();
    assert_eq!(reader.duration().unwrap(), 3.0 / 120.0);
    let first = reader.frame_at(0.0).unwrap();
    let last = reader.frame_at(2.0 / 120.0).unwrap();
    let width = last.decoded().format.width;
    assert_eq!(pixel(&last, width * 3 / 4, 1)[3], 0);
    assert_eq!(pixel(&first, 1, 1)[3], 255);
    std::fs::copy(fixtures("still-alpha.png"), &paths[1]).unwrap();
    let replaced = reader.frame_at(0.0).unwrap();
    assert_eq!(
        pixel(&replaced, replaced.decoded().format.width * 3 / 4, 1)[3],
        0
    );
    let duplicate = vec![paths[0].clone(), dir.path().join("frame_012.png")];
    assert!(PreviewReader::open_sequence(&duplicate, 30.0).is_err());
    assert!(PreviewReader::open_sequence(&paths, f64::NAN).is_err());
    std::fs::remove_file(&paths[0]).unwrap();
    assert!(reader.frame_at(2.0 / 120.0).is_err());
}

#[test]
fn preview_memory_stays_accounted_while_consumers_hold_frames() {
    let dir = tempfile::Builder::new()
        .prefix("pm-preview-memory-")
        .tempdir()
        .unwrap();
    let path = dir.path().join("original");
    std::fs::copy(fixtures("still-red.png"), &path).unwrap();
    let mut reader = PreviewReader::open(&path).unwrap();
    let first = reader.poster().unwrap();
    let bytes = reader.image_bytes();
    assert!(bytes > 0);
    std::fs::copy(fixtures("animated-apng.png"), &path).unwrap();
    let next = reader.frame_at(0.1).unwrap();
    assert!(reader.image_bytes() > bytes);
    drop(first);
    drop(next);
    assert_eq!(reader.image_bytes(), 0);
    std::fs::remove_file(path).unwrap();

    use pm_media::decode::service::{DecodeService, MediaLease, ServiceConfig};
    let service = std::sync::Arc::new(DecodeService::new(ServiceConfig::default()));
    let mut reader =
        PreviewReader::open_with_service(&fixtures("h264-aac.mp4"), service.clone()).unwrap();
    assert!(reader.video_probe().unwrap().is_some());
    let frame = reader.poster().unwrap();
    assert!(service.usage().sessions > 0);
    drop(reader);
    assert_eq!(service.usage().sessions, 0);
    let lease = MediaLease {
        frame_cache: 0,
        decoders: 0,
    };
    assert!(
        service.apply_lease(lease).decoders > 0,
        "the returned frame is still held"
    );
    drop(frame);
    assert_eq!(service.apply_lease(lease).decoders, 0);
}
