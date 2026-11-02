use pm_acceptance::{
    catalog::Catalog,
    runner::{Filter, run_with_runtime},
    runtime::NativeRuntime,
};
use std::path::PathBuf;
fn main() -> std::process::ExitCode {
    // Worker-only blocking APIs stay off macOS's main thread, including Drop.
    let result = std::thread::spawn(cli)
        .join()
        .unwrap_or_else(|_| Err("acceptance worker panicked".into()));
    match result {
        Ok(code) => std::process::ExitCode::from(code),
        Err(error) => {
            eprintln!("pm-acceptance: {error}");
            std::process::ExitCode::from(3)
        }
    }
}
fn cli() -> Result<u8, String> {
    let mut filter = Filter::default();
    let mut format = "json".to_owned();
    let mut strict = false;
    let mut corpus = vec![];
    let mut runtime_runner = None;
    let mut compat_bundle = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workflow" => filter.workflow = Some(args.next().ok_or("--workflow requires W ID")?),
            "--case" => filter.case = Some(args.next().ok_or("--case requires exact case ID")?),
            "--format" => {
                format = args
                    .next()
                    .ok_or("--format requires json, markdown, e2e-csv, blockers-csv or catalog")?
            }
            "--strict" => strict = true,
            "--runtime-runner" => {
                runtime_runner = Some(PathBuf::from(
                    args.next()
                        .ok_or("--runtime-runner requires executable path")?,
                ))
            }
            "--compat-bundle" => {
                compat_bundle = Some(PathBuf::from(
                    args.next()
                        .ok_or("--compat-bundle requires bundled JS path")?,
                ))
            }
            "--corpus" => {
                let dir = PathBuf::from(args.next().ok_or("--corpus requires a directory")?);
                if !dir.is_absolute() {
                    return Err("corpus path must be absolute".into());
                }
                let mut paths = std::fs::read_dir(dir)
                    .map_err(|_| "cannot list corpus directory")?
                    .map(|entry| entry.map(|e| e.path()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| "cannot read corpus entry")?;
                paths.sort();
                let before = corpus.len();
                for path in paths {
                    if path.extension().is_some_and(|e| e == "pmv" || e == "json") {
                        corpus.push(path);
                    }
                }
                if corpus.len() == before {
                    return Err("corpus directory has no .pmv/.json inputs".into());
                }
            }
            "--help" => {
                println!(
                    "pm-acceptance [--workflow W033] [--case A517] [--strict] [--runtime-runner /path/to/pm-acceptance-runtime] [--compat-bundle /path/to/probes.js] [--format json|markdown|e2e-csv|blockers-csv|catalog] [--corpus /absolute/synthetic-corpus]\nExit: 0 no failures (blockers allowed), 1 failed assertion, 2 strict blockers, 3 invalid configuration. Corpus inputs are copied before opening; use public/synthetic data only."
                );
                return Ok(0);
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    if !["json", "markdown", "e2e-csv", "blockers-csv", "catalog"].contains(&format.as_str()) {
        return Err("unknown report format".into());
    }
    let catalog = Catalog::embedded()?;
    if format == "catalog" {
        println!(
            "{}",
            serde_json::to_string_pretty(&catalog).map_err(|e| e.to_string())?
        );
        return Ok(0);
    }
    if compat_bundle.is_some() && runtime_runner.is_none() {
        return Err("--compat-bundle requires --runtime-runner".into());
    }
    let runtime = runtime_runner.map(|executable| NativeRuntime {
        executable,
        compat_bundle,
    });
    let report = run_with_runtime(
        &catalog,
        &filter,
        &corpus,
        runtime
            .as_ref()
            .map(|r| r as &dyn pm_acceptance::runtime::RuntimeHook),
    )?;
    match format.as_str() {
        "markdown" => print!("{}", report.markdown()),
        "e2e-csv" => print!("{}", report.e2e_csv()),
        "blockers-csv" => print!("{}", report.blockers_csv()),
        _ => println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        ),
    }
    Ok(report.exit_code(strict))
}
