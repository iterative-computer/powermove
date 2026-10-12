//! Times the GLSL pipeline on every distinct three.js program in the reference traces:
//! s7_glsl::compile (normalize, glslang, SPIR-V passes, naga front end, validation) and
//! naga's MSL back end (what wgpu-hal Metal hands to the Metal compiler).
//!
//!   s7-compile-bench <ref-root>
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).context("ref root")?);
    let mut programs: BTreeMap<(String, String), String> = BTreeMap::new();
    for entry in std::fs::read_dir(&root)? {
        let dir = entry?.path();
        if !dir.join("trace.json").exists() {
            continue;
        }
        let calls: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(dir.join("trace.json"))?)?;
        let mut shaders: BTreeMap<u64, String> = BTreeMap::new();
        let mut attached: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for c in &calls {
            match c[0].as_str() {
                Some("shaderSource") => {
                    shaders.insert(c[1]["o"].as_u64().unwrap(), c[2].as_str().unwrap().to_string());
                }
                Some("attachShader") => attached.entry(c[1]["o"].as_u64().unwrap()).or_default().push(c[2]["o"].as_u64().unwrap()),
                _ => {}
            }
        }
        for list in attached.values() {
            let srcs: Vec<&String> = list.iter().filter_map(|s| shaders.get(s)).collect();
            let fs = srcs.iter().find(|s| s.contains("pc_fragColor")).map(|s| s.to_string());
            let vs = srcs.iter().find(|s| !s.contains("pc_fragColor")).map(|s| s.to_string());
            if let (Some(vs), Some(fs)) = (vs, fs) {
                let name = fs.lines().find_map(|l| l.strip_prefix("#define SHADER_TYPE ")).unwrap_or("?").to_string();
                programs.entry((vs, fs)).or_insert(name);
            }
        }
    }
    let mut compile_ms = Vec::new();
    let mut msl_ms = Vec::new();
    let mut hand_ms = Vec::new();
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = 0;
    for ((vs, fs), name) in &programs {
        *by_type.entry(name.clone()).or_default() += 1;
        let t0 = Instant::now();
        let p = match s7_glsl::compile(vs, fs) {
            Ok(p) => p,
            Err(e) => {
                failures += 1;
                eprintln!("{name}: {e:#}");
                continue;
            }
        };
        compile_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        let t1 = Instant::now();
        for (m, info, stage) in [(&p.vertex, &p.vertex_info, naga::ShaderStage::Vertex), (&p.fragment, &p.fragment_info, naga::ShaderStage::Fragment)] {
            let ep = m.entry_points.iter().find(|e| e.stage == stage).unwrap();
            let pipeline = naga::back::msl::PipelineOptions { entry_point: Some((stage, ep.name.clone())), ..Default::default() };
            naga::back::msl::write_string(m, info, &naga::back::msl::Options { lang_version: (2, 4), fake_missing_bindings: true, ..Default::default() }, &pipeline).map_err(|e| anyhow::anyhow!("{name}: msl {e:?}"))?;
        }
        msl_ms.push(t1.elapsed().as_secs_f64() * 1000.0);
        let t2 = Instant::now();
        if let Some(r) = s7_replay::hand::override_standard(vs, fs) {
            r?;
            hand_ms.push(t2.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let stats = |v: &mut Vec<f64>| -> String {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if v.is_empty() {
            return "-".into();
        }
        format!("median {:.1} ms, p95 {:.1} ms, max {:.1} ms, total {:.0} ms", v[v.len() / 2], v[((v.len() - 1) as f64 * 0.95) as usize], v[v.len() - 1], v.iter().sum::<f64>())
    };
    println!("{} distinct programs ({failures} failed): {by_type:?}", programs.len());
    println!("s7_glsl::compile: {}", stats(&mut compile_ms));
    println!("naga MSL back end (both stages): {}", stats(&mut msl_ms));
    println!("hand-written WGSL variants ({}): {}", hand_ms.len(), stats(&mut hand_ms));
    Ok(())
}
