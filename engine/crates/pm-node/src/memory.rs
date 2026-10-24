//! Opt-in allocator attribution for macOS measurement runs.
//!
//! These are process-wide libmalloc zone statistics, including allocations
//! made by native libraries. They are neither Rust-only heap usage nor OS
//! residency, and do not include every direct VM or GPU allocation.

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MallocMemory {
    blocks_in_use: u32,
    size_in_use_bytes: usize,
    max_size_in_use_bytes: usize,
    reserved_bytes: usize,
}

pub(crate) fn sample() -> MallocMemory {
    // SAFETY: the public malloc API fills this correctly sized output. A null
    // zone requests the sum over all zones (malloc/malloc.h). Individual zones
    // may change between samples; reserved bytes are not resident bytes.
    let stats = unsafe {
        let mut stats = std::mem::zeroed::<libc::malloc_statistics_t>();
        libc::malloc_zone_statistics(std::ptr::null_mut(), &mut stats);
        stats
    };
    MallocMemory {
        blocks_in_use: stats.blocks_in_use,
        size_in_use_bytes: stats.size_in_use,
        max_size_in_use_bytes: stats.max_size_in_use,
        reserved_bytes: stats.size_allocated,
    }
}
