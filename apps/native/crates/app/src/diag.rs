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

/// The process clock the lines carry (`t`, ms since the first diagnostic
/// call, which `main` makes first).
fn start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

pub fn line(value: serde_json::Value) {
    if enabled() {
        let mut value = value;
        if let Some(map) = value.as_object_mut() {
            map.insert("t".into(), (start().elapsed().as_secs_f64() * 1000.).round().into());
        }
        eprintln!("PM_MAIN {value}");
    }
}

/// Starts the process clock (call first thing in `main`).
pub fn mark_start() {
    if enabled() {
        start();
    }
}

/// Milestones of one open (`{"open": stage, "ms": since the open began}`).
#[derive(Default)]
pub struct OpenTimer(Option<Instant>);
impl OpenTimer {
    pub fn begin(&mut self) {
        self.0 = enabled().then(Instant::now);
    }
    pub fn mark(&self, stage: &str, extra: serde_json::Value) {
        if let Some(t) = self.0 {
            let mut v = serde_json::json!({"open": stage, "ms": (t.elapsed().as_secs_f64() * 1000.).round()});
            if let (Some(m), serde_json::Value::Object(e)) = (v.as_object_mut(), extra) {
                m.extend(e);
            }
            line(v);
        }
    }
    pub fn active(&self) -> bool {
        self.0.is_some()
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
