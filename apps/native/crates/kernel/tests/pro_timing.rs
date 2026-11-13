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
fn ripple_trim_out_takes_the_tail_and_refuses_outside_the_layer() {
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
    let m = refused(
        &mut s,
        TimingCommand::RippleTrim {
            layer_ids: vec!["b".into()],
            edge: TimeEdge::Out,
            time: 8.0,
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
