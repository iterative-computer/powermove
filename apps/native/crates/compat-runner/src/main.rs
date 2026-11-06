//! One JSON request, one JSON result. The parent owns temp inputs and wall timeout.
use pm_compat_runner::{CompatRunner, initial};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read},
    path::Path,
    thread,
};
fn run() -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let mut input = String::new();
    io::stdin()
        .take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    if input.len() > 16 * 1024 * 1024 {
        return Err("request too large".into());
    }
    let input: Value = serde_json::from_str(&input)?;
    let initial = input.get("initial").cloned().unwrap_or_else(initial);
    let prelude = input
        .get("clock_fixture")
        .and_then(Value::as_str)
        .map(fs::read_to_string)
        .transpose()?
        .unwrap_or_default();
    let runner = CompatRunner::with_prelude(&initial, &prelude)?;
    if let Some(driver) = input["driver"].as_str() {
        runner.eval(&fs::read_to_string(driver)?)?;
    }
    let directory = Path::new(input["extension"].as_str().ok_or("extension required")?);
    let raw: Value = serde_json::from_slice(&fs::read(directory.join("manifest.json"))?)?;
    let manifest = pm_compat_runner::manifest::parse(&raw)?;
    let entry = manifest["entry"].as_str().unwrap_or("index.ts");
    let graph = runner.compile(directory, entry)?;
    let activation = runner.install(&manifest, &graph.to_json())?;
    let mut results = vec![];
    if activation["ok"] == true {
        for expression in input["expressions"].as_array().into_iter().flatten() {
            results.push(runner.json(expression.as_str().ok_or("expression must be a string")?)?);
        }
    }
    let dom_batches = if input["inspect_dom"] == true {
        runner.take_dom_batches()
    } else {
        vec![]
    };
    Ok(
        json!({"version":1,"logic":"runtime-v8","activation":activation,"results":results,"dom_batches":dom_batches,"diagnostics":runner.diagnostics()}),
    )
}
fn main() {
    match thread::spawn(run).join() {
        Ok(Ok(value)) => println!("{value}"),
        Ok(Err(error)) => {
            // The harness redirects stderr to its private disposable capture directory.
            eprintln!("{error}");
            std::process::exit(1)
        }
        Err(_) => {
            eprintln!("compat runner failed; inspect private capture artifacts");
            std::process::exit(1)
        }
    }
}
