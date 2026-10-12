//! S3 spike runner: translate every corpus shader through the section 11
//! pipeline (naga and SPIRV-Cross back ends), render it offscreen with wgpu,
//! and compare against the WebGL2 reference rendered by `scripts/webgl.ts`.
//!
//! Usage:
//!   s3-shaders run --corpus work/corpus.json --webgl work/webgl/metal --out work/results
//!                  [--paths naga,spvc] [--filter <substring>]
//!   s3-shaders msl --corpus work/corpus.json --case <id> [--path naga|spvc]
//!   s3-shaders diff --a <reference.f16> --b <rendered.f16> [--width 256]
//!   s3-shaders refcmp --a work/webgl/metal --b work/webgl/swiftshader --out <file.jsonl>
//!   s3-shaders fuzz [--iterations N] [--seed S] [--bombs] [--metal] [--deadline seconds]
//!   s3-shaders fuzz-dump [--iterations N] [--seed S] [--bombs] [--out dir]
//!   s3-shaders translate --frag <file> [--path naga|spvc]
//!   s3-shaders envelope --perturbed work/webgl/metal-perturbed,... [--webgl work/webgl/metal] [--results work/results]

mod case;
mod compare;
mod compile;
mod errors;
mod fuzz;
mod limits;
mod normalize;
mod pipeline;
mod reflect;
mod render;
mod rewrite;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path as FsPath, PathBuf};
use std::time::Instant;

use half::f16;
use pipeline::{Failure, Path};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

pub fn sanitize(id: &str) -> String {
    id.replace('/', "__")
}

#[derive(serde::Serialize)]
struct VariantResult {
    variant: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    metrics: Option<compare::Metrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(serde::Serialize)]
struct CaseResult {
    id: String,
    group: String,
    kind: String,
    path: String,
    compiled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<Failure>,
    renamed: Vec<String>,
    polyfilled: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    warnings: String,
    timings_ms: BTreeMap<String, f64>,
    variants: Vec<VariantResult>,
}

fn read_f16(path: &FsPath) -> Option<Vec<f16>> {
    let bytes = std::fs::read(path).ok()?;
    Some(bytes.chunks_exact(2).map(|c| f16::from_le_bytes([c[0], c[1]])).collect())
}

fn write_f16(path: &FsPath, data: &[f16]) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    let bytes: Vec<u8> = data.iter().flat_map(|h| h.to_le_bytes()).collect();
    std::fs::write(path, bytes)
}

fn run(args: &[String]) -> anyhow::Result<()> {
    let corpus_path = PathBuf::from(arg(args, "--corpus").unwrap_or_else(|| "work/corpus.json".into()));
    let webgl = PathBuf::from(arg(args, "--webgl").unwrap_or_else(|| "work/webgl/metal".into()));
    let out = PathBuf::from(arg(args, "--out").unwrap_or_else(|| "work/results".into()));
    let filter = arg(args, "--filter");
    let paths: Vec<Path> = arg(args, "--paths")
        .unwrap_or_else(|| "naga,spvc".into())
        .split(',')
        .map(|p| match p {
            "naga" => Path::Naga,
            "spvc" => Path::SpirvCross,
            other => panic!("unknown path {other}"),
        })
        .collect();
    let corpus: case::Corpus = serde_json::from_slice(&std::fs::read(&corpus_path)?)?;
    let base = corpus_path.parent().unwrap().to_path_buf();
    let gpu = render::Gpu::new()?;
    eprintln!("adapter: {} ({:?}), passthrough: {}", gpu.adapter_info.name, gpu.adapter_info.backend, gpu.passthrough);
    let images = gpu.load_images(&corpus, &base)?;
    std::fs::create_dir_all(&out)?;
    for path in &paths {
        let mut results = std::fs::File::create(out.join(format!("{}.jsonl", path.name())))?;
        let started = Instant::now();
        let mut done = 0;
        for case in corpus.cases.iter().filter(|c| filter.as_ref().is_none_or(|f| c.id.contains(f.as_str()))) {
            let mut result = CaseResult {
                id: case.id.clone(),
                group: case.group.clone(),
                kind: case.kind.clone(),
                path: path.name().into(),
                compiled: false,
                failure: None,
                renamed: Vec::new(),
                polyfilled: Vec::new(),
                warnings: String::new(),
                timings_ms: BTreeMap::new(),
                variants: Vec::new(),
            };
            let translated = pipeline::translate(&corpus.vertex, &case.frag, case.line_offset, *path);
            match translated {
                Err(failure) => result.failure = Some(failure),
                Ok(translated) => {
                    result.renamed = translated.renamed.clone();
                    result.polyfilled = translated.polyfilled.clone();
                    result.warnings = translated.warnings.clone();
                    result.timings_ms = translated.timings_ms.clone().into_iter().collect();
                    let start = Instant::now();
                    match gpu.build(translated, *path, &case.id) {
                        Err(failure) => result.failure = Some(failure),
                        Ok(program) => {
                            result.timings_ms.insert("pipeline".into(), start.elapsed().as_secs_f64() * 1000.0);
                            result.compiled = true;
                            for variant in &case.variants {
                                let reference = webgl.join(sanitize(&case.id)).join(format!("{}.f16", variant.name));
                                let start = Instant::now();
                                let rendered = gpu.render(&program, variant, &images);
                                result.timings_ms.insert(format!("render-{}", variant.name), start.elapsed().as_secs_f64() * 1000.0);
                                match rendered {
                                    Err(failure) => {
                                        result.variants.push(VariantResult { variant: variant.name.clone(), metrics: None, note: Some(format!("{}: {}", failure.stage, failure.message)) });
                                    }
                                    Ok(pixels) => {
                                        let (metrics, note) = match read_f16(&reference) {
                                            Some(expected) if expected.len() == pixels.len() => (Some(compare::compare(&expected, &pixels)), None),
                                            Some(_) => (None, Some("reference size mismatch".to_string())),
                                            None => (None, Some("no WebGL reference (WebGL failed to compile or was not run)".to_string())),
                                        };
                                        if metrics.as_ref().is_none_or(|m| !m.pass) {
                                            write_f16(&out.join(path.name()).join(sanitize(&case.id)).join(format!("{}.f16", variant.name)), &pixels)?;
                                        }
                                        result.variants.push(VariantResult { variant: variant.name.clone(), metrics, note });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            writeln!(results, "{}", serde_json::to_string(&result)?)?;
            done += 1;
        }
        eprintln!("{}: {} cases in {:.1}s", path.name(), done, started.elapsed().as_secs_f64());
    }
    Ok(())
}

/// Print the generated MSL (both stages) for one case, for debugging.
fn msl(args: &[String]) -> anyhow::Result<()> {
    let corpus_path = PathBuf::from(arg(args, "--corpus").unwrap_or_else(|| "work/corpus.json".into()));
    let id = arg(args, "--case").expect("--case");
    let path = match arg(args, "--path").as_deref() {
        Some("spvc") => Path::SpirvCross,
        _ => Path::Naga,
    };
    let corpus: case::Corpus = serde_json::from_slice(&std::fs::read(&corpus_path)?)?;
    let case = corpus.cases.iter().find(|c| c.id == id).expect("case");
    let translated = pipeline::translate(&corpus.vertex, &case.frag, case.line_offset, path).map_err(|f| anyhow::anyhow!("{}: {}", f.stage, f.message))?;
    for stage in [&translated.vertex, &translated.fragment] {
        match path {
            Path::SpirvCross => println!("{}", stage.msl.as_deref().unwrap_or_default()),
            Path::Naga => {
                let module = stage.naga.as_ref().unwrap();
                let info = pipeline::naga_validate(module).map_err(|e| anyhow::anyhow!(e))?;
                // Same language version wgpu-hal picks on this machine (MSL 3.x on macOS 14+).
                let options = naga::back::msl::Options { lang_version: (3, 0), ..Default::default() };
                let (source, _) = naga::back::msl::write_string(module, &info, &options, &Default::default()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
                println!("{source}");
            }
        }
    }
    Ok(())
}

/// For every variant that failed the parity check on a path, split the
/// engine's out-of-tolerance pixels into those inside WebGL's own
/// perturbation envelope (pixels where WebGL, given uniforms scaled by a few
/// float ulps, already moves by more than 1 CV) and those outside it. A
/// mismatch entirely inside the envelope is a property of the shader's
/// conditioning, not of the translation. One JSON line per failing variant.
fn envelope(args: &[String]) -> anyhow::Result<()> {
    let corpus_path = PathBuf::from(arg(args, "--corpus").unwrap_or_else(|| "work/corpus.json".into()));
    let reference = PathBuf::from(arg(args, "--webgl").unwrap_or_else(|| "work/webgl/metal".into()));
    let results = PathBuf::from(arg(args, "--results").unwrap_or_else(|| "work/results".into()));
    let perturbed: Vec<PathBuf> = arg(args, "--perturbed").expect("--perturbed dir[,dir...]").split(',').map(PathBuf::from).collect();
    let corpus: case::Corpus = serde_json::from_slice(&std::fs::read(&corpus_path)?)?;
    let q = |v: f16| if v.to_f32().is_nan() { -1 } else { (v.to_f32().clamp(0.0, 1.0) * 255.0).round() as i32 };
    let worst = |a: &[f16], b: &[f16], p: usize| (0..4).map(|c| (q(a[p * 4 + c]) - q(b[p * 4 + c])).unsigned_abs()).max().unwrap();
    for path in ["naga", "spvc"] {
        for case in &corpus.cases {
            for variant in &case.variants {
                let name = format!("{}.f16", variant.name);
                let Some(engine) = read_f16(&results.join(path).join(sanitize(&case.id)).join(&name)) else { continue };
                let Some(webgl) = read_f16(&reference.join(sanitize(&case.id)).join(&name)) else { continue };
                if engine.len() != webgl.len() {
                    continue;
                }
                let runs: Vec<Vec<f16>> = perturbed.iter().filter_map(|d| read_f16(&d.join(sanitize(&case.id)).join(&name))).filter(|r| r.len() == webgl.len()).collect();
                let pixels = webgl.len() / 4;
                let (mut over, mut inside, mut outside_max, mut envelope_size) = (0usize, 0usize, 0u32, 0usize);
                let mut outside_over_one = 0usize;
                for p in 0..pixels {
                    let in_envelope = runs.iter().any(|r| worst(&webgl, r, p) > 1);
                    envelope_size += usize::from(in_envelope);
                    let d = worst(&webgl, &engine, p);
                    if d > 1 {
                        over += 1;
                        if in_envelope {
                            inside += 1;
                        } else {
                            outside_over_one += 1;
                        }
                    }
                    if !in_envelope {
                        outside_max = outside_max.max(d);
                    }
                }
                let outside_pass = outside_max <= 4 && (outside_over_one as f64) <= 0.001 * (pixels - envelope_size).max(1) as f64;
                println!(
                    "{}",
                    serde_json::json!({
                        "path": path, "id": case.id, "variant": variant.name, "perturbed_runs": runs.len(),
                        "envelope_pixels": envelope_size, "over_one": over, "over_one_inside_envelope": inside,
                        "over_one_outside_envelope": outside_over_one, "max_diff_outside_envelope": outside_max, "pass_outside_envelope": outside_pass,
                    })
                );
            }
        }
    }
    Ok(())
}

/// Translate one fragment source file (with the corpus vertex shader) and
/// print the outcome and per-stage timings, for triaging slow or failing
/// inputs.
fn translate_file(args: &[String]) -> anyhow::Result<()> {
    let corpus_path = PathBuf::from(arg(args, "--corpus").unwrap_or_else(|| "work/corpus.json".into()));
    let file = PathBuf::from(arg(args, "--frag").expect("--frag <file>"));
    let path = match arg(args, "--path").as_deref() {
        Some("spvc") => Path::SpirvCross,
        _ => Path::Naga,
    };
    let corpus: case::Corpus = serde_json::from_slice(&std::fs::read(&corpus_path)?)?;
    let source = std::fs::read_to_string(&file)?;
    let started = Instant::now();
    let result = pipeline::translate(&corpus.vertex, &source, 0, path);
    let total = started.elapsed().as_secs_f64() * 1000.0;
    match result {
        Ok(t) => {
            let mut timings: Vec<_> = t.timings_ms.into_iter().collect();
            timings.sort_by(|a, b| a.0.cmp(&b.0));
            println!("ok in {total:.1} ms: {timings:?}");
        }
        Err(f) => println!("reject at {} in {total:.1} ms: {}", f.stage, f.message.lines().next().unwrap_or_default()),
    }
    Ok(())
}

/// List the pixels of a rendered variant that differ from the WebGL reference
/// by more than 1 (8-bit), with coordinates in GL row order.
fn diff(args: &[String]) -> anyhow::Result<()> {
    let a = PathBuf::from(arg(args, "--a").expect("--a reference.f16"));
    let b = PathBuf::from(arg(args, "--b").expect("--b rendered.f16"));
    let width: usize = arg(args, "--width").map_or(256, |w| w.parse().unwrap());
    let limit: usize = arg(args, "--limit").map_or(40, |w| w.parse().unwrap());
    let (a, b) = (read_f16(&a).expect("read a"), read_f16(&b).expect("read b"));
    anyhow::ensure!(a.len() == b.len(), "size mismatch");
    let q = |v: f16| (v.to_f32().clamp(0.0, 1.0) * 255.0).round() as i32;
    let mut shown = 0;
    for p in 0..a.len() / 4 {
        let worst = (0..4).map(|c| (q(a[p * 4 + c]) - q(b[p * 4 + c])).abs()).max().unwrap();
        if worst > 1 {
            if shown < limit {
                let fmt = |x: &[f16]| x.iter().map(|v| format!("{:.4}", v.to_f32())).collect::<Vec<_>>().join(",");
                println!("x={} y={} diff={} a=[{}] b=[{}]", p % width, p / width, worst, fmt(&a[p * 4..p * 4 + 4]), fmt(&b[p * 4..p * 4 + 4]));
            }
            shown += 1;
        }
    }
    println!("{shown} pixels differ by more than 1");
    Ok(())
}

/// Compare two reference sets (for example WebGL on ANGLE Metal against WebGL
/// on SwiftShader) with the same tolerance, to show how implementation
/// sensitive each shader is. Writes one JSON line per case variant.
fn refcmp(args: &[String]) -> anyhow::Result<()> {
    let corpus_path = PathBuf::from(arg(args, "--corpus").unwrap_or_else(|| "work/corpus.json".into()));
    let a = PathBuf::from(arg(args, "--a").unwrap_or_else(|| "work/webgl/metal".into()));
    let b = PathBuf::from(arg(args, "--b").unwrap_or_else(|| "work/webgl/swiftshader".into()));
    let out = PathBuf::from(arg(args, "--out").unwrap_or_else(|| "work/results/webgl-metal-vs-swiftshader.jsonl".into()));
    let corpus: case::Corpus = serde_json::from_slice(&std::fs::read(&corpus_path)?)?;
    std::fs::create_dir_all(out.parent().unwrap())?;
    let mut file = std::fs::File::create(&out)?;
    for case in &corpus.cases {
        for variant in &case.variants {
            let name = format!("{}.f16", variant.name);
            let (x, y) = (read_f16(&a.join(sanitize(&case.id)).join(&name)), read_f16(&b.join(sanitize(&case.id)).join(&name)));
            let metrics = match (x, y) {
                (Some(x), Some(y)) if x.len() == y.len() => Some(compare::compare(&x, &y)),
                _ => None,
            };
            writeln!(file, "{}", serde_json::json!({ "id": case.id, "group": case.group, "variant": variant.name, "metrics": metrics }))?;
        }
    }
    Ok(())
}

/// Stack for the thread that translates and builds pipelines. glslang's ES
/// front end, glslang's SPIR-V back end, naga, and SPIRV-Cross all recurse
/// per expression nesting level; a 100,000-deep swizzle chain (which WebGL
/// accepts) overflows the default 8 MiB main stack. Address space is reserved,
/// not committed. See S3.md, "Resource limits".
const TRANSLATION_STACK_BYTES: usize = 1 << 30;

fn main() -> anyhow::Result<()> {
    let worker = std::thread::Builder::new().name("s3-translate".into()).stack_size(TRANSLATION_STACK_BYTES).spawn(dispatch)?;
    worker.join().map_err(|_| anyhow::anyhow!("translation thread panicked"))?
}

fn dispatch() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => run(&args),
        Some("msl") => msl(&args),
        Some("diff") => diff(&args),
        Some("refcmp") => refcmp(&args),
        Some("fuzz") => fuzz::run(&args),
        Some("fuzz-worker") => fuzz::worker(&args),
        Some("fuzz-dump") => fuzz::dump(&args),
        Some("translate") => translate_file(&args),
        Some("envelope") => envelope(&args),
        _ => {
            eprintln!("usage: s3-shaders run|msl|diff|refcmp|fuzz [options]");
            std::process::exit(2);
        }
    }
}
