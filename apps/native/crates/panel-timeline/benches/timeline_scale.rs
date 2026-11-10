//! The timeline's rebuild after an edit on synthetic large projects, as the
//! app's timeline worker runs it (`timeline_surface::build`): sync the model
//! to the new publication, build the scene, hand a copy of the model to the
//! surface. Prints the median and worst per edit kind.
//!
//! cargo bench -p pm-panel-timeline --bench timeline_scale [-- 5000 20000]
//!
//! Target: under 8 ms a change.
use std::time::{Duration, Instant};

use pm_kernel::commands::properties::SetProperty;
use pm_kernel::commands::{KeyframeCommand, PropertyCommand};
use pm_kernel::*;
use pm_panel_timeline::render::{Interpolation, Label, Media, ModelPresenter, Scene};
use pm_panel_timeline::{Layer, RowId, TimelineClient};

const BUDGET: Duration = Duration::from_millis(8);
const RUNS: usize = 25;

/// The app's presenter rules (`timeline_surface::Presenter`), without media.
struct Presenter;
impl ModelPresenter for Presenter {
    fn accessibility_label(&self, row: &RowId) -> String {
        match row {
            RowId::Layer(id) => id.clone(),
            RowId::Property { channel, .. } => channel.clone(),
        }
    }
    fn layer_label(&self, layer: &Layer) -> Label {
        Label::for_kind(&layer.kind)
    }
    fn media(&self, _: &Layer) -> Media {
        Media::Ready
    }
    fn interpolation(&self, key: &pm_panel_timeline::Key) -> Interpolation {
        match key.stored.get("e").and_then(pm_model::Value::as_str) {
            Some("hold") => Interpolation::Hold,
            Some("linear") | None => Interpolation::Linear,
            _ => Interpolation::Ease,
        }
    }
    fn track_accessibility_label(
        &self,
        _: pm_panel_timeline::tracks::Area,
        lane: usize,
    ) -> Option<String> {
        Some(format!("Track {lane}"))
    }
}

fn edit(kind: &str, layer: &str, n: usize) -> Command {
    match kind {
        "move" => PropertyCommand::Set(SetProperty::new(
            layer,
            "position.x",
            pm_model::Value::Number(100.0 + n as f64),
            1.0,
        ))
        .into(),
        "keyframe" => KeyframeCommand::Add {
            layer_id: layer.into(),
            channel: "rotation".into(),
            time: 0.5 + n as f64 * 0.1,
            key_id: format!("bench{n}"),
            value: Some(pm_model::Value::Number(n as f64)),
        }
        .into(),
        "rename" => Command::SetLayerName {
            layer_id: layer.into(),
            name: format!("Renamed {n}"),
        },
        "toggle" => Command::SetLayerVisible {
            layer_id: layer.into(),
            visible: n % 2 == 1,
        },
        _ => unreachable!(),
    }
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn run(layers: usize) -> bool {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    kernel
        .open(
            "scale",
            OpenSource::Untitled(pm_kernel::synthetic::project(layers)),
        )
        .wait()
        .unwrap()
        .unwrap();
    let tokens =
        pm_design::Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default());
    let started = Instant::now();
    let mut client = TimelineClient::new(kernel.clone(), "scale", Caller::native()).unwrap();
    let first = Scene::from_model(&client.model, &Presenter, &tokens, 0.).unwrap();
    println!(
        "{layers} layers: first build {:.0} ms ({} rows)",
        ms(started.elapsed()),
        first.total()
    );
    let ids = pm_kernel::synthetic::main_layer_ids(layers);
    let target = ids[ids.len() / 2].clone();
    let mut ok = true;
    for kind in ["move", "keyframe", "rename", "toggle"] {
        let mut times = vec![];
        for n in 0..RUNS {
            kernel
                .call_blocking(
                    "scale",
                    Request::new(
                        Caller::native(),
                        Call::Transact {
                            label: Some(kind.into()),
                            body: WriteBody::Commands(vec![edit(kind, &target, n)]),
                            expected_revision: None,
                        },
                    ),
                )
                .unwrap();
            let t = Instant::now();
            client.sync().unwrap();
            client.adopt_view().unwrap();
            let scene = Scene::from_model(&client.model, &Presenter, &tokens, 0.).unwrap();
            let model = std::sync::Arc::new(client.model.clone());
            times.push(t.elapsed());
            std::hint::black_box((scene, model));
        }
        let worst = *times.iter().max().unwrap();
        let med = median(times);
        ok &= med < BUDGET;
        println!(
            "  {kind:<8} rebuild median {:>7.2} ms  worst {:>7.2} ms{}",
            ms(med),
            ms(worst),
            if med < BUDGET { "" } else { "  OVER 8 MS" }
        );
    }
    actor.shutdown();
    ok
}

fn main() {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let sizes = if sizes.is_empty() {
        vec![5000, 20_000]
    } else {
        sizes
    };
    // `wait` refuses the process's main thread (the app's rule): bench on a worker.
    let ok = std::thread::spawn(move || sizes.into_iter().filter(|n| !run(*n)).count() == 0)
        .join()
        .unwrap();
    if !ok {
        std::process::exit(1);
    }
}
