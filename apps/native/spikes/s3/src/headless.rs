//! Same renderer/worker as the viewer, with no GPUI or WindowServer dependency.
#![allow(dead_code)]
mod color;
mod mailbox;
mod metrics;
mod policy;
mod render;
mod trace;
mod worker;
fn arg(name: &str) -> Option<String> {
    std::env::args().find_map(|a| a.strip_prefix(&format!("--{name}=")).map(str::to_owned))
}
fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == format!("--{name}"))
}
fn main() -> anyhow::Result<()> {
    let doc = arg("document").ok_or_else(|| anyhow::anyhow!("--document required"))?;
    let seconds = arg("seconds").unwrap_or("60".into()).parse::<f64>()?;
    let hz = arg("hz").unwrap_or("120".into()).parse::<f64>()?;
    anyhow::ensure!(
        seconds > 0. && seconds <= 300. && hz > 0. && hz <= 240.,
        "invalid duration/rate"
    );
    worker::offscreen(
        pm_gpu::Gpu::new("S3 headless engine GPU")?,
        worker::Options {
            scene: render::Scene::load(&doc)?,
            pq: arg("pq").map(|p| render::Scene::pq(&p)).transpose()?,
            seconds,
            hz,
            verify: flag("verify"),
            measure_overlays: false,
            report: arg("report").unwrap_or("/tmp/s3b-headless.json".into()),
            mode: "offscreen".into(),
            legacy_sync: flag("legacy-sync"),
            synthetic_scrub: flag("synthetic-scrub"),
            cache: !flag("no-cache"),
        },
    )
}
