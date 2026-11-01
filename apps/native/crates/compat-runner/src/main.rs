//! One JSON request, one JSON result. The parent owns temp inputs and wall timeout.
use pm_compat_runner::{CompatRunner, initial};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read},
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
    let bundle = fs::read_to_string(
        input["compat_bundle"]
            .as_str()
            .ok_or("compat_bundle required")?,
    )?;
    let module = fs::read_to_string(input["module"].as_str().ok_or("module required")?)?;
    let runner = CompatRunner::new(&bundle, &initial())?;
    let activation = runner.activate(&input["manifest"], &module)?;
    let mut results = vec![];
    if activation["ok"] == true {
        for expression in input["expressions"].as_array().into_iter().flatten() {
            results.push(runner.json(expression.as_str().ok_or("expression must be a string")?)?);
        }
    }
    Ok(
        json!({"version":1,"logic":"runtime-v8","activation":activation,"results":results,"diagnostics":runner.diagnostics(),
        "gates":{"kernel":false,"webkit_panels":false,"full_recorder":false,"store_helper":false}}),
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
