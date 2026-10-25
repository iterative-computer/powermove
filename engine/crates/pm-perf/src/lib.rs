//! Deterministic work counters (docs/engine/perfbench.md).
//!
//! Wall-clock time on a shared machine is noisy; the work a frame does is
//! not. Every engine crate, and the vendored wgpu front crate
//! (`engine/third_party/wgpu`), adds to the counters here as it works: render
//! passes, draw calls, pipeline and bind group creations, GPU buffer and
//! texture allocations, uploads and readbacks, shader compiles, cache hits
//! and misses, and (when a binary installs [`CountingAlloc`]) heap
//! allocations. A bench reads a [`Counters`] snapshot before and after the
//! work and keeps the difference ([`Scope`]).
//!
//! The counters are per thread: a bench measures the thread that does the
//! work (the engine's render thread, an export's thread) and tests that run
//! in parallel never see each other's counts. Each add is a thread-local
//! cell increment (no lock, no allocation) plus one relaxed load of a flag
//! that is false unless a bench turned the process-wide totals on
//! ([`count_globally`]); only then does an add also do a relaxed atomic
//! add to a total shared by every thread. So the counters are always on, in
//! release builds too, and the shipped app (which never turns the totals
//! on) touches no shared cache line per add. The `noop` feature compiles
//! every add out: it exists only to measure the counters' own cost
//! (docs/engine/perfbench.md, "Overhead"); [`ENABLED`] tells a ratchet it
//! is off.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

macro_rules! counters {
    ($($(#[$doc:meta])* $variant:ident = $name:literal,)*) => {
        /// One counter. [`Counter::name`] is its stable name in reports and
        /// ratchet baselines.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(usize)]
        pub enum Counter {
            $($(#[$doc])* $variant,)*
        }

        impl Counter {
            /// Every counter, in catalog order.
            pub const ALL: &'static [Counter] = &[$(Counter::$variant,)*];

            /// The counter's stable name.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Counter::$variant => $name,)*
                }
            }

            /// Whether more is better (a cache hit): reported, never an
            /// upper bound.
            pub fn is_benefit(self) -> bool {
                self.name().ends_with(".hits")
            }

            /// The counter with this name.
            pub fn from_name(name: &str) -> Option<Counter> {
                match name {
                    $($name => Some(Counter::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

counters! {
    // GPU work (counted by the vendored wgpu front crate).
    /// Render passes begun.
    RenderPasses = "gpu.render_passes",
    /// Compute passes begun.
    ComputePasses = "gpu.compute_passes",
    /// Draw calls (`draw`, `draw_indexed`, indirect draws), in passes and bundles.
    DrawCalls = "gpu.draw_calls",
    /// Compute dispatches.
    Dispatches = "gpu.dispatches",
    /// `set_pipeline` calls in passes.
    PipelineBinds = "gpu.pipeline_binds",
    /// `set_bind_group` calls in passes.
    BindGroupBinds = "gpu.bind_group_binds",
    /// Render and compute pipelines created.
    PipelineCreations = "gpu.pipeline_creations",
    /// Pipeline layouts created.
    PipelineLayoutCreations = "gpu.pipeline_layout_creations",
    /// Bind groups created.
    BindGroupCreations = "gpu.bind_group_creations",
    /// Bind group layouts created.
    BindGroupLayoutCreations = "gpu.bind_group_layout_creations",
    /// Shader modules created (each is a backend shader compile).
    ShaderModules = "gpu.shader_modules",
    /// Samplers created.
    SamplerCreations = "gpu.sampler_creations",
    /// GPU buffers allocated.
    BufferAllocs = "gpu.buffer_allocs",
    /// Bytes of GPU buffers allocated.
    BufferAllocBytes = "gpu.buffer_alloc_bytes",
    /// GPU textures allocated.
    TextureAllocs = "gpu.texture_allocs",
    /// Bytes of GPU textures allocated (all mips and layers, times the sample count).
    TextureAllocBytes = "gpu.texture_alloc_bytes",
    /// Texture views created.
    TextureViews = "gpu.texture_views",
    /// Command encoders created.
    CommandEncoders = "gpu.command_encoders",
    /// Queue submissions.
    Submits = "gpu.submits",
    /// CPU to GPU uploads: `write_buffer`, `write_texture`, buffers created
    /// with contents, and buffers mapped for writing.
    Uploads = "gpu.uploads",
    /// Bytes uploaded.
    UploadBytes = "gpu.upload_bytes",
    /// GPU to CPU readbacks: buffers mapped for reading.
    Readbacks = "gpu.readbacks",
    /// Bytes mapped for reading.
    ReadbackBytes = "gpu.readback_bytes",
    /// Copies recorded on the GPU (buffer and texture, either way).
    GpuCopies = "gpu.copies",
    /// Bytes of texture-to-buffer copies recorded (the readbacks' GPU side).
    TextureToBufferBytes = "gpu.texture_to_buffer_bytes",

    // Heap (counted when the binary installs `CountingAlloc`).
    /// Heap allocations (`alloc`, `alloc_zeroed`, and growing `realloc`s).
    HeapAllocs = "cpu.heap_allocs",
    /// Bytes requested by those allocations (a `realloc` counts its new size).
    HeapAllocBytes = "cpu.heap_alloc_bytes",

    // Engine work.
    /// GLSL programs compiled to naga IR (glslang and the rewrites).
    GlslCompiles = "engine.glsl_compiles",
    /// Layer rasters the engine asked another process for (fallback runner
    /// pixels).
    RasterRequests = "engine.raster_requests",
    /// Of those, answered.
    RasterAnswers = "engine.raster_answers",
    /// Media files' size and modification time read from the file system
    /// (a `stat` each).
    FileStats = "engine.file_stats",
    /// Layers planned (drawn from scratch) in a composition pass.
    LayersPlanned = "engine.layers_planned",
    /// Compositions evaluated and planned (passes built).
    CompPasses = "engine.comp_passes",

    // Caches: one hit and one miss counter each.
    /// Render pipelines found in the renderer's pipeline caches.
    PipelineCacheHits = "cache.pipeline.hits",
    PipelineCacheMisses = "cache.pipeline.misses",
    /// Effect and shader-layer programs found compiled.
    ProgramCacheHits = "cache.program.hits",
    ProgramCacheMisses = "cache.program.misses",
    /// Media textures (stills, decoded video frames) found uploaded.
    MediaCacheHits = "cache.media.hits",
    MediaCacheMisses = "cache.media.misses",
    /// Vector (shape, mask, path, SVG) textures found.
    VectorCacheHits = "cache.vector.hits",
    VectorCacheMisses = "cache.vector.misses",
    /// Text runs found rendered.
    TextCacheHits = "cache.text.hits",
    TextCacheMisses = "cache.text.misses",
    /// Layer draws given back by the paused-frame draw reuse.
    DrawReuseHits = "cache.draw_reuse.hits",
    DrawReuseMisses = "cache.draw_reuse.misses",
    /// Viewer frames drawn from a kept bottom of the last frame.
    FramePrefixHits = "cache.frame_prefix.hits",
    FramePrefixMisses = "cache.frame_prefix.misses",
    /// Bind groups kept across frames, given back / made.
    BindCacheHits = "cache.bind_group.hits",
    BindCacheMisses = "cache.bind_group.misses",
    /// Pooled render targets reused / allocated.
    TargetPoolHits = "cache.target_pool.hits",
    TargetPoolMisses = "cache.target_pool.misses",
    /// SVG source rasters reused across output translations and rotations.
    SvgRasterHits = "cache.svg_raster.hits",
    SvgRasterMisses = "cache.svg_raster.misses",
}

/// The number of counters.
pub const COUNT: usize = Counter::ALL.len();

thread_local! {
    static COUNTS: [Cell<u64>; COUNT] = const { [const { Cell::new(0) }; COUNT] };
}

/// Every thread's counts together (work spread over threads: an export's
/// encoder and pool threads, decode threads), while [`count_globally`] is on.
static GLOBAL: [AtomicU64; COUNT] = [const { AtomicU64::new(0) }; COUNT];

/// Whether adds also go to [`GLOBAL`] (benches turn it on; the app never does).
static GLOBAL_ON: AtomicBool = AtomicBool::new(false);

/// Whether this build counts at all (false with the measurement-only `noop`
/// feature).
pub const ENABLED: bool = !cfg!(feature = "noop");

/// Turns the process-wide totals ([`snapshot_global`], [`Scope::finish_global`])
/// on or off. Off by default: then an add touches only its thread's cells.
/// Benches turn it on before they measure; counts made while it was off are
/// not in the totals.
pub fn count_globally(on: bool) {
    GLOBAL_ON.store(on, Ordering::Relaxed);
}

/// Whether the process-wide totals are being counted.
pub fn counting_globally() -> bool {
    GLOBAL_ON.load(Ordering::Relaxed)
}

/// Adds `n` to `counter` on this thread (and to the process's total while
/// [`count_globally`] is on).
#[inline]
pub fn add(counter: Counter, n: u64) {
    if !ENABLED {
        return;
    }
    // `try_with`: a thread being torn down (and the allocator running in
    // its destructors) just doesn't count.
    let _ = COUNTS.try_with(|c| {
        let cell = &c[counter as usize];
        cell.set(cell.get().wrapping_add(n));
    });
    if GLOBAL_ON.load(Ordering::Relaxed) {
        GLOBAL[counter as usize].fetch_add(n, Ordering::Relaxed);
    }
}

/// Adds one to `counter` on this thread.
#[inline]
pub fn bump(counter: Counter) {
    add(counter, 1);
}

/// Counts a cache lookup: `hit` adds to `hits`, else to `misses`.
#[inline]
pub fn cache(hits: Counter, misses: Counter, hit: bool) {
    bump(if hit { hits } else { misses });
}

/// This thread's counters since it started.
pub fn snapshot() -> Counters {
    let mut v = [0; COUNT];
    let _ = COUNTS.try_with(|c| {
        for (out, cell) in v.iter_mut().zip(c.iter()) {
            *out = cell.get();
        }
    });
    Counters { v }
}

/// Every thread's counters together since the process started.
pub fn snapshot_global() -> Counters {
    let mut v = [0; COUNT];
    for (out, a) in v.iter_mut().zip(GLOBAL.iter()) {
        *out = a.load(Ordering::Relaxed);
    }
    Counters { v }
}

/// A set of counter values: a snapshot, or the difference of two.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Counters {
    v: [u64; COUNT],
}

impl Default for Counters {
    fn default() -> Self {
        Counters { v: [0; COUNT] }
    }
}

impl Counters {
    pub fn get(&self, counter: Counter) -> u64 {
        self.v[counter as usize]
    }

    pub fn set(&mut self, counter: Counter, value: u64) {
        self.v[counter as usize] = value;
    }

    /// `self - earlier`, counter by counter (saturating: a counter never
    /// goes down on one thread).
    pub fn since(&self, earlier: &Counters) -> Counters {
        let mut v = [0; COUNT];
        for (i, out) in v.iter_mut().enumerate() {
            *out = self.v[i].saturating_sub(earlier.v[i]);
        }
        Counters { v }
    }

    /// Adds `other` counter by counter.
    pub fn accumulate(&mut self, other: &Counters) {
        for (a, b) in self.v.iter_mut().zip(other.v.iter()) {
            *a = a.wrapping_add(*b);
        }
    }

    /// Every counter and its value, in catalog order.
    pub fn iter(&self) -> impl Iterator<Item = (Counter, u64)> + '_ {
        Counter::ALL.iter().map(|&c| (c, self.v[c as usize]))
    }

    /// The counters that are not zero.
    pub fn nonzero(&self) -> impl Iterator<Item = (Counter, u64)> + '_ {
        self.iter().filter(|&(_, v)| v != 0)
    }

    /// A JSON object of the counters that are not zero, by name, in catalog
    /// order (no dependencies: names are fixed ASCII).
    pub fn to_json(&self) -> String {
        let mut s = String::from("{");
        for (i, (c, v)) in self.nonzero().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push('"');
            s.push_str(c.name());
            s.push_str("\":");
            s.push_str(&v.to_string());
        }
        s.push('}');
        s
    }
}

impl std::fmt::Debug for Counters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.nonzero().map(|(c, v)| (c.name(), v)))
            .finish()
    }
}

/// Counts the work between its creation and [`Scope::finish`]: on this
/// thread, and on every thread ([`Scope::finish_global`]).
pub struct Scope {
    start: Counters,
    start_global: Counters,
}

impl Scope {
    pub fn begin() -> Scope {
        Scope {
            start: snapshot(),
            start_global: snapshot_global(),
        }
    }

    /// This thread's counts since [`Scope::begin`].
    pub fn finish(&self) -> Counters {
        snapshot().since(&self.start)
    }

    /// Every thread's counts since [`Scope::begin`].
    pub fn finish_global(&self) -> Counters {
        snapshot_global().since(&self.start_global)
    }
}

/// A global allocator that counts this thread's heap allocations
/// ([`Counter::HeapAllocs`], [`Counter::HeapAllocBytes`]) and otherwise is
/// the system allocator. Bench and test binaries install it:
///
/// ```ignore
/// #[global_allocator]
/// static ALLOC: pm_perf::CountingAlloc = pm_perf::CountingAlloc;
/// ```
///
/// [`alloc_counting_installed`] tells a report whether the heap counters
/// are real.
pub struct CountingAlloc;

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
}

static ALLOC_INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a [`CountingAlloc`] has allocated in this process (so the heap
/// counters are counted, not just zero).
pub fn alloc_counting_installed() -> bool {
    // Allocating once makes sure an installed allocator has run.
    drop(std::hint::black_box(Box::new(0u8)));
    ALLOC_INSTALLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Counts one heap allocation of `bytes` on this thread: what
/// [`CountingAlloc`] does, for allocators that wrap their own (profilers).
#[inline]
pub fn count_alloc(bytes: usize) {
    add(Counter::HeapAllocs, 1);
    add(Counter::HeapAllocBytes, bytes as u64);
    let _ = INSTALLED.try_with(|i| {
        if !i.get() {
            i.set(true);
            ALLOC_INSTALLED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
}

// SAFETY: every call is forwarded to `System` unchanged; counting touches
// only const-initialized thread-local cells without destructors (no
// allocation, no reentrancy).
unsafe impl GlobalAlloc for CountingAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_alloc(layout.size());
        unsafe { System.alloc(layout) }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_alloc(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() {
            count_alloc(new_size);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[cfg(all(test, not(feature = "noop")))]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for &c in Counter::ALL {
            assert!(seen.insert(c.name()), "duplicate name {}", c.name());
            assert_eq!(Counter::from_name(c.name()), Some(c));
        }
        assert_eq!(Counter::from_name("nope"), None);
    }

    #[test]
    fn scopes_count_this_thread_only() {
        count_globally(true);
        let scope = Scope::begin();
        bump(Counter::DrawCalls);
        add(Counter::UploadBytes, 40);
        cache(Counter::MediaCacheHits, Counter::MediaCacheMisses, false);
        std::thread::spawn(|| add(Counter::DrawCalls, 1000))
            .join()
            .unwrap();
        assert!(scope.finish_global().get(Counter::DrawCalls) >= 1001);
        let c = scope.finish();
        assert_eq!(c.get(Counter::DrawCalls), 1);
        assert_eq!(c.get(Counter::UploadBytes), 40);
        assert_eq!(c.get(Counter::MediaCacheMisses), 1);
        assert_eq!(c.get(Counter::MediaCacheHits), 0);
        assert_eq!(
            c.to_json(),
            r#"{"gpu.draw_calls":1,"gpu.upload_bytes":40,"cache.media.misses":1}"#
        );
    }

    #[test]
    fn since_and_accumulate() {
        let mut a = Counters::default();
        a.set(Counter::Submits, 5);
        let mut b = Counters::default();
        b.set(Counter::Submits, 7);
        assert_eq!(b.since(&a).get(Counter::Submits), 2);
        assert_eq!(a.since(&b).get(Counter::Submits), 0);
        a.accumulate(&b);
        assert_eq!(a.get(Counter::Submits), 12);
    }
}
