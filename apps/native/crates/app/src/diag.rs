//! Opt-in main-thread diagnostics (`PM_NATIVE_MAIN_TRACE=1`). Off by default:
//! no timer, probe or output in normal runs. Lines go to stderr as
//! `PM_MAIN {json}` so stdout stays reserved for MCP and the perf gate.
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("PM_NATIVE_MAIN_TRACE").as_deref() == Ok("1"))
}

pub fn line(value: serde_json::Value) {
    if enabled() {
        eprintln!("PM_MAIN {value}");
    }
}

/// Times a main-thread span; reports it when tracing is on.
pub struct Span {
    label: &'static str,
    start: Option<Instant>,
}
pub fn span(label: &'static str) -> Span {
    Span {
        label,
        start: enabled().then(Instant::now),
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            let ms = start.elapsed().as_secs_f64() * 1000.;
            if ms >= 1. {
                line(serde_json::json!({"span":self.label,"ms":ms}));
            }
        }
    }
}

/// Main-thread responsiveness: a foreground task asks for a 5 ms timer and
/// reports how late it resumes. A late resume is time the UI thread could
/// not paint or handle input.
pub fn install_probe(cx: &mut gpui::App) {
    if !enabled() {
        return;
    }
    let timer = cx.background_executor().clone();
    cx.spawn(async move |_| {
        let mut worst = 0f64;
        let mut reported = Instant::now();
        loop {
            let asked = Instant::now();
            timer.timer(Duration::from_millis(5)).await;
            let late = (asked.elapsed().as_secs_f64() - 0.005).max(0.) * 1000.;
            worst = worst.max(late);
            if late >= 50. {
                line(serde_json::json!({"stall_ms":late}));
            }
            if reported.elapsed() >= Duration::from_secs(1) {
                line(serde_json::json!({"probe_worst_ms":worst}));
                worst = 0.;
                reported = Instant::now();
            }
        }
    })
    .detach();
}
