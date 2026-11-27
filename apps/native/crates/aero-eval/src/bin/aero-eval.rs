//! `aero-eval`: check the suite, run tasks or batches, summarise, regrade,
//! and serve recorded model streams. See docs/native/aero-eval.md.
use pm_aero_eval::credits::{self, Pricing};
use pm_aero_eval::replay::Recording;
use pm_aero_eval::replay::replayer::{ReplayServer, Replayer};
use pm_aero_eval::runner::harness::HarnessId;
use pm_aero_eval::runner::{self, Config};
use pm_aero_eval::stats;
use pm_aero_eval::suite::Suite;
use std::path::PathBuf;

const USAGE: &str = "usage:
  aero-eval check [--data DIR]
  aero-eval list [--data DIR]
  aero-eval run --task ID[,ID…|all] --harness H[,H…] --model M --effort E --mcp-bin PATH --out DIR
                [--repeats N] [--harness-bin PATH] [--approved \"who, when\"] [--replay REC] [--ablate a,b]
                [--cohort ID] [--sandbox-read PATH]…
                (exit 1 when a replayed run diverged, did not complete or errored)
  aero-eval summarize DIR [--data DIR] [--pricing FILE] [--credit-nano-usd N]
  aero-eval credits --pricing FILE [--credit-nano-usd N] [--runs DIR | --recording REC] [--data DIR]
  aero-eval regrade RUN_DIR [--data DIR]
  aero-eval replay check REC
  aero-eval replay serve REC [--folder PATH]…   (serves until stdin closes, then prints the report)
harnesses: aero, codex, claude-code, none";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("aero-eval: {e}");
            std::process::exit(2);
        }
    }
}

struct Args<'a>(&'a [String]);
impl<'a> Args<'a> {
    fn value(&self, flag: &str) -> Option<&'a str> {
        self.0
            .iter()
            .position(|a| a == flag)
            .and_then(|i| self.0.get(i + 1))
            .map(String::as_str)
    }
    fn values(&self, flag: &str) -> Vec<&'a str> {
        self.0
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == flag)
            .filter_map(|(i, _)| self.0.get(i + 1).map(String::as_str))
            .collect()
    }
    fn need(&self, flag: &str) -> Result<&'a str, String> {
        self.value(flag)
            .ok_or_else(|| format!("{flag} is required\n{USAGE}"))
    }
    fn data(&self) -> PathBuf {
        self.value("--data")
            .map(PathBuf::from)
            .unwrap_or_else(pm_aero_eval::data_root)
    }
}

/// The exit status of a command that ran: 0, or 1 for a failed gate.
fn run(args: &[String]) -> Result<i32, String> {
    let a = Args(args);
    match args.first().map(String::as_str) {
        Some("check") => {
            let data = a.data();
            let suite = Suite::load(&data)?;
            let tools = pm_aero_eval::suite::tool_names(&catalog())?;
            let problems = suite.validate(&data, &tools);
            if problems.is_empty() {
                println!(
                    "suite v{}: {} tasks, all checks pass",
                    suite.version,
                    suite.tasks.len()
                );
                Ok(0)
            } else {
                Err(problems.join("\n"))
            }
        }
        Some("list") => {
            let suite = Suite::load(&a.data())?;
            for t in &suite.tasks {
                println!(
                    "{:<28} {:<15} {}",
                    t.id,
                    t.kind.as_str(),
                    t.graders
                        .iter()
                        .map(|g| g.name())
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            Ok(0)
        }
        Some("run") => {
            let data = a.data();
            let suite = Suite::load(&data)?;
            let harnesses = a
                .need("--harness")?
                .split(',')
                .map(HarnessId::parse)
                .collect::<Result<Vec<_>, _>>()?;
            let wanted = a.need("--task")?;
            let tasks: Vec<_> = if wanted == "all" {
                suite.tasks.iter().collect()
            } else {
                wanted
                    .split(',')
                    .map(|id| suite.task(id).ok_or_else(|| format!("no task {id}")))
                    .collect::<Result<_, _>>()?
            };
            let cfg = Config {
                data_root: data.clone(),
                mcp_bin: PathBuf::from(a.need("--mcp-bin")?),
                harness_bin: a.value("--harness-bin").map(PathBuf::from),
                out: PathBuf::from(a.need("--out")?),
                model: a.need("--model")?.into(),
                effort: a.need("--effort")?.into(),
                repeats: a
                    .value("--repeats")
                    .map(|r| r.parse().map_err(|_| "--repeats is a number"))
                    .transpose()?
                    .unwrap_or(1),
                approved: a.value("--approved").map(str::to_owned),
                replay: a.value("--replay").map(PathBuf::from),
                ablate: a
                    .value("--ablate")
                    .map(|s| s.split(',').map(str::to_owned).collect())
                    .unwrap_or_default(),
                cohort: a
                    .value("--cohort")
                    .map(str::to_owned)
                    .unwrap_or_else(runner::new_cohort),
                deadlines: Default::default(),
                sandbox_read: a
                    .values("--sandbox-read")
                    .into_iter()
                    .map(PathBuf::from)
                    .collect(),
                sandbox_protect: Vec::new(),
            };
            for h in &harnesses {
                runner::guard(*h, &cfg)?;
            }
            let plan = runner::plan(&tasks, &harnesses, cfg.repeats);
            let mut records = Vec::new();
            let mut gate_failed = false;
            for slot in &plan {
                let task = suite.task(&slot.task).unwrap();
                let r = match runner::run_slot(&suite, task, slot, &cfg) {
                    Ok(r) => r,
                    Err(e) => {
                        println!(
                            "[{}/{}] {} {} r{}: not run ({e})",
                            slot.order + 1,
                            plan.len(),
                            slot.task,
                            slot.harness.as_str(),
                            slot.rep
                        );
                        gate_failed = true;
                        continue;
                    }
                };
                // A replay is a regression gate: it fails on divergence, an
                // incomplete harness turn or a run error, whatever the graders.
                if r.mode == "replay"
                    && (r.replay_passed != Some(true)
                        || r.harness_completed != Some(true)
                        || r.error.is_some())
                {
                    gate_failed = true;
                }
                println!(
                    "[{}/{}] {} {} r{}: {}{}{}",
                    slot.order + 1,
                    plan.len(),
                    r.task,
                    r.harness,
                    r.rep,
                    if r.success { "success" } else { "fail" },
                    match r.replay_passed {
                        Some(false) => " (replay diverged or was incomplete)",
                        _ => "",
                    },
                    r.error
                        .as_deref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default()
                );
                records.push(r);
            }
            write_summary(&cfg.out, &records, suite.version)?;
            Ok(if gate_failed { 1 } else { 0 })
        }
        Some("summarize") => {
            let dir = PathBuf::from(args.get(1).ok_or(USAGE)?);
            let suite = Suite::load(&a.data())?;
            let records = runner::load_runs(&dir)?;
            write_summary(&dir, &records, suite.version)?;
            if let Some(pricing) = load_pricing(&a)? {
                let groups = credits::summarize_runs(&records, suite.version, &pricing);
                println!("\n{}", credits::render_run_credits(&groups, &pricing));
            }
            Ok(0)
        }
        Some("credits") => {
            let pricing = load_pricing(&a)?.ok_or_else(|| format!("--pricing is required\n{USAGE}"))?;
            if let Some(runs_dir) = a.value("--runs") {
                let suite = Suite::load(&a.data())?;
                let records = runner::load_runs(&PathBuf::from(runs_dir))?;
                let groups = credits::summarize_runs(&records, suite.version, &pricing);
                print!("{}", credits::render_run_credits(&groups, &pricing));
            } else if let Some(rec_path) = a.value("--recording") {
                let rec = Recording::load(&PathBuf::from(rec_path))?;
                let exchanges = credits::recording_exchanges(&rec);
                let title = format!(
                    "recording {} ({} / {} / {})",
                    rec.task, rec.harness, rec.model, rec.effort
                );
                print!("{}", credits::render_exchanges_table(&title, &exchanges, &pricing));
            } else {
                print!("{}", credits::render_profiles(&pricing));
            }
            Ok(0)
        }
        Some("regrade") => {
            let dir = PathBuf::from(args.get(1).ok_or(USAGE)?);
            let data = a.data();
            let r = runner::regrade(&Suite::load(&data)?, &dir, &data)?;
            println!("{}", serde_json::to_string_pretty(&r).unwrap());
            Ok(0)
        }
        Some("replay") => match args.get(1).map(String::as_str) {
            Some("check") => {
                let r = Recording::load(&PathBuf::from(args.get(2).ok_or(USAGE)?))?;
                println!(
                    "{} on {} ({} {}), {} exchanges: valid",
                    r.task,
                    r.harness,
                    r.model,
                    r.effort,
                    r.exchanges.len()
                );
                Ok(0)
            }
            Some("serve") => {
                let r = Recording::load(&PathBuf::from(args.get(2).ok_or(USAGE)?))?;
                let folders = a
                    .values("--folder")
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                let server = ReplayServer::start(Replayer::new(r, folders))?;
                println!("{}", server.origin());
                let mut sink = String::new();
                while std::io::stdin().read_line(&mut sink).is_ok_and(|n| n > 0) {}
                let report = server.finish();
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
                if report.passed {
                    Ok(0)
                } else {
                    eprintln!("aero-eval: replay diverged or was incomplete");
                    Ok(1)
                }
            }
            _ => Err(USAGE.into()),
        },
        _ => Err(USAGE.into()),
    }
}

fn load_pricing(a: &Args<'_>) -> Result<Option<Pricing>, String> {
    let Some(path) = a.value("--pricing") else {
        return Ok(None);
    };
    let mut pricing = Pricing::load(&PathBuf::from(path))?;
    if let Some(unit) = a.value("--credit-nano-usd") {
        let parsed: u64 = unit
            .parse()
            .map_err(|_| "--credit-nano-usd must be a positive integer")?;
        if parsed == 0 {
            return Err("--credit-nano-usd must be positive".into());
        }
        pricing.credit_nano_usd = parsed;
    }
    Ok(Some(pricing))
}

fn catalog() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mcp/fixtures/catalog-v1.2.0.json")
}

fn write_summary(
    dir: &std::path::Path,
    records: &[pm_aero_eval::record::RunRecord],
    version: u32,
) -> Result<(), String> {
    let s = stats::summarize(records, version);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("summary.json"),
        serde_json::to_vec_pretty(&s).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    print!("{}", stats::render(&s));
    Ok(())
}
