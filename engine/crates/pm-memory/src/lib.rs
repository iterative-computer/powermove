//! Process-wide cache leases. Owners report the protocol's categories and
//! register a nonblocking eviction request; only the owner evicts its cache.
use pm_protocol::{common::ByteSize, lease::LeaseCategories};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

const MIB: u64 = 1 << 20;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pressure {
    Normal,
    Warning,
    Critical,
}
type Evict = Arc<dyn Fn(u64, Pressure) + Send + Sync>;
struct Client {
    weight: u64,
    grant: Arc<AtomicU64>,
    usage: LeaseCategories,
    evict: Evict,
}
struct Governor {
    next: u64,
    normal: u64,
    budget: u64,
    pressure: Pressure,
    epoch: u64,
    clients: BTreeMap<u64, Client>,
}
impl Governor {
    fn new(normal: u64) -> Self {
        Self {
            next: 1,
            normal,
            budget: normal,
            pressure: Pressure::Normal,
            epoch: 1,
            clients: BTreeMap::new(),
        }
    }
    fn rebalance(&mut self) -> Vec<(Evict, u64, Pressure)> {
        self.epoch += 1;
        let weights = self.clients.values().map(|c| c.weight).sum::<u64>().max(1);
        self.clients
            .values()
            .map(|c| {
                let share = self.budget * c.weight / weights;
                c.grant.store(share, Ordering::Release);
                (c.evict.clone(), share, self.pressure)
            })
            .collect()
    }
    fn pressure(&mut self, pressure: Pressure) -> Vec<(Evict, u64, Pressure)> {
        self.pressure = pressure;
        self.budget = match pressure {
            Pressure::Normal => self.normal,
            Pressure::Warning => self.normal / 2,
            Pressure::Critical => (32 * MIB).min(self.normal),
        };
        self.rebalance()
    }
}
fn global() -> &'static Mutex<Governor> {
    static STATE: OnceLock<Mutex<Governor>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(Governor::new(default_budget())))
}
fn notify(requests: Vec<(Evict, u64, Pressure)>) {
    for (evict, share, level) in requests {
        evict(share, level);
    }
}

/// Live allocations are never forcibly destroyed. Reports may exceed a grant
/// while a frame is in flight or an isolate has live user objects.
pub struct Registration {
    id: u64,
    grant: Arc<AtomicU64>,
}
impl Registration {
    pub fn new(weight: u64, evict: impl Fn(u64, Pressure) + Send + Sync + 'static) -> Self {
        start_pressure_source();
        let grant = Arc::new(AtomicU64::new(0));
        let (id, requests) = {
            let mut g = global().lock().unwrap();
            let id = g.next;
            g.next += 1;
            g.clients.insert(
                id,
                Client {
                    weight: weight.max(1),
                    grant: grant.clone(),
                    usage: Default::default(),
                    evict: Arc::new(evict),
                },
            );
            (id, g.rebalance())
        };
        notify(requests);
        Self { id, grant }
    }
    pub fn grant(&self) -> u64 {
        self.grant.load(Ordering::Acquire)
    }
    pub fn report(&self, usage: LeaseCategories) {
        if let Some(c) = global().lock().unwrap().clients.get_mut(&self.id) {
            c.usage = usage;
        }
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        let requests = {
            let mut g = global().lock().unwrap();
            g.clients.remove(&self.id);
            g.rebalance()
        };
        notify(requests);
    }
}
pub fn bytes(value: u64) -> ByteSize {
    ByteSize::new(value.min((1u64 << 53) - 1)).expect("safe byte count")
}
pub fn other(value: u64) -> LeaseCategories {
    LeaseCategories {
        other: bytes(value),
        ..Default::default()
    }
}
/// Each client's weight, current grant and reported total (diagnostics).
pub fn clients() -> Vec<(u64, u64, u64)> {
    global()
        .lock()
        .unwrap()
        .clients
        .values()
        .map(|c| {
            (
                c.weight,
                c.grant.load(Ordering::Acquire),
                c.usage.total().map_or(u64::MAX, |b| b.get()),
            )
        })
        .collect()
}
pub fn budget() -> u64 {
    global().lock().unwrap().budget
}
pub fn pressure() -> Pressure {
    global().lock().unwrap().pressure
}
/// Also used by headless acceptance tests; does not create OS-wide pressure.
pub fn simulate_pressure(level: Pressure) {
    let requests = global().lock().unwrap().pressure(level);
    notify(requests);
    if level == Pressure::Critical {
        pressure_relief();
    }
}
pub fn usage() -> LeaseCategories {
    let g = global().lock().unwrap();
    let mut out = LeaseCategories::default();
    for c in g.clients.values() {
        macro_rules! add { ($($field:ident),*) => { $(out.$field = bytes(out.$field.get().saturating_add(c.usage.$field.get()));)* }; }
        add!(
            frame_cache,
            render_cache,
            blender_surfaces,
            geometry_3d,
            surface_pool,
            decoders,
            pipelines,
            other
        );
    }
    out
}
fn default_budget() -> u64 {
    let mut ram = 0u64;
    #[cfg(target_os = "macos")]
    unsafe {
        let mut size = std::mem::size_of::<u64>();
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut ram as *mut u64).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        );
    }
    // Leave the rest of a 500 MiB footprint for the document, UI, live JS,
    // driver and in-flight work. Larger machines can cache more.
    160 * MIB * (ram / (32u64 << 30)).clamp(1, 2)
}
static ALLOCATOR_RELIEF: OnceLock<fn()> = OnceLock::new();

/// A binary using a separate allocator can collect its freed pages along
/// with the system allocator. The callback must be thread safe and reentrant.
pub fn set_allocator_relief(collect: fn()) {
    let _ = ALLOCATOR_RELIEF.set(collect);
}

pub fn pressure_relief() {
    if let Some(collect) = ALLOCATOR_RELIEF.get() {
        collect();
    }
    #[cfg(target_os = "macos")]
    unsafe {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
}

#[cfg(target_os = "macos")]
fn start_pressure_source() {
    static SOURCE: OnceLock<usize> = OnceLock::new();
    SOURCE.get_or_init(|| unsafe {
        use std::ffi::c_void;
        unsafe extern "C" {
            static _dispatch_source_type_memorypressure: u8;
            fn dispatch_get_global_queue(priority: isize, flags: usize) -> *mut c_void;
            fn dispatch_source_create(
                kind: *const u8,
                handle: usize,
                mask: usize,
                queue: *mut c_void,
            ) -> *mut c_void;
            fn dispatch_set_context(object: *mut c_void, context: *mut c_void);
            fn dispatch_source_set_event_handler_f(
                source: *mut c_void,
                handler: extern "C" fn(*mut c_void),
            );
            fn dispatch_source_get_data(source: *mut c_void) -> usize;
            fn dispatch_resume(object: *mut c_void);
        }
        extern "C" fn event(source: *mut c_void) {
            let flags = unsafe { dispatch_source_get_data(source) };
            simulate_pressure(if flags & 4 != 0 {
                Pressure::Critical
            } else if flags & 2 != 0 {
                Pressure::Warning
            } else {
                Pressure::Normal
            });
        }
        // Process lifetime: no application, window or project is retained.
        let source = dispatch_source_create(
            &raw const _dispatch_source_type_memorypressure,
            0,
            7,
            dispatch_get_global_queue(-2, 0),
        );
        if !source.is_null() {
            dispatch_set_context(source, source);
            dispatch_source_set_event_handler_f(source, event);
            dispatch_resume(source);
        }
        source as usize
    });
}
#[cfg(not(target_os = "macos"))]
fn start_pressure_source() {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leases_share_one_budget_and_pressure_has_a_floor() {
        let mut g = Governor::new(160 * MIB);
        for id in 1..=3 {
            g.clients.insert(
                id,
                Client {
                    weight: id,
                    grant: Arc::default(),
                    usage: Default::default(),
                    evict: Arc::new(|_, _| {}),
                },
            );
        }
        for (pressure, budget) in [
            (Pressure::Normal, 160 * MIB),
            (Pressure::Warning, 80 * MIB),
            (Pressure::Critical, 32 * MIB),
        ] {
            let requests = g.pressure(pressure);
            assert_eq!(g.budget, budget);
            assert!(requests.iter().map(|(_, b, _)| b).sum::<u64>() <= budget);
        }
    }
}
