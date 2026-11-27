//! All times are monotonic host seconds, matching NSEvent and presentedTime.
//! A missing physical presentation stays null; GPU completion is never display.
use serde_json::{Value, json};
#[derive(Default, Debug)]
pub struct Trace {
    pub id: u64,
    pub measured: bool,
    pub scrub: bool,
    pub input: f64,
    pub request: f64,
    pub worker_start: f64,
    pub link_callback: f64,
    pub target: f64,
    pub render_start: f64,
    pub render_submitted: f64,
    pub producer_wait_end: f64,
    pub producer_ready: f64,
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
impl Trace {
    pub fn json(&self) -> Value {
        json!({"id":self.id,"measured":self.measured,"scrub":self.scrub,
            "input":self.input,"request":self.request,"worker_start":self.worker_start,
            "link_callback":self.link_callback,"target":self.target,
            "render_start":self.render_start,"render_submitted":self.render_submitted,
            "producer_wait_end":self.producer_wait_end,"producer_ready_upper_bound":self.producer_ready,
            "drawable_asked":self.drawable_asked,"drawable_acquired":self.drawable_acquired,
            "encode_end":self.encode_end,"present_requested":self.present_requested,"commit":self.commit,"scheduled":self.scheduled,
            "consumer_gpu_start":self.gpu_start,"consumer_gpu_done":self.gpu_done,
            "completed_callback":self.completed,"display":self.display,
            "display_callback":self.display_callback,"cache_hit":self.cache_hit,"gpu_ok":self.gpu_ok})
    }
}
