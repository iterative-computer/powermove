//! Cache accounting uses nominal bytes; the OS footprint includes driver and
//! allocator storage as well. Never substitute RSS for Activity Monitor Memory.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CacheUsage {
    pub target_pool_bytes: u64,
    pub target_pool_idle: usize,
    pub targets_created: u64,
    pub image_bytes: u64,
    pub converted_frame_bytes: u64,
    pub decoded_frame_bytes: u64,
    pub decoder_bytes: u64,
    pub decoder_sessions: usize,
    pub decoder_pool_free_bytes: u64,
    pub disk_read_ahead_bytes: u64,
    pub surface_bytes: u64,
    pub text_bytes: u64,
    pub vector_bytes: u64,
    pub pyramid_bytes: u64,
    pub geometry_bytes: u64,
    /// Gross resources held by cached bindings; can overlap other caches.
    pub binding_bytes: u64,
    /// Metal-device allocation, including work in flight; overlaps caches.
    pub gpu_allocated_bytes: u64,
    /// PM_SCRUB_EXTRA_MIB: the scrub allowance currently borrowed on top of
    /// the grant (0 outside drags).
    pub scrub_extra_bytes: u64,
    /// Times a closed document's caches, programs and override answers were
    /// released since launch. Showing another composition of an open
    /// document never adds to it.
    pub project_releases: u64,
}

impl CacheUsage {
    /// Resident cache categories, without counting the decoder pool twice.
    pub fn resident_bytes(&self) -> u64 {
        self.target_pool_bytes
            + self.image_bytes
            + self.converted_frame_bytes
            + self.decoded_frame_bytes
            + self.decoder_bytes
            + self.disk_read_ahead_bytes
            + self.surface_bytes
            + self.text_bytes
            + self.vector_bytes
            + self.pyramid_bytes
            + self.geometry_bytes
    }
}

/// TASK_VM_INFO's stable prefix through phys_footprint (revision 1).
#[cfg(target_os = "macos")]
pub fn phys_footprint() -> Option<u64> {
    unsafe extern "C" {
        static mach_task_self_: u32;
        fn task_info(task: u32, flavor: u32, info: *mut u32, count: *mut u32) -> i32;
    }
    let mut info = [0u64; 64];
    let mut count = (std::mem::size_of_val(&info) / 4) as u32;
    // TASK_VM_INFO = 22. phys_footprint is at byte 144, after compressed_lifetime.
    let result = unsafe { task_info(mach_task_self_, 22, info.as_mut_ptr().cast(), &mut count) };
    (result == 0 && count >= 38).then_some(info[18])
}

#[cfg(not(target_os = "macos"))]
pub fn phys_footprint() -> Option<u64> {
    None
}

pub use pm_memory::{
    Pressure, Registration, budget, bytes, pressure, pressure_relief, set_allocator_relief,
    simulate_pressure, usage,
};
