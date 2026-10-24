//! Deterministic 8/16-bit work comparison on the exact app-parity slice.
//! Build/run after merging pm-perfbench. No Electron is started here.
//! Usage: cargo run -p pm-perfbench --example color_depth -- /external/capture [--composition]
//! The optional mode measures the production GL-row snapshot endpoint; the
//! default retains the generic top-down frame/export endpoint comparison.
#[global_allocator]
static ALLOC: pm_perf::CountingAlloc = pm_perf::CountingAlloc;

#[path = "../../pm-render/tests/support/main_thread_fonts.rs"]
mod text;

#[cfg(target_os = "macos")]
mod mac {
    use pm_perf::{Counter, Counters};
    use pm_protocol::{
        color::Precision,
        common::{Size, TICKS_PER_SECOND, Ticks},
    };
    use pm_render::{MapResolver, RenderRequest, Renderer, SnapshotRows};
    use serde_json::{Value, json};
    use std::{collections::BTreeMap, path::PathBuf, time::Instant};

    fn counters(c: Counters) -> Value {
        serde_json::from_str(&c.to_json()).unwrap()
    }

    pub fn run() {
        assert!(pm_perf::ENABLED, "work counters must be enabled");
        pm_perf::count_globally(true);
        let dir = PathBuf::from(
            std::env::args_os()
                .nth(1)
                .expect("external app-parity capture"),
        );
        let composition = std::env::args_os().any(|arg| arg == "--composition");
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap();
        assert!(!dir.canonicalize().unwrap().starts_with(repo));
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let mut measured: BTreeMap<String, BTreeMap<u64, (Counters, Counters)>> = BTreeMap::new();
        let mut rows = Vec::new();
        for sample in manifest["samples"].as_array().unwrap() {
            let depth = sample["depth"].as_u64().unwrap();
            if depth == 32 {
                continue;
            }
            assert!(matches!(depth, 8 | 16));
            let id = sample["id"].as_str().unwrap();
            assert!(id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
            let kind = id.rsplit_once('-').expect("depth suffix").0.to_owned();
            let value: Value =
                serde_json::from_slice(&std::fs::read(dir.join(format!("{id}.json"))).unwrap())
                    .unwrap();
            assert_eq!(value["colorDepth"], depth);
            let doc = pm_model::Document::new(
                Some(pm_model::json::parse(&value.to_string()).unwrap()),
                1,
            );
            let comp = value["compId"].as_str().unwrap();
            let mut renderer = Renderer::headless().unwrap();
            assert!(
                renderer
                    .set_definitions(&pm_render::all_builtin_definitions())
                    .unwrap()
                    .is_empty()
            );
            renderer.set_text_fonts(Some(Box::new(crate::text::MainThreadFonts(
                pm_eval::text_measure::CompatMeasure::new(
                    pm_text::registry::CoreTextRegistry::new(),
                ),
                None,
            ))));
            let request = RenderRequest::sized(
                Size::new(
                    sample["width"].as_u64().unwrap() as u32,
                    sample["height"].as_u64().unwrap() as u32,
                ),
                Precision::Float32,
            );
            let at = Ticks::new(
                (sample["time"].as_f64().unwrap() * TICKS_PER_SECOND as f64).round() as i64,
            )
            .unwrap();
            // Allocate the caller-owned output inside the cold measurement and
            // retain it across warm frames, just as the snapshot presenter does.
            let mut snapshot_target: Option<pm_gpu::Target> = None;
            let mut draw = |renderer: &mut Renderer| {
                let (format, bytes, frame) = if composition {
                    let target = snapshot_target.get_or_insert_with(|| {
                        pm_gpu::Target::new(
                            renderer.device(),
                            "color depth snapshot counter",
                            sample["width"].as_u64().unwrap() as u32,
                            sample["height"].as_u64().unwrap() as u32,
                            renderer.device_target_format(&doc, &request),
                            pm_gpu::target::working_usage(),
                        )
                    });
                    let (rendered, rows) = renderer
                        .render_snapshot_into(
                            &doc,
                            comp,
                            at,
                            &request,
                            &MapResolver::default(),
                            target,
                        )
                        .unwrap();
                    assert!(
                        rendered.failures.is_empty(),
                        "{id}: {:?}",
                        rendered.failures
                    );
                    assert_eq!(rows, SnapshotRows::Gl, "the Standard composition endpoint");
                    (target.format(), target.bytes(), None)
                } else {
                    let frame = renderer
                        .render(&doc, comp, at, &request, &MapResolver::default())
                        .unwrap();
                    assert!(frame.failures.is_empty(), "{id}: {:?}", frame.failures);
                    (frame.color.format(), frame.color.bytes(), Some(frame))
                };
                renderer
                    .device()
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                if let Some(frame) = frame {
                    renderer.recycle(frame);
                }
                (format, bytes)
            };
            let before = pm_perf::snapshot_global();
            let cold = draw(&mut renderer);
            let cold_counts = pm_perf::snapshot_global().since(&before);
            assert!(
                cold_counts.get(Counter::UploadBytes) > 0,
                "{id}: missing upload instrumentation"
            );
            assert!(
                cold_counts.get(Counter::TextureAllocBytes) > 0,
                "{id}: missing allocation instrumentation"
            );
            assert!(
                cold_counts.get(Counter::RenderPasses) > 0,
                "{id}: missing pass instrumentation"
            );
            assert_eq!(
                cold.0,
                if depth == 8 {
                    wgpu::TextureFormat::Rgba8Unorm
                } else {
                    wgpu::TextureFormat::Rgba16Float
                },
                "{id}: actual working format"
            );
            let working_format = format!("{:?}", cold.0);
            let output_bytes = cold.1;
            // Exports return the output target to the renderer after readback.
            // Reuse it on every subsequent frame, as the production path does.
            for _ in 0..2 {
                draw(&mut renderer);
            }
            let mut timings = Vec::new();
            let before = pm_perf::snapshot_global();
            for _ in 0..12 {
                let start = Instant::now();
                draw(&mut renderer);
                timings.push(start.elapsed().as_secs_f64() * 1000.);
            }
            let warm_counts = pm_perf::snapshot_global().since(&before);
            drop(draw);
            // Readback and byte verification are outside both counter spans.
            // The snapshot returns GL rows; reverse them exactly as the mapped
            // production encoder does before comparing today's top-down pixels.
            let parity_differences = snapshot_target.as_ref().map(|target| {
                let pixels = pm_gpu::readback::read_f32(
                    renderer.device(),
                    renderer.queue(),
                    &target.texture,
                )
                .unwrap();
                let reference = std::fs::read(dir.join(format!("{id}.rgba32f"))).unwrap();
                assert_eq!(reference.len(), pixels.len() * 4);
                let width = sample["width"].as_u64().unwrap() as usize;
                let height = sample["height"].as_u64().unwrap() as usize;
                let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
                let mut differences = 0;
                for y in 0..height {
                    for x in 0..width * 4 {
                        let expected = f32::from_le_bytes(
                            reference[(y * width * 4 + x) * 4..][..4]
                                .try_into()
                                .unwrap(),
                        );
                        differences += usize::from(
                            byte(pixels[(height - 1 - y) * width * 4 + x]) != byte(expected),
                        );
                    }
                }
                assert_eq!(differences, 0, "{id}: snapshot endpoint byte parity");
                differences
            });
            timings.sort_by(f64::total_cmp);
            println!(
                "{id}: cold upload={} texture={} passes={}, warm upload={} texture={} passes={} (12 frames)",
                cold_counts.get(Counter::UploadBytes),
                cold_counts.get(Counter::TextureAllocBytes),
                cold_counts.get(Counter::RenderPasses),
                warm_counts.get(Counter::UploadBytes),
                warm_counts.get(Counter::TextureAllocBytes),
                warm_counts.get(Counter::RenderPasses)
            );
            measured
                .entry(kind)
                .or_default()
                .insert(depth, (cold_counts, warm_counts));
            rows.push(json!({"id":id,"depth":depth,"workingFormat":working_format,"cold":counters(cold_counts),"warm12":counters(warm_counts),"p50Ms":timings[6],"targetPoolBytes":renderer.target_pool_bytes(),"frameReuseBytes":renderer.frame_reuse_bytes(),"outputBytes":output_bytes,"byteParityDifferences":parity_differences}));
        }
        assert_eq!(
            measured.len(),
            7,
            "the complete seven-project app-parity slice"
        );
        let watched = [
            Counter::UploadBytes,
            Counter::TextureAllocBytes,
            Counter::RenderPasses,
            Counter::ComputePasses,
        ];
        let mut regressions = Vec::new();
        let mut cold_texture = [0u64; 2];
        for (kind, depths) in measured {
            cold_texture[0] += depths[&8].0.get(Counter::TextureAllocBytes);
            cold_texture[1] += depths[&16].0.get(Counter::TextureAllocBytes);
            let (a, b) = (&depths[&8], &depths[&16]);
            for (phase, a, b) in [("cold", a.0, b.0), ("warm12", a.1, b.1)] {
                for counter in watched {
                    if a.get(counter) > b.get(counter) {
                        regressions.push(format!(
                            "{kind}/{phase}/{}: {} > {}",
                            counter.name(),
                            a.get(counter),
                            b.get(counter)
                        ));
                    }
                }
            }
        }
        assert!(
            cold_texture[0] < cold_texture[1],
            "8-bit allocation bytes must improve: {cold_texture:?}"
        );
        std::fs::write(
            dir.join(if composition { "composition-counters.json" } else { "counters.json" }),
            serde_json::to_vec_pretty(
                &json!({"endpoint":if composition {"render_snapshot_into GL rows; CPU row reversal/readback excluded"} else {"render top-down frame; GPU row reversal included"},"globalCounters":true,"coldTextureBytes8":cold_texture[0],"coldTextureBytes16":cold_texture[1],"warmFrames":12,"regressions":regressions,"samples":rows}),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            regressions.is_empty(),
            "8-bit work regressed: {regressions:?}"
        );
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    mac::run();
    #[cfg(not(target_os = "macos"))]
    panic!("This qualification fixture requires the macOS app/font reference");
}
