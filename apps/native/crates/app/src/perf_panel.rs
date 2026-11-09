//! Window ▸ Panels ▸ Performance (S115) over pm-services
//! `maintenance::Performance`. It only observes: sampling runs at most twice a
//! second and stops while the panel isn't painted, so the panel never costs
//! what it measures. Counters the native engine doesn't report read `—`.
use crate::session::Controller;
use gpui::{prelude::*, *};
use pm_services::{
    ServiceErrorCode,
    surface_backing::{
        copy::PERFORMANCE_LABELS,
        maintenance::{Metric, Performance, PerformanceSample},
    },
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const SAMPLE: Duration = Duration::from_millis(500);
/// A panel that stops painting for this long stops sampling.
const HIDDEN: Duration = Duration::from_secs(1);
const NOT_REPORTED: &str = "Not available on this Mac.";

#[derive(Default)]
pub struct State {
    perf: Performance,
    painted: Option<Instant>,
    /// Presented-frame count and when it was read, for the frame rate.
    last: Option<(Instant, u64)>,
    started: Option<Instant>,
}

pub fn panel(v: &mut Controller, id: &str, cx: &mut Context<Controller>) -> Option<AnyElement> {
    (id == "perf").then(|| render(v, cx))
}

fn render(v: &mut Controller, cx: &mut Context<Controller>) -> AnyElement {
    let t = v.host.borrow().tokens.clone();
    if v.session.is_none() {
        v.perf.perf.visible = false;
        return pm_shell::view::empty_state(
            &t,
            "No Project Open",
            "Open a project to see its performance.",
        )
        .into_any_element();
    }
    v.perf.painted = Some(Instant::now());
    if !v.perf.perf.visible {
        v.perf.perf.visible = true;
        start(cx);
    }
    let (c, m) = (&t.colors, &t.metrics);
    let sample = v.perf.perf.sample.as_ref();
    let mut rows = div()
        .id("performance")
        .role(Role::Table)
        .aria_label("Performance")
        .flex()
        .flex_col()
        .p(px(m.gap))
        .text_size(px(t.typography.body));
    for (key, label) in PERFORMANCE_LABELS {
        let metric = sample.and_then(|s| s.metrics.get(*key));
        let value = metric.and_then(text);
        let spoken = format!(
            "{label}, {}",
            value.clone().unwrap_or("not available".into())
        );
        rows = rows.child(
            div()
                .id(*key)
                .role(Role::Row)
                .aria_label(spoken)
                .flex()
                .items_center()
                .h(px(m.row))
                .child(
                    div()
                        .flex_none()
                        .w(px(m.settings_sidebar / 1.5))
                        .text_color(rgb(c.secondary))
                        .child(*label),
                )
                .child({
                    let unreported = matches!(metric, Some(Metric::Unavailable(_)));
                    div()
                        .id(SharedString::from(format!("{key}-value")))
                        .font_features(FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])))
                        .when_some(value.clone(), |d, value| d.child(value))
                        .when(value.is_none(), |d| d.text_color(rgb(c.muted)).child("—"))
                        .when(unreported, |d| {
                            d.tooltip(pm_shell::view::tooltip(&t, NOT_REPORTED))
                        })
                }),
        );
    }
    rows.into_any_element()
}

fn text(metric: &Metric) -> Option<String> {
    match metric {
        Metric::Count(n) => Some(n.to_string()),
        Metric::PerSecond(n) => Some(format!("{} fps", n.round())),
        Metric::Milliseconds(n) => Some(format!("{n:.2} ms")),
        Metric::Unavailable(_) => None,
    }
}

/// Samples while the panel is painted; ends itself once it isn't.
fn start(cx: &mut Context<Controller>) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(SAMPLE).await;
            let alive = this.update(cx, |v, cx| {
                if v.perf.painted.is_none_or(|p| p.elapsed() > HIDDEN) {
                    v.perf.perf.visible = false;
                    v.perf.last = None;
                    return false;
                }
                let sample = sample(v);
                if v.perf.perf.observe(sample).unwrap_or(false) {
                    cx.notify();
                }
                true
            });
            if !matches!(alive, Ok(true)) {
                break;
            }
        }
    })
    .detach();
}

fn sample(v: &mut Controller) -> PerformanceSample {
    let now = Instant::now();
    let started = *v.perf.started.get_or_insert(now);
    let status = v.viewer.as_ref().map(|viewer| viewer.status());
    let playing = status.as_ref().is_some_and(|s| {
        s.clock
            .is_some_and(|c| c.state == pm_protocol::clock::TransportState::Playing)
    });
    let mut metrics = BTreeMap::new();
    let presented = status.as_ref().map_or(0, |s| s.presented);
    if playing {
        if let Some((at, before)) = v.perf.last {
            let seconds = now.duration_since(at).as_secs_f64();
            if seconds > 0. {
                let rate = presented.saturating_sub(before) as f64 / seconds;
                metrics.insert("frame_rate".into(), Metric::PerSecond(rate));
            }
        }
        if let Some(ms) = status
            .as_ref()
            .and_then(|s| s.frame.as_ref())
            .and_then(|f| f.worker_to_completion_ms)
            .filter(|ms| ms.is_finite() && *ms >= 0.)
        {
            metrics.insert("frame_time".into(), Metric::Milliseconds(ms));
        }
    }
    v.perf.last = Some((now, presented));
    for key in [
        "draw_calls",
        "effect_passes",
        "gpu_programs",
        "raster_cache",
    ] {
        metrics.insert(
            key.into(),
            Metric::Unavailable(ServiceErrorCode::Unavailable),
        );
    }
    metrics.insert("layers".into(), Metric::Count(v.layers().len() as u64));
    metrics.insert("keyframes".into(), Metric::Count(v.keyframes as u64));
    PerformanceSample {
        at_ms: now.duration_since(started).as_millis() as u64 + SAMPLE.as_millis() as u64,
        metrics,
    }
}

#[cfg(test)]
mod tests {
    use super::{Metric, text};
    #[test]
    fn values_read_as_the_spec_writes_them() {
        assert_eq!(text(&Metric::PerSecond(59.6)).as_deref(), Some("60 fps"));
        assert_eq!(text(&Metric::Milliseconds(4.0)).as_deref(), Some("4.00 ms"));
        assert_eq!(text(&Metric::Count(12)).as_deref(), Some("12"));
        assert_eq!(
            text(&Metric::Unavailable(
                pm_services::ServiceErrorCode::Unavailable
            )),
            None
        );
    }
}
