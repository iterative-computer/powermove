//! Headless retained-allocation probe. Inputs must be disposable copies.
//! Usage: history_memory COPY_DIRECTORY COPIED_LIBRARY_ID
//! Directory: A.pmv and the four project*.ID.json library files. Creates B.json.
//! Set PM_HISTORY_TIMING=1 for timings without counting-allocator contention.
//! Payload deltas are measured by dropping imported history and then its raw
//! envelope. Totals include the document and permanent process caches, exclude
//! allocator overhead, and are not the app's RSS. Never run on a user library.
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
    if std::env::var_os("PM_HISTORY_TIMING").is_some() {
        ENABLED.store(false, Relaxed);
    }
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("directory of copies"));
    let id = std::env::args().nth(2).expect("copied library project id");
    if std::env::var_os("PM_HISTORY_LIBRARY_ONLY").is_some() {
        // Exercise the actual library entry path too, without an intermediate
        // file envelope or a configured writer/journal.
        let mut kernel = pm_kernel::Kernel::new(Default::default());
        let before = live();
        let start = std::time::Instant::now();
        let project = pm_kernel::library::LibraryStore::open(&dir)
            .load(&id)
            .unwrap()
            .unwrap();
        let info = kernel
            .open("probe", pm_kernel::OpenSource::Library(project))
            .unwrap();
        assert!(!info.history_rejected);
        let elapsed = start.elapsed();
        let all = live();
        // Clone shares all payloads. Clear the kernel's reference, then drop
        // the clone to isolate the resident history from the document.
        let history = kernel.project("probe").unwrap().store().history().clone();
        kernel
            .handle(
                "probe",
                pm_kernel::Request::new(pm_kernel::Caller::native(), pm_kernel::HistoryCall::Clear),
            )
            .unwrap();
        let held = live();
        drop(history);
        let cleared = live();
        println!(
            "B-library: open_ms={} retained_bytes={} allocations={} history_bytes={} history_allocations={}",
            elapsed.as_millis(),
            all.0 - before.0,
            all.1 - before.1,
            held.0 - cleared.0,
            held.1 - cleared.1
        );
        return;
    }
    // Use the production library codec, then the production file opener.
    let p = pm_kernel::library::LibraryStore::open(&dir)
        .load(&id)
        .unwrap()
        .unwrap();
    let mut envelope = pm_model::Map::new();
    envelope.insert("proj", p.project);
    envelope.insert("history", p.history.unwrap_or_default());
    envelope.insert("ws", p.workspace.unwrap_or_default());
    std::fs::write(
        dir.join("B.json"),
        pm_model::json::to_canonical_string(&pm_model::Value::Object(envelope)),
    )
    .unwrap();
    for name in ["A.pmv", "B.json"] {
        let before = live();
        let start = std::time::Instant::now();
        let mut opened =
            pm_kernel::project_io::open_project(&dir.join(name), Default::default()).unwrap();
        let elapsed = start.elapsed();
        let all = live();
        drop(opened.history.take());
        let no_history = live();
        drop(opened.file.envelope.history.take());
        let no_header = live();
        println!(
            "{name}: open_ms={} retained_bytes={} allocations={} history_bytes={} history_allocations={} header_bytes={} header_allocations={}",
            elapsed.as_millis(),
            all.0 - before.0,
            all.1 - before.1,
            all.0 - no_history.0,
            all.1 - no_history.1,
            no_history.0 - no_header.0,
            no_history.1 - no_header.1
        );
        drop(opened);
        let t = std::time::Instant::now();
        let mut store = pm_kernel::DocumentStore::open(
            pm_kernel::project_io::open_project(&dir.join(name), Default::default()).unwrap(),
        );
        let ready = t.elapsed();
        let t = std::time::Instant::now();
        let undo = store.undo();
        let undo_time = t.elapsed();
        let t = std::time::Instant::now();
        let redo = store.redo();
        let redo_time = t.elapsed();
        println!(
            "{name}: store_ready_ms={} undo_ms={} redo_ms={} undo_ok={} redo_ok={}",
            ready.as_millis(),
            undo_time.as_millis(),
            redo_time.as_millis(),
            undo.is_ok(),
            redo.is_ok()
        );
    }
}
