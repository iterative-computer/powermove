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
    pub surface_bytes: u64,
    pub text_bytes: u64,
    pub vector_bytes: u64,
    pub pyramid_bytes: u64,
    pub geometry_bytes: u64,
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
    Pressure, Registration, budget, bytes, pressure, pressure_relief, simulate_pressure, usage,
};
