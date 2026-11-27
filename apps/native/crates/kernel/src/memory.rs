//! Return unused allocator pages after large document lifecycle operations.
//! This does not change any live allocation or the document/history format.
pub fn pressure_relief() {
    #[cfg(target_os = "macos")]
    unsafe {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
}
