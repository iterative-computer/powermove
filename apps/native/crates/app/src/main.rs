fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--smoke") {
        pm_shell::smoke()
    } else {
        pm_shell::run()
    }
}
