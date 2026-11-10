//! Agent edits through MCP `apply_commands` on synthetic large projects: the
//! whole call an agent waits on (planning, the kernel transaction, the
//! selection it leaves), timed per typical command.
//!
//! cargo bench -p pm-mcp --features synthetic --bench apply_scale [-- 5000]
//!
//! Target: a typical call under one 60 Hz frame (16.7 ms).
use std::time::{Duration, Instant};

use pm_kernel::project_io::{SaveOptions, SaveTarget};
use pm_mcp::{Automation, McpSession};
use serde_json::{Value, json};

const FRAME: Duration = Duration::from_micros(16_667);
const RUNS: usize = 15;

fn command(kind: &str, layer: &str, spare: &str, n: usize) -> Value {
    match kind {
        "set_property" => {
            json!({"type":"set_property","target":layer,"path":"position.x","value":100 + n})
        }
        "keyframe" => {
            json!({"type":"set_property","target":layer,"path":"rotation","value":n,"time":0.5 + n as f64 * 0.1,"mode":"keyframe"})
        }
        "rename" => {
            json!({"type":"set_layer","target":layer,"patch":{"name":format!("Renamed {n}")}})
        }
        "add_layer" => {
            json!({"type":"add_layer","id":format!("bench{n}"),"layerType":"solid","name":format!("Bench {n}")})
        }
        "delete" => json!({"type":"delete_layers","targets":[format!("bench{n}")]}),
        "by_index" => {
            json!({"type":"set_layer","target":2500,"patch":{"name":format!("Indexed {n}")}})
        }
        "set_easing" => {
            json!({"type":"set_easing","keyframes":[format!("k{layer}_position.x_{}", n % 4)],
            "curve":if n.is_multiple_of(2) { "easeInOut" } else { "linear" }})
        }
        "simplify" => json!({"type":"simplify_keyframes","scope":"layers","targets":[layer]}),
        "reorder" => json!({"type":"reorder_layer","target":layer,"index":n % 7}),
        // A different layer each run: a layer is grouped once.
        "group" => json!({"type":"group_layers","targets":[spare],"name":format!("Group {n}")}),
        _ => unreachable!(),
    }
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn run(layers: usize) -> bool {
    let dir = std::env::temp_dir().join(format!("pm-apply-scale-{}-{layers}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scale.pmv");
    let mut store = pm_kernel::DocumentStore::new_untitled(pm_kernel::synthetic::project(layers));
    let plan = store
        .prepare_save(
            SaveTarget::SaveAs {
                path: path.clone(),
                replace: true,
            },
            SaveOptions::default(),
        )
        .unwrap();
    store.finish_save(plan.run().unwrap()).unwrap();
    drop(store);

    let mut host = Automation::new(dir.join("profile"), dir.join("exports")).unwrap();
    host.open_file("scale", path).unwrap();
    let mut session = McpSession::new(std::sync::Arc::new(std::sync::Mutex::new(host)));
    let started = session.call("start_session", json!({"prompt":"Scale bench"}));
    assert_eq!(started["isError"], false, "{started}");
    let ids = pm_kernel::synthetic::main_layer_ids(layers);
    let target = ids[ids.len() / 2].clone();
    let mut ok = true;
    println!("{layers} layers");
    for kind in [
        "set_property",
        "keyframe",
        "rename",
        "by_index",
        "add_layer",
        "delete",
        "set_easing",
        "simplify",
        "reorder",
        "group",
    ]
    .into_iter()
    // `PM_BENCH_ONLY=reorder,group` times those alone (profiling).
    .filter(|kind| {
        std::env::var("PM_BENCH_ONLY").map_or(true, |only| only.split(',').any(|o| o == *kind))
    }) {
        let mut times = vec![];
        for n in 0..RUNS {
            let t = Instant::now();
            let reply = session.call(
                "apply_commands",
                json!({"commands":[command(kind, &target, &ids[100 + n], n)]}),
            );
            times.push(t.elapsed());
            assert_eq!(reply["isError"], false, "{kind}: {reply}");
        }
        let worst = *times.iter().max().unwrap();
        let med = median(times);
        ok &= med < FRAME;
        println!(
            "  {kind:<13} median {:>8.2} ms  worst {:>8.2} ms{}",
            med.as_secs_f64() * 1e3,
            worst.as_secs_f64() * 1e3,
            if med < FRAME { "" } else { "  OVER A FRAME" }
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

fn main() {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let sizes = if sizes.is_empty() { vec![5000] } else { sizes };
    let mut ok = true;
    for layers in sizes {
        ok &= run(layers);
    }
    if !ok {
        std::process::exit(1);
    }
}
