use pm_fx_defs::{catalog::CatalogError, graph::*, inputs::*};
use std::collections::BTreeMap;
fn r(id: &str, stage: Stage, time: f64, prefix: Option<usize>) -> Request {
    Request::new(id, stage, Space::Source, Some(time), prefix).unwrap()
}
fn dep(request: Request, input_lease: bool) -> Dependency {
    Dependency {
        request,
        input_lease,
    }
}
#[test]
fn hidden_source_and_shared_inputs_run_before_each_consumer_once() {
    let source = r("hidden", Stage::Source, 1.234, None);
    let a = r("A", Stage::Effects, 1.234, None);
    let b = r("B", Stage::Effects, 1.234, None);
    let mut expansions = BTreeMap::new();
    let plan = input_plan([a.clone(), b.clone()], |request| {
        *expansions.entry(request.clone()).or_insert(0) += 1;
        Ok(if request == &a || request == &b {
            vec![dep(source.clone(), true)]
        } else {
            vec![]
        })
    })
    .unwrap();
    assert_eq!(plan.order, vec![source.clone(), a, b]);
    assert_eq!(plan.requests, vec![source]);
    assert!(expansions.values().all(|count| *count == 1));
}
#[test]
fn source_masks_effects_stages_keep_authored_order() {
    let effects = r("L", Stage::Effects, 0., None);
    let masks = r("L", Stage::Masks, 0., None);
    let source = r("L", Stage::Source, 0., None);
    let plan = input_plan([effects.clone()], |request| {
        Ok(match request.stage {
            Stage::Effects => vec![dep(masks.clone(), false)],
            Stage::Masks => vec![dep(source.clone(), false)],
            Stage::Source => vec![],
        })
    })
    .unwrap();
    assert_eq!(plan.order, vec![source, masks, effects]);
    assert!(plan.requests.is_empty());
}
#[test]
fn prefix_and_subframe_are_distinct_even_at_reverse_seek() {
    let full = r("L", Stage::Effects, 2., None);
    let prefix = r("L", Stage::Effects, 2., Some(1));
    let past = r("L", Stage::Effects, 1.999, Some(1));
    let run = || {
        input_plan([full.clone()], |request| {
            Ok(if request == &full {
                vec![dep(prefix.clone(), true), dep(past.clone(), true)]
            } else {
                vec![]
            })
        })
        .unwrap()
    };
    let plan = run();
    let repeated = run();
    assert_eq!(plan.order, vec![prefix.clone(), past.clone(), full]);
    assert_eq!(plan.requests, vec![prefix, past]);
    assert_eq!(plan.order, repeated.order);
}
#[test]
fn self_and_multi_layer_cycles_are_named_errors_with_no_partial_plan() {
    let a = r("A", Stage::Effects, 0., None);
    let b = r("B", Stage::Effects, 0., None);
    let error = input_plan([a.clone()], |_| Ok(vec![dep(a.clone(), true)])).unwrap_err();
    assert!(error.0.contains("cycle") && error.0.contains("A"));
    let error = input_plan([a.clone()], |request| {
        Ok(vec![dep(
            if request == &a { b.clone() } else { a.clone() },
            true,
        )])
    })
    .unwrap_err();
    assert!(error.0.contains("A") && error.0.contains("B"));
}
#[test]
fn missing_provider_and_lease_failures_propagate() {
    let error = input_plan([r("L", Stage::Effects, 0., None)], |_| {
        Err(CatalogError("missing source lease".into()))
    })
    .unwrap_err();
    assert_eq!(error.0, "missing source lease");
}
#[test]
fn request_budget_counts_unique_leases_and_retains_the_256_boundary() {
    let full = r("L", Stage::Effects, 0., None);
    let run = |count| {
        input_plan([full.clone()], |request| {
            Ok(if request == &full {
                (0..count)
                    .map(|i| dep(r("L", Stage::Effects, f64::from(i), Some(0)), true))
                    .collect()
            } else {
                vec![]
            })
        })
    };
    assert_eq!(run(256).unwrap().requests.len(), 256);
    assert!(run(257).unwrap_err().0.contains("request limit"));
    let same = r("hidden", Stage::Source, 0., None);
    assert_eq!(
        input_plan([full.clone()], |request| Ok(if request == &full {
            vec![dep(same.clone(), true); 1000]
        } else {
            vec![]
        }))
        .unwrap()
        .requests
        .len(),
        1
    );
}
#[test]
fn deep_graphs_use_a_heap_stack_and_enforce_the_node_ceiling() {
    let run = |count: u32| {
        input_plan([r("L", Stage::Effects, 0., None)], |request| {
            let time = request.time().unwrap();
            Ok(if time < f64::from(count - 1) {
                vec![dep(r("L", Stage::Effects, time + 1., None), false)]
            } else {
                vec![]
            })
        })
    };
    assert_eq!(run(4096).unwrap().order.len(), 4096);
    assert!(
        run(MAX_GRAPH_NODES as u32 + 1)
            .unwrap_err()
            .0
            .contains("node limit")
    );
}
