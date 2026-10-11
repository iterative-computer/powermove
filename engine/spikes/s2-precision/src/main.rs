//! `s2` — run spike S2 experiments and print JSON reports.
//!
//! Usage: s2 <experiment> [work-dir]
//! Experiments: material, formats, transparency, precision, generation, tags,
//! quicktime, hevc-alpha, math, caps, all.

use std::path::PathBuf;

use s2_precision::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let exp = args.get(1).map(String::as_str).unwrap_or("all");
    let work = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/s2-work"));
    if let Err(e) = run(exp, &work) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn emit<T: serde::Serialize>(work: &std::path::Path, name: &str, v: &T) -> cf::Result<()> {
    let s = serde_json::to_string_pretty(v).unwrap();
    let dir = work.join("results");
    std::fs::create_dir_all(&dir).map_err(|e| cf::Error(e.to_string()))?;
    std::fs::write(dir.join(format!("{name}.json")), &s).map_err(|e| cf::Error(e.to_string()))?;
    println!("{s}");
    Ok(())
}

fn run(exp: &str, work: &std::path::Path) -> cf::Result<()> {
    let all = exp == "all";
    let m = material::ensure(&work.join("material"))?;
    if exp == "material" {
        println!("material in {}", m.dir.display());
    }
    if all || exp == "formats" {
        emit(work, "formats", &exp_formats::run(&m)?)?;
    }
    let needs_gpu = all || matches!(exp, "transparency" | "precision" | "generation" | "math" | "caps" | "tags" | "quicktime" | "hevc-alpha");
    let gpu = if needs_gpu { Some(gpu::Gpu::new()?) } else { None };
    if all || exp == "transparency" {
        emit(work, "transparency", &exp_transparency::run(gpu.as_ref().unwrap(), &m)?)?;
    }
    if all || exp == "precision" {
        emit(work, "precision", &exp_precision::run(gpu.as_ref().unwrap())?)?;
    }
    if all || exp == "generation" {
        emit(work, "generation", &exp_generation::run(gpu.as_ref().unwrap(), &m)?)?;
    }
    if all || exp == "tags" {
        emit(work, "tags", &exp_tags::run(gpu.as_ref().unwrap(), &work.join("tags"))?)?;
    }
    if all || exp == "quicktime" {
        emit(work, "quicktime", &exp_quicktime::run(gpu.as_ref().unwrap(), &work.join("quicktime"))?)?;
    }
    if all || exp == "math" {
        emit(work, "math", &exp_math::run(gpu.as_ref().unwrap())?)?;
    }
    if all || exp == "caps" {
        emit(work, "caps", &exp_caps::run(gpu.as_ref().unwrap())?)?;
    }
    Ok(())
}
