//! Headless 200-edit retained-allocation probe on a disposable PMV copy.
//! Usage: edit_memory COPY.pmv [--legacy-log]
//! `--legacy-log` retains the same last 256 decoded changes as the old
//! store, via a subscriber. The fixed run uses only the byte-bounded log.
//! Counts requested live Rust allocation bytes, not RSS or GPU/V8 memory.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering::Relaxed};
static ENABLED: AtomicBool = AtomicBool::new(true);
static BYTES: AtomicI64 = AtomicI64::new(0);
static COUNT: AtomicI64 = AtomicI64::new(0);
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() && ENABLED.load(Relaxed) {
            BYTES.fetch_add(l.size() as i64, Relaxed);
            COUNT.fetch_add(1, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if ENABLED.load(Relaxed) {
            BYTES.fetch_sub(l.size() as i64, Relaxed);
            COUNT.fetch_sub(1, Relaxed);
        }
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() && ENABLED.load(Relaxed) {
            BYTES.fetch_add(n as i64 - l.size() as i64, Relaxed);
        }
        q
    }
}
#[global_allocator]
static ALLOC: Counting = Counting;
fn live() -> (i64, i64) {
    (BYTES.load(Relaxed), COUNT.load(Relaxed))
}
fn main() {
    use pm_kernel::commands::{CompCommand, GroupCommand, LayerCommand};
    use pm_kernel::{Command, DocumentStore, Origin, StoreEvent};
    use std::sync::{Arc, Mutex};
    let file = std::path::PathBuf::from(std::env::args().nth(1).expect("disposable copy"));
    let legacy = std::env::args().any(|a| a == "--legacy-log");
    let baseline = live();
    let mut store = DocumentStore::open(
        pm_kernel::project_io::open_project(&file, Default::default()).unwrap(),
    );
    let root = store.snapshot().document.to_value().unwrap();
    let comps = root
        .get("comps")
        .and_then(pm_model::Value::as_object)
        .expect("compositions");
    let (id, _) = comps
        .iter()
        .find(|(_, c)| c.get("name").and_then(pm_model::Value::as_str) == Some("Native UI"))
        .expect("Native UI");
    let id = id.as_str().unwrap().to_owned();
    drop(root);
    store
        .execute(
            Command::Comp(CompCommand::Open { comp_id: id }),
            Origin::Mcp,
        )
        .unwrap();
    let layers = store.layers();
    let mut buckets = std::collections::HashMap::<Option<String>, Vec<String>>::new();
    for l in &layers {
        if !l.effectively_locked && matches!(l.kind.as_str(), "shape" | "text") {
            buckets
                .entry(l.group.clone())
                .or_default()
                .push(l.id.clone());
        }
    }
    let ids: Vec<String> = buckets
        .into_values()
        .max_by_key(Vec::len)
        .unwrap()
        .into_iter()
        .take(37)
        .collect();
    assert_eq!(ids.len(), 37);
    println!(
        "opened bytes={} history={:?} layers={}",
        live().0 - baseline.0,
        store.history().stats(),
        layers.len()
    );
    // Recreate the previous 256-entry decoded change retention exactly,
    // while the corrected store itself uses its byte budget.
    let held = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    if legacy {
        let held = held.clone();
        store.subscribe(move |e| {
            if let StoreEvent::Changed(c) = e {
                let mut h = held.lock().unwrap();
                h.push_back(c.clone());
                while h.len() > 256 {
                    h.pop_front();
                }
            }
        });
    }
    let mut peak_record: Option<pm_kernel::history::HistoryRecord> = None;
    for i in 0..40 {
        let new: Vec<String> = (0..37).map(|n| format!("memory-{i}-{n}")).collect();
        for cmd in [
            Command::Layer(LayerCommand::Duplicate {
                layer_ids: ids.clone(),
                new_ids: new.clone(),
            }),
            Command::Transform(pm_kernel::commands::TransformCommand::Nudge {
                layer_ids: new.clone(),
                dx: 1.,
                dy: 0.,
                time: 0.,
            }),
            Command::Group(GroupCommand::Group {
                layer_ids: new.clone(),
                group_id: format!("memory-group-{i}"),
                name: None,
                time: 0.,
            }),
            Command::Group(GroupCommand::Ungroup {
                group_ids: vec![format!("memory-group-{i}")],
                time: 0.,
            }),
            Command::Layer(LayerCommand::Delete { layer_ids: new }),
        ] {
            store.execute(cmd, Origin::Mcp).unwrap();
            if let Some(record) = store.history().records().last() {
                if peak_record.as_ref().is_none_or(|p| record.bytes > p.bytes) {
                    println!(
                        "peak_latest_record label={:?} accounted_bytes={} revision={}",
                        record.label, record.bytes, record.revision
                    );
                    peak_record = Some(record.clone());
                }
            }
        }
        if i % 10 == 9 {
            println!(
                "edits={} bytes={} history={:?}",
                (i + 1) * 5,
                live().0 - baseline.0,
                store.history().stats()
            );
        }
    }
    let history = store.history().clone();
    let all = live().0;
    drop(store);
    let without_store = live().0;
    drop(history);
    let without_history = live().0;
    drop(peak_record);
    let without_peak = live().0;
    drop(held);
    println!(
        "owners store_document_queries_log={} history_except_peak={} peak_record_retained={} legacy_log={} remaining={}",
        all - without_store,
        without_store - without_history,
        without_history - without_peak,
        without_peak - live().0,
        live().0 - baseline.0
    );
}
