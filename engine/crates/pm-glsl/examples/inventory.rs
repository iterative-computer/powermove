//! Read-only corpus probe. Reports aggregate counts only. Source, private
//! identifiers, shader diagnostics and pixel digests never enter this report.
//! cargo run -p pm-glsl --example inventory -- <corpus directory> ...
//! Each case (including Metal compilation and GPU completion) runs in a
//! disposable process with a wall deadline. This is a gate harness, not a
//! substitute for the production device-owner boundary.
use pm_glsl::{Failure, FailureStage};
use pm_gpu::{Gpu, Target};
use pm_render::effects::program::{Call, GpuProgram};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn step(stage: &str) {
    println!("{stage}");
    std::io::stdout().flush().unwrap();
}
fn fail(stage: FailureStage, e: impl std::fmt::Display) -> Failure {
    Failure::new(stage, e.to_string())
}
fn calls(value: &Value) -> Vec<(String, Call)> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let v: Vec<f32> = c["v"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_f64().unwrap_or(0.) as f32)
                .collect();
            let call = match c["fn"].as_str().unwrap() {
                "1i" => Call::I(v[0] as i32),
                "m3" => Call::M3(v.try_into().unwrap()),
                _ => Call::F(v),
            };
            (c["name"].as_str().unwrap().to_owned(), call)
        })
        .collect()
}

fn run_case(dir: &Path, case: &Value) -> Result<String, Failure> {
    let frag = case["frag"].as_str().unwrap();
    // Check each front end explicitly so the last emitted stage is exact,
    // including source limits versus post-glslang limits.
    pm_glsl::limits::check_source(frag).map_err(|e| fail(FailureStage::Limits, e))?;
    pm_glsl::compile::validate_es(pm_glsl::VERTEX, &pm_glsl::normalize::rename_reserved(frag))
        .map_err(|e| fail(FailureStage::EsValidate, e))?;
    step("validation");
    let normalized =
        pm_glsl::normalize::normalize(frag).map_err(|e| fail(FailureStage::Normalize, e.0))?;
    pm_glsl::compile::compile(&normalized.source, pm_glsl::compile::Stage::Fragment)
        .map_err(|e| fail(FailureStage::Glslang, e))?;
    step("glslang");
    let prepared = pm_glsl::prepare(frag, case["lineOffset"].as_u64().unwrap_or(0) as u32)?;
    step("rewrite");
    let program = prepared.finish()?;
    step("naga");
    if std::env::var_os("PM_GLSL_VERIFY_MSL").is_some() {
        for (stage, data) in [
            (naga::ShaderStage::Vertex, &program.vertex),
            (naga::ShaderStage::Fragment, &program.fragment),
        ] {
            let mut entries = Vec::new();
            if let Some(block) = &data.reflection.block {
                entries.push(pm_render::glsl::Entry::Buffer {
                    group: block.set,
                    binding: block.binding,
                });
            }
            for sampler in &data.reflection.samplers {
                entries.push(pm_render::glsl::Entry::Texture {
                    group: sampler.set,
                    binding: sampler.binding,
                });
                entries.push(pm_render::glsl::Entry::Sampler {
                    group: sampler.set,
                    binding: sampler.binding + pm_glsl::SAMPLER_BINDING_OFFSET,
                });
            }
            let slots = pm_render::glsl::slots(&entries);
            let original = pm_gpu::msl::write(
                &data.module,
                &pm_glsl::pipeline::naga_validate(&data.module).unwrap(),
                stage,
                &slots,
                pm_gpu::msl::MathMode::Fast,
            )
            .unwrap();
            let isolated = pm_render::glsl::write(&data.module, stage, &entries)
                .map_err(|e| fail(FailureStage::NagaValidate, e))?;
            if (pm_render::glsl::angle_calls(&original.0), original.1) != isolated {
                return Err(fail(
                    FailureStage::NagaValidate,
                    "MSL differs from the original writer",
                ));
            }
        }
    }
    let gpu = Gpu::new("S4 inventory").map_err(|e| fail(FailureStage::Metal, e))?;
    let mut compiled = GpuProgram::new(&gpu.device, program, "S4 inventory")?;
    // Force native compilation before calling this a draw failure.
    compiled.pipeline(&gpu.device, wgpu::TextureFormat::Rgba16Float)?;
    step("metal");
    let mut images = BTreeMap::new();
    for image in serde_json::from_slice::<Value>(&std::fs::read(dir.join("corpus.json")).unwrap())
        .unwrap()["images"]
        .as_array()
        .unwrap()
    {
        let (w, h) = (
            image["width"].as_u64().unwrap() as u32,
            image["height"].as_u64().unwrap() as u32,
        );
        let tex = Target::new(
            &gpu.device,
            "S4 input",
            w,
            h,
            wgpu::TextureFormat::Rgba16Float,
            pm_gpu::target::working_usage(),
        );
        gpu.queue.write_texture(
            tex.texture.as_image_copy(),
            &std::fs::read(dir.join(image["file"].as_str().unwrap())).unwrap(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 8),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        images.insert(image["name"].as_str().unwrap().to_owned(), tex);
    }
    let empty = Target::new(
        &gpu.device,
        "S4 empty",
        1,
        1,
        wgpu::TextureFormat::Rgba16Float,
        pm_gpu::target::working_usage(),
    );
    let rgba: Vec<u8> = [0u16, 0, 0, pm_gpu::readback::f32_to_half(1.)]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    gpu.queue.write_texture(
        empty.texture.as_image_copy(),
        &rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let mut digest = Sha256::new();
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    for (variant_index, variant) in case["variants"].as_array().unwrap().iter().enumerate() {
        let (w, h) = (
            variant["width"].as_u64().unwrap() as u32,
            variant["height"].as_u64().unwrap() as u32,
        );
        let mut passes: Vec<Target> = Vec::new();
        for pass in variant["passes"].as_array().unwrap() {
            let calls = calls(&pass["calls"]);
            let buffers: Vec<_> = [
                compiled.fragment_block.as_ref(),
                compiled.vertex_block.as_ref(),
            ]
            .iter()
            .map(|b| {
                b.map(|b| {
                    let bytes = GpuProgram::block_bytes(b, &calls);
                    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: None,
                        size: bytes.len() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    gpu.queue.write_buffer(&buffer, 0, &bytes);
                    buffer
                })
            })
            .collect();
            let mut entries = Vec::new();
            if let (Some(block), Some(buffer)) = (&compiled.fragment_block, &buffers[0]) {
                entries.push(wgpu::BindGroupEntry {
                    binding: block.binding,
                    resource: buffer.as_entire_binding(),
                });
            }
            let units = compiled.sampler_units(&calls);
            for ((_, binding), unit) in compiled.samplers.iter().zip(units) {
                let src = pass["units"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|u| u["unit"].as_u64() == Some(unit as u64))
                    .and_then(|u| u["src"].as_str());
                let texture = src
                    .and_then(|s| {
                        s.strip_prefix("pass:")
                            .and_then(|i| i.parse::<usize>().ok())
                            .and_then(|i| passes.get(i))
                            .or_else(|| images.get(s))
                    })
                    .unwrap_or(&empty);
                entries.push(wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: wgpu::BindingResource::TextureView(&texture.view),
                });
                entries.push(wgpu::BindGroupEntry {
                    binding: *binding + pm_glsl::SAMPLER_BINDING_OFFSET,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                });
            }
            let fragment = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &compiled.fragment_layout,
                entries: &entries,
            });
            let entries: Vec<_> = compiled
                .vertex_block
                .as_ref()
                .zip(buffers[1].as_ref())
                .map(|(b, buffer)| wgpu::BindGroupEntry {
                    binding: b.binding,
                    resource: buffer.as_entire_binding(),
                })
                .into_iter()
                .collect();
            let vertex = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &compiled.vertex_layout,
                entries: &entries,
            });
            let target = Target::new(
                &gpu.device,
                "S4 output",
                w,
                h,
                wgpu::TextureFormat::Rgba16Float,
                pm_gpu::target::working_usage(),
            );
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let mut draw = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                draw.set_pipeline(
                    compiled.pipeline(&gpu.device, wgpu::TextureFormat::Rgba16Float)?,
                );
                draw.set_bind_group(0, &fragment, &[]);
                draw.set_bind_group(1, &vertex, &[]);
                draw.draw(0..4, 0..1);
            }
            gpu.queue.submit([encoder.finish()]);
            passes.push(target);
        }
        if let Some(target) = passes.last() {
            let pixels = pm_gpu::readback::read_raw(&gpu.device, &gpu.queue, &target.texture)
                .map_err(|e| fail(FailureStage::Draw, e))?;
            if let Some(path) = std::env::var_os("PM_GLSL_INVENTORY_PIXELS") {
                let path = Path::new(&path);
                private_directory(path);
                std::fs::write(path.join(format!("variant-{variant_index}.f16")), &pixels).unwrap();
            }
            digest.update(pixels);
        }
    }
    if let Some(e) = pollster::block_on(scope.pop()) {
        return Err(fail(FailureStage::Draw, e));
    }
    step("draw");
    Ok(format!("{:x}", digest.finalize()))
}

fn private_directory(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    let path = path.canonicalize().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    assert!(
        !path.starts_with(repo),
        "private inventory artifacts must stay outside the repository"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--case") {
        let dir = Path::new(&args[1]);
        let corpus: Value =
            serde_json::from_slice(&std::fs::read(dir.join("corpus.json")).unwrap()).unwrap();
        let case = &corpus["cases"][args[2].parse::<usize>().unwrap()];
        match run_case(dir, case) {
            Ok(hash) => println!("digest {hash}"),
            Err(f) => {
                println!("failure {}", f.stage.name());
                eprintln!("{f}");
            }
        }
        return;
    }
    let mut rows = BTreeMap::<String, Value>::new();
    let mut results = Vec::new();
    for arg in args {
        let dir = Path::new(&arg);
        let corpus: Value =
            serde_json::from_slice(&std::fs::read(dir.join("corpus.json")).unwrap()).unwrap();
        for (index, case) in corpus["cases"].as_array().unwrap().iter().enumerate() {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--case", &arg, &index.to_string()])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let stdout = child.stdout.take().unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let reader = std::thread::spawn(move || {
                for line in std::io::BufReader::new(stdout)
                    .lines()
                    .map_while(Result::ok)
                {
                    let _ = tx.send(line);
                }
            });
            let start = Instant::now();
            let mut completed = Vec::new();
            let mut failure = None;
            let mut digest = None;
            loop {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(s) if s.starts_with("failure ") => failure = Some(s[8..].to_owned()),
                    Ok(s) if s.starts_with("digest ") => digest = Some(s[7..].to_owned()),
                    Ok(s) => completed.push(s),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    _ => {}
                }
                if start.elapsed() > Duration::from_secs(30) {
                    failure = Some("deadline".into());
                    let _ = child.kill();
                    break;
                }
            }
            let status = child.wait().unwrap();
            let _ = reader.join();
            if !status.success() && failure.is_none() {
                failure = Some("compiler-process".into());
            }
            let group = case["group"].as_str().unwrap();
            let row=rows.entry(group.into()).or_insert_with(||json!({"sources":0,"variants":0,"validation":0,"glslang":0,"rewrite":0,"naga":0,"metal":0,"draw":0,"failures":{}}));
            row["sources"] = json!(row["sources"].as_u64().unwrap() + 1);
            row["variants"] = json!(
                row["variants"].as_u64().unwrap()
                    + case["variants"].as_array().unwrap().len() as u64
            );
            for stage in &completed {
                row[stage] = json!(row[stage].as_u64().unwrap_or(0) + 1);
            }
            if let Some(f) = &failure {
                let n = row["failures"][f].as_u64().unwrap_or(0);
                row["failures"][f] = json!(n + 1);
            }
            // Indices and digests are for an optional private comparison file;
            // aggregate stdout remains free of identifiers and shader text.
            results.push(json!({"corpus":arg,"index":index,"completed":completed,"failure":failure,"digest":digest}));
        }
    }
    if let Some(path) = std::env::var_os("PM_GLSL_INVENTORY_PRIVATE_RESULTS") {
        let path = std::path::PathBuf::from(path);
        private_directory(path.parent().unwrap());
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    println!("{}", serde_json::to_string_pretty(&rows).unwrap());
}
