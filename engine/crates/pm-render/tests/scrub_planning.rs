#[path = "support/doc.rs"]
mod doc;

use pm_eval::{Comp, Evaluator, Scope};
use pm_model::patch::{Patch, PathPart};
use pm_perf::{Counter, Scope as Counters};
use pm_protocol::{
    color::Precision,
    common::{Size, TICKS_PER_SECOND, Ticks},
};
use pm_render::plan::{Content, Source};
use pm_render::{MapResolver, OutputKind, RenderRequest, Renderer, sampling::Sampling};
use serde_json::json;
use std::sync::Arc;

fn ticks(seconds: f64) -> Ticks {
    Ticks::new((seconds * TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

fn playback() -> RenderRequest {
    RenderRequest {
        viewer: true,
        sampling: Sampling::Playback,
        ..RenderRequest::at_scale(Size::new(256, 256), 1.0, Precision::Float16)
    }
}

#[test]
fn heavy_scrub_visits_only_active_passes_and_keeps_static_group_draws() {
    let mut layers = Vec::new();
    for group in 0..200 {
        let id = format!("g{group}");
        let mut parent = doc::layer(&id, "group", json!({}), 0.0, 0.0, 1.0);
        doc::set(&mut parent, "opacity", json!(80));
        parent["on"] = json!(group >= 50);
        layers.push(parent);
        for child in 0..if group < 100 { 6 } else { 7 } {
            let mut shape = doc::layer(
                &format!("s{group}.{child}"),
                "solid",
                json!({"w":5,"h":5,"color":"#ffffff"}),
                128.0,
                128.0,
                3.0,
            );
            shape["group"] = json!(id);
            shape["from"] = json!(if group < 60 { 0 } else { 5 });
            layers.push(shape);
        }
    }
    assert_eq!(layers.len(), 1500);
    let document = doc::load(&doc::project(256, 256, 10.0, doc::no_fill(), layers));
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_draw_reuse(true);
    let assets = MapResolver::default();
    let request = playback();
    let trace = Counters::begin();
    let (first, failures) = renderer
        .plan(&document, "Ptest-root", ticks(1.0), &request, &assets)
        .unwrap();
    assert!(failures.is_empty());
    assert_eq!(first.draws.len(), 10);
    assert_eq!(trace.finish().get(Counter::CompPasses), 11);
    let (second, _) = renderer
        .plan(&document, "Ptest-root", ticks(1.5), &request, &assets)
        .unwrap();
    assert_eq!(first, second);
    let trace = Counters::begin();
    let (third, _) = renderer
        .plan(&document, "Ptest-root", ticks(2.0), &request, &assets)
        .unwrap();
    assert_eq!(third, second);
    assert_eq!(trace.finish().get(Counter::CompPasses), 1);
    assert!(renderer.draw_reuse_stats().reused >= 10);

    for time in [2.0, 2.1] {
        let frame = renderer
            .render(&document, "Ptest-root", ticks(time), &request, &assets)
            .unwrap();
        assert!(frame.failures.is_empty());
        renderer.recycle(frame);
    }
    renderer.wait_idle();
    assert!(renderer.draw_reuse_stats().group_hits > 0);

    let (later, _) = renderer
        .plan(&document, "Ptest-root", ticks(5.5), &request, &assets)
        .unwrap();
    assert_eq!(later.draws.len(), 140);
}

#[test]
fn large_static_path_geometry_survives_transform_edits() {
    use pm_model::Value;
    let paths = (0..80)
        .map(|i| {
            let vertices = (0..32).map(|j| {
            json!({"id":format!("v{j}"),"p":{"x":doc::ch(json!(j)),"y":doc::ch(json!(j % 5))}})
        }).collect::<Vec<_>>();
            json!({"id":format!("p{i}"),"p":{"x":doc::ch(json!(i)),
            "scaleX":doc::ch(json!(100)), "scaleY":doc::ch(json!(100)),
            "closed":doc::ch(json!(true)), "fill":doc::ch(json!("#fff")),
            "fillEnabled":doc::ch(json!(true)), "fillOpacity":doc::ch(json!(100))},
            "vertices":vertices})
        })
        .collect::<Vec<_>>();
    let layer = doc::layer("paths", "shape", json!({"paths":paths}), 20.0, 20.0, 10.0);
    let mut document = doc::load(&doc::project(256, 256, 10.0, doc::no_fill(), vec![layer]));
    let mut ev = Evaluator::new();
    let mut geometry = |document: &pm_model::Document, time| {
        ev.open_channel_memo(document);
        let comp = Comp::resolve(document, "Ptest-root").unwrap();
        let result = Scope::open(&mut ev, &comp)
            .shared_path_geometry(&comp.layers()[0], time)
            .unwrap();
        ev.close_channel_memo();
        result
    };
    let first = geometry(&document, 1.0);
    assert_eq!(first.0.len(), 80);
    assert_eq!(first.0[0].vertices.len(), 32);
    assert!(Arc::ptr_eq(&first, &geometry(&document, 2.0)));
    let key = |key: &str| PathPart::Key(key.into());
    document
        .apply(&[Patch::set(
            vec![
                key("layers"),
                PathPart::Index(0.0),
                key("p"),
                key("position.x"),
                key("v"),
            ],
            Value::Number(60.0),
        )])
        .unwrap();
    assert!(Arc::ptr_eq(&first, &geometry(&document, 2.0)));
    document
        .apply(&[Patch::set(
            vec![
                key("layers"),
                PathPart::Index(0.0),
                key("d"),
                key("paths"),
                PathPart::Index(0.0),
                key("p"),
                key("x"),
                key("v"),
            ],
            Value::Number(8.0),
        )])
        .unwrap();
    let edited = geometry(&document, 2.0);
    assert!(!Arc::ptr_eq(&first, &edited));
    assert_ne!(first.1, edited.1);
}

fn vector_key(content: &Content) -> Option<[u8; 32]> {
    match content {
        Content::Placed(p) => vector_key(&p.content),
        Content::Texture {
            source: Source::Vector(source),
            ..
        } => Some(source.key()),
        _ => None,
    }
}

#[test]
fn animated_plane_reuses_vector_density_and_culls_offscreen_content() {
    let mut group = doc::layer("group", "group", json!({}), 128.0, 128.0, 4.0);
    group["threeD"] = json!(true);
    for axis in ["scale.x", "scale.y"] {
        doc::set(
            &mut group,
            axis,
            doc::keys(json!(2425), &[(0.0, json!(2425)), (4.0, json!(100))]),
        );
    }
    let shape = |id, x| {
        let mut l = doc::layer(
            id,
            "shape",
            json!({"shape":"rect","w":8,"h":8,"color":"#fff"}),
            x,
            0.0,
            4.0,
        );
        l["group"] = json!("group");
        l
    };
    let document = doc::load(&doc::project(
        256,
        256,
        4.0,
        doc::no_fill(),
        vec![group, shape("visible", 0.0), shape("culled", 5000.0)],
    ));
    let mut renderer = Renderer::headless().unwrap();
    let assets = MapResolver::default();
    let request = playback();
    let frame = |renderer: &mut Renderer, t, req: &RenderRequest| {
        let (plan, failures) = renderer
            .plan(&document, "Ptest-root", ticks(t), req, &assets)
            .unwrap();
        assert!(failures.is_empty());
        assert_eq!(
            plan.draws.len(),
            1,
            "{:?}",
            plan.draws.iter().map(|d| &d.layer).collect::<Vec<_>>()
        );
        assert_eq!(plan.draws[0].layer, "visible");
        plan.draws.into_iter().next().unwrap()
    };
    let first = frame(&mut renderer, 0.0, &request);
    let second = frame(&mut renderer, 1.0 / 30.0, &request);
    assert_ne!(first, second);
    assert_eq!(vector_key(&first.content), vector_key(&second.content));
    assert!(vector_key(&first.content).is_some());
    let exact = RenderRequest {
        output_kind: OutputKind::Final,
        ..request
    };
    let final_first = frame(&mut renderer, 0.0, &exact);
    let final_second = frame(&mut renderer, 1.0 / 30.0, &exact);
    assert_ne!(
        vector_key(&final_first.content),
        vector_key(&final_second.content)
    );
}
