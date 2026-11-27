//! Timeline work after publication, on a representative large composition.
use std::{
    future::Future,
    hint::black_box,
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Instant,
};

use pm_kernel::*;
use pm_panel_timeline::{render::*, v4, *};
use serde_json::json;

const LAYERS: usize = 2000;
const GROUPS: usize = 200;
const KEYS: usize = 1000;

fn project() -> pm_model::Value {
    let mut layers = Vec::with_capacity(LAYERS);
    for g in 0..GROUPS {
        layers.push(
            json!({"id":format!("g{g}"), "type":"group", "name":"Group", "dur":30,
            "collapsed":false}),
        );
    }
    for n in 0..LAYERS - GROUPS {
        let keys: Vec<_> = (n < KEYS)
            .then(|| json!({"i":format!("k{n}"), "t":1, "v":42}))
            .into_iter()
            .collect();
        layers.push(json!({"id":format!("l{n}"), "type":"shape", "name":"Layer",
            "group":format!("g{}", n % GROUPS), "from":n % 4, "dur":20,
            "p":{"opacity":{"v":100,"kf":keys,"expr":null},
                 "position.x":{"v":0,"kf":[],"expr":null}}}));
    }
    // A dense static shape on the edited layer, representative of imported artwork.
    layers[GROUPS]["d"]["paths"] = json!([{
        "id":"path",
        "vertices":(0..6000).map(|n| json!({"id":format!("v{n}"),"x":n,"y":n}))
            .collect::<Vec<_>>()
    }]);
    pm_model::json::parse(
        &json!({"id":"fixture", "w":1920,"h":1080,
        "fps":30,"dur":60,"assets":{},"comps":{},"layers":layers})
        .to_string(),
    )
    .unwrap()
}

struct Presenter;
impl ModelPresenter for Presenter {
    fn accessibility_label(&self, _: &RowId) -> String {
        "Layer".into()
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
        Some("Track".into())
    }
}

fn measure(name: &str, budget_ms: f64, mut sample: impl FnMut(usize) -> f64) {
    let mut samples: Vec<_> = (0..35).map(&mut sample).skip(5).collect();
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    let p95 = samples[samples.len() * 95 / 100];
    println!("{name:<24} median {median:6.3} ms, p95 {p95:6.3} ms");
    assert!(median < budget_ms, "{name} median exceeded {budget_ms} ms");
}

fn ready<F: Future>(future: F) -> F::Output {
    let Poll::Ready(output) = pin!(future).poll(&mut Context::from_waker(Waker::noop())) else {
        panic!("local timeline work unexpectedly waited for another thread");
    };
    output
}

fn timed(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.
}

#[test]
fn heavy_comp_timeline_work() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    kernel
        .open("heavy", OpenSource::Untitled(project()))
        .wait()
        .unwrap()
        .unwrap();
    let mut published = (*kernel.published("heavy").unwrap()).clone();
    let tokens = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());
    let mut model = TimelineModel::default();
    model.sync(&published);
    assert_eq!(model.rows().len(), LAYERS);
    assert_eq!(
        model
            .index
            .layers
            .iter()
            .flat_map(|l| &l.channels)
            .map(|c| c.keys.len())
            .sum::<usize>(),
        KEYS
    );

    let rebuilds = model.rebuilds;
    measure("selection click", 1.0, |n| {
        let id = if n % 2 == 0 { "l0" } else { "l1" };
        let start = Instant::now();
        black_box(model.select_layer(id, Modifiers::default()).unwrap());
        published.selection = Selection::from_parts(vec![id.into()], Vec::new(), None);
        model.sync(&published);
        timed(start)
    });
    assert_eq!(model.rebuilds, rebuilds);
    assert!(model.reveal.layers.is_empty());
    measure("shift and cmd selection", 1., |_| {
        let start = Instant::now();
        black_box(
            model
                .select_layer(
                    "l1799",
                    Modifiers {
                        shift: true,
                        meta: true,
                        ..Default::default()
                    },
                )
                .unwrap(),
        );
        timed(start)
    });

    let mut property_reveal = model.clone();
    measure("property shortcuts", 1., |n| {
        let start = Instant::now();
        property_reveal
            .reveal_property(
                ['p', 's', 'r', 't', 'a', 'u'][n % 6],
                false,
                n as u64 * 1000,
                &PropertyFacts::default(),
            )
            .unwrap();
        timed(start)
    });

    measure("group disclosure", 1.0, |n| {
        let start = Instant::now();
        model.toggle_disclosure("g0").unwrap();
        let elapsed = timed(start);
        assert_eq!(
            model.rows().len(),
            if n % 2 == 0 { LAYERS - 9 } else { LAYERS }
        );
        elapsed
    });
    model.toggle_disclosure("g0").unwrap();

    measure("scene build", 6.0, |_| {
        let start = Instant::now();
        let scene = Scene::from_model(&model, &Presenter, &tokens, 0.).unwrap();
        let elapsed = timed(start);
        black_box(scene);
        elapsed
    });
    let scene = Scene::from_model(&model, &Presenter, &tokens, 0.).unwrap();
    let mut scene_model = model.clone();
    let mut disclosure_scene = scene.clone();
    let layer = v4::row_id(&RowId::Layer("l1".into()));
    measure("disclosure scene", 2., |_| {
        scene_model.toggle_disclosure("g0").unwrap();
        let start = Instant::now();
        let next = Scene::from_model_reusing(
            &scene_model,
            &Presenter,
            &tokens,
            0.,
            Some(&disclosure_scene),
        )
        .unwrap();
        let elapsed = timed(start);
        assert!(std::ptr::eq(
            disclosure_scene.row(&layer).unwrap(),
            next.row(&layer).unwrap()
        ));
        disclosure_scene = next;
        elapsed
    });
    measure("viewport plan", 1.5, |n| {
        let start = Instant::now();
        let frame = scene
            .prepare(
                Viewport {
                    width: 1280.,
                    height: 760.,
                    scroll_y: n as f64 * 750.,
                    scroll_time: n as f64 / 2.,
                    pps: 75. + n as f64 * 3.,
                    layer_column: 300.,
                },
                &tokens,
            )
            .unwrap();
        let elapsed = timed(start);
        assert!(frame.range.len() < 50);
        black_box(frame.hit_clip(540., 150.));
        black_box(frame.hit_key_across(540., 150., 8., 22.));
        elapsed
    });

    measure("playhead outline", 1.5, |n| {
        let start = Instant::now();
        let view = v4::project_window(&model, 1000 + n, 20).unwrap();
        let elapsed = timed(start);
        assert_eq!(view["window"]["total"], LAYERS);
        assert_eq!(view["window"]["rows"].as_array().unwrap().len(), 60);
        elapsed
    });

    let mut client = TimelineClient::new(kernel.clone(), "heavy", Caller::native()).unwrap();
    let facts = PropertyFacts::default();
    let context = v4::Context {
        time: 0.,
        expected_document: client.model.index.doc_id,
        expected_revision: client.model.index.revision,
        millis: 0,
        in_field: false,
        keys_admitted: true,
        property_facts: &facts,
    };
    measure("v4 seek", 1., |n| {
        let intent = json!({"type":"seek","time": n});
        let start = Instant::now();
        let effects = ready(v4::dispatch(&mut client, &intent, &context)).unwrap();
        let elapsed = timed(start);
        assert!(matches!(effects.as_slice(), [HostEffect::Seek(time)] if *time == n as f64));
        elapsed
    });
    for (name, kind) in [
        (
            "local clip move",
            DragKind::Layers {
                ids: vec!["l0".into()],
            },
        ),
        (
            "local trim in",
            DragKind::TrimIn {
                ids: vec!["l1".into()],
            },
        ),
        (
            "local trim out",
            DragKind::TrimOut {
                ids: vec!["l1".into()],
            },
        ),
        (
            "local key drag",
            DragKind::Keys {
                ids: vec!["k0".into()],
            },
        ),
    ] {
        measure(name, 1.5, |n| {
            let start = Instant::now();
            client.begin_preview_drag(kind.clone()).unwrap();
            ready(client.drag(DragSample {
                time_delta: (n + 1) as f64 / 30.,
                ..Default::default()
            }))
            .unwrap();
            let elapsed = timed(start);
            ready(client.finish_drag(false)).unwrap();
            elapsed
        });
    }
    measure("clip move scene", 1., |n| {
        let mut preview = scene.clone();
        let starts = [("l0".into(), n as f64 / 30.)];
        let start = Instant::now();
        preview.preview_times(&model, &starts);
        let elapsed = timed(start);
        assert!(std::ptr::eq(
            preview.row(&layer).unwrap(),
            scene.row(&layer).unwrap()
        ));
        elapsed
    });

    let mut scrub = Scrub::begin(model.index.clone(), 0.).unwrap();
    let mut planning = model.clone();
    let source = vec!["l0".into()];
    let key_ids = vec!["k0".into()];
    measure("ruler scrub", 1., |n| {
        let start = Instant::now();
        black_box(scrub.update(n as f64 / 2., n % 2 == 0, 0.1).unwrap());
        timed(start)
    });
    measure("tool plans", 1.5, |_| {
        let start = Instant::now();
        let time = planning.split_time(4.9, 2., 0.1);
        black_box(planning.split_at("l0", time, false).unwrap());
        black_box(planning.reorder(&source, "l200", true, time).unwrap());
        black_box(planning.parent(&source, Some("l200".into()), time));
        black_box(planning.switch("l0", LayerSwitch::ThreeD).unwrap());
        black_box(planning.switch("l0", LayerSwitch::Locked).unwrap());
        black_box(planning.switch("l0", LayerSwitch::Shy).unwrap());
        black_box(planning.switch("l0", LayerSwitch::Solo).unwrap());
        let copied = planning.copy_keys(&key_ids);
        black_box(
            planning
                .paste_keys("l0", "opacity", time, &copied[0].keys)
                .unwrap(),
        );
        black_box(planning.marquee(source.clone(), key_ids.clone(), true));
        timed(start)
    });

    let mut tracks = model.clone();
    tracks.mode = TimelineMode::Tracks;
    let first = v4::project_window(&tracks, 1000, 20).unwrap();
    assert_eq!(first["window"]["total"], LAYERS);
    measure("scrolled Tracks outline", 1., |n| {
        let start = Instant::now();
        let view = v4::project_window(&tracks, 1000 + n, 20).unwrap();
        let elapsed = timed(start);
        assert_eq!(view["window"]["rows"].as_array().unwrap().len(), 60);
        elapsed
    });

    let previous_rows = model.clone();
    measure("property edit + sync", 1.0, |n| {
        use pm_kernel::commands::{PropertyCommand, properties::SetProperty};
        kernel
            .call_blocking(
                "heavy",
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: None,
                        expected_revision: None,
                        body: WriteBody::Commands(vec![
                            PropertyCommand::Set(SetProperty::new(
                                "l0",
                                "position.x",
                                pm_model::Value::Number((n + 1) as f64),
                                0.,
                            ))
                            .into(),
                        ]),
                    },
                ),
            )
            .unwrap();
        let updated = kernel.published("heavy").unwrap();
        let previous = model.index.clone();
        let start = Instant::now();
        model.sync(&updated);
        let elapsed = timed(start);
        assert!(!Arc::ptr_eq(&model.index, &previous));
        assert_eq!(
            model.index.channel("l0", "position.x").unwrap().value,
            pm_model::Value::Number((n + 1) as f64)
        );
        assert!(std::ptr::eq(
            model.index.layer("l1").unwrap(),
            previous.layer("l1").unwrap()
        ));
        elapsed
    });
    assert!(std::ptr::eq(model.rows(), previous_rows.rows()));
    actor.shutdown();
}
