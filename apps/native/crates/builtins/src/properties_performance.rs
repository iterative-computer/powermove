use super::*;
use pm_kernel::{KernelActor, KernelConfig, OpenSource, SelectionCall};
use std::time::Instant;

const PROJECT: &str = "heavy-inspector";

fn project() -> pm_model::Value {
    let mut layers = (0..1998)
        .map(|i| {
            let group = i < 200;
            json!({"id":format!("layer{i}"),"type":if group {"group"} else {"text"},
                "name":format!("Layer {i}"),"from":0,"dur":10,"on":true})
        })
        .collect::<Vec<_>>();
    layers[1997]["p"] = json!({"opacity":{"v":100,"kf":[
        {"id":"start","t":0,"v":100},{"id":"end","t":1,"v":0}
    ]}});
    let paths = (0..32)
        .map(|i| json!({"id":format!("path{i}"),"pts":vec![[123.45, 678.90]; 550]}))
        .collect::<Vec<_>>();
    assert!(json!(paths).to_string().len() >= 250_000);
    for (id, data) in [
        (
            "normal",
            json!({"shape":"rect","w":100,"h":100,"color":"#FFFFFF"}),
        ),
        ("heavy", json!({"paths":paths})),
    ] {
        layers.push(json!({"id":id,"type":"shape","name":id,"group":"layer0",
        "from":0,"dur":10,"on":true,"d":data,"p":{
            "position.x":{"v":100,"kf":[]},"position.y":{"v":100,"kf":[]},
            "scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},
            "anchor.x":{"v":0,"kf":[]},"anchor.y":{"v":0,"kf":[]},
            "rotation":{"v":0,"kf":[]},"opacity":{"v":100,"kf":[]}
        }}));
    }
    pm_model::json::parse(
        &json!({"id":PROJECT,"name":"Heavy Inspector","w":1920,"h":1080,
        "fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":layers})
        .to_string(),
    )
    .unwrap()
}

#[test]
fn heavy_comp_selection_snapshot() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    kernel
        .open(PROJECT, OpenSource::Untitled(project()))
        .wait()
        .unwrap()
        .unwrap();
    let mut model = Inspector::new(kernel.published(PROJECT).unwrap(), Default::default());
    let mut properties = Properties::default();
    let mut samples = [Vec::new(), Vec::new()];
    let walks = pm_kernel::model::layer_walks();
    for _ in 0..8 {
        for (i, id) in ["normal", "heavy"].into_iter().enumerate() {
            kernel
                .call(
                    PROJECT,
                    Request::new(
                        Caller::native(),
                        SelectionCall::Layers {
                            ids: vec![id.into()],
                            add: false,
                        },
                    ),
                )
                .wait()
                .unwrap()
                .unwrap();
            model.sync(kernel.published(PROJECT).unwrap());
            let started = Instant::now();
            let snapshot = properties.snapshot(&mut model, 0., false).unwrap();
            samples[i].push(started.elapsed().as_secs_f64() * 1_000.);
            assert_eq!(snapshot["target"]["first"]["id"], id);
            let layer_rows = snapshot["sections"]
                .as_array()
                .unwrap()
                .iter()
                .find(|section| section["id"] == "layer")
                .unwrap()["rows"]
                .as_array()
                .unwrap();
            let parent = layer_rows
                .iter()
                .find(|row| row["id"] == "layer.parent")
                .unwrap();
            assert_eq!(parent["control"]["rows"]["source"], "layer-parents");
            assert_eq!(parent["control"]["options"].as_array().unwrap().len(), 1);
        }
    }
    assert_eq!(pm_kernel::model::layer_walks(), walks);
    for (id, sample) in ["normal", "heavy"].into_iter().zip(samples) {
        let mean = sample.iter().sum::<f64>() / sample.len() as f64;
        let max = sample.into_iter().fold(0., f64::max);
        eprintln!("{id} selection: mean {mean:.3} ms, max {max:.3} ms");
        assert!(mean < 3., "{id} selection averaged {mean:.3} ms");
    }
}

#[test]
fn heavy_comp_scrub_skips_static_panels() {
    use crate::{Builtins, tests::finish};

    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    kernel
        .open(PROJECT, OpenSource::Untitled(project()))
        .wait()
        .unwrap()
        .unwrap();
    let select = |id: &str| {
        kernel
            .call(
                PROJECT,
                Request::new(
                    Caller::native(),
                    SelectionCall::Layers {
                        ids: vec![id.into()],
                        add: false,
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
    };
    select("normal");
    let mut panels = Builtins::new(kernel.clone(), PROJECT, || {}).unwrap();
    let facts = |time: f64| json!({"time":time,"playing":false,"timelineShown":true});
    panels.update_facts(facts(0.));
    for panel in [
        "inspector",
        "layer-effects",
        "assets",
        "fxbrowser",
        "easing",
        "viewer",
    ] {
        finish(panels.mount(panel).unwrap());
    }
    for (_, ticket) in panels.publish() {
        finish(ticket.unwrap());
    }

    select("heavy");
    let tickets = panels.publish();
    assert!(tickets.iter().any(|(id, _)| id == "inspector"));
    assert!(
        !tickets
            .iter()
            .any(|(id, _)| matches!(id.as_str(), "assets" | "easing"))
    );
    for (_, ticket) in tickets {
        finish(ticket.unwrap());
    }

    for tick in 1..=20 {
        panels.update_facts(facts(f64::from(tick) / 30.));
        let (tickets, waited) = panels.publish_live(|_| true);
        let ids: Vec<_> = tickets.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.is_empty(), "static panels queued during scrub: {ids:?}");
        assert!(!waited);
    }
    select("layer1997");
    for (_, ticket) in panels.publish() {
        finish(ticket.unwrap());
    }
    panels.update_facts(facts(0.9));
    let (tickets, _) = panels.publish_live(|_| true);
    assert_eq!(
        tickets
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["inspector"]
    );
    for (_, ticket) in tickets {
        finish(ticket.unwrap());
    }
    assert!(panels.take_errors().is_empty());
    panels.close();
    actor.shutdown();
}
