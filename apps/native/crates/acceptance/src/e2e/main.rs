fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let filter = match args.as_slice() {
        [] => None,
        [flag, id] if flag == "--check" => Some(id.clone()),
        _ => {
            eprintln!("Usage: pm-e2e-triage [--check native_check_id]");
            std::process::exit(64);
        }
    };
    // Actor waits are test/worker APIs. In particular, do not run the headless
    // checks on pthread_main_np when this CLI is used on macOS (N8).
    let outcome = std::thread::spawn(move || pm_acceptance_e2e::inventory::run(filter.as_deref()))
        .join()
        .unwrap_or_else(|_| Err("headless runner worker panicked".into()));
    match outcome {
        Ok(report) => {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
            std::process::exit(report.exit_code());
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(64);
        }
    }
}
