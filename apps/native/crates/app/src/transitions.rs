//! Where a transition goes: on a clip's edge (v1), or across the cut
//! between two clips on one track (Premiere, Final Cut). Across a cut both
//! clips run on into each other by the media they have past it, half the
//! transition each when they can; an audio cut crossfades. The commands
//! come back for one undo step; nothing here writes.
use pm_kernel::commands::effects::{Edge, add_transition};
use pm_kernel::{Command, ContentCommand, Published, TimingCommand, state::StateSnapshot};
use pm_model::Value;

/// The least transition the kernel takes.
const LEAST: f64 = 0.02;

/// A placed transition: its commands, its length, and what to tell the
/// person when the media fell short of the length asked for.
pub(crate) struct Placed {
    pub commands: Vec<Command>,
    pub notice: Option<String>,
}

/// One clip at the cut, as far as its media goes.
struct Side {
    id: String,
    from: f64,
    dur: f64,
    audio: bool,
    /// Its place in the stack (0 the top).
    stack: usize,
    /// Seconds of timeline the media runs on past its out point and
    /// before its in point (unlimited for a still, text or shape).
    tail: f64,
    head: f64,
}

fn number(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn side(published: &Published, id: &str) -> Option<Side> {
    let root = published.snapshot.root()?;
    let loc = pm_kernel::model::find_layer(root, id)?;
    let layer = pm_kernel::model::layer_at(root, &loc)?.to_value();
    let kind = layer.get("type").and_then(Value::as_str).unwrap_or("");
    let from = number(&layer, "from")?;
    let dur = number(&layer, "dur")?;
    let d = layer.get("d").cloned().unwrap_or(Value::Null);
    let timed = matches!(kind, "video" | "audio");
    let (tail, head) = if !timed {
        (f64::INFINITY, f64::INFINITY)
    } else if number(&d, "timeRemap").is_some_and(|r| r != 0.) {
        // A remapped clip's frames come from its keys: no handles.
        (0., 0.)
    } else {
        let rate = number(&d, "speed").filter(|r| *r > 0.).unwrap_or(1.);
        let trim = number(&d, "trim").unwrap_or(0.).max(0.);
        let length = d
            .get("asset")
            .and_then(Value::as_str)
            .and_then(|a| root.get("assets")?.get(a)?.get("dur"))
            .map(|n| n.to_value())
            .and_then(|v| v.as_f64())
            .filter(|n| n.is_finite());
        let tail = length.map_or(0., |l| ((l - trim - dur * rate) / rate).max(0.));
        (tail, trim / rate)
    };
    let stack = pm_panel_timeline::TimelineIndex::from_snapshot(&published.snapshot)
        .layers
        .iter()
        .position(|l| l.id == id)?;
    Some(Side {
        id: id.to_owned(),
        from,
        dur,
        audio: kind == "audio",
        stack,
        tail,
        head,
    })
}

/// How far each clip runs on past the cut for a transition of `want`
/// seconds: half each where the media allows, the rest from the other, in
/// whole frames and never past the other clip's far end.
pub(crate) fn handles(want: f64, tail: f64, head: f64, spans: (f64, f64), fps: f64) -> (f64, f64) {
    let frames = |s: f64| (s * fps + 1e-6).floor() / fps;
    let tail = tail.min(spans.1 - 1. / fps).max(0.);
    let head = head.min(spans.0 - 1. / fps).max(0.);
    let half = frames(want / 2.);
    let mut out = tail.min(half);
    let mut into = head.min(want - out);
    out = tail.min(want - into);
    into = head.min(want - out);
    (frames(out), frames(into))
}

/// The transition `id` of `want` seconds across the cut where `outgoing`
/// ends and `incoming` starts. The upper clip carries it (it reveals from
/// what is under it); audio fades both.
pub(crate) fn across_cut(
    state: &StateSnapshot,
    published: &Published,
    outgoing: &str,
    incoming: &str,
    id: &str,
    want: f64,
    time: f64,
    wrap: impl Fn(TimingCommand) -> TimingCommand,
) -> Result<Placed, String> {
    let (Some(a), Some(b)) = (side(published, outgoing), side(published, incoming)) else {
        return Err("The clips at the cut are gone".into());
    };
    let fps = pm_panel_timeline::TimelineIndex::from_snapshot(&published.snapshot).fps;
    let fps = if fps > 0. { fps } else { 30. };
    let (out, into) = handles(want, a.tail, b.head, (a.dur, b.dur), fps);
    let total = out + into;
    if total < LEAST - 1e-9 {
        return Err(
            "Neither clip has media past the cut. Trim one back to make room for a transition."
                .into(),
        );
    }
    let mut commands = vec![];
    if out > 0. {
        commands.push(
            wrap(TimingCommand::TrimOut {
                layer_ids: vec![a.id.clone()],
                time: a.from + a.dur + out,
            })
            .into(),
        );
    }
    if into > 0. {
        commands.push(
            wrap(TimingCommand::TrimIn {
                layer_ids: vec![b.id.clone()],
                time: b.from - into,
            })
            .into(),
        );
    }
    if a.audio && b.audio {
        for (layer, field) in [(&a.id, "fadeOut"), (&b.id, "fadeIn")] {
            let mut patch = pm_model::Map::new();
            patch.insert(field, Value::Number(total));
            commands.push(
                ContentCommand::Set {
                    layer_id: layer.clone(),
                    patch,
                    time,
                }
                .into(),
            );
        }
    } else {
        let (upper, edge) = if b.stack < a.stack {
            (&b.id, Edge::In)
        } else {
            (&a.id, Edge::Out)
        };
        commands.push(
            add_transition(state, upper, edge, id, Some(total))
                .map_err(|e| e.to_string())?
                .into(),
        );
    }
    let seconds = |s: f64| format!("{:.2} s", s);
    let notice = (total < want - 1e-6).then(|| {
        let short = match (a.tail < want / 2., b.head < want / 2.) {
            (true, true) => "Both clips have little media past the cut",
            (true, false) => "The outgoing clip has little media past its end",
            _ => "The incoming clip has little media before its start",
        };
        format!("{short}: the transition is {}", seconds(total))
    });
    Ok(Placed { commands, notice })
}

/// The clip on `layer`'s track whose end meets its start (`Edge::In`) or
/// whose start meets its end (`Edge::Out`), as the Tracks timeline shows
/// them: the other side of a cut.
pub(crate) fn neighbour(
    index: &pm_panel_timeline::TimelineIndex,
    reveal: &pm_panel_timeline::RevealState,
    layer: &str,
    edge: Edge,
) -> Option<String> {
    let layout = pm_panel_timeline::tracks::layout(index, reveal);
    let at = layout.item(layer)?;
    let half = 0.5 / index.fps.max(1.);
    layout
        .items
        .iter()
        .filter(|i| i.id != layer && i.area == at.area && i.lane == at.lane)
        .filter(|i| index.layer(&i.id).is_some_and(|l| l.kind != "group"))
        .find(|i| match edge {
            Edge::In => (i.from + i.duration - at.from).abs() < half,
            Edge::Out => (i.from - (at.from + at.duration)).abs() < half,
        })
        .map(|i| i.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cut_takes_half_from_each_side_and_the_rest_from_whichever_has_it() {
        let fps = 30.;
        // Both have plenty: half a second each side of a one-second cut.
        assert_eq!(handles(1., 5., 5., (10., 10.), fps), (0.5, 0.5));
        // The outgoing clip ends at its media's end: all from the incoming.
        assert_eq!(handles(1., 0., 5., (10., 10.), fps), (0., 1.));
        // The incoming clip starts at its media's start: all from the outgoing.
        assert_eq!(handles(1., 5., 0., (10., 10.), fps), (1., 0.));
        // Short on both: what each has, in whole frames.
        let (a, b) = handles(1., 0.2, 0.1, (10., 10.), fps);
        assert!((a - 0.2).abs() < 1e-9 && (b - 0.1).abs() < 1e-9, "{a} {b}");
        // No media past the cut on either side.
        assert_eq!(handles(1., 0., 0., (10., 10.), fps), (0., 0.));
        // Never past the other clip's far end (it keeps a frame).
        let (a, b) = handles(4., 9., 9., (1., 1.), fps);
        assert!(
            a <= 1. - 1. / fps + 1e-9 && b <= 1. - 1. / fps + 1e-9,
            "{a} {b}"
        );
    }

    /// Across a cut both clips run on into each other by the media they
    /// have; the upper clip carries the transition; a cut with no media past
    /// it says so and changes nothing; a short one says how long it is.
    #[test]
    fn a_transition_across_a_cut_uses_both_clips_handles_and_says_when_there_are_none() {
        use pm_kernel::{
            Call, Caller, KernelActor, KernelConfig, OpenSource, Reply, Request, WriteBody,
        };
        let run = |a_media: f64, b_trim: f64, kind: &str| {
            let actor = KernelActor::spawn(KernelConfig::default());
            let kernel = actor.handle();
            let project = pm_model::json::parse(&format!(
                r##"{{"id":"p","name":"Cut","compId":"root","w":96,"h":54,"fps":30,"dur":20,
                "assets":{{"m1":{{"id":"m1","name":"one","type":"{kind}","dur":{a_media}}},"m2":{{"id":"m2","name":"two","type":"{kind}","dur":20}}}},
                "comps":{{}},"layers":[
                {{"id":"b","name":"B","type":"{kind}","from":5,"dur":5,"on":true,"p":{{}},"d":{{"asset":"m2","trim":{b_trim}}}}},
                {{"id":"a","name":"A","type":"{kind}","from":0,"dur":5,"on":true,"p":{{}},"d":{{"asset":"m1","trim":0}}}}]}}"##
            ))
            .unwrap();
            kernel
                .open("p", OpenSource::Untitled(project))
                .wait()
                .unwrap()
                .unwrap();
            let published = kernel.published("p").unwrap();
            let placed = across_cut(
                &kernel.state(),
                &published,
                "a",
                "b",
                "crossfade",
                0.5,
                0.,
                |c| c,
            );
            let placed = match placed {
                Ok(placed) => placed,
                Err(message) => return (Err(message), None),
            };
            let ack = kernel
                .call(
                    "p",
                    Request::new(
                        Caller::native(),
                        Call::Transact {
                            label: Some("Set Crossfade".into()),
                            body: WriteBody::Commands(placed.commands),
                            expected_revision: None,
                        },
                    ),
                )
                .wait()
                .unwrap()
                .unwrap();
            assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
            let p = kernel.published("p").unwrap();
            let layer = |id: &str| {
                let root = p.snapshot.root().unwrap();
                let loc = pm_kernel::model::find_layer(root, id).unwrap();
                pm_kernel::model::layer_at(root, &loc).unwrap().to_value()
            };
            let out = (layer("a"), layer("b"));
            drop(actor);
            (Ok(placed.notice), Some(out))
        };
        let n = |v: &Value, path: &[&str]| {
            path.iter()
                .fold(Some(v), |v, k| v?.get(*k))
                .and_then(Value::as_f64)
                .unwrap_or(f64::NAN)
        };
        let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
        // Media on both sides: half each way in whole frames (7 and 8 at
        // 30 fps), on B (the upper).
        let (notice, layers) = run(20., 2., "video");
        let (a, b) = layers.unwrap();
        assert_eq!(notice, Ok(None));
        assert!(near(n(&a, &["dur"]), 5. + 7. / 30.), "{a:?}");
        assert!(
            near(n(&b, &["from"]), 5. - 8. / 30.) && near(n(&b, &["d", "trim"]), 2. - 8. / 30.),
            "{b:?}"
        );
        assert!(near(n(&b, &["transitionIn", "dur"]), 0.5), "{b:?}");
        assert_eq!(
            b.get("transitionIn")
                .and_then(|t| t.get("type"))
                .and_then(Value::as_str),
            Some("crossfade")
        );
        // A ends at its media's end: all from B's head, and it says nothing
        // since the length is whole.
        let (notice, layers) = run(5., 2., "video");
        let (a, b) = layers.unwrap();
        assert_eq!(notice, Ok(None));
        assert!(
            near(n(&a, &["dur"]), 5.) && near(n(&b, &["from"]), 4.5),
            "{a:?} {b:?}"
        );
        // Little media: what there is, and the notice says how long.
        let (notice, layers) = run(5., 0.2, "video");
        let (_, b) = layers.unwrap();
        assert!(near(n(&b, &["transitionIn", "dur"]), 0.2), "{b:?}");
        assert!(notice.unwrap().unwrap().contains("0.20 s"));
        // None at all: refused, and it says so.
        let (refused, layers) = run(5., 0., "video");
        assert!(layers.is_none());
        assert!(
            refused
                .unwrap_err()
                .contains("Neither clip has media past the cut")
        );
        // Audio crossfades: both fade over the overlap.
        let (_, layers) = run(20., 2., "audio");
        let (a, b) = layers.unwrap();
        assert!(
            near(n(&a, &["d", "fadeOut"]), 0.5) && near(n(&b, &["d", "fadeIn"]), 0.5),
            "{a:?} {b:?}"
        );
        assert!(
            b.get("transitionIn")
                .is_none_or(|t| t.as_object().is_none())
        );
    }
}
