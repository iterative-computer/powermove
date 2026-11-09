use std::io::{self, Write};
fn main() -> std::process::ExitCode {
    #[cfg(target_os = "macos")]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|s| s == "--engine-job")
    {
        return match pm_engine_service::jobs::run_child() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e:#}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    #[cfg(target_os = "macos")]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|s| s == "render-frames")
        && !std::env::args_os()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|a| a[0] == "--backend" && a[1] == "test-double")
    {
        return write_result(run_with_fonts());
    }
    // Kernel waits and all I/O stay off main, including on macOS (N8).
    let worker = std::thread::spawn(|| {
        pm_native_cli::options::Options::parse(std::env::args_os().skip(1), &mut io::stdin().lock())
            .and_then(pm_native_cli::execute)
    });
    let result = worker.join().unwrap_or_else(|_| {
        Err(pm_native_cli::error(
            "internal_error",
            "CLI worker panicked",
        ))
    });
    write_result(result)
}
fn write_result(result: pm_native_cli::Result<(serde_json::Value, u8)>) -> std::process::ExitCode {
    let (value, code) = match result {
        Ok(result) => result,
        Err(error) => {
            let code = pm_native_cli::exit_code(&error);
            (pm_mcp::failure(error), code)
        }
    };
    let mut stdout = io::stdout().lock();
    if serde_json::to_writer(&mut stdout, &value).is_err() || writeln!(stdout).is_err() {
        return std::process::ExitCode::from(5);
    }
    std::process::ExitCode::from(code)
}

#[cfg(target_os = "macos")]
fn run_with_fonts() -> pm_native_cli::Result<(serde_json::Value, u8)> {
    use pm_render::text::host::RenderFonts;
    use pm_text::host::{FontBroker, FontService, serve};
    let mut source = FontService::new().map_err(|e| pm_native_cli::error("font_error", e))?;
    let broker = FontBroker::new();
    broker.set_generation(source.generation());
    let main = std::thread::current();
    broker.set_wake(std::sync::Arc::new(move || main.unpark()));
    let fonts = RenderFonts::new(broker.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = pm_native_cli::options::Options::parse(
            std::env::args_os().skip(1),
            &mut io::stdin().lock(),
        )
        .and_then(|options| {
            pm_native_cli::execute_with_backend_factory(options, |name| {
                pm_native_cli::backend::native_with_fonts(name, fonts)
            })
        });
        let _ = tx.send(result);
    });
    let result = loop {
        serve(&mut source, broker.take_asks());
        match rx.try_recv() {
            Ok(result) => break result,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                break Err(pm_native_cli::error(
                    "internal_error",
                    "Font worker stopped",
                ));
            }
            Err(_) => std::thread::park_timeout(std::time::Duration::from_millis(2)),
        }
    };
    broker.close();
    worker
        .join()
        .map_err(|_| pm_native_cli::error("internal_error", "Font worker panicked"))?;
    result
}
