use std::{env, fs, path::Path};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 5 || args[1] != "shaders" {
        return Err("usage: pm-corpus-checks shaders COMPILER DEFINITIONS PRIVATE_REPORT".into());
    }
    let definitions = serde_json::from_slice(&fs::read(&args[3])?)?;
    let report = pm_corpus_checks::shaders::report(Path::new(&args[2]), &definitions);
    fs::write(&args[4], serde_json::to_vec_pretty(&report)?)?;
    if report
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["status"] != "pass")
    {
        return Err("shader translation failed; inspect private diagnostics".into());
    }
    Ok(())
}
