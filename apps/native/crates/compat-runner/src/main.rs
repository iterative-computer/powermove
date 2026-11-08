//! One JSON request, one JSON result. The parent owns temp inputs and wall timeout.
use pm_compat_runner::{CompatRunner, initial};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read},
    path::Path,
    thread,
};
/// A Store release through the production host: admitted, confined, activated.
fn store(input: &Value) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    fn copy(from: &Path, to: &Path) -> io::Result<()> {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let (entry, target) = {
                let entry = entry?;
                let target = to.join(entry.file_name());
                (entry, target)
            };
            if entry.file_type()?.is_dir() {
                copy(&entry.path(), &target)?;
            } else {
                fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
    let directory = Path::new(input["extension"].as_str().ok_or("extension required")?);
    let raw: Value = serde_json::from_slice(&fs::read(directory.join("manifest.json"))?)?;
    let id = raw["id"].as_str().ok_or("manifest id")?;
    let root = Path::new(input["work"].as_str().ok_or("work directory required")?);
    let extensions = root.join("extensions");
    copy(directory, &extensions.join(id))?;
    fs::write(
        extensions
            .join(id)
            .join(pm_compat_runner::store_host::STORE_MARKER),
        json!({"repoId":id,"releaseId":input["release"]}).to_string(),
    )?;
    let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
    let kernel = actor.handle();
    let document = pm_model::json::parse(&initial()["document"].to_string())?;
    kernel
        .open("window", pm_kernel::OpenSource::Untitled(document))
        .wait()
        .ok_or("kernel stopped")??;
    let host = pm_compat_runner::host::ExtensionHost::start(pm_compat_runner::host::HostConfig {
        kernel,
        project: "window".into(),
        roots: vec![extensions],
        cache: root.join("cache"),
        owner: None,
        text: None,
        wake: std::sync::Arc::new(|| ()),
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !host.is_ready() && std::time::Instant::now() < deadline {
        thread::sleep(std::time::Duration::from_millis(5));
    }
    let record = host.records().into_iter().find(|r| r["id"] == id);
    let health = record.as_ref().map(|r| r["health"].clone());
    let definitions = host
        .contributions()
        .map_or(0, |c| c["definitions"].as_array().map_or(0, Vec::len));
    drop(host);
    actor.shutdown();
    Ok(
        json!({"version":1,"activation":{"ok":health.as_ref().is_some_and(|h| h["state"] == "ok")},
        "health":health,"definitions":definitions}),
    )
}
fn run() -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let mut input = String::new();
    io::stdin()
        .take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    if input.len() > 16 * 1024 * 1024 {
        return Err("request too large".into());
    }
    let input: Value = serde_json::from_str(&input)?;
    if input["store"] == true {
        return store(&input);
    }
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
    let graph = runner.compile(directory, entry, 3)?;
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
        json!({"version":1,"logic":"runtime-v8","activation":activation,"results":results,"dom_batches":dom_batches,"diagnostics":runner.diagnostics(),
            "gates":{"webkit_panels":false,"full_recorder":false}}),
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
