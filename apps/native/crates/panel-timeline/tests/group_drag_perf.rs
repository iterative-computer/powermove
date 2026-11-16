//! Headless reproduction of an eleven-group bar drag. No app/GPUI harness.
use pm_kernel::*;
use pm_model::Value;
use pm_panel_timeline::{render::*, *};
use std::{
    future::Future,
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::Instant,
};

fn wait<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let w = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&w);
    let mut f = pin!(future);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}
fn project() -> Value {
    let mut layers = vec![];
    // 11 groups + 424 shape/text members, 831 layer-local keys.
    for g in 0..11 {
        layers.push(serde_json::json!({"id":format!("g{g}"),"name":format!("Group {g}"),"type":"group","from":0.125,"dur":20,"groupCollapsed":true,"p":{}}));
    }
    for n in 0..424 {
        let keys: Vec<_> = (0..if n < 407 { 2 } else { 1 })
            .map(|k| serde_json::json!({"i":format!("k{n}_{k}"),"t":k as f64 + 0.125,"v":k*100}))
            .collect();
        layers.push(serde_json::json!({"id":format!("l{n}"),"name":format!("Layer {n}"),"type":if n%3 == 0 {"text"} else {"shape"},"group":format!("g{}",n%11),"from":(n%17) as f64/30.0+0.125,"dur":10,"on":true,"p":{"opacity":{"v":100,"kf":keys,"expr":null}},"d":{"shape":"rect","w":80,"h":40,"color":"#ffffff"}}));
    }
    pm_model::json::parse(&serde_json::json!({"id":"drag","compId":"C","name":"Group drag","w":1920,"h":1080,"fps":30,"dur":60,"work":[0,60],"assets":{},"comps":{},"layers":layers}).to_string()).unwrap()
}
struct Presenter;
impl ModelPresenter for Presenter {
    fn accessibility_label(&self, row: &RowId) -> String {
        format!("{row:?}")
    }
    fn layer_label(&self, layer: &Layer) -> Label {
        Label::for_kind(&layer.kind)
    }
    fn media(&self, _: &Layer) -> Media {
        Media::Ready
    }
    fn interpolation(&self, _: &pm_panel_timeline::Key) -> Interpolation {
        Interpolation::Linear
    }
    fn track_accessibility_label(&self, _: tracks::Area, _: usize) -> Option<String> {
        None
    }
}
fn stats(name: &str, values: &mut [f64]) {
    values.sort_by(f64::total_cmp);
    println!(
        "{name}: p50={:.3} p95={:.3} max={:.3} ms",
        values[values.len() / 2],
        values[(values.len() * 95 / 100).min(values.len() - 1)],
        values[values.len() - 1]
    );
}
fn root(c: &TimelineClient) -> Value {
    c.kernel
        .published(&c.project)
        .unwrap()
        .snapshot
        .root()
        .unwrap()
        .to_value()
}
fn run(legacy: bool, snapping: bool) -> Value {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    wait(kernel.open("drag", OpenSource::Untitled(project())))
        .unwrap()
        .unwrap();
    let mut c = TimelineClient::new(kernel, "drag", Caller::native()).unwrap();
    assert_eq!(c.model.index.layers.len(), 435);
    assert_eq!(
        c.model
            .index
            .layers
            .iter()
            .flat_map(|l| &l.channels)
            .map(|c| c.keys.len())
            .sum::<usize>(),
        831
    );
    wait(
        c.call(
            SelectionCall::Layers {
                ids: (0..11).map(|g| format!("g{g}")).collect(),
                add: false,
            }
            .into(),
        ),
    )
    .unwrap();
    let before = root(&c);
    let kind = DragKind::Layers {
        ids: (0..11).map(|g| format!("g{g}")).collect(),
    };
    if legacy {
        wait(c.begin_drag(kind.clone())).unwrap();
    } else {
        c.begin_preview_drag(kind.clone()).unwrap();
    }
    let revision = c.model.index.revision;
    let mut oracle = Drag::begin(&c.model, kind.clone()).unwrap();
    // Baseline uses the same gesture and planner, with an inert time field
    // to retain the original generic SetLayer implementation.
    let gesture = if legacy {
        wait(c.finish_drag(false)).unwrap();
        match wait(
            c.call(
                GestureCall::Begin {
                    label: "Move clip".into(),
                }
                .into(),
            ),
        )
        .unwrap()
        .reply
        {
            Reply::Gesture(id) => id,
            _ => panic!(),
        }
    } else {
        0
    };
    let tokens =
        pm_design::Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default());
    let mut total = vec![];
    let mut kernel_times = vec![];
    let mut scene_times = vec![];
    let mut paint_times = vec![];
    let mut actor_times = vec![];
    let mut publication_times = vec![];
    let mut scene = Scene::from_model(&c.model, &Presenter, &tokens, 0.).unwrap();
    for n in 0..60 {
        let sample = DragSample {
            time_delta: (n + 1) as f64 / 25.0,
            playhead: 8.,
            modifiers: Modifiers {
                shift: snapping,
                ..Default::default()
            },
            acquire_tolerance: 0.08,
            release_tolerance: 0.08,
            ..Default::default()
        };
        let started = Instant::now();
        if legacy {
            let mut commands = oracle.commands(sample).unwrap();
            for command in &mut commands {
                if let Command::Layer(LayerCommand::Set { patch, .. }) = command {
                    patch.time = 1.;
                }
            }
            wait(
                c.call(
                    GestureCall::Step {
                        gesture,
                        body: WriteBody::Commands(commands),
                        replace: true,
                    }
                    .into(),
                ),
            )
            .unwrap();
        } else {
            wait(c.drag(sample)).unwrap();
        }
        kernel_times.push(started.elapsed().as_secs_f64() * 1000.);
        let published = c.kernel.published("drag").unwrap();
        actor_times.push(if legacy {published.actor_work.as_secs_f64() * 1000.} else {0.});
        publication_times.push(if legacy {published.publication_work.as_secs_f64() * 1000.} else {0.});
        drop(published);
        let started_scene = Instant::now();
        if legacy {
            scene = Scene::from_model(&c.model, &Presenter, &tokens, 0.).unwrap();
        } else {
            scene.preview_times(&c.model, &c.layer_preview().unwrap().starts);
            assert_eq!(c.kernel.published("drag").unwrap().revision(), revision);
        }
        scene_times.push(started_scene.elapsed().as_secs_f64() * 1000.);
        let started_paint = Instant::now();
        let frame = scene
            .prepare(
                Viewport {
                    width: 1400.,
                    height: 800.,
                    scroll_y: 0.,
                    scroll_time: 0.,
                    pps: 100.,
                    layer_column: 300.,
                },
                &tokens,
            )
            .unwrap();
        std::hint::black_box(frame);
        paint_times.push(started_paint.elapsed().as_secs_f64() * 1000.);
        total.push(started.elapsed().as_secs_f64() * 1000.);
    }
    let name = if legacy { "before" } else { "after" };
    println!("{name}, shift={snapping}, 60 events, 435 layers / 831 keys");
    stats("event end-to-end", &mut total);
    stats("gesture / preview worker", &mut kernel_times);
    stats("kernel actor", &mut actor_times);
    stats("publication", &mut publication_times);
    stats("scene build (worker)", &mut scene_times);
    stats("frame prepare (main-thread work)", &mut paint_times);
    let preview = if legacy {
        None
    } else {
        assert_eq!(root(&c), before);
        Some(
            c.layer_preview()
                .unwrap()
                .snapshot
                .root()
                .unwrap()
                .to_value(),
        )
    };
    if legacy {
        wait(c.call(GestureCall::Commit { gesture }.into())).unwrap();
    } else {
        wait(c.finish_drag(true)).unwrap();
    }
    assert_eq!(c.kernel.published("drag").unwrap().history.entries, 1);
    let after = root(&c);
    if let Some(preview) = preview {
        assert_eq!(preview, after);
    }
    wait(c.call(HistoryCall::Undo.into())).unwrap();
    assert_eq!(root(&c), before);
    wait(c.call(HistoryCall::Redo.into())).unwrap();
    assert_eq!(root(&c), after);
    c.begin_preview_drag(kind).unwrap();
    for n in 0..3 {
        wait(c.drag(DragSample {
            time_delta: n as f64 + 2.,
            ..Default::default()
        }))
        .unwrap();
    }
    wait(c.finish_drag(false)).unwrap();
    assert_eq!(root(&c), after);
    actor.shutdown();
    after
}
#[test]
fn group_drag_matches_original_commit_undo_and_escape() {
    for snapping in [false, true] {
        assert_eq!(run(true, snapping), run(false, snapping));
    }
}

#[test]
fn scalar_start_planner_preserves_validation_and_leaf_operations() {
    use pm_kernel::commands::layers::LayerPatch;
    let doc = DocumentStore::new_untitled(project());
    let snapshot = doc.snapshot();
    for id in ["l0", "g0", "missing"] {
        for from in [0., -0., -2.125, 13.5, f64::NAN, f64::INFINITY] {
            let make = |time| {
                Command::from(LayerCommand::Set {
                    layer_id: id.into(),
                    patch: LayerPatch {
                        from: Some(from),
                        time,
                        ..Default::default()
                    },
                })
            };
            let fast = make(0.).plan(snapshot.root().unwrap(), Origin::Interface);
            let legacy = make(1.).plan(snapshot.root().unwrap(), Origin::Interface);
            match (fast, legacy) {
                (Ok(a), Ok(b)) => assert_eq!(a, b),
                (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string()),
                _ => panic!("planner mismatch for {id} {from}"),
            }
        }
    }
}

#[test]
fn batch_start_planner_matches_original_locks_cycles_and_duplicate_targets() {
    use pm_kernel::commands::layers::LayerPatch;
    for locked in [false, true] {
        for cycle in [false, true] {
            let value=pm_model::json::parse(&serde_json::json!({"id":"batch","compId":"C","fps":30,"dur":20,"assets":{},"comps":{},"layers":[
                {"id":"g0","type":"group","from":0,"dur":20,"lock":locked,"group":if cycle {"g1"}else{""},"p":{}},
                {"id":"g1","type":"group","from":0,"dur":20,"group":"g0","p":{}},
                {"id":"l","type":"shape","from":0,"dur":20,"group":"g1","p":{}}
            ]}).to_string()).unwrap();
            for ids in [vec!["g0", "g1", "l"], vec!["l", "l"], vec!["missing", "l"]] {
                let mut fast = DocumentStore::new_untitled(value.clone());
                let mut old = DocumentStore::new_untitled(value.clone());
                let commands = |time| {
                    ids.iter()
                        .enumerate()
                        .map(|(n, id)| {
                            LayerCommand::Set {
                                layer_id: (*id).into(),
                                patch: LayerPatch {
                                    from: Some(n as f64 + 1.),
                                    time,
                                    ..Default::default()
                                },
                            }
                            .into()
                        })
                        .collect::<Vec<Command>>()
                };
                let a = fast.transact("Move", Origin::Interface, &commands(0.));
                let b = old.transact("Move", Origin::Interface, &commands(1.));
                match (a, b) {
                    (Ok(_), Ok(_)) => assert_eq!(
                        fast.snapshot().root().unwrap().to_value(),
                        old.snapshot().root().unwrap().to_value()
                    ),
                    (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string()),
                    _ => panic!("batch behavior changed"),
                }
            }
        }
    }
}

#[test]
fn preview_shares_unmoved_rows_and_cancel_keeps_foreign_edits() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    wait(kernel.open("drag", OpenSource::Untitled(project())))
        .unwrap()
        .unwrap();
    let mut c = TimelineClient::new(kernel, "drag", Caller::native()).unwrap();
    let tokens =
        pm_design::Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default());
    let scene = Scene::from_model(&c.model, &Presenter, &tokens, 0.).unwrap();
    let mut moved = scene.clone();
    c.begin_preview_drag(DragKind::Layers {
        ids: vec!["g0".into()],
    })
    .unwrap();
    wait(c.drag(DragSample {
        time_delta: 2.,
        ..Default::default()
    }))
    .unwrap();
    moved.preview_times(&c.model, &c.layer_preview().unwrap().starts);
    let row = |id: &str| pm_panel_timeline::v4::row_id(&RowId::Layer(id.into()));
    assert!(std::ptr::eq(
        scene.row(&row("g1")).unwrap(),
        moved.row(&row("g1")).unwrap()
    ));
    assert!(!std::ptr::eq(
        scene.row(&row("g0")).unwrap(),
        moved.row(&row("g0")).unwrap()
    ));
    wait(c.call(Call::Transact {
        label: None,
        body: WriteBody::Commands(vec![Command::SetLayerName {
            layer_id: "l1".into(),
            name: "Foreign edit".into(),
        }]),
        expected_revision: None,
    }))
    .unwrap();
    let foreign = root(&c);
    wait(c.finish_drag(false)).unwrap();
    assert_eq!(root(&c), foreign);
    assert_eq!(c.kernel.published("drag").unwrap().history.entries, 1);
    actor.shutdown();
}
