//! The keyboard edits editors expect: ripple trim, roll, slide, lift and
//! extract, and the inspector's time remap, freeze frame and reverse
//! (`extensions/inspector/retiming.ts`). What remains of an edited layer keeps
//! its media and keys where they were in composition time.

use pm_kernel::commands::timing::{RetimeMode, TimeEdge};
use pm_kernel::{CommandError, DocumentStore, Origin, TimingCommand, TransactionError};
use pm_model::json::{Value, parse};

const FPS: f64 = 30.0;
const FRAME: f64 = 1.0 / FPS;

/// A sequence a [0,2] | b [2,5] | c [5,7] on one track, an overlay over the
/// whole composition, and a locked layer after the cut.
const PROJECT: &str = r##"{
  "id": "P", "name": "Test", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 10, "bg": "#000000",
  "assets": {"v1": {"id": "v1", "kind": "video", "name": "clip.mov", "dur": 20}},
  "layers": [
    {"id": "title", "type": "shape", "name": "Title", "from": 0, "dur": 10, "on": true,
     "d": {"shape": "rect", "color": "#ffffff", "w": 20, "h": 20, "radius": 0},
     "p": {"opacity": {"v": 100, "kf": [{"t": 4, "v": 0, "i": "kt"}], "expr": null}}},
    {"id": "a", "type": "video", "name": "A", "from": 0, "dur": 2, "on": true,
     "d": {"asset": "v1", "fit": "cover", "trim": 0, "speed": 1, "w": 10, "h": 10}, "p": {}},
    {"id": "b", "type": "video", "name": "B", "from": 2, "dur": 3, "on": true,
     "d": {"asset": "v1", "fit": "cover", "trim": 5, "speed": 1, "w": 10, "h": 10},
     "p": {"opacity": {"v": 100, "kf": [{"t": 1, "v": 50, "i": "kb"}], "expr": null}}},
    {"id": "c", "type": "shape", "name": "C", "from": 5, "dur": 2, "on": true,
     "d": {"shape": "rect", "color": "#ffffff", "w": 20, "h": 20, "radius": 0}, "p": {}},
    {"id": "lock", "type": "shape", "name": "Locked", "lock": true, "from": 6, "dur": 2, "p": {}}
  ]
}"##;

fn store() -> DocumentStore {
    DocumentStore::new_untitled(parse(PROJECT).unwrap())
}
fn layer(s: &DocumentStore, id: &str) -> Value {
    let root = s.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, id).unwrap_or_else(|| panic!("no layer {id}"));
    pm_kernel::model::layer_at(root, &loc).unwrap().to_value()
}
fn num(v: &Value, path: &[&str]) -> f64 {
    let mut cur = v;
    for p in path {
        cur = match p.parse::<usize>() {
            Ok(i) => &cur.as_array().unwrap()[i],
            Err(_) => cur.get(p).unwrap_or_else(|| panic!("no {p} in {cur:?}")),
        };
    }
    cur.as_f64()
        .or_else(|| cur.get("v").and_then(Value::as_f64))
        .unwrap()
}
fn span(s: &DocumentStore, id: &str) -> (f64, f64) {
    let l = layer(s, id);
    (num(&l, &["from"]), num(&l, &["dur"]))
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}
fn run(s: &mut DocumentStore, c: TimingCommand) {
    s.execute(c.into(), Origin::Interface).unwrap();
}
fn refused(s: &mut DocumentStore, c: TimingCommand) -> String {
    let before = s.revision();
    match s.execute(c.into(), Origin::Interface) {
        Err(TransactionError::Command(CommandError::Invalid(m))) => {
            assert_eq!(s.revision(), before, "a refusal writes nothing");
            m
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn ripple_trim_in_takes_the_head_and_the_rest_closes_up() {
    let mut s = store();
    run(
        &mut s,
        TimingCommand::RippleTrim {
            layer_ids: vec!["b".into()],
            edge: TimeEdge::In,
            time: 3.0,
        },
    );
    let b = layer(&s, "b");
    assert_eq!(
        span(&s, "b"),
        (2.0, 2.0),
        "b keeps its start, a second shorter"
    );
    assert_eq!(
        num(&b, &["d", "trim"]),
        6.0,
        "and shows the source from 3 s on"
    );
    assert_eq!(
        num(&b, &["p", "opacity", "kf", "0", "t"]),
        0.0,
        "its key (at 3 s) is now at its start"
    );
    assert_eq!(span(&s, "c").0, 4.0, "c closes up");
    assert_eq!(span(&s, "lock").0, 6.0, "a locked layer stays");
    assert_eq!(span(&s, "title"), (0.0, 10.0), "an overlapping layer stays");
    assert_eq!(span(&s, "a"), (0.0, 2.0));
    assert_eq!(s.history().undo_label(), Some("Ripple Trim"));
    s.undo().unwrap();
    assert_eq!(span(&s, "b"), (2.0, 3.0));
    assert_eq!(span(&s, "c").0, 5.0);
}

#[test]
fn ripple_trim_out_takes_the_tail_and_refuses_on_the_edge() {
    let mut s = store();
    run(
        &mut s,
        TimingCommand::RippleTrim {
            layer_ids: vec!["b".into()],
            edge: TimeEdge::Out,
            time: 4.0,
        },
    );
    assert_eq!(span(&s, "b"), (2.0, 2.0));
    assert_eq!(num(&layer(&s, "b"), &["d", "trim"]), 5.0);
    assert_eq!(span(&s, "c").0, 4.0);
    // On the layer's own edge there is nothing to trim.
    let m = refused(
        &mut s,
        TimingCommand::RippleTrim {
            layer_ids: vec!["b".into()],
            edge: TimeEdge::Out,
            time: 2.0,
        },
    );
    assert!(m.contains("playhead"), "{m}");
}

#[test]
fn roll_moves_the_edit_between_neighbours_and_clamps() {
    let mut s = store();
    run(
        &mut s,
        TimingCommand::Roll {
            layer_id: "b".into(),
            edge: TimeEdge::Out,
            time: 4.5,
        },
    );
    assert_eq!(span(&s, "b"), (2.0, 2.5));
    assert_eq!(span(&s, "c"), (4.5, 2.5), "c takes the other side");
    run(
        &mut s,
        TimingCommand::Roll {
            layer_id: "b".into(),
            edge: TimeEdge::In,
            time: 1.0,
        },
    );
    let b = layer(&s, "b");
    assert_eq!(span(&s, "a"), (0.0, 1.0));
    assert_eq!(span(&s, "b"), (1.0, 3.5));
    assert_eq!(num(&b, &["d", "trim"]), 4.0, "b's media stays where it was");
    assert_eq!(
        num(&b, &["p", "opacity", "kf", "0", "t"]),
        2.0,
        "its key stays at 3 s"
    );
    // a keeps a frame.
    run(
        &mut s,
        TimingCommand::Roll {
            layer_id: "b".into(),
            edge: TimeEdge::In,
            time: -5.0,
        },
    );
    assert!(close(span(&s, "a").1, FRAME));
    assert!(close(span(&s, "b").0, FRAME));
    assert_eq!(s.history().undo_label(), Some("Roll Edit"));
}

#[test]
fn slide_moves_the_layer_between_its_neighbours() {
    let mut s = store();
    run(
        &mut s,
        TimingCommand::Slide {
            layer_id: "b".into(),
            delta: 0.5,
        },
    );
    assert_eq!(span(&s, "a"), (0.0, 2.5));
    assert_eq!(span(&s, "b"), (2.5, 3.0));
    assert_eq!(
        num(&layer(&s, "b"), &["d", "trim"]),
        5.0,
        "b's content moves with it"
    );
    assert_eq!(span(&s, "c"), (5.5, 1.5));
    // Clamped where c keeps a frame; then it cannot go further.
    run(
        &mut s,
        TimingCommand::Slide {
            layer_id: "b".into(),
            delta: 100.0,
        },
    );
    assert!(close(span(&s, "c").1, FRAME));
    let m = refused(
        &mut s,
        TimingCommand::Slide {
            layer_id: "b".into(),
            delta: 1.0,
        },
    );
    assert!(m.contains("cannot slide"), "{m}");
}

#[test]
fn lift_leaves_a_gap_and_extract_closes_it() {
    for close_gap in [false, true] {
        let mut s = store();
        let before = s.layers().len();
        run(
            &mut s,
            TimingCommand::RemoveRange {
                layer_ids: vec!["title".into(), "b".into(), "c".into()],
                start: 3.0,
                end: 4.0,
                close: close_gap,
            },
        );
        let ids: Vec<String> = s.layers().into_iter().map(|l| l.id).collect();
        assert_eq!(ids.len(), before + 2, "title and b each leave a tail");
        assert_eq!(span(&s, "title"), (0.0, 3.0));
        assert_eq!(span(&s, "b"), (2.0, 1.0));
        let tail = |id: &str| {
            let i = ids.iter().position(|x| x == id).unwrap();
            ids[i - 1].clone()
        };
        let (title_tail, b_tail) = (layer(&s, &tail("title")), layer(&s, &tail("b")));
        let shift = if close_gap { 1.0 } else { 0.0 };
        assert_eq!(num(&title_tail, &["from"]), 4.0 - shift);
        assert_eq!(num(&title_tail, &["dur"]), 6.0);
        assert_eq!(
            num(&title_tail, &["from"]) + num(&title_tail, &["p", "opacity", "kf", "0", "t"]),
            4.0 - shift,
            "the title's key (4 s) stays on its frame of the tail"
        );
        assert_eq!(num(&b_tail, &["from"]), 4.0 - shift);
        assert_eq!(
            num(&b_tail, &["d", "trim"]),
            7.0,
            "b's tail shows the source from 4 s"
        );
        assert_eq!(
            span(&s, "c").0,
            5.0 - shift,
            "a chosen later layer moves on Extract"
        );
        assert_eq!(span(&s, "a"), (0.0, 2.0), "an unchosen layer never moves");
        assert_eq!(span(&s, "lock").0, 6.0);
        let label = if close_gap {
            "Extract Work Area"
        } else {
            "Lift Work Area"
        };
        assert_eq!(s.history().undo_label(), Some(label));
        s.undo().unwrap();
        assert_eq!(s.layers().len(), before, "one undo step");
        assert_eq!(span(&s, "title"), (0.0, 10.0));
    }
    // A layer wholly inside the range goes.
    let mut s = store();
    run(
        &mut s,
        TimingCommand::RemoveRange {
            layer_ids: vec!["b".into()],
            start: 1.0,
            end: 6.0,
            close: true,
        },
    );
    assert!(s.layers().iter().all(|l| l.id != "b"));
    let m = refused(
        &mut s,
        TimingCommand::RemoveRange {
            layer_ids: vec!["a".into()],
            start: 8.0,
            end: 9.0,
            close: false,
        },
    );
    assert!(m.contains("work area"), "{m}");
}

#[test]
fn time_remap_freeze_and_reverse_write_the_source_time_channel() {
    let last = 5.0 + 3.0 - FRAME;
    let mut s = store();
    run(
        &mut s,
        TimingCommand::Retime {
            layer_ids: vec!["b".into()],
            mode: RetimeMode::Remap,
            time: 0.0,
        },
    );
    let b = layer(&s, "b");
    assert_eq!(
        b.get("d").unwrap().get("timeRemap").unwrap().get("v"),
        Some(&Value::Bool(true))
    );
    let keys = |b: &Value| {
        b.get("d")
            .and_then(|d| d.get("sourceTime"))
            .and_then(|t| t.get("kf"))
            .and_then(Value::as_array)
            .unwrap()
            .clone()
    };
    let k = keys(&b);
    assert_eq!((num(&k[0], &["t"]), num(&k[0], &["v"])), (0.0, 5.0));
    assert!(close(num(&k[1], &["t"]), 3.0 - FRAME) && close(num(&k[1], &["v"]), last));
    assert_eq!(s.history().undo_label(), Some("Enable Time Remapping"));
    // Reverse reads the remapped source time: the same ends, swapped.
    run(
        &mut s,
        TimingCommand::Retime {
            layer_ids: vec!["b".into()],
            mode: RetimeMode::Reverse,
            time: 0.0,
        },
    );
    let k = keys(&layer(&s, "b"));
    assert!(close(num(&k[0], &["v"]), last) && close(num(&k[1], &["v"]), 5.0));
    assert_eq!(s.history().undo_label(), Some("Time-Reverse Layer"));
    s.undo().unwrap();
    s.undo().unwrap();
    // Freeze holds the frame under the playhead, on a frame boundary.
    run(
        &mut s,
        TimingCommand::Retime {
            layer_ids: vec!["b".into()],
            mode: RetimeMode::Freeze,
            time: 3.01,
        },
    );
    let k = keys(&layer(&s, "b"));
    assert_eq!(k.len(), 1);
    assert_eq!(num(&k[0], &["t"]), 1.0);
    assert!(close(num(&k[0], &["v"]), 6.01));
    assert_eq!(k[0].get("hold"), Some(&Value::Bool(true)));
    let m = refused(
        &mut s,
        TimingCommand::Retime {
            layer_ids: vec!["c".into()],
            mode: RetimeMode::Freeze,
            time: 5.0,
        },
    );
    assert!(m.contains("video or pre-comp"), "{m}");
}

#[test]
fn a_ripple_trim_past_the_edge_lengthens_the_layer_and_pushes_what_follows() {
    let mut s = store();
    run(
        &mut s,
        TimingCommand::RippleTrim {
            layer_ids: vec!["b".into()],
            edge: TimeEdge::In,
            time: 1.5,
        },
    );
    let b = layer(&s, "b");
    assert_eq!(
        span(&s, "b"),
        (2.0, 3.5),
        "b keeps its start and shows more head"
    );
    assert_eq!(num(&b, &["d", "trim"]), 4.5);
    assert_eq!(
        num(&b, &["p", "opacity", "kf", "0", "t"]),
        1.5,
        "its key stays on the source frame it marked"
    );
    assert_eq!(span(&s, "c").0, 5.5, "c moves on");
    assert_eq!(span(&s, "a"), (0.0, 2.0));
    assert_eq!(span(&s, "lock").0, 6.0);
    s.undo().unwrap();
    run(
        &mut s,
        TimingCommand::RippleTrim {
            layer_ids: vec!["b".into()],
            edge: TimeEdge::Out,
            time: 6.0,
        },
    );
    assert_eq!(span(&s, "b"), (2.0, 4.0));
    assert_eq!(span(&s, "c").0, 6.0);
}

/// The Tracks timeline: A [0,2] and B [2,4] on one lane, C [3,5] on the lane
/// above, none with a stored track (as v1 files are). An edit made there
/// keeps every clip on the lane it shows on (`TimingCommand::InTracks`).
const LANES: &str = r##"{
  "id": "P", "name": "Lanes", "compId": "C1", "compName": "Main",
  "w": 200, "h": 100, "fps": 30, "dur": 10, "bg": "#000000", "assets": {},
  "layers": [
    {"id": "c", "type": "shape", "name": "C", "from": 3, "dur": 2, "p": {}},
    {"id": "b", "type": "shape", "name": "B", "from": 2, "dur": 2, "p": {}},
    {"id": "a", "type": "shape", "name": "A", "from": 0, "dur": 2, "p": {}}
  ]
}"##;

fn lanes() -> Vec<(String, u32)> {
    vec![("a".into(), 0), ("b".into(), 0), ("c".into(), 1)]
}

fn field(s: &DocumentStore, id: &str, key: &str) -> Option<f64> {
    layer(s, id).get(key).and_then(Value::as_f64)
}

#[test]
fn a_ripple_trim_in_tracks_closes_time_on_its_own_track_only() {
    let mut s = DocumentStore::new_untitled(parse(LANES).unwrap());
    // In the Layers timeline (After Effects) every later layer moves back.
    let mut layers = DocumentStore::new_untitled(parse(LANES).unwrap());
    run(
        &mut layers,
        TimingCommand::RippleTrim {
            layer_ids: vec!["a".into()],
            edge: TimeEdge::Out,
            time: 1.0,
        },
    );
    assert_eq!(field(&layers, "c", "from"), Some(2.0));
    run(
        &mut s,
        TimingCommand::InTracks {
            lanes: lanes(),
            command: Box::new(TimingCommand::RippleTrim {
                layer_ids: vec!["a".into()],
                edge: TimeEdge::Out,
                time: 1.0,
            }),
        },
    );
    assert_eq!(field(&s, "a", "dur"), Some(1.0));
    assert_eq!(
        field(&s, "b", "from"),
        Some(1.0),
        "B follows A on its track"
    );
    assert_eq!(
        field(&s, "c", "from"),
        Some(3.0),
        "C, on another track, stays"
    );
    assert_eq!(
        field(&s, "c", "track"),
        Some(1.0),
        "lanes are written as tracks"
    );
    assert_eq!(
        field(&s, "b", "track"),
        None,
        "a lane equal to the stored track is not written"
    );
    s.undo().unwrap();
    assert_eq!(field(&s, "c", "track"), None);
    assert_eq!(field(&s, "b", "from"), Some(2.0));
}

#[test]
fn a_slide_in_tracks_takes_its_neighbours_on_its_own_track() {
    let mut s = DocumentStore::new_untitled(parse(LANES).unwrap());
    run(
        &mut s,
        TimingCommand::InTracks {
            lanes: lanes(),
            command: Box::new(TimingCommand::Slide {
                layer_id: "b".into(),
                delta: -0.5,
            }),
        },
    );
    assert_eq!(field(&s, "b", "from"), Some(1.5));
    assert_eq!(
        field(&s, "a", "dur"),
        Some(1.5),
        "A, before B on its track, gives way"
    );
    assert_eq!(field(&s, "c", "from"), Some(3.0));
    assert_eq!(field(&s, "c", "dur"), Some(2.0));
}

/// V1 and A1 are different tracks though both are lane 0 of their area: a
/// ripple on a video clip leaves a later audio clip where it is.
#[test]
fn a_ripple_in_tracks_keeps_to_its_area() {
    let project = LANES.replace(
        r#"{"id": "c", "type": "shape", "name": "C", "from": 3, "dur": 2, "p": {}}"#,
        r#"{"id": "c", "type": "audio", "name": "C", "from": 3, "dur": 2, "d": {"trim": 0, "speed": 1}, "p": {}}"#,
    );
    let mut s = DocumentStore::new_untitled(parse(&project).unwrap());
    run(
        &mut s,
        TimingCommand::InTracks {
            lanes: vec![("a".into(), 0), ("b".into(), 0), ("c".into(), 0)],
            command: Box::new(TimingCommand::RippleTrim {
                layer_ids: vec!["a".into()],
                edge: TimeEdge::Out,
                time: 1.0,
            }),
        },
    );
    assert_eq!(field(&s, "b", "from"), Some(1.0));
    assert_eq!(field(&s, "c", "from"), Some(3.0), "A1 is not V1");
}

/// Two clips on one track rippled out together: the later one moves on by
/// the earlier one's growth, so they never overlap.
#[test]
fn two_clips_rippled_out_together_stay_in_order() {
    let mut s = DocumentStore::new_untitled(parse(LANES).unwrap());
    run(
        &mut s,
        TimingCommand::InTracks {
            lanes: lanes(),
            command: Box::new(TimingCommand::RippleTrim {
                layer_ids: vec!["a".into(), "b".into()],
                edge: TimeEdge::Out,
                time: 5.0,
            }),
        },
    );
    assert_eq!(
        (field(&s, "a", "from"), field(&s, "a", "dur")),
        (Some(0.0), Some(5.0))
    );
    assert_eq!(field(&s, "b", "from"), Some(5.0));
}

/// Speed/Duration: at 2× a clip runs half as long from the same source In;
/// with ripple the next clip on its track moves up to it (no gap), and a
/// clip on another track stays. Undo restores all.
#[test]
fn a_speed_change_scales_the_clip_and_ripples_its_track() {
    let mut s = store();
    run(
        &mut s,
        TimingCommand::SetSpeed {
            layer_id: "b".into(),
            speed: 2.0,
            ripple: true,
        },
    );
    let b = layer(&s, "b");
    assert_eq!(b.get("dur").and_then(Value::as_f64), Some(1.5));
    assert_eq!(
        b.get("d")
            .and_then(|d| d.get("speed"))
            .and_then(Value::as_f64),
        Some(2.0)
    );
    assert_eq!(
        field(&s, "c", "from"),
        Some(3.5),
        "C follows B on its track"
    );
    assert_eq!(field(&s, "title", "from"), Some(0.0));
    s.undo().unwrap();
    assert_eq!(field(&s, "c", "from"), Some(5.0));
    assert_eq!(field(&s, "b", "dur"), Some(3.0));
    // Without ripple, the gap stays (v1's Speed field).
    run(
        &mut s,
        TimingCommand::SetSpeed {
            layer_id: "b".into(),
            speed: 2.0,
            ripple: false,
        },
    );
    assert_eq!(field(&s, "c", "from"), Some(5.0));
    assert!(
        refused(
            &mut s,
            TimingCommand::SetSpeed {
                layer_id: "title".into(),
                speed: 2.0,
                ripple: false,
            },
        )
        .contains("video or audio")
    );
}

/// A trim never runs a clip past its media's end at its speed: B starts 5 s
/// into a 20 s source; at 2× it can run 7.5 s.
#[test]
fn a_trim_stops_at_the_end_of_the_media_at_the_clips_speed() {
    let mut s = store();
    run(
        &mut s,
        TimingCommand::SetSpeed {
            layer_id: "b".into(),
            speed: 2.0,
            ripple: false,
        },
    );
    run(
        &mut s,
        TimingCommand::TrimOut {
            layer_ids: vec!["b".into()],
            time: 12.0,
        },
    );
    assert_eq!(field(&s, "b", "dur"), Some(7.5));
}
