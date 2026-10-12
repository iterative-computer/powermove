//! `v1_ingest` (docs/rust-engine.md, section 5, "At the document boundary").

use std::path::PathBuf;

use pm_model::canonical::{canonical_project, project_of};
use pm_model::ingest::{
    FRAME_TOLERANCE, IngestRate, TimeKind, classify_time_path, ingest_project,
    ingest_stored_project, root_is_composition,
};
use pm_model::json::{Value, parse};
use pm_model::patch::PathPart;
use pm_model::path;
use pm_time::{Rate, RateSource, SUPPORTED_RATES, TICKS_PER_SECOND, TimeError, seconds_from_ticks};
use proptest::prelude::*;

/// The float rates documents store for NTSC, with the exact rate each means.
const NTSC_FLOATS: [(f64, Rate); 4] = [
    (29.97, Rate::FPS_29_97),
    (23.976, Rate::FPS_23_976),
    (59.94, Rate::FPS_59_94),
    (29.97002997, Rate::FPS_29_97),
];

fn rate(fps: f64) -> IngestRate {
    IngestRate::from_document(None, Some(fps)).unwrap()
}

/// Ways a frame-aligned time reaches a document from JavaScript: `n / fps`,
/// `n × (1 / fps)`, and a step from the previous frame.
fn legacy_times(n: i64, fps: f64) -> [f64; 3] {
    let nf = n as f64;
    [nf / fps, nf * (1.0 / fps), (nf - 1.0) / fps + 1.0 / fps]
}

#[test]
fn ntsc_floats_ingest_on_their_frames_at_one_million() {
    for (fps, exact) in NTSC_FLOATS {
        let r = rate(fps);
        assert_eq!(r.rate, exact, "{fps}");
        assert_eq!(r.source, RateSource::Ntsc, "{fps}");
        assert_eq!(
            r.fps.to_bits(),
            fps.to_bits(),
            "{fps}: rule 1 uses the stored float"
        );
        for base in [1_000_000i64, -1_000_000] {
            for n in base - 50..=base + 50 {
                for t in legacy_times(n, fps) {
                    let time = r.time(t).unwrap();
                    assert_eq!(time.frame, Some(n), "{fps} n={n} t={t}");
                    assert_eq!(time.ticks, exact.frame_start(n).unwrap(), "{fps} n={n}");
                    assert_eq!(exact.frame_of(time.ticks), n);
                    assert_eq!(time.original.to_bits(), t.to_bits());
                    assert_eq!(time.write_back(&r, time.ticks).to_bits(), t.to_bits());
                }
            }
        }
    }
}

#[test]
fn the_one_million_vectors() {
    // Frame 10^6 at 30000/1001 starts at 10^6 × 23,543,520 ticks
    // (33,366.666… s); at 24000/1001, 10^6 × 29,429,400; at 60000/1001,
    // 10^6 × 11,771,760.
    let cases = [
        (29.97, 23_543_520_000_000i64),
        (29.97002997, 23_543_520_000_000),
        (23.976, 29_429_400_000_000),
        (59.94, 11_771_760_000_000),
    ];
    for (fps, ticks) in cases {
        let time = rate(fps).time(1_000_000.0 / fps).unwrap();
        assert_eq!(time.ticks, ticks, "{fps}");
        // The float time itself is off the exact grid by about one frame
        // (29.97 is 1e-6 slower than 30000/1001); ingest puts it on frame 10^6.
        let nearest = pm_time::ticks_from_seconds(1_000_000.0 / fps).unwrap();
        if fps != 29.97002997 {
            assert!((nearest - ticks).abs() > TICKS_PER_SECOND / 100, "{fps}");
        }
    }
}

#[test]
fn integer_and_exact_rates_ingest_on_their_frames() {
    for (fps, exact) in [
        (24.0, Rate::FPS_24),
        (25.0, Rate::FPS_25),
        (30.0, Rate::FPS_30),
        (60.0, Rate::FPS_60),
        (120.0, Rate::FPS_120),
        (30000.0 / 1001.0, Rate::FPS_29_97),
        (24000.0 / 1001.0, Rate::FPS_23_976),
        (120000.0 / 1001.0, Rate::FPS_119_88),
        (240000.0 / 1001.0, Rate::FPS_239_76),
    ] {
        let r = rate(fps);
        assert_eq!(r.rate, exact);
        for n in [
            0i64,
            1,
            2,
            7,
            1799,
            1800,
            17982,
            1_000_000,
            -1,
            -1_000_000,
            100_000_000,
        ] {
            let time = r.time(n as f64 / fps).unwrap();
            assert_eq!(time.ticks, exact.frame_start(n).unwrap(), "{fps} {n}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 20_000, ..ProptestConfig::default() })]

    #[test]
    fn frame_aligned_floats_land_on_their_frames(which in 0usize..4, n in -100_000_000i64..=100_000_000) {
        let (fps, exact) = NTSC_FLOATS[which];
        let r = rate(fps);
        for t in legacy_times(n, fps) {
            let time = r.time(t).unwrap();
            prop_assert_eq!(time.frame, Some(n));
            prop_assert_eq!(time.ticks, exact.frame_start(n).unwrap());
        }
    }

    #[test]
    fn seconds_of_round_trips_frame_starts(which in 0usize..4, n in -100_000_000i64..=100_000_000) {
        let (fps, exact) = NTSC_FLOATS[which];
        let r = rate(fps);
        let ticks = exact.frame_start(n).unwrap();
        let seconds = r.seconds_of(ticks);
        prop_assert_eq!(seconds.to_bits(), (n as f64 / fps).to_bits());
        prop_assert_eq!(r.time(seconds).unwrap().ticks, ticks);
    }

    #[test]
    fn seconds_of_round_trips_frame_starts_at_every_rate(which in 0usize..SUPPORTED_RATES.len(), n in -100_000_000i64..=100_000_000) {
        // Amendment 1: created frame-aligned times are n / fps_f64, which
        // re-ingests to the same frame at every n, for every supported rate
        // stored as its nearest float.
        let exact = SUPPORTED_RATES[which];
        let r = IngestRate::from_document(None, Some(exact.to_f64())).unwrap();
        prop_assert_eq!(r.rate, exact);
        let ticks = exact.frame_start(n).unwrap();
        prop_assert_eq!(r.time(r.seconds_of(ticks)).unwrap().ticks, ticks);
    }

    #[test]
    fn seconds_of_round_trips_mid_frame_ticks(which in 0usize..4, n in -199_999i64..=199_999, eighths in 2i64..=6) {
        // Ticks at least a quarter frame from a frame start are written as
        // ticks / TICKS. The float grid of a 29.97 document drifts 1e-6 × n
        // frames from the exact one and the tolerance adds 1e-9 × |n|; below
        // 200,000 frames the two stay under a quarter frame.
        let (fps, exact) = NTSC_FLOATS[which];
        let r = rate(fps);
        let ticks = exact.frame_start(n).unwrap() + exact.exact_frame_ticks().unwrap() * eighths / 8;
        let seconds = r.seconds_of(ticks);
        prop_assert_eq!(seconds.to_bits(), seconds_from_ticks(ticks).to_bits());
        prop_assert_eq!(r.time(seconds).unwrap().ticks, ticks);
    }

    #[test]
    fn off_grid_ticks_round_trip_with_exact_floats(which in 0usize..SUPPORTED_RATES.len(), n in -100_000_000i64..=100_000_000, eighths in 1i64..=7) {
        // With the exact rate stored (fpsRational, or a float that is the
        // exact rate), the float grid is the exact grid; off-grid ticks an
        // eighth of a frame or more from a frame start round-trip to ±10⁸.
        let exact = SUPPORTED_RATES[which];
        let r = IngestRate::from_document(Some([exact.num(), exact.den()]), Some(exact.to_f64())).unwrap();
        let ticks = exact.frame_start(n).unwrap() + exact.exact_frame_ticks().unwrap() * eighths / 8;
        let seconds = r.seconds_of(ticks);
        prop_assert_eq!(seconds.to_bits(), seconds_from_ticks(ticks).to_bits());
        let back = r.time(seconds).unwrap();
        prop_assert_eq!(back.ticks, ticks);
        prop_assert_eq!(back.frame, None);
    }

    #[test]
    fn off_grid_floats_keep_their_offset_to_1e8(which in 0usize..4, n in -100_000_000i64..=100_000_000, fraction in 0.2f64..0.8) {
        // Amendment 1: a time a fifth of a frame or more from the document's
        // float grid is never pulled onto a frame, however far from zero
        // (with section 5's 1e-6 this failed beyond 500,000 frames).
        let (fps, _) = NTSC_FLOATS[which];
        let t = (n as f64 + fraction) / fps;
        let time = rate(fps).time(t).unwrap();
        prop_assert_eq!(time.frame, None);
        prop_assert_eq!(time.ticks, pm_time::ticks_from_seconds(t).unwrap());
    }
}

#[test]
fn ticks_over_ticks_drifts_a_frame_at_one_million_with_float_29_97() {
    // Why amendment 1 writes created frame starts with seconds_of: in a
    // document that stores fps 29.97, frame 10^6 written as ticks / TICKS
    // re-ingests as frame 999,999. `seconds_of` writes 10^6 / 29.97, which
    // holds.
    let r = rate(29.97);
    let ticks = Rate::FPS_29_97.frame_start(1_000_000).unwrap();
    let written = seconds_from_ticks(ticks);
    let back = r.time(written).unwrap();
    assert_eq!(back.frame, Some(999_999));
    assert_ne!(back.ticks, ticks);
    let seconds = r.seconds_of(ticks);
    assert_eq!(seconds.to_bits(), (1_000_000.0f64 / 29.97).to_bits());
    let back = r.time(seconds).unwrap();
    assert_eq!((back.frame, back.ticks), (Some(1_000_000), ticks));
    // With the exact float stored, ticks / TICKS lands on the frame too.
    let exact = rate(30000.0 / 1001.0);
    assert_eq!(exact.time(written).unwrap().ticks, ticks);
}

#[test]
fn the_one_million_round_trip_at_every_float_rate() {
    // n = 10^6 (about 9.3 hours at 29.97): ingest, move by whole frames,
    // write back, re-ingest.
    for (fps, exact) in NTSC_FLOATS {
        let r = rate(fps);
        let time = r.time(1_000_000.0 / fps).unwrap();
        assert_eq!(time.ticks, exact.frame_start(1_000_000).unwrap());
        assert_eq!(
            time.write_back(&r, time.ticks).to_bits(),
            (1_000_000.0 / fps).to_bits()
        );
        for delta in [-3i64, -1, 1, 2, 1_000] {
            let moved = exact.frame_start(1_000_000 + delta).unwrap();
            let written = time.write_back(&r, moved);
            assert_eq!(
                written.to_bits(),
                ((1_000_000 + delta) as f64 / fps).to_bits(),
                "{fps} {delta}"
            );
            let back = r.time(written).unwrap();
            assert_eq!(
                (back.frame, back.ticks),
                (Some(1_000_000 + delta), moved),
                "{fps} {delta}"
            );
        }
        // Moved off the grid: ticks / TICKS, and the offset survives.
        let off = time.ticks + exact.exact_frame_ticks().unwrap() / 2;
        let written = time.write_back(&r, off);
        assert_eq!(written.to_bits(), seconds_from_ticks(off).to_bits());
        let back = r.time(written).unwrap();
        assert_eq!((back.frame, back.ticks), (None, off), "{fps}");
    }
}

#[test]
fn off_frame_times_take_the_nearest_tick() {
    let r = rate(29.97);
    for (n, fraction) in [
        (0i64, 0.25),
        (10, 0.5),
        (1_000, 0.01),
        (-30, 0.75),
        (100_000, 0.2),
        (1_000_000, 0.25),
        (100_000_000, 0.3),
    ] {
        let t = (n as f64 + fraction) / 29.97;
        let time = r.time(t).unwrap();
        assert_eq!(time.frame, None, "{n} {fraction}");
        assert_eq!(time.ticks, pm_time::ticks_from_seconds(t).unwrap());
        assert_eq!(time.write_back(&r, time.ticks).to_bits(), t.to_bits());
    }
    // A time within the tolerance of a frame is on it.
    let near = (1_000.0 + 0.5 * FRAME_TOLERANCE * 1_000.0) / 29.97;
    assert_eq!(r.time(near).unwrap().frame, Some(1_000));
    let far = (1_000.0 + 2.0 * FRAME_TOLERANCE * 1_000.0) / 29.97;
    assert_eq!(r.time(far).unwrap().frame, None);
    // Below one frame the tolerance is absolute (1e-9 of a frame).
    let t = 0.4e-9 / 29.97;
    assert_eq!(r.time(t).unwrap().frame, Some(0));
    assert_eq!(r.time(t).unwrap().ticks, 0);
    let t = 2e-9 / 29.97;
    assert_eq!(r.time(t).unwrap().frame, None);
}

#[test]
fn tolerance_is_relative_and_amended() {
    // Amendment 1: 1e-9 × |n|. A quarter frame off the grid at 10^6 frames
    // keeps its offset; with section 5's 1e-6 it landed on frame 10^6.
    assert_eq!(FRAME_TOLERANCE, 1e-9);
    let r = rate(29.97);
    let t = (1_000_000.0 + 0.25) / 29.97;
    let time = r.time(t).unwrap();
    assert_eq!(time.frame, None);
    assert_eq!(time.write_back(&r, time.ticks).to_bits(), t.to_bits());
    // At 10^8 frames the tolerance is a tenth of a frame.
    let t = (100_000_000.0 + 0.05) / 29.97;
    assert_eq!(r.time(t).unwrap().frame, Some(100_000_000));
    let t = (100_000_000.0 + 0.15) / 29.97;
    assert_eq!(r.time(t).unwrap().frame, None);
}

#[test]
fn write_back_echoes_and_creates() {
    let r = rate(24.0);
    let time = r.time(-0.0).unwrap();
    assert_eq!(time.ticks, 0);
    assert_eq!(time.write_back(&r, 0).to_bits(), (-0.0f64).to_bits());
    let time = r.time(0.1).unwrap();
    assert_eq!(time.write_back(&r, time.ticks), 0.1);
    // Moved by a frame from an off-grid time: still off the grid.
    let moved = time.ticks + Rate::FPS_24.exact_frame_ticks().unwrap();
    assert_eq!(time.write_back(&r, moved), seconds_from_ticks(moved));
    // Moved onto a frame start: n / fps.
    let frame = Rate::FPS_24.frame_start(7).unwrap();
    assert_eq!(
        time.write_back(&r, frame).to_bits(),
        (7.0f64 / 24.0).to_bits()
    );
    assert_eq!(r.seconds_of(TICKS_PER_SECOND / 2), 0.5);
}

#[test]
fn rates_follow_fps_rational_then_the_float() {
    let r = IngestRate::from_document(Some([30000, 1001]), Some(29.97)).unwrap();
    assert_eq!(
        (r.rate, r.source, r.fps),
        (Rate::FPS_29_97, RateSource::Exact, 29.97)
    );
    // An fps that disagrees with fpsRational is not used for rule 1.
    let r = IngestRate::from_document(Some([30000, 1001]), Some(30.0)).unwrap();
    assert_eq!(r.rate, Rate::FPS_29_97);
    assert_eq!(r.fps, 30000.0 / 1001.0);
    let r = IngestRate::from_document(Some([25, 1]), None).unwrap();
    assert_eq!((r.rate, r.fps), (Rate::FPS_25, 25.0));
    // An invalid fpsRational falls back to the float.
    let r = IngestRate::from_document(Some([0, 1]), Some(59.94)).unwrap();
    assert_eq!((r.rate, r.source), (Rate::FPS_59_94, RateSource::Ntsc));
    // Unusual rates keep working through the ceil and Euclidean math.
    let r = IngestRate::from_document(None, Some(12.5)).unwrap();
    assert_eq!(r.rate, Rate::new(25, 2).unwrap());
    assert_eq!(
        r.time(1.0 / 12.5 * 3.0).unwrap().ticks,
        3 * TICKS_PER_SECOND * 2 / 25
    );
    assert_eq!(
        IngestRate::from_document(None, None),
        Err(TimeError::InvalidFloatRate)
    );
    assert_eq!(
        IngestRate::from_document(None, Some(0.0)),
        Err(TimeError::InvalidFloatRate)
    );
}

#[test]
fn invalid_times_are_errors() {
    let r = rate(30.0);
    assert_eq!(r.time(f64::NAN), Err(TimeError::NonFiniteSeconds));
    assert_eq!(r.time(f64::INFINITY), Err(TimeError::NonFiniteSeconds));
    assert_eq!(r.time(1e300), Err(TimeError::Overflow));
    assert_eq!(r.time(1e13), Err(TimeError::Overflow));
    // About ±414 years fit.
    assert!(r.time(1e10).is_ok());
}

fn corpus() -> Vec<(String, Value)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corpus");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let value = parse(&std::fs::read_to_string(&p).unwrap()).unwrap();
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                project_of(&value).clone(),
            )
        })
        .collect()
}

#[test]
fn every_corpus_project_ingests_and_paths_classify() {
    let mut total = 0;
    for (name, project) in corpus() {
        let canonical = canonical_project(project.clone());
        let ingested = ingest_project(&canonical).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(ingested, ingest_stored_project(&project).unwrap());
        let root = root_is_composition(&canonical);
        assert!(!ingested.compositions.is_empty(), "{name}");
        for (comp, field) in ingested.times() {
            total += 1;
            assert_eq!(
                classify_time_path(&field.path, root),
                Some((field.kind, comp.path.clone())),
                "{name}: {:?}",
                field.path
            );
            // The path addresses the original number.
            let mut node = &canonical;
            for part in &field.path {
                node = match part {
                    PathPart::Key(k) => node.as_object().unwrap().get_key(k).unwrap(),
                    PathPart::Index(i) => &node.as_array().unwrap()[*i as usize],
                };
            }
            assert_eq!(
                node.as_f64().unwrap().to_bits(),
                field.time.original.to_bits()
            );
            // Synthetic corpus times are frame-aligned or whole milliseconds.
            assert_eq!(
                field
                    .time
                    .write_back(&comp.rate, field.time.ticks)
                    .to_bits(),
                field.time.original.to_bits()
            );
        }
    }
    assert!(total > 500, "{total} times");
}

#[test]
fn corpus_ntsc_compositions_are_exact() {
    for (name, expected) in [
        ("composition-fps-29_970.json", Rate::FPS_29_97),
        ("composition-fps-23_976.json", Rate::FPS_23_976),
    ] {
        let (_, project) = corpus().into_iter().find(|(n, _)| n == name).unwrap();
        let ingested = ingest_stored_project(&project).unwrap();
        for comp in &ingested.compositions {
            assert_eq!(comp.rate.rate, expected, "{name}");
            assert_eq!(comp.rate.source, RateSource::Ntsc);
        }
        let durations: Vec<_> = ingested
            .times()
            .filter(|(_, f)| f.kind == TimeKind::CompositionDuration)
            .collect();
        assert_eq!(durations.len(), ingested.compositions.len());
    }
}

const NESTED: &str = r##"{
  "id": "P", "name": "legacy", "w": 100, "h": 100, "fps": 29.97, "dur": 10, "work": [0, 2.002],
  "markers": [{"t": 1.001001001001001, "name": "m"}],
  "layers": [{
    "id": "L", "type": "captions", "from": 0.5005005005005005, "dur": 3,
    "transitionIn": {"type": "fade", "dur": 0.3336670003336670, "p": {}},
    "transitionOut": null,
    "p": {"opacity": {"v": 1, "kf": [{"t": 0, "v": 0}, {"t": 1, "v": 1}], "expr": null}},
    "fx": [{"id": "e", "type": "blur", "on": true, "p": {"r": {"v": 1, "kf": [{"t": 2, "v": 3}], "expr": null}}}],
    "d": {"cues": [{"id": "c", "start": 0, "end": 1.5, "text": "hi",
                    "words": [{"text": "hi", "start": 0.1, "end": 0.2}]}],
          "trim": 4, "fadeIn": 0.25, "data": {"object": {"p": {"x": {"v": 0, "kf": [{"t": 5, "v": 1}]}}}}}
  }],
  "comps": {"N1": {"id": "N1", "fps": 24, "dur": 1, "layers": [{"id": "a", "from": 0.5, "dur": 1}],
                   "comps": {"N2": {"fps": 25, "dur": 2, "layers": []}}},
            "bad": 5}
}"##;

#[test]
fn legacy_roots_and_nested_compositions() {
    let project = parse(NESTED).unwrap();
    // No compId: the root is the composition.
    assert!(root_is_composition(&project));
    let ingested = ingest_project(&project).unwrap();
    let paths: Vec<_> = ingested
        .compositions
        .iter()
        .map(|c| c.path.clone())
        .collect();
    assert_eq!(
        paths,
        vec![
            vec![],
            path!["comps", "N1"].to_vec(),
            path!["comps", "N1", "comps", "N2"].to_vec(),
        ]
    );
    let rates: Vec<_> = ingested.compositions.iter().map(|c| c.rate.rate).collect();
    assert_eq!(rates, [Rate::FPS_29_97, Rate::FPS_24, Rate::FPS_25]);
    let root = &ingested.compositions[0];
    let kinds: Vec<_> = root.times.iter().map(|f| f.kind).collect();
    let count = |k: TimeKind| kinds.iter().filter(|x| **x == k).count();
    assert_eq!(count(TimeKind::CompositionDuration), 1);
    assert_eq!(count(TimeKind::WorkArea), 2);
    assert_eq!(count(TimeKind::Marker), 1);
    assert_eq!(count(TimeKind::LayerStart), 1);
    assert_eq!(count(TimeKind::LayerDuration), 1);
    assert_eq!(count(TimeKind::TransitionDuration), 1);
    assert_eq!(count(TimeKind::Keyframe), 4);
    assert_eq!(count(TimeKind::Caption), 4);
    assert_eq!(count(TimeKind::SourceOffset), 1);
    assert_eq!(count(TimeKind::Fade), 1);
    // 1.001001… s at 29.97 is frame 30 of the float grid.
    let marker = root
        .times
        .iter()
        .find(|f| f.kind == TimeKind::Marker)
        .unwrap();
    assert_eq!(marker.time.frame, Some(30));
    assert_eq!(marker.time.ticks, Rate::FPS_29_97.frame_start(30).unwrap());
    for (comp, field) in ingested.times() {
        assert_eq!(
            classify_time_path(&field.path, true),
            Some((field.kind, comp.path.clone())),
            "{:?}",
            field.path
        );
    }
}

#[test]
fn a_composition_without_a_rate_is_an_error() {
    let project =
        parse(r#"{"compId": "A", "comps": {"A": {"id": "A", "dur": 1, "layers": []}}}"#).unwrap();
    let error = ingest_project(&project).unwrap_err();
    assert_eq!(error.path, path!["comps", "A"].to_vec());
    assert_eq!(error.error, TimeError::InvalidFloatRate);
    let project =
        parse(r#"{"compId": "A", "comps": {"A": {"fps": 30, "layers": [{"from": 1e300}]}}}"#)
            .unwrap();
    let error = ingest_project(&project).unwrap_err();
    assert_eq!(
        error.path,
        path!["comps", "A", "layers", 0, "from"].to_vec()
    );
    assert_eq!(
        error.to_string(),
        "time value out of range at comps.A.layers[0].from"
    );
}

#[test]
fn classifier_rejects_non_time_paths() {
    let c = |p: &[PathPart], root: bool| classify_time_path(p, root).map(|(k, _)| k);
    assert_eq!(
        c(&path!["comps", "A", "dur"], false),
        Some(TimeKind::CompositionDuration)
    );
    assert_eq!(c(&path!["dur"], false), None);
    assert_eq!(c(&path!["dur"], true), Some(TimeKind::CompositionDuration));
    assert_eq!(c(&path!["comps", "A", "layers", 0], false), None);
    assert_eq!(c(&path!["comps", "A", "layers", 0, "name"], false), None);
    assert_eq!(
        c(
            &path!["comps", "A", "layers", 0, "p", "opacity", "kf", 2, "v"],
            false
        ),
        None
    );
    assert_eq!(
        c(
            &path!["comps", "A", "layers", "3", "p", "opacity", "kf", "2", "t"],
            false
        ),
        Some(TimeKind::Keyframe)
    );
    assert_eq!(c(&path!["comps", "A", "layers", "x", "from"], false), None);
    assert_eq!(c(&path!["comps", "A", "work", 2], false), None);
    assert_eq!(
        c(&path!["comps", "A", "work", 1], false),
        Some(TimeKind::WorkArea)
    );
    assert_eq!(
        c(
            &path!["comps", "A", "layers", 0, "d", "cues", 0, "words", 1, "end"],
            false
        ),
        Some(TimeKind::Caption)
    );
    assert_eq!(
        c(
            &path!["comps", "A", "layers", 0, "d", "cues", 0, "text"],
            false
        ),
        None
    );
    assert_eq!(
        c(
            &path!["comps", "A", "layers", 0, "transitionOut", "dur"],
            false
        ),
        Some(TimeKind::TransitionDuration)
    );
    assert_eq!(
        c(&path!["comps", "A", "layers", 0, "d", "trim"], false),
        Some(TimeKind::SourceOffset)
    );
    assert_eq!(
        c(&path!["comps", "A", "layers", 0, "d", "trim", "v"], false),
        None
    );
    assert_eq!(c(&path!["compId"], false), None);
    assert_eq!(
        c(&path!["comps", "A", "comps", "B", "markers", 0, "t"], false),
        Some(TimeKind::Marker)
    );
    assert_eq!(
        classify_time_path(&path!["comps", "A", "comps", "B", "dur"], false)
            .unwrap()
            .1,
        path!["comps", "A", "comps", "B"].to_vec()
    );
}
