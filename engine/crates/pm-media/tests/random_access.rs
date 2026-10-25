//! Section 2's Seeking bar as a regression suite: for every file and every
//! backend that decodes it, random access to every composition frame
//! (section 5 rule 7) equals the sequential decode of that frame, with a new
//! session per probe and within one session (seeks with end-of-sequence
//! samples and flushes); backends agree on the normalized picture; FFmpeg's
//! own decode (our LGPL build) gives the same frames; and every file reaches
//! GPU-shareable memory through at least one backend without a copy.
//!
//! The e2e fixtures run by default. The generated corpus
//! (`tests/corpus/gen.sh`) runs with `--ignored`:
//!
//! ```sh
//! cargo test -p pm-media --release --test random_access -- --ignored --test-threads=1
//! ```

mod common;

use common::*;
use pm_media::vt::DecodePurpose;

#[test]
fn e2e_fixtures() {
    run_all(&videos(&fixtures_dir(), &|_| true), DecodePurpose::Display);
}

/// Cancellation can stop a seek after reset, before any picture is fed.
/// EOS must also finish when the next operation is another reset or teardown.
#[test]
fn videotoolbox_reset_without_a_following_picture() {
    use pm_media::decode::service::MediaSource;
    use pm_media::ff::PacketReader;
    use pm_media::vt::{VtConfig, VtDecoder};

    for file in ["h264-aac.mp4", "hevc.mp4"] {
        let src = MediaSource::open(&fixtures_dir().join(file)).unwrap();
        let cfg =
            VtConfig::new(&src.info, &src.index, &src.extradata, DecodePurpose::Export).unwrap();
        for warm in [false, true] {
            for next in ["reset", "sync", "drop"] {
                watchdog::watch(
                    || format!("{file}: reset -> {next}, warm={warm}"),
                    || {
                        let mut decoder = VtDecoder::new(&cfg, src.index.timebase, true).unwrap();
                        assert!(
                            decoder.hardware(),
                            "{file}: this regression requires hardware decode"
                        );
                        let mut output = Vec::new();
                        if warm {
                            let mut reader = PacketReader::new(src.path(), src.stream).unwrap();
                            let packets = reader.read(&src.index, &[0]).unwrap();
                            decoder.send(&packets[0], &mut output).unwrap();
                            decoder.sync(&mut output).unwrap();
                            assert!(!output.is_empty(), "{file}: warm decoder produced no frame");
                            output.clear();
                        }
                        decoder.start_sequence().unwrap();
                        match next {
                            "reset" => decoder.start_sequence().unwrap(),
                            "sync" => decoder.sync(&mut output).unwrap(),
                            _ => {}
                        }
                        assert!(output.is_empty(), "EOS produced a picture");
                        drop(decoder);
                    },
                );
            }
        }
    }
}

/// The corpus files whose names start with `PM_MEDIA_ONLY` (comma
/// separated prefixes), for looking into one file.
#[test]
#[ignore]
fn only() {
    let Ok(only) = std::env::var("PM_MEDIA_ONLY") else {
        return;
    };
    let prefixes: Vec<&str> = only.split(',').collect();
    run_all(
        &videos(&corpus_dir(), &|n| {
            prefixes.iter().any(|p| n.starts_with(p))
        }),
        DecodePurpose::Display,
    );
}

/// The S6 corpus: long GOP (B-pyramid, open-GOP H.264 and HEVC), 4:2:2
/// 10-bit, ProRes, edit lists, VP9 and VP8 with alpha, AV1, coarse
/// timebases (1 ms WebM, 90 kHz TS, 1/600 MOV at 23.976 and 29.97), and 4K.
#[test]
#[ignore]
fn s6_corpus() {
    let s6 = [
        "h264-bpyramid",
        "h264-opengop",
        "hevc-opengop",
        "hevc-main10",
        "hevc-422-10",
        "h264-422-10",
        "prores",
        "editlist-trim",
        "vp9-10bit",
        "vp9-alpha",
        "vp8-alpha",
        "av1-10bit",
        "webm-1ms",
        "ts-90k",
        "mov-600",
        "uhd-",
    ];
    run_all(
        &videos(&corpus_dir(), &|n| s6.iter().any(|p| n.starts_with(p))),
        DecodePurpose::Display,
    );
}

/// ProRes in MXF (RDD 44) and Matroska (`icpf` header stripped):
/// VideoToolbox from the codec tag or profile.
#[test]
#[ignore]
fn prores_in_mxf_and_matroska() {
    let files = videos(&corpus_dir(), &|n| {
        n == "prores-hq.mxf" || n == "prores-hq-2997.mxf" || n == "prores-422.mkv"
    });
    assert_eq!(files.len(), 3);
    for f in &files {
        let r = run_file(f, None, DecodePurpose::Display);
        eprintln!("{}", summary(&r));
        assert_eq!(r.runs[0].backend, pm_media::decode::Backend::VideoToolbox);
        let failures = check(&r);
        assert!(failures.is_empty(), "{failures:#?}");
    }
}

/// ProRes 422 decoded for export: VideoToolbox outputs `v216` (amendment 6).
#[test]
#[ignore]
fn prores_422_export_bound() {
    let files = videos(&corpus_dir(), &|n| {
        n.starts_with("prores422") || n.starts_with("mov-600-prores")
    });
    for f in &files {
        let r = run_file(f, None, DecodePurpose::Export);
        eprintln!("{}", summary(&r));
        assert_eq!(r.runs[0].output, "v216", "{}", r.name);
        let failures = check(&r);
        assert!(failures.is_empty(), "{failures:#?}");
    }
}

/// Interlaced MXF: XDCAM HD422-style long-GOP MPEG-2 (B frames: temporal
/// offsets in the index) and DNxHD intra, both 1080i, from MXF index tables.
#[test]
#[ignore]
fn interlaced_mxf() {
    let files = videos(&corpus_dir(), &|n| {
        n.starts_with("mxf-") && !n.contains("opatom")
    });
    assert_eq!(files.len(), 2);
    for f in &files {
        let r = run_file(f, None, DecodePurpose::Display);
        eprintln!("{}", summary(&r));
        assert_eq!(
            r.source.index.source,
            pm_media::index::IndexSource::MxfIndexTables,
            "{}",
            r.name
        );
        let fields = &r.source.probe.video.as_ref().unwrap().fields;
        assert_eq!(
            fields.order,
            pm_media::probe::FieldOrder::TopFirst,
            "{}: {fields:?}",
            r.name
        );
        let failures = check(&r);
        assert!(failures.is_empty(), "{failures:#?}");
    }
}

/// OP-Atom MXF: clip-wrapped essence, edit units at constant byte offsets
/// inside one KLV.
#[test]
#[ignore]
fn mxf_clip_wrapped() {
    let files = videos(&corpus_dir(), &|n| n.contains("opatom"));
    assert_eq!(files.len(), 1);
    let r = run_file(&files[0], None, DecodePurpose::Display);
    eprintln!("{}", summary(&r));
    assert_eq!(
        r.source.index.source,
        pm_media::index::IndexSource::MxfIndexTables
    );
    assert!(
        r.source
            .index
            .stats
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("clip-wrapped")
    );
    let failures = check(&r);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
#[ignore]
fn dnxhr() {
    run_all(
        &videos(&corpus_dir(), &|n| n.starts_with("dnxhr-")),
        DecodePurpose::Display,
    );
}

#[test]
#[ignore]
fn cineform() {
    run_all(
        &videos(&corpus_dir(), &|n| n.starts_with("cineform")),
        DecodePurpose::Display,
    );
}

#[test]
#[ignore]
fn dv() {
    let files = videos(&corpus_dir(), &|n| n.starts_with("dv"));
    assert_eq!(files.len(), 3);
    for f in &files {
        let r = run_file(f, None, DecodePurpose::Display);
        eprintln!("{}", summary(&r));
        let fields = &r.source.probe.video.as_ref().unwrap().fields;
        assert_eq!(
            fields.order,
            pm_media::probe::FieldOrder::BottomFirst,
            "{}: {fields:?}",
            r.name
        );
        if r.name.ends_with(".dv") {
            assert_eq!(
                r.source.index.source,
                pm_media::index::IndexSource::DvConstantFrames
            );
        }
        let failures = check(&r);
        assert!(failures.is_empty(), "{failures:#?}");
    }
}

#[test]
#[ignore]
fn mjpeg() {
    let files = videos(&corpus_dir(), &|n| n.starts_with("mjpeg"));
    for f in &files {
        let r = run_file(f, None, DecodePurpose::Display);
        eprintln!("{}", summary(&r));
        // MJPEG reads as full range (section 9).
        assert_eq!(r.source.color.range, pm_protocol::color::Range::Full);
        if r.name.ends_with(".avi") {
            assert_eq!(
                r.source.index.source,
                pm_media::index::IndexSource::AviIndex
            );
        }
        let failures = check(&r);
        assert!(failures.is_empty(), "{failures:#?}");
    }
}

/// Fragmented MP4, negative composition offsets, variable frame rate, and
/// MPEG-2 program stream.
#[test]
#[ignore]
fn other_containers() {
    run_all(
        &videos(&corpus_dir(), &|n| {
            n.starts_with("h264-fragmented")
                || n.starts_with("h264-negative-cts")
                || n.starts_with("vfr-")
                || n.starts_with("mpeg2")
        }),
        DecodePurpose::Display,
    );
}

/// The rest of section 13's container list: ASF (WMV), FLV (Sorenson and
/// H.264), M2TS and MTS (192-byte packets), 3GP (H.263), 3G2, Ogg (VP8), a
/// raw MPEG-2 elementary stream (M2V), M4V (MPEG-4 part 2 with B frames), and
/// AVI with B frames (MPEG-4 part 2 and H.264). MP4-family files index from
/// their tables; the rest are scanned, and streams without presentation
/// timestamps on a reordering codec take their order from a decode pass.
#[test]
#[ignore]
fn container_breadth() {
    let files = videos(&corpus_dir(), &|n| n.starts_with("wrap-"));
    assert_eq!(files.len(), 12, "{files:?}");
    let mut failures = vec![];
    for f in &files {
        let r = run_file(f, None, DecodePurpose::Display);
        eprintln!("{}", summary(&r));
        let ext = f.extension().unwrap().to_str().unwrap();
        use pm_media::index::IndexSource::*;
        let want = match ext {
            "3gp" | "3g2" | "m4v" => Mp4Tables,
            "m2v" | "avi" => DemuxScanDecodeOrder,
            _ => DemuxScan,
        };
        let source = r.source.index.source;
        if source != want {
            failures.push(format!("{}: indexed by {source:?}", r.name));
        }
        failures.extend(check(&r));
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
