fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--engine-job") {
        return pm_engine_service::jobs::run_child();
    }
    anyhow::bail!("pm-engine-service is a library; this binary handles --engine-job")
}
