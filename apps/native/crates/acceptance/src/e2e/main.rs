fn main() {
    let mut args = std::env::args().skip(1);
    let mut reference = "main".to_owned();
    let mut reference_explicit = false;
    let mut filter = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--reference" => {
                reference_explicit = true;
                reference = args.next().unwrap_or_default();
            }
            "--check" => {
                filter = Some(args.next().unwrap_or_else(|| {
                    eprintln!("--check requires a case ID");
                    std::process::exit(64)
                }))
            }
            _ => {
                eprintln!("Usage: pm-e2e-triage [--reference main|v1.2.0] [--check case_id]");
                std::process::exit(64);
            }
        }
    }
    if !reference_explicit && filter.as_deref().is_some_and(|id| !id.starts_with("MAIN.")) {
        reference = "v1.2.0".into();
    }
    if !["main", "v1.2.0"].contains(&reference.as_str()) {
        std::process::exit(64);
    }
    let outcome = std::thread::spawn(move || {
        if reference == "main" {
            let report = pm_acceptance::main_reference::run(
                &pm_acceptance::catalog::Catalog::embedded()?,
                &pm_acceptance::runner::Filter {
                    workflow: None,
                    case: filter,
                },
                &[],
                None,
            )?;
            let exit = i32::from(report.exit_code(true));
            Ok((serde_json::to_string_pretty(&report).unwrap(), exit))
        } else {
            let report = pm_acceptance_e2e::inventory::run(filter.as_deref())?;
            let exit = report.exit_code();
            Ok((serde_json::to_string_pretty(&report).unwrap(), exit))
        }
    })
    .join()
    .unwrap_or_else(|_| Err("headless runner worker panicked".to_owned()));
    match outcome {
        Ok((json, exit)) => {
            println!("{json}");
            std::process::exit(exit);
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(64);
        }
    }
}
