// Shared fixture for the large-composition viewer drag regression tests.
use pm_kernel::{
    Call, Caller, GestureCall, KernelActor, KernelConfig, KernelHandle, OpenSource,
    PropertyCommand, Published, Reply, Request, SelectionCall, WriteBody,
    commands::properties::SetProperty,
};
use serde_json::json;
use std::sync::Arc;

pub const PROJECT: &str = "drag-fixture";
pub const TEXT: &str = "headline";
pub const TIME: f64 = 12.4;
pub const STEPS: usize = 20;

pub fn open(journal_root: std::path::PathBuf) -> (KernelActor, KernelHandle) {
    let mut layers = vec![
        json!({"id":TEXT,"type":"text","name":"fgsgfsgsfgs","from":12,"dur":91,
            "d":{"text":"fgsgfsgsfgs","font":"Geist","size":65,"color":"#FFFFFF"},
            "p":{"position.x":{"v":960,"kf":[]},"position.y":{"v":540,"kf":[]}}}),
        json!({"id":"solid","type":"solid","name":"Solid","from":0,"dur":51,
            "d":{"w":1920,"h":1080,"color":"#000000"}}),
    ];
    for (group, count) in [(0, 316), (1, 317)] {
        let id = format!("group{group}");
        let from = group * 6;
        layers.push(json!({
            "id":format!("adjustment{group}"),"name":"Adjustment Layer","type":"adjustment",
            "from":from,"dur":6
        }));
        layers.push(json!({
            "id":id,"name":format!("Powermove UI {}", group+1),"type":"group",
            "threeD":true,"from":from,"dur":6
        }));
        for i in 0..count {
            let (kind, data) = match i % 10 {
                _ if group == 0 && i < 7 => (
                    "shader",
                    json!({
                        "code":"void mainImage(out vec4 c, in vec2 p) { c = vec4(1.); }",
                    }),
                ),
                0..=2 => (
                    "text",
                    json!({"text":format!("Label {i}"),"font":"Geist","size":20}),
                ),
                3 => ("null", json!({})),
                4 => ("solid", json!({"w":100,"h":40,"color":"#000000"})),
                _ => (
                    "shape",
                    json!({"w":120,"h":32,"shape":"rect","color":"#FFFFFF"}),
                ),
            };
            layers.push(json!({
                "id":format!("{id}-{i}"),"type":kind,"name":format!("Layer {i}"),
                "group":id,"from":from,"dur":6,"on":true,"d":data
            }));
        }
    }
    for layer in &mut layers {
        let layer = layer.as_object_mut().unwrap();
        layer.entry("on").or_insert(json!(true));
        layer.entry("d").or_insert(json!({}));
        layer.entry("fx").or_insert(json!([]));
        let channels = layer.entry("p").or_insert(json!({})).as_object_mut().unwrap();
        for (key, default) in [
            ("position.x", 960.), ("position.y", 540.),
            ("anchor.x", 0.), ("anchor.y", 0.),
            ("scale.x", 100.), ("scale.y", 100.),
            ("rotation", 0.), ("opacity", 100.),
        ] {
            channels.entry(key).or_insert(json!({"v":default,"kf":[]}));
        }
    }
    assert_eq!(layers.len(), 639);
    let project = json!({"id":PROJECT,"name":"Do anything","w":1920,"h":1080,
        "fps":30,"dur":103,"bg":"#000000","assets":{},"comps":{},"layers":layers});
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(journal_root),
        ..Default::default()
    });
    let kernel = actor.handle();
    kernel
        .open(
            PROJECT,
            OpenSource::Untitled(pm_model::json::parse(&project.to_string()).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    call(
        &kernel,
        SelectionCall::Layers {
            ids: vec![TEXT.into()],
            add: false,
        }
        .into(),
    );
    (actor, kernel)
}

fn call(kernel: &KernelHandle, call: Call) -> Reply {
    kernel
        .call(PROJECT, Request::new(Caller::native(), call))
        .wait()
        .unwrap()
        .unwrap()
        .reply
}

pub fn drag(kernel: &KernelHandle, mut measure: impl FnMut(Arc<Published>)) {
    let Reply::Gesture(gesture) = call(
        kernel,
        GestureCall::Begin {
            label: "Move".into(),
        }
        .into(),
    ) else {
        panic!("gesture did not start");
    };
    for i in 1..=STEPS {
        let body = WriteBody::Commands(
            [("position.x", 960.), ("position.y", 540.)]
                .into_iter()
                .map(|(channel, start)| {
                    let mut set = SetProperty::new(TEXT, channel, start + i as f64, TIME);
                    set.mark_intent = true;
                    set.preserve_hand_edits = false;
                    PropertyCommand::Set(set).into()
                })
                .collect(),
        );
        call(
            kernel,
            GestureCall::Step {
                gesture,
                body,
                replace: true,
            }
            .into(),
        );
        let published = kernel.published(PROJECT).unwrap();
        assert!(published.entry.is_some());
        measure(published);
    }
    call(kernel, GestureCall::Commit { gesture }.into());
}

pub fn report<const N: usize>(names: [&str; N], samples: &[[f64; N]]) {
    for (i, name) in names.into_iter().enumerate() {
        let mean = samples.iter().map(|s| s[i]).sum::<f64>() / samples.len() as f64;
        let max = samples.iter().map(|s| s[i]).fold(0_f64, f64::max);
        eprintln!("{name}: mean {mean:.3} ms, max {max:.3} ms");
    }
}

/// Use a mean deadline so an occasional system scheduling interruption does
/// not fail the test. Publish the worst step alongside it for diagnosis.
pub fn assert_budget<const N: usize>(
    samples: &[[f64; N]],
    stages: impl IntoIterator<Item = usize>,
) {
    assert_eq!(samples.len(), STEPS);
    for i in stages {
        let mean = samples.iter().map(|step| step[i]).sum::<f64>() / samples.len() as f64;
        assert!(mean < 5., "stage {i} averaged {mean:.3} ms");
    }
}
