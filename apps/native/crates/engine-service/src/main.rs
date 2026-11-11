mod bench;
mod memory_bench;
fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--memory-bench") {
        return memory_bench::run();
    }
    if std::env::args().nth(1).as_deref() == Some("--engine-job") {
        return pm_engine_service::jobs::run_child();
    }
    if std::env::args().any(|a| a == "--bench") {
        return bench::run();
    }
    anyhow::bail!("pm-engine-service is a library; this binary handles --engine-job")
}
