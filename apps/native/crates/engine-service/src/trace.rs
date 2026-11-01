//! Monotonic host seconds, matching NSEvent and drawable presentedTime.
//! Zero CPU/GPU fields mean unobserved; physical display stays Option.
#[derive(Default, Debug)]
pub struct Trace {
    pub worker_start: f64,
    pub render_start: f64,
    pub render_submitted: f64,
    pub drawable_asked: f64,
    pub drawable_acquired: f64,
    pub encode_end: f64,
    pub present_requested: f64,
    pub commit: f64,
    pub scheduled: f64,
    pub gpu_start: f64,
    pub gpu_done: f64,
    pub completed: f64,
    pub display: Option<f64>,
    pub display_callback: f64,
    pub cache_hit: bool,
    pub gpu_ok: bool,
}
pub fn now() -> f64 {
    objc2_quartz_core::CACurrentMediaTime()
}
