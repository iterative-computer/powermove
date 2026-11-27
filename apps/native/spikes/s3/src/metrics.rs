use crate::policy::percentile;
use serde_json::{Value, json};
pub fn distribution(v: &[f64]) -> Value {
    if v.is_empty() {
        return json!({"samples":0,"p50":null,"p95":null,"max":null});
    }
    json!({"samples":v.len(),"p50":percentile(v,0.5),"p95":percentile(v,0.95),"max":percentile(v,1.)})
}
fn n(row: &Value, key: &str) -> f64 {
    row[key].as_f64().unwrap_or(0.)
}
fn delta(rows: &[&Value], a: &str, b: &str) -> Value {
    distribution(
        &rows
            .iter()
            .filter_map(|r| {
                let (a, b) = (n(r, a), n(r, b));
                (a > 0. && b > 0. && a >= b).then_some((a - b) * 1000.)
            })
            .collect::<Vec<_>>(),
    )
}
pub fn report(rows: &[Value], interval: f64, mode: &str) -> Value {
    let measured: Vec<_> = rows.iter().filter(|r| r["measured"] == true).collect();
    let scrub: Vec<_> = measured
        .iter()
        .copied()
        .filter(|r| r["scrub"] == true)
        .collect();
    let cached: Vec<_> = scrub
        .iter()
        .copied()
        .filter(|r| r["cache_hit"] == true)
        .collect();
    let mut scanout: Vec<_> = measured
        .iter()
        .filter_map(|r| {
            let t = n(r, "display");
            (t > 0.).then_some(t)
        })
        .collect();
    scanout.sort_by(f64::total_cmp);
    let gaps: Vec<_> = scanout.windows(2).map(|w| (w[1] - w[0]) * 1000.).collect();
    let stages = |r: &[&Value]| {
        json!({
        "input_to_request_ms":delta(r,"request","input"),
        "request_to_worker_ms":delta(r,"worker_start","request"),
        "producer_cpu_encode_ms":delta(r,"render_submitted","render_start"),
        "producer_sync_wait_ms":delta(r,"producer_wait_end","render_submitted"),
        "drawable_wait_ms":delta(r,"drawable_acquired","drawable_asked"),
        "worker_to_commit_ms":delta(r,"commit","worker_start"),
        "commit_to_scheduled_callback_ms":delta(r,"scheduled","commit"),
        "consumer_gpu_execution_ms":delta(r,"consumer_gpu_done","consumer_gpu_start"),
        "worker_to_gpu_done_ms":delta(r,"consumer_gpu_done","worker_start"),
        "worker_to_completion_callback_ms":delta(r,"completed_callback","worker_start"),
        "target_slack_at_commit_ms":delta(r,"target","commit"),
        "input_to_gpu_done_ms":delta(r,"consumer_gpu_done","input"),
        "gpu_done_to_display_ms":delta(r,"display","consumer_gpu_done"),
        "input_to_display_ms":delta(r,"display","input"),
        "display_callback_delay_ms":delta(r,"display_callback","display")})
    };
    json!({"mode":mode,"clock":if mode=="offscreen" {"synthetic monotonic timer; no display"} else if mode=="hidden" {"synthetic monotonic timer; hidden CAMetalLayer"} else {"NSView CADisplayLink"},
        "measured_frames":measured.len(),"presented_callbacks":measured.iter().filter(|r|!r["display"].is_null()).count(),
        "not_presented":measured.iter().filter(|r|r["display"]==0.).count(),
        "missing_presented_callbacks":measured.iter().filter(|r|r["display"].is_null()).count(),
        "gpu_failures":measured.iter().filter(|r|r["gpu_ok"]!=true).count(),
        "scanout_gaps_over_1_5_intervals":if gaps.is_empty() {Value::Null} else {json!(gaps.iter().filter(|g|**g>interval*1500.).count())},
        "scanout_gap_ms":distribution(&gaps),"stages":stages(&measured),"scrub":stages(&scrub),"cached_scrub":stages(&cached),
        "scrub_samples":scrub.len(),"cached_scrub_samples":cached.len(),
        "headroom_budget_ms":interval*500.,"cached_scrub_budget_ms":interval*1000.+1.,
        "build_profile":if cfg!(debug_assertions) {"dev"} else {"release"},
        "physical_gate":"UNVERIFIED: require foreground continuity, input and paired Electron runs",
        "scope":"one public shape fixture at 1280x720; see build_profile; GPU time is not physical display latency; producer marker is a readiness upper bound; synthetic requests are not AppKit input"})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_display_never_turns_gpu_completion_into_a_latency_pass() {
        let rows = [
            json!({"measured":true,"scrub":true,"cache_hit":true,"input":1.,
            "consumer_gpu_start":1.001,"consumer_gpu_done":1.002,"display":null,"gpu_ok":true}),
        ];
        let r = report(&rows, 1. / 120., "offscreen");
        assert!(r["cached_scrub"]["input_to_display_ms"]["p95"].is_null());
        assert_eq!(r["cached_scrub"]["input_to_gpu_done_ms"]["samples"], 1);
        assert_eq!(r["missing_presented_callbacks"], 1);
    }
    #[test]
    fn zero_presentations_and_out_of_order_callbacks_do_not_hide_gaps() {
        let rows: Vec<_> = [1., 1.025, 1.008333, 0.]
            .iter()
            .map(|t| json!({"measured":true,"display":t}))
            .collect();
        let r = report(&rows, 1. / 120., "hidden");
        assert_eq!(r["not_presented"], 1);
        assert_eq!(r["scanout_gaps_over_1_5_intervals"], 1);
    }
}
