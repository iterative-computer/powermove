//! Batched coverage scenes (`VectorCache::prepare_scenes_in`) are the
//! textures each scene renders alone: animated shapes in nested
//! compositions draw the same pixels with batching on and off, with fewer
//! GPU passes and bind groups per frame. Headless Metal, no window.
#![cfg(feature = "kernel")]
use pm_perf::Counter;
use pm_protocol::{
    color::Precision,
    common::{Size, Ticks},
};
use pm_render::{OutputKind, RenderRequest, Renderer};
use serde_json::{Value, json};
use std::sync::Arc;

const TICKS_PER_SECOND: i64 = 705_600_000;
const KEYS: [f64; 3] = [0.0, 1.5, 3.0];

fn channel(v: f64, keys: Option<[f64; 3]>, id: &str) -> Value {
    let kf: Vec<Value> = keys
        .map(|vals| {
            KEYS.iter()
                .zip(vals)
                .enumerate()
                .map(|(i, (t, x))| {
                    json!({"t":t,"v":x,"i":format!("{id}_{i}"),"inInterp":"linear","outInterp":"linear",
                        "inEase":{"speed":0,"influence":33.333333333333336},
                        "outEase":{"speed":0,"influence":33.333333333333336},
                        "autoBezier":false,"continuous":false})
                })
                .collect()
        })
        .unwrap_or_default();
    json!({"v":v,"kf":kf,"expr":null})
}

fn layer(id: &str, ty: &str, content: Value, tracks: &[(&str, f64, Option<[f64; 3]>)]) -> Value {
    let mut p = serde_json::Map::new();
    for (k, v) in [
        ("perspective", 50.),
        ("position.x", 0.),
        ("position.y", 0.),
        ("position.z", 0.),
        ("anchor.x", 0.),
        ("anchor.y", 0.),
        ("anchor.z", 0.),
        ("scale.x", 100.),
        ("scale.y", 100.),
        ("scale.z", 100.),
        ("rotation", 0.),
        ("rotation.x", 0.),
        ("rotation.y", 0.),
        ("orientation.x", 0.),
        ("orientation.y", 0.),
        ("orientation.z", 0.),
        ("opacity", 100.),
        ("skew", 0.),
    ] {
        p.insert(k.into(), channel(v, None, id));
    }
    for (k, v, keys) in tracks {
        p.insert((*k).into(), channel(*v, *keys, &format!("{id}_{k}")));
    }
    json!({"id":id,"type":ty,"name":id,"from":0,"dur":3,"on":true,"lock":false,"shy":false,
        "collapsed":true,"threeD":false,"solo":false,"color":"#6395F2","blend":"normal",
        "mblur":false,"parent":null,"group":null,"matteSource":null,"p":p,"fx":[],
        "transitionIn":null,"transitionOut":null,"masks":[],"d":content,"locked_intent":{}})
}

fn comp(id: &str, w: u32, h: u32, layers: Vec<Value>) -> Value {
    json!({"id":id,"name":id,"w":w,"h":h,"fps":30,"fpsRational":{"num":30,"den":1},"dur":3,
        "bg":"#101724","backgroundFill":{"type":"none"},"layers":layers,"markers":[],
        "work":[0,3],"shutter":0.5})
}

/// `n` shapes animating position, rotation, scale and opacity, alternating
/// rectangles and ellipses, some stroked, some reaching past the edge of
/// their composition (clipped).
fn shapes(prefix: &str, n: usize, w: u32, h: u32) -> Vec<Value> {
    let palette = ["#ED6A5A", "#F4B942", "#5BC0BE", "#6395F2", "#A57BE8"];
    (0..n)
        .map(|i| {
            let fi = i as f64;
            let x = 12.0 + (fi * 37.0) % f64::from(w);
            let y = 10.0 + (fi * 23.0) % f64::from(h);
            let (dx, dy, r) = (5.0 + fi % 7.0, 4.0 + fi % 5.0, (fi * 13.0) % 41.0 - 20.0);
            let kind = if i % 2 == 1 { "ellipse" } else { "rect" };
            let stroke = if i % 3 == 0 { 3.0 } else { 0.0 };
            let id = format!("{prefix}{i:02}");
            layer(
                &id,
                "shape",
                json!({"shape":kind,"color":palette[i % palette.len()],"w":18.0 + (fi * 7.0) % 23.0,
                    "h":14.0 + (fi * 11.0) % 19.0,"radius":0,"stroke":stroke,
                    "strokeColor":"#FFFFFF","points":5}),
                &[
                    ("position.x", x, Some([x, x + dx + 0.37, x - dx])),
                    ("position.y", y, Some([y, y - dy, y + dy + 0.21])),
                    ("rotation", r, Some([r, r + 50.0, r - 35.0])),
                    ("scale.x", 100.0, Some([100.0, 81.0 + fi % 9.0, 112.0])),
                    ("scale.y", 100.0, Some([100.0, 110.0, 78.0 + fi % 7.0])),
                    ("opacity", 70.0, Some([70.0, 95.0, 55.0])),
                ],
            )
        })
        .collect()
}

fn document() -> Arc<pm_model::Document> {
    let mut inner = shapes("i", 10, 120, 80);
    // A shape past the inner composition's corner: clipped on both sides.
    inner.push(layer(
        "edge",
        "shape",
        json!({"shape":"ellipse","color":"#57BB83","w":60,"h":44,"radius":0,"stroke":2,
            "strokeColor":"#FFFFFF","points":5}),
        &[
            ("position.x", 118.0, Some([118.0, 112.5, 121.25])),
            ("position.y", 2.0, Some([2.0, 6.5, -3.25])),
            ("rotation", 10.0, Some([10.0, 40.0, -20.0])),
        ],
    ));
    let mut outer = shapes("o", 14, 200, 120);
    outer.insert(
        3,
        layer(
            "pre_inner",
            "precomp",
            json!({"comp":"inner","w":120,"h":80}),
            &[
                ("position.x", 60.0, Some([60.0, 70.5, 55.25])),
                ("position.y", 30.0, None),
                ("opacity", 85.0, None),
            ],
        ),
    );
    let main = vec![
        layer(
            "pre_outer",
            "precomp",
            json!({"comp":"outer","w":200,"h":120}),
            &[("position.x", 40.0, None), ("position.y", 30.0, None)],
        ),
        layer(
            "pre_outer_2",
            "precomp",
            json!({"comp":"outer","w":200,"h":120}),
            &[
                ("position.x", 150.0, None),
                ("position.y", 70.0, None),
                ("rotation", 7.0, Some([7.0, -5.0, 12.0])),
            ],
        ),
        layer(
            "background",
            "solid",
            json!({"color":"#101724","w":320,"h":180}),
            &[],
        ),
    ];
    let project = json!({"id":"vector-batch","name":"vector batch","compId":"main",
        "comps":{"inner":comp("inner",120,80,inner),"outer":comp("outer",200,120,outer),
            "main":comp("main",320,180,main)},
        "assets":{},"colorDepth":8,"color":{"mode":"standard","linearBlending":false},
        "params":{},"revision":0,"edits":[],"created":0});
    Arc::new(
        pm_kernel::DocumentStore::new_untitled(
            pm_model::json::parse(&project.to_string()).unwrap(),
        )
        .snapshot()
        .document
        .as_ref()
        .clone(),
    )
}

fn request(precision: Precision) -> RenderRequest {
    let mut r = RenderRequest::sized(Size::new(320, 180), precision);
    r.output_kind = OutputKind::Final;
    r
}

/// Pixels of `seconds`, and the frame's counters.
fn draw(
    renderer: &mut Renderer,
    doc: &pm_model::Document,
    seconds: f64,
    precision: Precision,
) -> (Vec<f32>, pm_perf::Counters) {
    let before = pm_perf::snapshot();
    let ticks = Ticks::new((seconds * TICKS_PER_SECOND as f64).round() as i64).unwrap();
    let f = renderer
        .render(
            doc,
            "main",
            ticks,
            &request(precision),
            &pm_render::MapResolver::default(),
        )
        .unwrap();
    let counters = pm_perf::snapshot().since(&before);
    assert!(f.failures.is_empty(), "{:?}", f.failures);
    (renderer.read_rgba(&f).unwrap(), counters)
}

#[test]
fn batched_vector_scenes_draw_the_unbatched_pixels_with_less_gpu_setup() {
    let doc = document();
    for precision in [Precision::Unorm8, Precision::Float16, Precision::Float32] {
        let gpu = pm_gpu::Gpu::new("vector batch on").unwrap();
        let mut batched = Renderer::new(gpu.device, gpu.queue);
        let gpu = pm_gpu::Gpu::new("vector batch off").unwrap();
        let mut alone = Renderer::new(gpu.device, gpu.queue);
        alone.set_vector_batching(false);
        let mut previous: Option<Vec<f32>> = None;
        // Every shape moves, turns and scales between these: all miss.
        for seconds in [0.0, 0.7333, 1.4, 2.2667, 2.9667] {
            let (a, on) = draw(&mut batched, &doc, seconds, precision);
            let (b, off) = draw(&mut alone, &doc, seconds, precision);
            assert_eq!(a.len(), b.len());
            let differing = a
                .iter()
                .zip(&b)
                .filter(|(x, y)| x.to_bits() != y.to_bits())
                .count();
            assert_eq!(
                differing,
                0,
                "{precision:?} at {seconds}s: {differing} of {} values differ",
                a.len()
            );
            let misses = off.get(Counter::VectorCacheMisses);
            // 25 animated shapes; the outer composition's two instances may
            // share its rendered shapes, which are hits in both modes.
            assert!(misses >= 20, "{precision:?}: every shape misses ({misses})");
            assert_eq!(on.get(Counter::VectorCacheMisses), misses);
            assert_eq!(
                on.get(Counter::VectorCacheHits),
                off.get(Counter::VectorCacheHits),
                "a batched scene is not a hit"
            );
            assert!(
                on.get(Counter::VectorBatchedScenes) * 10 >= misses * 9,
                "{precision:?}: most misses batch ({} of {misses})",
                on.get(Counter::VectorBatchedScenes)
            );
            assert_eq!(off.get(Counter::VectorBatchedScenes), 0);
            for counter in [
                Counter::RenderPasses,
                Counter::ComputePasses,
                Counter::BindGroupCreations,
                Counter::BufferAllocs,
            ] {
                assert!(
                    on.get(counter) <= off.get(counter),
                    "{precision:?} {counter:?}: {} batched, {} alone",
                    on.get(counter),
                    off.get(counter)
                );
            }
            // GPU work is counted only where the vendored wgpu front counts
            // it; without counts there is nothing to compare.
            let gpu_counted = off.get(Counter::RenderPasses) > 0;
            if !gpu_counted && seconds == 0.0 {
                eprintln!(
                    "{precision:?}: GPU work counters not recorded; pass counts not compared"
                );
            }
            for counter in [Counter::RenderPasses, Counter::ComputePasses] {
                if !gpu_counted {
                    break;
                }
                assert!(
                    on.get(counter) * 2 < off.get(counter),
                    "{precision:?} {counter:?}: {} batched, {} alone",
                    on.get(counter),
                    off.get(counter)
                );
            }
            if let Some(p) = &previous {
                assert!(
                    p.iter().zip(&a).any(|(x, y)| x != y),
                    "the shapes animate between frames"
                );
            }
            previous = Some(a);
            eprintln!(
                "{precision:?} {seconds}s misses {misses} batched {} | render passes {} vs {} | compute passes {} vs {} | bind groups {} vs {} | textures {} vs {}",
                on.get(Counter::VectorBatchedScenes),
                on.get(Counter::RenderPasses),
                off.get(Counter::RenderPasses),
                on.get(Counter::ComputePasses),
                off.get(Counter::ComputePasses),
                on.get(Counter::BindGroupCreations),
                off.get(Counter::BindGroupCreations),
                on.get(Counter::TextureAllocs),
                off.get(Counter::TextureAllocs),
            );
        }
        // The same frame again: every scene is a hit, nothing is made.
        let (a, again) = draw(&mut batched, &doc, 2.9667, precision);
        assert_eq!(again.get(Counter::VectorCacheMisses), 0);
        assert_eq!(again.get(Counter::VectorBatchedScenes), 0);
        assert!(again.get(Counter::VectorCacheHits) >= 20);
        assert_eq!(Some(a), previous);
    }
}

/// A 1280×720 composition of 60 larger animated shapes: more coverage than
/// one sub-batch holds, so a frame batches in several sub-batches.
fn large_document() -> Arc<pm_model::Document> {
    let palette = ["#ED6A5A", "#F4B942", "#5BC0BE", "#6395F2", "#A57BE8"];
    let layers: Vec<Value> = (0..60)
        .map(|i| {
            let fi = i as f64;
            let (x, y) = (70.0 + (i % 10) as f64 * 120.0, 60.0 + (i / 10) as f64 * 110.0);
            let r = (fi * 13.0) % 41.0 - 20.0;
            layer(
                &format!("L{i:02}"),
                "shape",
                json!({"shape": if i % 2 == 1 { "ellipse" } else { "rect" },
                    "color": palette[i % palette.len()], "w": 110.0 + (fi * 17.0) % 50.0,
                    "h": 90.0 + (fi * 13.0) % 40.0, "radius": 0, "stroke": if i % 4 == 0 { 4.0 } else { 0.0 },
                    "strokeColor": "#FFFFFF", "points": 5}),
                &[
                    ("position.x", x, Some([x, x + 9.37, x - 7.0])),
                    ("position.y", y, Some([y, y - 6.0, y + 8.21])),
                    ("rotation", r, Some([r, r + 50.0, r - 35.0])),
                    ("scale.x", 100.0, Some([100.0, 85.0, 112.0])),
                    ("scale.y", 100.0, Some([100.0, 110.0, 80.0])),
                    ("opacity", 80.0, Some([80.0, 95.0, 60.0])),
                ],
            )
        })
        .collect();
    let project = json!({"id":"vector-batch-large","name":"vector batch large","compId":"main",
        "comps":{"main":comp("main",1280,720,layers)},
        "assets":{},"colorDepth":8,"color":{"mode":"standard","linearBlending":false},
        "params":{},"revision":0,"edits":[],"created":0});
    Arc::new(
        pm_kernel::DocumentStore::new_untitled(
            pm_model::json::parse(&project.to_string()).unwrap(),
        )
        .snapshot()
        .document
        .as_ref()
        .clone(),
    )
}

#[test]
fn sub_batched_and_split_batches_draw_the_unbatched_pixels() {
    let doc = large_document();
    let draw_large = |renderer: &mut Renderer, seconds: f64| {
        let before = pm_perf::snapshot();
        let ticks = Ticks::new((seconds * TICKS_PER_SECOND as f64).round() as i64).unwrap();
        let mut r = RenderRequest::sized(Size::new(1280, 720), Precision::Float16);
        r.output_kind = OutputKind::Final;
        let f = renderer
            .render(&doc, "main", ticks, &r, &pm_render::MapResolver::default())
            .unwrap();
        let counters = pm_perf::snapshot().since(&before);
        assert!(f.failures.is_empty(), "{:?}", f.failures);
        (renderer.read_rgba(&f).unwrap(), counters)
    };
    // (scratch bound, expectation): the standing bound sub-batches; a low
    // one splits batches and leaves some scenes to resolve alone.
    for limit in [None, Some(2u64 << 20)] {
        let gpu = pm_gpu::Gpu::new("vector batch large on").unwrap();
        let mut batched = Renderer::new(gpu.device, gpu.queue);
        if let Some(bytes) = limit {
            batched.limit_vector_frame_scratch(bytes);
        }
        let gpu = pm_gpu::Gpu::new("vector batch large off").unwrap();
        let mut alone = Renderer::new(gpu.device, gpu.queue);
        alone.set_vector_batching(false);
        for seconds in [0.4333, 1.7, 2.8] {
            let (a, on) = draw_large(&mut batched, seconds);
            let (b, off) = draw_large(&mut alone, seconds);
            let differing = a
                .iter()
                .zip(&b)
                .filter(|(x, y)| x.to_bits() != y.to_bits())
                .count();
            assert_eq!(
                differing, 0,
                "{limit:?} at {seconds}s: {differing} values differ"
            );
            let misses = off.get(Counter::VectorCacheMisses);
            assert_eq!(misses, 60);
            assert_eq!(on.get(Counter::VectorCacheMisses), misses);
            let batches = on.get(Counter::VectorBatches);
            let splits = on.get(Counter::VectorBatchSplits);
            eprintln!(
                "{limit:?} {seconds}s batched {} batches {batches} splits {splits} refused_scratch {} unplaced {}",
                on.get(Counter::VectorBatchedScenes),
                on.get(Counter::VectorBatchRefusedScratch),
                on.get(Counter::VectorBatchUnplaced),
            );
            match limit {
                None => {
                    assert!(batches >= 2, "several sub-batches ({batches})");
                    assert_eq!(on.get(Counter::VectorBatchedScenes), misses);
                }
                Some(_) => assert!(splits > 0, "a low bound splits batches"),
            }
        }
    }
}

/// The benchmark fixture itself (`PM_VECTOR_BATCH_FIXTURE`: the canonical
/// project JSON of the Public 2D Motion Grid), frames 0, 37 and 150 at
/// 1920×1080: batched and unbatched pixels are identical. Skipped when the
/// variable is not set.
#[test]
fn benchmark_fixture_frames_draw_the_unbatched_pixels() {
    let Some(path) = std::env::var_os("PM_VECTOR_BATCH_FIXTURE") else {
        eprintln!("PM_VECTOR_BATCH_FIXTURE not set; skipped");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let main = serde_json::from_str::<Value>(&text).unwrap()["compId"]
        .as_str()
        .unwrap()
        .to_owned();
    let doc = Arc::new(
        pm_kernel::DocumentStore::new_untitled(pm_model::json::parse(&text).unwrap())
            .snapshot()
            .document
            .as_ref()
            .clone(),
    );
    let gpu = pm_gpu::Gpu::new("fixture batched").unwrap();
    let mut batched = Renderer::new(gpu.device, gpu.queue);
    let gpu = pm_gpu::Gpu::new("fixture alone").unwrap();
    let mut alone = Renderer::new(gpu.device, gpu.queue);
    alone.set_vector_batching(false);
    for frame in [0i64, 37, 150] {
        let ticks = Ticks::new(frame * (TICKS_PER_SECOND / 30)).unwrap();
        let mut read = |r: &mut Renderer| {
            let before = pm_perf::snapshot();
            let mut q = RenderRequest::sized(Size::new(1920, 1080), Precision::Unorm8);
            q.output_kind = OutputKind::Final;
            let f = r
                .render(&doc, &main, ticks, &q, &pm_render::MapResolver::default())
                .unwrap();
            assert!(f.failures.is_empty(), "{:?}", f.failures);
            (r.read_rgba(&f).unwrap(), pm_perf::snapshot().since(&before))
        };
        let (a, on) = read(&mut batched);
        let (b, off) = read(&mut alone);
        let differing = a
            .iter()
            .zip(&b)
            .filter(|(x, y)| x.to_bits() != y.to_bits())
            .count();
        eprintln!(
            "fixture frame {frame}: misses {} batched {} differing {differing}",
            off.get(Counter::VectorCacheMisses),
            on.get(Counter::VectorBatchedScenes)
        );
        assert_eq!(differing, 0, "frame {frame}");
        assert_eq!(off.get(Counter::VectorCacheMisses), 240);
        assert!(on.get(Counter::VectorBatchedScenes) > 0);
    }
}
